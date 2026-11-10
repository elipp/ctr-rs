//! One kart on a track, alone, dropped on a random frame of a random bot path, driven by the
//! policy's choices; and what each choice earned ([`race::nn::Weights`]).
//!
//! The episodes never end for good: they are cut (at a time limit, after a few mask grabs, when
//! it has got nowhere for a while), and the trainer goes on from the critic's value of where it
//! was cut (see [`Done`]). Getting nowhere costs a penalty as well, as a mask grab does: an end
//! for good would cost all the reward to come, which with a long horizon (`gamma` 0.995: some
//! 400) dwarfs everything else, and the policy learns to risk nothing (no drifts) and to drive
//! off a ledge rather than be stuck (a grab starts the count over).

use race::bot::Tracker;
use race::nn::{self, Action, Latch, Obs, Progress, Weights};
use race::{KartSpec, Pad, Race};

use crate::rng::Rng;
use crate::tracks::Track;

/// How the episodes go.
#[derive(Clone, Copy, Debug)]
pub struct Rules {
    /// Each choice is held for this many ticks.
    pub repeat: u32,
    /// An episode is cut after this many ticks ...
    pub max_ticks: u32,
    /// ... or this many mask grabs ...
    pub max_grabs: u32,
    /// ... or this many ticks without getting further along than it had been
    /// (`Progress::stalled`), which costs this (unscaled, like [`Rules::weights`]).
    pub stuck_ticks: u32,
    pub stuck_penalty: f32,
    pub weights: Weights,
    /// The reward the trainer sees is the sum of [`Rules::weights`]' over the held ticks, times
    /// this (for the critic's targets to be of a size of 1 or so).
    pub reward_scale: f32,
}

impl Default for Rules {
    fn default() -> Rules {
        Rules {
            repeat: 2,
            // A minute at 30 ticks a second.
            max_ticks: 30 * 60,
            max_grabs: 3,
            stuck_ticks: nn::STALL_TICKS,
            // As much as a mask grab.
            stuck_penalty: 30.0,
            weights: Weights::DEFAULT,
            reward_scale: 0.1,
        }
    }
}

/// How an episode ended, for the stats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Done {
    Time,
    Grabs,
    Stuck,
}


/// The sums of an episode, for the stats.
#[derive(Clone, Copy, Debug, Default)]
pub struct Episode {
    /// Of [`Rules::weights`], unscaled.
    pub reward: f32,
    pub ticks: u32,
    /// Level units along the path.
    pub progress: i64,
    pub grabs: u32,
    /// Drift boosts fired (the meter turned into reserves in time), each of a drift's.
    pub boosts: u32,
    /// R1 pressed (a hop, when the kart can).
    pub hops: u32,
    /// Ticks spent drifting, and with R1 held.
    pub drifting: u32,
    pub r1: u32,
    /// Ticks in the air past `nn::AIR_GRACE_TICKS`, and the sum of `Step::air_misalign` over
    /// them.
    pub air_ticks: u32,
    pub air_misalign: f32,
    /// Drifts started; the boost tries in them (taps of R1 or L1 with the meter on) that came
    /// too early, while the meter was still over half; and the drifts whose meter ran out
    /// untouched (which ends their tries). The tries in time are [`Episode::boosts`].
    pub drifts: u32,
    pub early_taps: u32,
    pub meter_outs: u32,
    /// How the drifts ended, by [`DRIFT_ENDS`].
    pub drift_ends: [u32; DRIFT_ENDS.len()],
    /// Choices made, and the steering bins the stick moved over them.
    pub choices: u32,
    pub steer_change: u32,
}

/// The ways a drift ends, for [`Episode::drift_ends`]: the sim's own reasons
/// (`vehicle::powerslide::DriftEnd`, back to driving), a spin-out, and anything else (the mask,
/// a weapon).
pub const DRIFT_ENDS: [&str; 7] = ["switch", "slow", "wall", "brake", "button", "spin", "other"];

fn drift_end(driver: &vehicle::Driver) -> usize {
    use vehicle::powerslide::DriftEnd;
    match driver.kart_state {
        vehicle::kart_state::NORMAL => match driver.last_drift_end {
            Some(DriftEnd::SwitchWayLimit) => 0,
            Some(DriftEnd::TooSlow) => 1,
            Some(DriftEnd::WallScrape) => 2,
            Some(DriftEnd::Brake) => 3,
            Some(DriftEnd::ButtonReleased) => 4,
            None => 6,
        },
        vehicle::kart_state::SPINNING => 5,
        _ => 6,
    }
}

/// What a step did.
#[derive(Clone, Debug)]
pub struct Step {
    /// Scaled ([`Rules::reward_scale`]).
    pub reward: f32,
    /// The episode was cut at this step: the env has been reset, and this is the last observation
    /// of the old one (to take the critic's value of).
    pub cut: Option<(Done, Obs, Episode)>,
}

pub struct Env<'a> {
    tracks: &'a [Track],
    pub rules: Rules,
    rng: Rng,
    track: usize,
    path: usize,
    race: Race,
    tracker: Tracker,
    latch: Latch,
    progress: Progress,
    episode: Episode,
}

impl<'a> Env<'a> {
    pub fn new(tracks: &'a [Track], rules: Rules, seed: u64) -> Env<'a> {
        let first = &tracks[0];
        let race = Race::new(&first.level, first.objects.clone(), &[spec(0)], false);
        let mut env = Env {
            tracks,
            rules,
            rng: Rng::new(seed),
            track: 0,
            path: 0,
            race,
            tracker: Tracker::default(),
            latch: Latch::default(),
            progress: Progress::default(),
            episode: Episode::default(),
        };
        env.reset();
        env
    }

    fn reset(&mut self) {
        loop {
            self.track = self.rng.below(self.tracks.len());
            let track = &self.tracks[self.track];
            if track.paths.is_empty() {
                continue;
            }
            self.path = track.paths[self.rng.below(track.paths.len())];
            let frames = &track.level.nav_paths[self.path].frames;
            let spec = spec(self.rng.below(vehicle::stats::CHARACTER_ENGINES.len()));
            let mut race = Race::new(&track.level, track.objects.clone(), &[spec], false);
            race.laps = race::FREE_DRIVE_LAPS;
            if !race.place_on_path(&track.level, 0, &spec, self.path, self.rng.below(frames.len())) {
                continue;
            }
            self.race = race;
            let driver = &self.race.karts[0].driver;
            self.tracker = Tracker::find(frames, driver.pos_curr);
            self.progress = Progress::new(frames, &track.lengths[self.path], &self.tracker, driver);
            self.latch = Latch::default();
            self.episode = Episode::default();
            return;
        }
    }

    pub fn observe(&self) -> Obs {
        let frames = &self.tracks[self.track].level.nav_paths[self.path].frames;
        nn::observe(frames, &self.tracker, &self.progress, &self.race.karts[0], &self.latch)
    }

    pub fn step(&mut self, action: Action) -> Step {
        let track = &self.tracks[self.track];
        let frames = &track.level.nav_paths[self.path].frames;
        let lengths = &track.lengths[self.path];
        let mut reward = 0.0;
        self.episode.choices += 1;
        for _ in 0..self.rules.repeat {
            let pressed = self.latch.r1;
            let pad: Pad = self.latch.pad(action);
            let (was_drifting, attempted) = {
                let d = &self.race.karts[0].driver;
                (d.kart_state == vehicle::kart_state::DRIFTING, d.num_boosts_attempted)
            };
            self.race.tick(&track.level, &[pad]);
            let driver = &self.race.karts[0].driver;
            let drifting = driver.kart_state == vehicle::kart_state::DRIFTING;
            let relocated = nn::track(&mut self.tracker, frames, driver);
            let step = nn::Step {
                steer_change: self.latch.steer_change,
                ..self.progress.step(frames, lengths, &self.tracker, relocated, driver)
            };
            let r = self.rules.weights.reward(&step);
            reward += r;
            let e = &mut self.episode;
            e.reward += r;
            e.ticks += 1;
            e.progress += step.progress;
            e.grabs += step.grabbed as u32;
            e.boosts += step.boosts as u32;
            // A tap is one try (`powerslide::update`), and the meter running out takes all three.
            let tries = driver.num_boosts_attempted as i32 - attempted as i32;
            if drifting && !was_drifting {
                e.drifts += 1;
            } else if was_drifting && !drifting {
                e.drift_ends[drift_end(driver)] += 1;
            } else if drifting && was_drifting {
                e.early_taps += (tries == 1 && step.boosts == 0) as u32;
                e.meter_outs += (tries >= 3) as u32;
            }
            e.drifting += (driver.kart_state == vehicle::kart_state::DRIFTING) as u32;
            e.r1 += (pad.held & vehicle::buttons::R1 != 0) as u32;
            e.hops += (!pressed && pad.held & vehicle::buttons::R1 != 0) as u32;
            // (The ones `Step::air_misalign` counts.)
            e.air_ticks += (self.progress.airborne > nn::AIR_GRACE_TICKS) as u32;
            e.air_misalign += step.air_misalign;
            e.steer_change += step.steer_change as u32;
        }
        let e = self.episode;
        let done = if self.progress.stalled >= self.rules.stuck_ticks {
            Some(Done::Stuck)
        } else if e.ticks >= self.rules.max_ticks {
            Some(Done::Time)
        } else if e.grabs >= self.rules.max_grabs {
            Some(Done::Grabs)
        } else {
            None
        };
        if done == Some(Done::Stuck) {
            reward -= self.rules.stuck_penalty;
            self.episode.reward -= self.rules.stuck_penalty;
        }
        let e = self.episode;
        let cut = done.map(|done| {
            let last = self.observe();
            self.reset();
            (done, last, e)
        });
        Step {
            reward: reward * self.rules.reward_scale,
            cut,
        }
    }
}

/// A kart of `character`, driven by the pads it is given.
fn spec(character: usize) -> KartSpec {
    KartSpec {
        character,
        anim_num_frames: [0; 8],
        bot: false,
        line: None,
    }
}
