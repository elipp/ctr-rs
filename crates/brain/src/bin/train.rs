//! Trains a policy from scratch by PPO on the race tracks.
//!
//! `cargo run --release -p brain --bin train -- [key=value ...]`, the keys being:
//!
//! - `envs` (64): karts driving at once, each in a race of its own.
//! - `iters` (1000): rollouts and updates.
//! - `hidden` (256): the width of the actor's and the critic's two layers.
//! - `tracks` (all): the `LevelId`s to train on, as `0,3,7`.
//! - `out` (`target/brain`): where the checkpoints and `log.csv` go.
//! - `load`: a checkpoint (`.safetensors`) to go on from.
//! - `save_every` (25): iterations between checkpoints.
//! - `seed` (1).
//! - `reserves` (0.01): the reward for each millisecond of reserves gained.
//! - `reserves_fade` (none): `start,end` iterations over which that reward fades to 0, as
//!   `500,1500` (full until 500, none from 1500).
//! - `gamma` 0.995 is worth it for drifts (their boost pays off seconds after the hop).
//! - the [`brain::ppo::Config`] ones: `steps`, `gamma`, `lambda`, `clip`, `epochs`, `minibatch`,
//!   `lr`, `entropy_coef`, `target_kl`.
//!
//! The disc image is `CTR_BIN`, or `assets/ctr-u.bin`.

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;

use brain::env::{Done, Env, Rules};
use brain::policy::Policy;
use brain::ppo::{self, Config, Trainer};
use candle_nn::VarMap;

type Error = Box<dyn std::error::Error>;

fn main() -> Result<(), Error> {
    let mut args: HashMap<String, String> = HashMap::new();
    for arg in std::env::args().skip(1) {
        let (k, v) = arg
            .split_once('=')
            .ok_or_else(|| format!("expected key=value, got {arg:?}"))?;
        args.insert(k.to_string(), v.to_string());
    }
    let mut take = |k: &str| args.remove(k);
    fn parse<T: std::str::FromStr>(v: Option<String>, default: T) -> Result<T, Error>
    where
        T::Err: std::fmt::Debug,
    {
        match v {
            Some(v) => v.parse().map_err(|e| format!("{v:?}: {e:?}").into()),
            None => Ok(default),
        }
    }

    let d = Config::default();
    let config = Config {
        steps: parse(take("steps"), d.steps)?,
        gamma: parse(take("gamma"), d.gamma)?,
        lambda: parse(take("lambda"), d.lambda)?,
        clip: parse(take("clip"), d.clip)?,
        epochs: parse(take("epochs"), d.epochs)?,
        minibatch: parse(take("minibatch"), d.minibatch)?,
        lr: parse(take("lr"), d.lr)?,
        entropy_coef: parse(take("entropy_coef"), d.entropy_coef)?,
        target_kl: parse(take("target_kl"), d.target_kl)?,
        ..d
    };
    let envs: usize = parse(take("envs"), 64)?;
    let iters: usize = parse(take("iters"), 1000)?;
    let hidden: usize = parse(take("hidden"), 256)?;
    let save_every: usize = parse(take("save_every"), 25)?;
    let seed: u64 = parse(take("seed"), 1)?;
    let out = PathBuf::from(take("out").unwrap_or_else(|| "target/brain".into()));
    let load = take("load").map(PathBuf::from);
    let reserves: f32 = parse(take("reserves"), race::nn::Weights::DEFAULT.reserves)?;
    let fade: Option<(usize, usize)> = match take("reserves_fade") {
        Some(v) => {
            let (a, b) = v
                .split_once(',')
                .ok_or_else(|| format!("reserves_fade: expected start,end, got {v:?}"))?;
            Some((a.trim().parse()?, b.trim().parse()?))
        }
        None => None,
    };
    let ids: Vec<u32> = match take("tracks") {
        Some(list) => list
            .split(',')
            .map(|s| s.trim().parse())
            .collect::<Result<_, _>>()?,
        None => brain::tracks::RACE_TRACKS.collect(),
    };
    if let Some(k) = args.keys().next() {
        return Err(format!("unknown key {k:?}").into());
    }

    let disc = brain::tracks::disc_path();
    let tracks = brain::tracks::load(&disc, ids)
        .map_err(|e| format!("reading the tracks off {}: {e}", disc.display()))?;
    println!("{} tracks, {envs} envs, {config:?}", tracks.len());

    let device = ppo::device();
    let mut vars = VarMap::new();
    let policy = Policy::new(&vars, hidden, &device)?;
    if let Some(path) = &load {
        vars.load(path)?;
        println!("going on from {}", path.display());
    }
    let rules = Rules::default();
    let envs: Vec<Env> = (0..envs)
        .map(|i| Env::new(&tracks, rules, seed.wrapping_mul(1_000_003).wrapping_add(i as u64)))
        .collect();
    let mut trainer = Trainer::new(config, vars, policy, envs, seed)?;

    std::fs::create_dir_all(&out)?;
    let mut log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(out.join("log.csv"))?;
    writeln!(
        log,
        "iter,episodes,ep_reward,ep_ticks,units_per_tick,grabs_per_min,boosts_per_min,hops_per_min,r1_pct,drift_pct,cut_time,cut_grabs,\
         cut_stuck,mean_reward,policy_loss,value_loss,entropy,approx_kl,clip_frac,updates,\
         rollout_secs,update_secs"
    )?;

    for iter in 1..=iters {
        let w = match fade {
            Some((_, end)) if iter >= end => 0.0,
            Some((start, end)) if iter > start => {
                reserves * (end - iter) as f32 / (end - start) as f32
            }
            _ => reserves,
        };
        trainer.set_reserves_weight(w);
        let s = trainer.iterate()?;
        let n = s.episodes.len().max(1) as f32;
        let sum = |f: &dyn Fn(&brain::env::Episode) -> f32| {
            s.episodes.iter().map(|(_, e)| f(e)).sum::<f32>()
        };
        let ticks = sum(&|e| e.ticks as f32);
        let ep_reward = sum(&|e| e.reward) / n;
        let units_per_tick = sum(&|e| e.progress as f32) / ticks.max(1.0);
        let minutes = ticks.max(1.0) / (30.0 * 60.0);
        let grabs_per_min = sum(&|e| e.grabs as f32) / minutes;
        let boosts_per_min = sum(&|e| e.boosts as f32) / minutes;
        let hops_per_min = sum(&|e| e.hops as f32) / minutes;
        let drifting = sum(&|e| e.drifting as f32) / ticks.max(1.0) * 100.0;
        let r1 = sum(&|e| e.r1 as f32) / ticks.max(1.0) * 100.0;
        let cuts = |d: Done| s.episodes.iter().filter(|(c, _)| *c == d).count();
        println!(
            "{iter:5} | eps {:4} reward {ep_reward:7.1} | {units_per_tick:5.1} u/tick, \
             {grabs_per_min:4.2} grabs/min, {boosts_per_min:5.1} boosts/min, {hops_per_min:4.0} hops/min, R1 {r1:3.0}% drift {drifting:3.0}% | cut T/G/S {}/{}/{} | ent {:.2} kl {:.4} clip {:.2} \
             vloss {:.3} | {} upd, {:.1}s + {:.1}s",
            s.episodes.len(),
            cuts(Done::Time),
            cuts(Done::Grabs),
            cuts(Done::Stuck),
            s.entropy,
            s.approx_kl,
            s.clip_frac,
            s.value_loss,
            s.updates,
            s.rollout_secs,
            s.update_secs,
        );
        writeln!(
            log,
            "{iter},{},{ep_reward},{},{units_per_tick},{grabs_per_min},{boosts_per_min},{hops_per_min},{r1},{drifting},{},{},{},{},{},{},{},{},{},{},{},{}",
            s.episodes.len(),
            ticks / n,
            cuts(Done::Time),
            cuts(Done::Grabs),
            cuts(Done::Stuck),
            s.mean_reward,
            s.policy_loss,
            s.value_loss,
            s.entropy,
            s.approx_kl,
            s.clip_frac,
            s.updates,
            s.rollout_secs,
            s.update_secs,
        )?;
        if iter % save_every == 0 || iter == iters {
            let path = out.join(format!("policy-{iter:05}.safetensors"));
            trainer.vars.save(&path)?;
            trainer.vars.save(out.join("policy.safetensors"))?;
            println!("saved {}", path.display());
        }
    }
    Ok(())
}
