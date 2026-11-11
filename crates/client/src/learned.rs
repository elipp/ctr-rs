//! The karts a policy the `brain` crate trained drives, as its `eval` drives one: the bots that
//! fill the races of a host (`/bots brain`). Their pads are made here and go into the race like
//! ours (`drive_tick`): the host makes them from the race as it predicts it, and sends them out
//! with its own (`net::Session::bot_input`). To everyone else they are a player's.
//!
//! The policy runs on [`race::infer`], without candle: the `brain` crate only trains it.

use std::path::PathBuf;

use bevy::prelude::*;
use loader::lev::Level;
use race::bot::{Bot, PathLength, Tracker};
use race::infer::Actor;
use race::nn::{self, Action, Latch, Progress};
use race::rng::Rng;
use race::rollback::Rollback;
use race::{KartSpec, Pad, Race};

/// A policy kart taps Circle (fires its item, if it has one) every this many ticks or so, drawn
/// anew each time: 0.5 to 5 s. Not the policy's choice: it was trained without weapons.
const FIRE_EVERY: std::ops::RangeInclusive<u32> = 15..=150;

/// A wait to the next tap of Circle, in [`FIRE_EVERY`].
fn fire_wait(rng: &mut Rng) -> u32 {
    let span = FIRE_EVERY.end() - FIRE_EVERY.start() + 1;
    FIRE_EVERY.start() + rng.below(span as usize) as u32
}

/// The policy when none is given (nor `BRAIN_POLICY`): built in, so it works without a file
/// system (wasm). `bots/BOTS.md` has how it was trained.
const DEFAULT_POLICY: &[u8] = include_bytes!("../../../bots/policy/switch-v4-01000.safetensors");
/// The name of [`DEFAULT_POLICY`], for the log.
const DEFAULT_POLICY_NAME: &str = "switch-v4-01000 (built in)";

/// The policy (its actor: what drives), and the karts it drives in the race.
#[derive(Resource)]
pub struct Brains {
    policy: Option<Actor>,
    /// The policy file `/bots policy` asked for, over `BRAIN_POLICY`; none, for that (or the
    /// built-in one).
    path: Option<PathBuf>,
    karts: Vec<Driven>,
    /// For the choices of the karts that are stuck, or just put back by the mask
    /// (`nn::draw_choices`).
    rng: Rng,
}

impl Default for Brains {
    fn default() -> Brains {
        Brains {
            policy: None,
            path: None,
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
    /// The ticks to its next tap of Circle ([`FIRE_EVERY`]).
    fire_in: u32,
}

impl Brains {
    /// The policy file to read: the one `/bots policy` asked for, or `BRAIN_POLICY`; none, for
    /// [`DEFAULT_POLICY`].
    fn path(&self) -> Option<PathBuf> {
        self.path
            .clone()
            .or_else(|| std::env::var_os("BRAIN_POLICY").map(PathBuf::from))
    }

    /// The name of the policy [`Brains::load`] takes.
    pub fn name(&self) -> String {
        self.path()
            .map_or(DEFAULT_POLICY_NAME.to_string(), |p| p.display().to_string())
    }

    /// Drives with the policy at `path` from now on (`/bots policy`), or with `BRAIN_POLICY` or
    /// the built-in one for none. Read now, to say at once what is wrong with it: if it does
    /// not read, the one before stays. Returns the name of the one taken.
    pub fn set_path(&mut self, path: Option<PathBuf>) -> Result<String, String> {
        let before = std::mem::replace(&mut self.path, path);
        self.load().inspect_err(|_| self.path = before)
    }

    /// Reads the policy (again, it may have been trained further) from the file of
    /// [`Brains::path`], or else takes [`DEFAULT_POLICY`]. Returns the name of the one taken.
    pub fn load(&mut self) -> Result<String, String> {
        let (policy, name) = match self.path() {
            Some(path) => (
                Actor::load(&path)
                    .map_err(|e| format!("Reading the policy {}: {e}", path.display()))?,
                path.display().to_string(),
            ),
            None => (
                Actor::from_safetensors(DEFAULT_POLICY)
                    .map_err(|e| format!("The built-in policy: {e}"))?,
                DEFAULT_POLICY_NAME.to_string(),
            ),
        };
        self.policy = Some(policy);
        Ok(name)
    }

    /// The race starts at tick 0 (of `specs`): the policy drives the karts of `slots`, each by
    /// the path of its slot, or of its `KartSpec::line`.
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
                    fire_in: fire_wait(&mut self.rng),
                })
            })
            .collect();
    }

    /// The pads of the karts for the next tick of `race`, by slot, unless they are in already.
    /// A new choice every [`nn::REPEAT`] ticks, as in training.
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
        let decide = tick % nn::REPEAT == 0;
        for k in &mut self.karts {
            let frames = &level.nav_paths[k.path].frames;
            if frames.len() < 2 {
                continue;
            }
            let kart = &race.karts[k.slot];
            let relocated = nn::track(&mut k.tracker, frames, &kart.driver);
            k.progress
                .step(frames, &k.lengths, &k.tracker, relocated, &kart.driver);
            // Stuck: try another path, as in training.
            if k.progress.switch_due(&kart.driver)
                && let Some((path, tracker)) =
                    nn::next_path(level, k.path, kart.driver.pos_curr)
            {
                k.path = path;
                k.tracker = tracker;
                k.lengths = PathLength::of(&level.nav_paths[path].frames);
                let frames = &level.nav_paths[path].frames;
                k.progress.switched(frames, &k.lengths, &k.tracker, &kart.driver);
            }
            let frames = &level.nav_paths[k.path].frames;
            if decide {
                // The likeliest choice, or a drawn one (`nn::draw_choices`).
                let greedy = !nn::draw_choices(&k.progress);
                let obs = nn::observe(frames, &k.tracker, &k.progress, kart, &k.latch);
                k.action = policy.act(&obs, greedy, &mut self.rng);
            }
        }
        let rng = &mut self.rng;
        self.karts
            .iter_mut()
            .map(|k| {
                let has_path = level.nav_paths[k.path].frames.len() >= 2;
                let d = &race.karts[k.slot].driver;
                let drifting = d.kart_state == vehicle::kart_state::DRIFTING;
                let tried = d.num_boosts_attempted > 0;
                let mut pad = if has_path {
                    k.latch.pad(k.action, drifting, tried)
                } else {
                    Pad::default()
                };
                // A tap: held for this tick only (the next has it released, the fire is on the
                // press).
                if k.fire_in == 0 {
                    pad.held |= vehicle::buttons::CIRCLE;
                    k.fire_in = fire_wait(rng);
                } else {
                    k.fire_in -= 1;
                }
                (k.slot, pad)
            })
            .collect()
    }
}
