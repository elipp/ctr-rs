//! PPO (clipped surrogate, GAE), from scratch: rollouts of all the envs at once (stepped on as
//! many threads), then a few epochs of minibatch updates of the actor and the critic.

use candle_core::{D, Device, Result, Tensor, Var};
use candle_nn::{AdamW, Optimizer, ParamsAdamW, VarMap};
use race::nn::{Action, N_HEADS, OBS_LEN, Obs};

use crate::env::{Done, Env, Episode, LipKey, LipStats};
use crate::policy::Policy;
use crate::rng::Rng;

#[derive(Clone, Copy, Debug)]
pub struct Config {
    /// Steps of each env in a rollout.
    pub steps: usize,
    pub gamma: f32,
    pub lambda: f32,
    pub clip: f32,
    pub epochs: usize,
    pub minibatch: usize,
    pub lr: f64,
    pub value_coef: f32,
    pub entropy_coef: f32,
    pub max_grad_norm: f32,
    /// An update's epochs stop early once the approximate KL from the rollout's policy is past
    /// this.
    pub target_kl: f32,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            steps: 256,
            // 100 choices (a choice is held two ticks): about 7 seconds.
            gamma: 0.99,
            lambda: 0.95,
            clip: 0.2,
            epochs: 4,
            minibatch: 2048,
            lr: 3e-4,
            value_coef: 0.5,
            entropy_coef: 0.01,
            max_grad_norm: 0.5,
            target_kl: 0.03,
        }
    }
}

/// The rollout of all the envs, `steps × envs` long, step-major.
struct Rollout {
    obs: Vec<f32>,
    actions: Vec<u32>,
    log_probs: Vec<f32>,
    values: Vec<f32>,
    rewards: Vec<f32>,
    /// The episode was cut after this step (its reward has the critic's value of where on top).
    cut: Vec<bool>,
}

/// What a rollout and its update came to, for the log.
#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub episodes: Vec<(Done, Episode)>,
    /// The flights from each ramp lip, over the rollout's envs.
    pub lips: std::collections::BTreeMap<LipKey, LipStats>,
    pub mean_reward: f32,
    pub policy_loss: f32,
    pub value_loss: f32,
    pub entropy: f32,
    pub approx_kl: f32,
    pub clip_frac: f32,
    pub updates: usize,
    pub rollout_secs: f32,
    pub update_secs: f32,
}

pub struct Trainer<'a> {
    pub config: Config,
    pub vars: VarMap,
    pub policy: Policy,
    opt: AdamW,
    envs: Vec<Env<'a>>,
    obs: Vec<Obs>,
    rng: Rng,
    threads: usize,
    /// The takeoffs and the failed flights (not made) from each ramp lip, over all the envs,
    /// fading by [`Trainer::lip_memory`] an iteration: what [`Env::set_lip_odds`] is fed.
    lip_tally: std::collections::BTreeMap<LipKey, (f32, f32)>,
    /// What is left of the tally after an iteration (0.97: half of it is gone in some 23).
    pub lip_memory: f32,
}

/// A lip's weight for the lip starts (`Rules::lip_starts`), of its takeoffs and failed flights:
/// its share of failures, as if it had failed one in two more (a lip seen once is not 0 or 1).
pub fn lip_weight(takeoffs: f32, failed: f32) -> f32 {
    (failed + 1.0) / (takeoffs + 2.0)
}

impl<'a> Trainer<'a> {
    pub fn new(
        config: Config,
        vars: VarMap,
        policy: Policy,
        envs: Vec<Env<'a>>,
        seed: u64,
    ) -> Result<Trainer<'a>> {
        let opt = AdamW::new(
            vars.all_vars(),
            ParamsAdamW {
                lr: config.lr,
                weight_decay: 0.0,
                ..ParamsAdamW::default()
            },
        )?;
        let obs = envs.iter().map(Env::observe).collect();
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
        Ok(Trainer {
            config,
            vars,
            policy,
            opt,
            envs,
            obs,
            rng: Rng::new(seed),
            threads,
            lip_tally: Default::default(),
            lip_memory: 0.97,
        })
    }

    /// Sets what each millisecond of reserves gained and each drift boost are worth, in every env
    /// (to fade the bonuses out over the training).
    pub fn set_bonuses(&mut self, reserves: f32, drift_boost: f32) {
        for env in &mut self.envs {
            env.rules.weights.reserves = reserves;
            env.rules.weights.drift_boost = drift_boost;
        }
    }

    /// Adds flights to the lip tally (as from an earlier run's `lips.csv`), and hands the odds to
    /// the envs.
    pub fn add_lips(&mut self, lips: &std::collections::BTreeMap<LipKey, LipStats>) {
        for (&key, lip) in lips {
            let t = self.lip_tally.entry(key).or_default();
            t.0 += lip.takeoffs as f32;
            t.1 += lip.takeoffs.saturating_sub(lip.made) as f32;
        }
        let odds = std::sync::Arc::new(self.lip_weights());
        for env in &mut self.envs {
            env.set_lip_odds(odds.clone());
        }
    }

    /// Each lip's weight ([`lip_weight`]) by the tally.
    fn lip_weights(&self) -> Vec<(LipKey, f32)> {
        self.lip_tally
            .iter()
            .filter(|(_, t)| t.0 > 0.0)
            .map(|(&key, &(takeoffs, failed))| (key, lip_weight(takeoffs, failed)))
            .collect()
    }

    /// Each lip's share of the lip starts, in percent, by the tally.
    pub fn lip_odds(&self) -> std::collections::BTreeMap<LipKey, f32> {
        let weights = self.lip_weights();
        let total: f32 = weights.iter().map(|(_, w)| w).sum::<f32>().max(f32::MIN_POSITIVE);
        weights.into_iter().map(|(key, w)| (key, w / total * 100.0)).collect()
    }

    /// Steps every env by its action, on the threads.
    fn step_envs(&mut self, actions: &[Action]) -> Vec<crate::env::Step> {
        let n = self.envs.len();
        let chunk = n.div_ceil(self.threads);
        let mut out = Vec::with_capacity(n);
        std::thread::scope(|s| {
            let handles: Vec<_> = self
                .envs
                .chunks_mut(chunk)
                .zip(actions.chunks(chunk))
                .map(|(envs, actions)| {
                    s.spawn(move || {
                        envs.iter_mut()
                            .zip(actions)
                            .map(|(env, &a)| {
                                let step = env.step(a);
                                (step, env.observe())
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            for h in handles {
                out.extend(h.join().expect("an env's thread panicked"));
            }
        });
        let (steps, obs): (Vec<_>, Vec<_>) = out.into_iter().unzip();
        self.obs = obs;
        steps
    }

    fn rollout(&mut self, stats: &mut Stats) -> Result<(Rollout, Vec<f32>)> {
        let (t, n) = (self.config.steps, self.envs.len());
        let mut r = Rollout {
            obs: Vec::with_capacity(t * n * OBS_LEN),
            actions: Vec::with_capacity(t * n * N_HEADS),
            log_probs: Vec::with_capacity(t * n),
            values: Vec::with_capacity(t * n),
            rewards: Vec::with_capacity(t * n),
            cut: Vec::with_capacity(t * n),
        };
        for _ in 0..t {
            let choices = self.policy.act(&self.obs, false, &mut self.rng)?;
            for (o, c) in self.obs.iter().zip(&choices) {
                r.obs.extend_from_slice(o);
                r.actions.extend(c.action.heads().map(|h| h as u32));
                r.log_probs.push(c.log_prob);
                r.values.push(c.value);
            }
            let actions: Vec<Action> = choices.iter().map(|c| c.action).collect();
            let steps = self.step_envs(&actions);
            // The episodes cut here go on from the critic's value of where they were cut.
            let cut: Vec<(usize, Obs)> = steps
                .iter()
                .enumerate()
                .filter_map(|(i, s)| s.cut.as_ref().map(|(_, last, _)| (i, *last)))
                .collect();
            let mut bootstrap = vec![0.0; n];
            if !cut.is_empty() {
                let last: Vec<Obs> = cut.iter().map(|&(_, o)| o).collect();
                let values: Vec<f32> = self.policy.value(&self.policy.batch(&last)?)?.to_vec1()?;
                for (&(i, _), v) in cut.iter().zip(values) {
                    bootstrap[i] = v;
                }
            }
            for (i, s) in steps.into_iter().enumerate() {
                r.rewards.push(s.reward + self.config.gamma * bootstrap[i]);
                r.cut.push(s.cut.is_some());
                if let Some((done, _, episode)) = s.cut {
                    stats.episodes.push((done, episode));
                }
            }
        }
        let last_values = self.policy.value(&self.policy.batch(&self.obs)?)?.to_vec1()?;
        Ok((r, last_values))
    }

    /// Generalized advantage estimation: the advantages and the critic's targets.
    fn advantages(&self, r: &Rollout, last_values: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let (t, n) = (self.config.steps, self.envs.len());
        let (gamma, lambda) = (self.config.gamma, self.config.lambda);
        let mut adv = vec![0.0; t * n];
        for e in 0..n {
            let mut running = 0.0;
            for s in (0..t).rev() {
                let i = s * n + e;
                let go_on = if r.cut[i] { 0.0 } else { 1.0 };
                let next = if s + 1 == t { last_values[e] } else { r.values[i + n] };
                let delta = r.rewards[i] + gamma * next * go_on - r.values[i];
                running = delta + gamma * lambda * go_on * running;
                adv[i] = running;
            }
        }
        let returns = adv.iter().zip(&r.values).map(|(a, v)| a + v).collect();
        (adv, returns)
    }

    /// A rollout and the updates of it.
    pub fn iterate(&mut self) -> Result<Stats> {
        let mut stats = Stats::default();
        let start = std::time::Instant::now();
        let (r, last_values) = self.rollout(&mut stats)?;
        for env in &mut self.envs {
            for (key, lip) in env.take_lips() {
                let total = stats.lips.entry(key).or_default();
                total.takeoffs += lip.takeoffs;
                total.hopped += lip.hopped;
                total.made += lip.made;
                total.short += lip.short;
                total.fell += lip.fell;
            }
        }
        for t in self.lip_tally.values_mut() {
            t.0 *= self.lip_memory;
            t.1 *= self.lip_memory;
        }
        let lips = stats.lips.clone();
        self.add_lips(&lips);
        stats.rollout_secs = start.elapsed().as_secs_f32();
        stats.mean_reward = r.rewards.iter().sum::<f32>() / r.rewards.len() as f32;
        let (adv, returns) = self.advantages(&r, &last_values);

        let start = std::time::Instant::now();
        let total = r.log_probs.len();
        let mut order: Vec<usize> = (0..total).collect();
        let cfg = self.config;
        let (mut sums, mut count) = ([0.0f32; 5], 0);
        'epochs: for _ in 0..cfg.epochs {
            self.rng.shuffle(&mut order);
            for batch in order.chunks(cfg.minibatch) {
                let m = self.minibatch(&r, &adv, &returns, batch)?;
                let losses = self.update(&m)?;
                for (s, l) in sums.iter_mut().zip(losses) {
                    *s += l;
                }
                count += 1;
                if losses[3] > cfg.target_kl {
                    break 'epochs;
                }
            }
        }
        let mean = |k: usize| sums[k] / count.max(1) as f32;
        stats.policy_loss = mean(0);
        stats.value_loss = mean(1);
        stats.entropy = mean(2);
        stats.approx_kl = mean(3);
        stats.clip_frac = mean(4);
        stats.updates = count;
        stats.update_secs = start.elapsed().as_secs_f32();
        Ok(stats)
    }

    fn minibatch(
        &self,
        r: &Rollout,
        adv: &[f32],
        returns: &[f32],
        batch: &[usize],
    ) -> Result<Minibatch> {
        let dev = &self.policy.device;
        let b = batch.len();
        let mut obs = Vec::with_capacity(b * OBS_LEN);
        let mut actions: Vec<Vec<u32>> = (0..N_HEADS).map(|_| Vec::with_capacity(b)).collect();
        for &i in batch {
            obs.extend_from_slice(&r.obs[i * OBS_LEN..(i + 1) * OBS_LEN]);
            for (h, a) in actions.iter_mut().enumerate() {
                a.push(r.actions[i * N_HEADS + h]);
            }
        }
        // The advantages normalized over the minibatch.
        let a: Vec<f32> = batch.iter().map(|&i| adv[i]).collect();
        let mean = a.iter().sum::<f32>() / b as f32;
        let std = (a.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / b as f32).sqrt();
        let a: Vec<f32> = a.iter().map(|x| (x - mean) / (std + 1e-8)).collect();
        let pick = |v: &[f32]| batch.iter().map(|&i| v[i]).collect::<Vec<f32>>();
        let actions = actions
            .into_iter()
            .map(|v| Tensor::from_vec(v, (b, 1), dev))
            .collect::<Result<_>>()?;
        Ok(Minibatch {
            obs: Tensor::from_vec(obs, (b, OBS_LEN), dev)?,
            actions,
            old_log_probs: Tensor::from_vec(pick(&r.log_probs), b, dev)?,
            advantages: Tensor::from_vec(a, b, dev)?,
            returns: Tensor::from_vec(pick(returns), b, dev)?,
        })
    }

    /// One gradient step on `m`. Returns the policy loss, the value loss, the entropy, the
    /// approximate KL and the fraction of ratios clipped.
    fn update(&mut self, m: &Minibatch) -> Result<[f32; 5]> {
        let cfg = self.config;
        let heads = Policy::head_log_probs(&self.policy.logits(&m.obs)?)?;
        let mut log_prob: Option<Tensor> = None;
        let mut entropy: Option<Tensor> = None;
        for (lp, a) in heads.iter().zip(&m.actions) {
            let chosen = lp.gather(a, 1)?.squeeze(1)?;
            let ent = (lp.exp()? * lp)?.sum(D::Minus1)?.neg()?;
            log_prob = Some(match log_prob {
                Some(t) => (t + chosen)?,
                None => chosen,
            });
            entropy = Some(match entropy {
                Some(t) => (t + ent)?,
                None => ent,
            });
        }
        let (log_prob, entropy) = (log_prob.unwrap(), entropy.unwrap().mean_all()?);

        let log_ratio = (&log_prob - &m.old_log_probs)?;
        let ratio = log_ratio.exp()?;
        let surr1 = (&ratio * &m.advantages)?;
        let surr2 = (ratio.clamp(1.0 - cfg.clip, 1.0 + cfg.clip)? * &m.advantages)?;
        let policy_loss = surr1.minimum(&surr2)?.mean_all()?.neg()?;

        let value = self.policy.value(&m.obs)?;
        let value_loss = (value - &m.returns)?.sqr()?.mean_all()?;

        let loss = ((&policy_loss + (&value_loss * cfg.value_coef as f64)?)?
            - (&entropy * cfg.entropy_coef as f64)?)?;
        let mut grads = loss.backward()?;
        clip_grads(&mut grads, &self.vars.all_vars(), cfg.max_grad_norm)?;
        self.opt.step(&grads)?;

        // `(ratio - 1) - log ratio`: an estimate of the KL that is never negative.
        let approx_kl = ((&ratio - 1.0)? - &log_ratio)?.mean_all()?;
        let clipped = (&ratio - 1.0)?
            .abs()?
            .gt(cfg.clip as f64)?
            .to_dtype(candle_core::DType::F32)?
            .mean_all()?;
        let scalar = |t: &Tensor| t.to_scalar::<f32>();
        Ok([
            scalar(&policy_loss)?,
            scalar(&value_loss)?,
            scalar(&entropy)?,
            scalar(&approx_kl)?,
            scalar(&clipped)?,
        ])
    }
}

struct Minibatch {
    obs: Tensor,
    /// By head, `[b, 1]`.
    actions: Vec<Tensor>,
    old_log_probs: Tensor,
    advantages: Tensor,
    returns: Tensor,
}

/// Scales the gradients of `vars` down to a global norm of `max` (if over it).
fn clip_grads(grads: &mut candle_core::backprop::GradStore, vars: &[Var], max: f32) -> Result<()> {
    let mut sum = 0.0f32;
    for v in vars {
        if let Some(g) = grads.get(v.as_tensor()) {
            sum += g.sqr()?.sum_all()?.to_scalar::<f32>()?;
        }
    }
    let norm = sum.sqrt();
    if norm > max {
        let scale = (max / (norm + 1e-6)) as f64;
        for v in vars {
            if let Some(g) = grads.get(v.as_tensor()) {
                let g = (g * scale)?;
                grads.insert(v.as_tensor(), g);
            }
        }
    }
    Ok(())
}

/// The device the trainer runs on: the CPU. The nets are small and the batches are many, and
/// Metal's launches and copies cost more than the maths they spare (it was slower).
pub fn device() -> Device {
    Device::Cpu
}
