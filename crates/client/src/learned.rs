//! The karts a policy the `brain` crate trained drives, as its `eval` drives one: the bots that
//! fill the races of a host (`/bots brain`). Their pads are made here and go into the race like
//! ours (`drive_tick`): the host makes them from the race as it predicts it, and sends them out
//! with its own (`net::Session::bot_input`). To everyone else they are a player's.
//!
//! The policy runs on [`race::infer`], without candle: the `brain` crate only trains it.

use std::path::PathBuf;

use bevy::prelude::*;
use loader::lev::Level;
use loader::level::LevelId;
use race::bot::{Bot, PathLength, Tracker};
use race::infer::Actor;
use race::nn::{self, Action, Latch, Progress};
use race::rev::{self, Rev, Skill};
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

/// How our own kart revs while the policy drives it (`/autopilot`): as on hard.
const OWN_REV: Skill = Skill::of(vehicle::bot::difficulty_level::HARD);

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
    /// The level [`Brains::karts`] are on: their paths are into its `nav_paths`.
    level: Option<LevelId>,
    /// Whether our own kart is to be driven too (`/autopilot`).
    pub autopilot: bool,
    /// The slot of our kart in [`Brains::karts`], while the policy drives it.
    own: Option<usize>,
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
            level: None,
            autopilot: false,
            own: None,
            rng: Rng::new(1),
        }
    }
}

/// A kart the policy drives.
struct Driven {
    slot: usize,
    /// Into `Level::nav_paths`: the path it steers by, as a scripted bot of its slot would
    /// (unless that one is banned, `nn::BANNED_PATHS`).
    path: usize,
    /// The paths it may steer by (`nn::learned_paths`), for a switch to another when stuck.
    paths: Vec<usize>,
    lengths: PathLength,
    tracker: Tracker,
    progress: Progress,
    latch: Latch,
    /// The choice it holds.
    action: Action,
    /// Its gas while its engine revs (on the grid, and as the mask lowers it): not the policy's.
    rev: Rev,
    /// The ticks to its next tap of Circle ([`FIRE_EVERY`]).
    fire_in: u32,
    /// What its steps have been worth so far ([`nn::Weights::reward`], unscaled), for the debug
    /// HUD; `Brains::reset_own_reward` starts it over.
    reward: f32,
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
    /// the path of its slot, or of its `KartSpec::line`. They rev their engines as well as
    /// `difficulty` (`vehicle::bot::difficulty_level`) has them.
    pub fn start(
        &mut self,
        level: &Level,
        id: LevelId,
        race: &Race,
        specs: &[KartSpec],
        slots: impl Iterator<Item = usize>,
        difficulty: i16,
    ) {
        self.own = None;
        self.level = Some(id);
        let skill = Skill::of(difficulty);
        self.karts = slots
            .filter_map(|slot| {
                driven(level, id, race, slot, specs[slot].line, skill, &mut self.rng)
            })
            .collect();
    }

    /// Whether the policy is read, and so can drive.
    pub fn loaded(&self) -> bool {
        self.policy.is_some()
    }

    /// Reads the policy unless it is read already. Returns the name of the one taken.
    pub fn ensure_loaded(&mut self) -> Result<String, String> {
        if self.loaded() { Ok(self.name()) } else { self.load() }
    }

    /// Has the policy drive our kart, `own` of `race`, from this tick if `on` (wherever the race
    /// is: it takes the kart where it is), or gives it back. Call it each tick; it does
    /// nothing when it is as it was. [`Brains::start`] gives it back.
    pub fn drive_own(&mut self, level: &Level, id: LevelId, race: &Race, own: usize, on: bool) {
        // Another level (offline, without a `Brains::start`): the karts' paths are of the one
        // before, and so is our kart's if driven; it is taken again on this one below.
        if self.level != Some(id) {
            self.level = Some(id);
            self.karts.clear();
            self.own = None;
        }
        if self.own.is_some_and(|slot| !on || slot != own) {
            let slot = self.own.take();
            self.karts.retain(|k| Some(k.slot) != slot);
        }
        if on
            && self.own.is_none()
            && let Some(kart) = driven(level, id, race, own, None, OWN_REV, &mut self.rng)
        {
            self.karts.push(kart);
            self.own = Some(own);
        }
    }

    /// The reward our kart has earned from the policy's driving since it took over (or since
    /// [`Brains::reset_own_reward`]); none while it does not drive.
    pub fn own_reward(&self) -> Option<f32> {
        let slot = self.own?;
        self.karts.iter().find(|k| k.slot == slot).map(|k| k.reward)
    }

    /// Starts [`Brains::own_reward`] over from 0.
    pub fn reset_own_reward(&mut self) {
        let slot = self.own;
        for k in self.karts.iter_mut().filter(|k| Some(k.slot) == slot) {
            k.reward = 0.0;
        }
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
            let step = k
                .progress
                .step(frames, &k.lengths, &k.tracker, relocated, &kart.driver);
            k.reward += nn::Weights::DEFAULT.reward(&step);
            // Stuck: try another path, as in training.
            if k.progress.switch_due(&kart.driver)
                && let Some((path, tracker)) =
                    nn::next_path_of(level, k.path, kart.driver.pos_curr, &k.paths)
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
                    k.latch.pad(k.action, drifting, tried, k.progress.in_gap_flight())
                } else {
                    Pad::default()
                };
                if let Some(gas) = k.rev.gas(race, k.slot, rng) {
                    pad = rev::with_gas(pad, gas);
                }
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

/// The kart of `slot` as the policy drives it from now, by the path of its slot or of `line`; by
/// the next one if that is banned (`nn::BANNED_PATHS`). None if the level (of id `id`) has none
/// it may drive.
fn driven(
    level: &Level,
    id: LevelId,
    race: &Race,
    slot: usize,
    line: Option<u8>,
    skill: Skill,
    rng: &mut Rng,
) -> Option<Driven> {
    let driver = &race.karts.get(slot)?.driver;
    let bot = Bot::new(level, slot, driver, line);
    let paths = nn::learned_paths(level, id);
    let (path, tracker) = if paths.contains(&(bot.path as usize)) {
        (bot.path as usize, bot.tracker)
    } else {
        nn::next_path_of(level, bot.path as usize, driver.pos_curr, &paths)?
    };
    let frames = &level.nav_paths[path].frames;
    let lengths = PathLength::of(frames);
    Some(Driven {
        slot,
        path,
        paths,
        progress: Progress::new(frames, &lengths, &tracker, driver),
        lengths,
        tracker,
        latch: Latch::default(),
        action: Action::default(),
        rev: Rev::new(skill),
        fire_in: fire_wait(rng),
        reward: 0.0,
    })
}
