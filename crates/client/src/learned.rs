//! (Debug) The dummy karts of `/debug spawn-karts <n> brain`: driven by a policy the `brain`
//! crate trained, as its `eval` drives one. Their pads are made here and go into the race like
//! ours (`drive_tick`), which offline are all known before the tick plays: nothing to predict.
//! (Online the host would make them the same way, and send them out as a player's.)
//!
//! Only with `--features brain`.

use std::path::PathBuf;

use bevy::prelude::*;
use brain::policy::Policy;
use brain::rng::Rng;
use loader::lev::Level;
use race::bot::{Bot, PathLength, Tracker};
use race::nn::{self, Action, Latch, Progress};
use race::rollback::Rollback;
use race::{KartSpec, Pad, Race};

/// Where the policy is read from when none is given: `BRAIN_POLICY`, or else this (where `train`
/// saves it, from the workspace).
const DEFAULT_POLICY: &str = "target/brain/policy.safetensors";

/// The policy, and the karts it drives in the race.
#[derive(Resource)]
pub struct Brains {
    policy: Option<Policy>,
    /// The file of the policy last given on the console: the next spawn reads it again.
    given: Option<PathBuf>,
    karts: Vec<Driven>,
    /// (The choices are the likeliest: nothing is drawn from it.)
    rng: Rng,
}

impl Default for Brains {
    fn default() -> Brains {
        Brains {
            policy: None,
            given: None,
            karts: Vec::new(),
            rng: Rng::new(1),
        }
    }
}

/// A kart the policy drives.
struct Driven {
    slot: usize,
    /// Into `Level::nav_paths`: the path it steers by, as a scripted bot of its slot would.
    path: usize,
    lengths: PathLength,
    tracker: Tracker,
    progress: Progress,
    latch: Latch,
    /// The choice it holds.
    action: Action,
}

impl Brains {
    /// Reads the policy (again, it may have been trained further) from `path`, or the last one
    /// given, or `BRAIN_POLICY`, or [`DEFAULT_POLICY`]. Returns the file read.
    pub fn load(&mut self, path: Option<&str>) -> Result<PathBuf, String> {
        if let Some(path) = path {
            self.given = Some(PathBuf::from(path));
        }
        let path = self.given.clone().unwrap_or_else(|| {
            PathBuf::from(std::env::var("BRAIN_POLICY").unwrap_or_else(|_| DEFAULT_POLICY.into()))
        });
        let policy = Policy::load(&path, &brain::ppo::device())
            .map_err(|e| format!("Reading the policy {}: {e}", path.display()))?;
        self.policy = Some(policy);
        Ok(path)
    }

    /// The race starts at tick 0 (of `specs`): the policy drives the karts of `slots`, each by
    /// the path of its slot, or of its `KartSpec::line` (`/debug bot-line`).
    pub fn start(
        &mut self,
        level: &Level,
        race: &Race,
        specs: &[KartSpec],
        slots: impl Iterator<Item = usize>,
    ) {
        self.karts = slots
            .filter_map(|slot| {
                let driver = &race.karts.get(slot)?.driver;
                let bot = Bot::new(level, slot, driver, specs[slot].line);
                let frames = &level.nav_paths[bot.path as usize].frames;
                let lengths = PathLength::of(frames);
                Some(Driven {
                    slot,
                    path: bot.path as usize,
                    progress: Progress::new(frames, &lengths, &bot.tracker, driver),
                    lengths,
                    tracker: bot.tracker,
                    latch: Latch::default(),
                    action: Action::default(),
                })
            })
            .collect();
    }

    /// The pads of the karts for the next tick of `race`, by slot, unless they are in already.
    /// A new choice every `Rules::repeat` ticks, as in training: one forward pass for them all.
    pub fn pads(&mut self, level: &Level, race: &Rollback<Race>) -> Vec<(usize, Pad)> {
        let tick = race.tick();
        let Some(policy) = &self.policy else {
            return Vec::new();
        };
        if self
            .karts
            .iter()
            .any(|k| k.slot >= race.karts.len() || race.confirmed_until(k.slot) > tick)
        {
            return Vec::new();
        }
        let decide = tick % brain::env::Rules::default().repeat == 0;
        let mut obs = Vec::new();
        let mut deciding = Vec::new();
        for (i, k) in self.karts.iter_mut().enumerate() {
            let frames = &level.nav_paths[k.path].frames;
            if frames.len() < 2 {
                continue;
            }
            let kart = &race.karts[k.slot];
            let relocated = nn::track(&mut k.tracker, frames, &kart.driver);
            k.progress
                .step(frames, &k.lengths, &k.tracker, relocated, &kart.driver);
            if decide {
                obs.push(nn::observe(frames, &k.tracker, &k.progress, kart, &k.latch));
                deciding.push(i);
            }
        }
        if !obs.is_empty() {
            match policy.act(&obs, true, &mut self.rng) {
                Ok(choices) => {
                    for (i, choice) in deciding.into_iter().zip(choices) {
                        self.karts[i].action = choice.action;
                    }
                }
                // (They hold the choices they had.)
                Err(e) => error!("the policy's forward pass: {e}"),
            }
        }
        self.karts
            .iter_mut()
            .map(|k| {
                let has_path = level.nav_paths[k.path].frames.len() >= 2;
                let pad = if has_path { k.latch.pad(k.action) } else { Pad::default() };
                (k.slot, pad)
            })
            .collect()
    }
}
