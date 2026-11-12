//! Trains a policy from scratch by PPO on the race tracks.
//!
//! `cargo run --release -p brain --bin train -- [key=value ...]`, the keys being:
//!
//! - `envs` (64): karts driving at once, each in a race of its own.
//! - `iters` (1000): rollouts and updates.
//! - `hidden` (256): the width of the actor's and the critic's two layers.
//! - `tracks` (all): the `LevelId`s to train on, as `0,3,7`.
//! - `paths` (all): the bot paths to drive on some tracks, as `7:middle,11:0` (`middle` the
//!   one between the others, `brain::tracks::middle_path`).
//! - `weights` (none): how often the episodes are on some of them against the rest (1), as
//!   `11:3,12:3,5:2`.
//! - `out` (`target/brain`): where the checkpoints and `log.csv` go.
//! - `load`: a checkpoint (`.safetensors`) to go on from.
//! - `save_every` (25): iterations between checkpoints.
//! - `seed` (1).
//! - `reserves` (0.01): the reward for each millisecond of reserves gained.
//! - `drift_boost` (15): the reward for each drift boost fired ...
//! - `boost_growth` (1): ... times this for each boost of the drift before it (2: 1, 2, 4 times).
//! - `reserves_fade` (none): `start,end` iterations over which the reserves' reward fades to 0,
//!   as `500,1500` (full until 500, none from 1500).
//! - `boost_fade` (none): the same for the drift boosts' reward (the flat `drift_boost`, not
//!   `boost_timed`).
//! - `boost_timed` (10), `boost_timing_power` (2): the reward for each drift boost fired, on top
//!   of `drift_boost` and grown by `boost_growth` as it is, times how late in the meter's window
//!   the tap came (0 to 1: the reserves it gained, of a full bar's) to this power. With
//!   `boost_fade`, what is left of the boosts' reward once the flat one has gone.
//! - `air_heading` (0.2), `steer_change` (0.05): the costs of turning away from the landing's heading
//!   in a jump from a ramp lip (by the tick), and of moving the stick ([`race::nn::Weights`]).
//! - `gap_cleared` (20): the reward for a jump from a ramp lip that made it to the ground.
//! - `air_steer` (0.3): the cost of each tick in a jump from a ramp lip, times how far the stick
//!   is from the centre (0 to 1).
//! - `gap_apex` (0.15): the reward for each level unit a jump from a ramp lip rises above its
//!   takeoff, made or not, times how well it took off toward the landing (0 to 1); paid at the
//!   top of the flight. (A hop alone rises some 45.)
//! - `wall` (0.3): the cost of each tick against a wall.
//! - `reserves_lost` (0.01): the cost of each millisecond of reserves thrown away (Square, a
//!   wall's scrub, a crash), against the reward of each one gained.
//! - `brake` (0.5): the cost of each tick of reversing going forward faster than a quarter of
//!   the top speed (braking; backing out at low speed is free).
//! - `stall` (0.5): the cost of each tick getting nowhere, after the first second of it.
//! - `jump_starts` (0.25), `run_up` (1200): the share of the episodes that start this far
//!   (level units) before a jump ([`brain::env::Rules`]).
//! - `grab_starts` (0.15): the share of the episodes that start `run_up` before a spot where
//!   the kart was grabbed lately.
//! - `lip_starts` (0.2): the share of the episodes that start `run_up` before a ramp lip, picked
//!   by how often the flights from it failed lately, over all the envs.
//! - `lip_memory` (0.97): what is left of those lips' tally after an iteration.
//! - `lips` (none): a `lips.csv` of an earlier run, whose last checkpoint's rows the tally starts
//!   from (as `lips=target/<prev>/lips.csv`); otherwise the lip starts are jump starts until the
//!   first flights.
//! - `turned_starts` (0.15): the share of the episodes that start turned by a random angle.
//! - `stuck_penalty` (30): the cost of getting nowhere ([`brain::env::Rules`]).
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
    let mut rules = Rules::default();
    let drift_boost: f32 = parse(take("drift_boost"), rules.weights.drift_boost)?;
    rules.weights.boost_growth = parse(take("boost_growth"), rules.weights.boost_growth)?;
    rules.weights.boost_timed = parse(take("boost_timed"), rules.weights.boost_timed)?;
    rules.weights.boost_timing_power =
        parse(take("boost_timing_power"), rules.weights.boost_timing_power)?;
    rules.weights.air_heading = parse(take("air_heading"), rules.weights.air_heading)?;
    rules.weights.steer_change = parse(take("steer_change"), rules.weights.steer_change)?;
    rules.weights.gap_cleared = parse(take("gap_cleared"), rules.weights.gap_cleared)?;
    rules.weights.air_steer = parse(take("air_steer"), rules.weights.air_steer)?;
    rules.weights.gap_apex = parse(take("gap_apex"), rules.weights.gap_apex)?;
    rules.weights.wall = parse(take("wall"), rules.weights.wall)?;
    rules.weights.stall = parse(take("stall"), rules.weights.stall)?;
    rules.weights.brake = parse(take("brake"), rules.weights.brake)?;
    rules.weights.reserves_lost = parse(take("reserves_lost"), rules.weights.reserves_lost)?;
    rules.stuck_penalty = parse(take("stuck_penalty"), rules.stuck_penalty)?;
    rules.jump_starts = parse(take("jump_starts"), rules.jump_starts)?;
    rules.run_up = parse(take("run_up"), rules.run_up)?;
    rules.turned_starts = parse(take("turned_starts"), rules.turned_starts)?;
    rules.grab_starts = parse(take("grab_starts"), rules.grab_starts)?;
    rules.lip_starts = parse(take("lip_starts"), rules.lip_starts)?;
    let lip_memory: f32 = parse(take("lip_memory"), 0.97)?;
    let seed_lips = match take("lips") {
        Some(file) => Some(read_lips(
            &std::fs::read_to_string(&file).map_err(|e| format!("{file}: {e}"))?,
        )?),
        None => None,
    };
    let mut fade = |key: &str| -> Result<Option<(usize, usize)>, Error> {
        match take(key) {
            Some(v) => {
                let (a, b) = v
                    .split_once(',')
                    .ok_or_else(|| format!("{key}: expected start,end, got {v:?}"))?;
                Ok(Some((a.trim().parse()?, b.trim().parse()?)))
            }
            None => Ok(None),
        }
    };
    let reserves_fade = fade("reserves_fade")?;
    let boost_fade = fade("boost_fade")?;
    let paths = take("paths");
    let weights: Vec<(u32, f32)> = match take("weights") {
        Some(list) => list
            .split(',')
            .map(|s| {
                let (id, w) = s
                    .split_once(':')
                    .ok_or_else(|| format!("weights: expected id:weight, got {s:?}"))?;
                Ok::<_, Error>((id.trim().parse()?, w.trim().parse()?))
            })
            .collect::<Result<_, _>>()?,
        None => Vec::new(),
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
    let mut tracks = brain::tracks::load(&disc, ids)
        .map_err(|e| format!("reading the tracks off {}: {e}", disc.display()))?;
    if let Some(list) = &paths {
        brain::tracks::keep_paths(&mut tracks, list)?;
    }
    for &(id, w) in &weights {
        match tracks.iter_mut().find(|t| t.id.0 == id) {
            Some(track) => track.weight = w,
            None => return Err(format!("weights: track {id} is not among the ones trained on").into()),
        }
    }
    println!("{} tracks, {envs} envs, {config:?}, {rules:?}", tracks.len());
    // The launches the jump starts are before (`brain::tracks::Track::jumps`), as path:frame, a
    // `*` for a gap's (its flight is held to the way to the landing, `race::nn::gap_heading`).
    for track in &tracks {
        let jumps: Vec<String> = track
            .jumps
            .iter()
            .map(|&(p, f)| {
                let gap = race::nn::gap_heading(&track.level.nav_paths[p].frames, f).is_some();
                format!("{p}:{f}{}", if gap { "*" } else { "" })
            })
            .collect();
        let middle = brain::tracks::middle_path(track).map_or("-".into(), |p| p.to_string());
        println!(
            "  track {:2}: weight {}, paths {:?} (middle {middle}), {} jumps {}",
            track.id.0,
            track.weight,
            track.paths,
            jumps.len(),
            jumps.join(" ")
        );
    }

    let device = ppo::device();
    let mut vars = VarMap::new();
    let policy = Policy::new(&vars, hidden, &device)?;
    if let Some(path) = &load {
        vars.load(path)?;
        println!("going on from {}", path.display());
    }
    let envs: Vec<Env> = (0..envs)
        .map(|i| Env::new(&tracks, rules, seed.wrapping_mul(1_000_003).wrapping_add(i as u64)))
        .collect();
    let mut trainer = Trainer::new(config, vars, policy, envs, seed)?;
    trainer.lip_memory = lip_memory;
    if let Some((iter, lips)) = &seed_lips {
        trainer.add_lips(lips);
        println!("lip starts from {} lips of iteration {iter}", lips.len());
    }

    std::fs::create_dir_all(&out)?;
    let mut log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(out.join("log.csv"))?;
    writeln!(
        log,
        "iter,episodes,ep_reward,ep_ticks,units_per_tick,grabs_per_min,taken_back_per_grab,boosts_per_min,reserves_lost_per_min,hops_per_min,r1_pct,drift_pct,wall_pct,brake_pct,air_pct,askew_pct,drifts_per_min,drift_len,early_per_drift,boosts_per_drift,outs_per_drift,air_misalign,steer_change,cut_time,cut_grabs,\
         cut_stuck,mean_reward,policy_loss,value_loss,entropy,approx_kl,clip_frac,updates,\
         rollout_secs,update_secs,gaps_made,gap_takeoffs,gap_hops,gaps_short,gaps_fell,speed_made,speed_short,speed_fell,air_steer,gap_apex,boost_timing"
    )?;

    // The flights from each ramp lip since the last checkpoint, and the file of the same.
    let mut lips: std::collections::BTreeMap<brain::env::LipKey, brain::env::LipStats> =
        Default::default();
    let lips_path = out.join("lips.csv");
    if !lips_path.exists() {
        std::fs::write(&lips_path, "iter,level,path,lip,takeoffs,hopped,made,short,fell\n")?;
    }

    for iter in 1..=iters {
        // How much of a reward is left at this iteration, of its fade.
        let left = |fade: Option<(usize, usize)>| match fade {
            Some((_, end)) if iter >= end => 0.0,
            Some((start, end)) if iter > start => (end - iter) as f32 / (end - start) as f32,
            _ => 1.0,
        };
        trainer.set_bonuses(reserves * left(reserves_fade), drift_boost * left(boost_fade));
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
        // Per tick in the air, and bins per choice.
        // Per jump from a ramp lip: the turn away is paid once, so over the ticks it is tiny.
        let air_misalign = sum(&|e| e.air_misalign) / sum(&|e| e.gap_flights as f32).max(1.0);
        // The stick's distance from the centre, per tick of a flight from a ramp lip.
        let air_steer = sum(&|e| e.air_steer) / sum(&|e| e.air_ticks as f32).max(1.0);
        let steer_change = sum(&|e| e.steer_change as f32) / sum(&|e| e.choices as f32).max(1.0);
        // Where the drifts go: their length, and per drift the tries too early, the boosts, and
        // the meters that ran out.
        let drifts = sum(&|e| e.drifts as f32).max(1.0);
        let drift_len = sum(&|e| e.drifting as f32) / drifts;
        let early = sum(&|e| e.early_taps as f32) / drifts;
        let in_time = sum(&|e| e.boosts as f32) / drifts;
        // How late in the meter's window the boosts were tapped, on average (1 the last moment).
        let boost_timing = sum(&|e| e.boost_timing) / sum(&|e| e.boosts as f32).max(1.0);
        let outs = sum(&|e| e.meter_outs as f32) / drifts;
        let drifts_per_min = sum(&|e| e.drifts as f32) / minutes;
        // The share of the drifts that ended each way, in percent.
        let ends: Vec<f32> = (0..brain::env::DRIFT_ENDS.len())
            .map(|i| sum(&|e| e.drift_ends[i] as f32) / drifts * 100.0)
            .collect();
        let ends_text: Vec<String> = brain::env::DRIFT_ENDS
            .iter()
            .zip(&ends)
            .map(|(name, pct)| format!("{name} {pct:.0}%"))
            .collect();
        let wall = sum(&|e| e.wall as f32) / ticks.max(1.0) * 100.0;
        let braking = sum(&|e| e.braking as f32) / ticks.max(1.0) * 100.0;
        let switches = sum(&|e| e.switches as f32) / minutes;
        // Seconds of boost thrown away a minute.
        let lost = sum(&|e| e.reserves_lost as f32) / 1000.0 / minutes;
        // What a grab took back of the progress made in the air before it, on average.
        let taken_back = sum(&|e| e.taken_back as f32) / sum(&|e| e.grabs as f32).max(1.0);
        // Long flights: their share of the ticks, and the share of those more than 45° off.
        let air_ticks = sum(&|e| e.air_ticks as f32);
        let air_pct = air_ticks / ticks.max(1.0) * 100.0;
        // Takeoffs from a ramp lip, and the share of them that made it.
        let gap_flights = sum(&|e| e.gap_flights as f32);
        let gaps_cleared = sum(&|e| e.gaps_cleared as f32);
        let gap_hops = sum(&|e| e.gap_hops as f32);
        let gaps_short = sum(&|e| e.gaps_short as f32);
        let gaps_fell = sum(&|e| e.gaps_fell as f32);
        // How high the flights from a ramp lip rose (level units, by their aim), on average.
        let gap_apex = sum(&|e| e.gap_apex) / gap_flights.max(1.0);
        // The takeoff speed (share of the top speed, in percent) of each kind of ending.
        let speed_of = |total: f32, n: f32| total / n.max(1.0) * 100.0;
        let speed_made = speed_of(sum(&|e| e.speed_made), gaps_cleared);
        let speed_short = speed_of(sum(&|e| e.speed_short), gaps_short);
        let speed_fell = speed_of(sum(&|e| e.speed_fell), gaps_fell);
        let gaps_pct = gaps_cleared / gap_flights.max(1.0) * 100.0;
        let askew_pct = sum(&|e| e.air_askew as f32) / air_ticks.max(1.0) * 100.0;
        let starts = |f: &dyn Fn(&brain::env::Episode) -> bool| {
            s.episodes.iter().filter(|(_, e)| f(e)).count()
        };
        let cuts = |d: Done| s.episodes.iter().filter(|(c, _)| *c == d).count();
        println!(
            "{iter:5} | eps {:4} reward {ep_reward:7.1} | {units_per_tick:5.1} u/tick, \
             {grabs_per_min:4.2} grabs/min ({taken_back:4.0} u back each), {boosts_per_min:5.1} boosts/min ({lost:4.1}s lost/min), {hops_per_min:4.0} hops/min, R1 {r1:3.0}% drift {drifting:3.0}% wall {wall:3.1}% brake {braking:3.1}% | {drifts_per_min:4.1} drifts/min of {drift_len:4.1} ticks: early {early:4.2} boost {in_time:4.2} (late {boost_timing:4.2}) out {outs:4.2} | ended: {} | air {air_pct:3.1}% of ticks, {air_misalign:4.2} turned away/jump, stick {air_steer:4.2}, {askew_pct:3.0}% >45° | steer {steer_change:4.2} | gaps made {gaps_cleared:.0}/{gap_flights:.0} ({gaps_pct:3.0}%), short {gaps_short:.0}, fell {gaps_fell:.0}, hopped {gap_hops:.0}, apex {gap_apex:.0}u, speed at takeoff made/short/fell {speed_made:.0}/{speed_short:.0}/{speed_fell:.0}% | starts J/L/G {}/{}/{} switches {switches:.2}/min | cut T/G/S {}/{}/{} | ent {:.2} kl {:.4} clip {:.2} \
             vloss {:.3} | {} upd, {:.1}s + {:.1}s",
            s.episodes.len(),
            ends_text.join(" "),
            starts(&|e| e.jump_start),
            starts(&|e| e.lip_start),
            starts(&|e| e.grab_start),
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
            "{iter},{},{ep_reward},{},{units_per_tick},{grabs_per_min},{taken_back},{boosts_per_min},{lost},{hops_per_min},{r1},{drifting},{wall},{braking},{air_pct},{askew_pct},{drifts_per_min},{drift_len},{early},{in_time},{outs},{air_misalign},{steer_change},{},{},{},{},{},{},{},{},{},{},{},{},{gaps_cleared},{gap_flights},{gap_hops},{gaps_short},{gaps_fell},{speed_made},{speed_short},{speed_fell},{air_steer},{gap_apex},{boost_timing}",
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
        for (&key, lip) in &s.lips {
            let total = lips.entry(key).or_default();
            total.takeoffs += lip.takeoffs;
            total.hopped += lip.hopped;
            total.made += lip.made;
            total.short += lip.short;
            total.fell += lip.fell;
        }
        if iter % save_every == 0 || iter == iters {
            // (Level numbers, as `ctr-query` takes them; the lip is a frame of the bot path.)
            let mut file = std::fs::OpenOptions::new().append(true).open(&lips_path)?;
            println!("ramp lips since the last checkpoint (and their share of the lip starts now):");
            let odds = trainer.lip_odds();
            for (&key, l) in &lips {
                let (level, path, lip) = key;
                let pct = l.made as f32 / l.takeoffs.max(1) as f32 * 100.0;
                let share = odds.get(&key).copied().unwrap_or(0.0);
                println!(
                    "  level {level:2} path {path} lip {lip:4}: {:3} takeoffs, {:3} made ({pct:3.0}%), {:3} short, {:3} fell, {:3} hopped | {share:4.1}% of lip starts",
                    l.takeoffs, l.made, l.short, l.fell, l.hopped
                );
                writeln!(
                    file,
                    "{iter},{level},{path},{lip},{},{},{},{},{}",
                    l.takeoffs, l.hopped, l.made, l.short, l.fell
                )?;
            }
            lips.clear();
            let path = out.join(format!("policy-{iter:05}.safetensors"));
            trainer.vars.save(&path)?;
            trainer.vars.save(out.join("policy.safetensors"))?;
            println!("saved {}", std::path::Path::new("crates/brain").join(&path).display());
        }
    }
    Ok(())
}

/// The rows of the last iteration in a `lips.csv` (`iter,level,path,lip,takeoffs,hopped,made,
/// short,fell`), and that iteration.
fn read_lips(
    text: &str,
) -> Result<(usize, std::collections::BTreeMap<brain::env::LipKey, brain::env::LipStats>), Error> {
    let mut rows = Vec::new();
    for line in text.lines().skip(1).filter(|l| !l.trim().is_empty()) {
        let v: Vec<usize> = line
            .split(',')
            .map(|f| f.trim().parse())
            .collect::<Result<_, _>>()
            .map_err(|e| format!("lips: {line:?}: {e}"))?;
        let [iter, level, path, lip, takeoffs, hopped, made, short, fell] = v[..] else {
            return Err(format!("lips: expected 9 fields, got {line:?}").into());
        };
        let stats = brain::env::LipStats {
            takeoffs: takeoffs as u32,
            hopped: hopped as u32,
            made: made as u32,
            short: short as u32,
            fell: fell as u32,
        };
        rows.push((iter, (level as u32, path, lip), stats));
    }
    let last = rows.iter().map(|r| r.0).max().ok_or("lips: no rows")?;
    let lips = rows.into_iter().filter(|r| r.0 == last).map(|(_, k, s)| (k, s)).collect();
    Ok((last, lips))
}
