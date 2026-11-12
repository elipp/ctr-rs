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
use race::rev::{Rev, Skill};
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
    /// The share of the episodes that start before a jump (`Track::jumps`), this far up the path
    /// from it (level units): a run-up to near the top speed, as a kart standing still at the lip
    /// does not fly. Long flights are otherwise a sliver of the driving.
    pub jump_starts: f32,
    pub run_up: i64,
    /// The share of the episodes that start turned by a random angle (up to backwards): facing a
    /// wall, or the wrong way, to practise getting out. A stuck kart is otherwise rare, and cut
    /// soon, and a policy that has hardly seen one does nothing at all when it is.
    pub turned_starts: f32,
    /// The share of the episodes that start [`Rules::run_up`] before one of the spots the env's
    /// kart was grabbed at lately (the last [`GRAB_SPOTS`]): practice where the policy fails,
    /// on whatever track, without a list of them. A spot it no longer falls at drops out.
    pub grab_starts: f32,
    /// The share of the episodes that start [`Rules::run_up`] before a ramp lip (a gap's, as
    /// [`LipStats`] count them), picked by how often the flights from it fail, over all the envs
    /// lately ([`Env::set_lip_odds`]). Before any are known, these are jump starts.
    pub lip_starts: f32,
    pub weights: Weights,
    /// The reward the trainer sees is the sum of [`Rules::weights`]' over the held ticks, times
    /// this (for the critic's targets to be of a size of 1 or so).
    pub reward_scale: f32,
}

impl Default for Rules {
    fn default() -> Rules {
        Rules {
            repeat: race::nn::REPEAT,
            // A minute at 30 ticks a second.
            max_ticks: 30 * 60,
            max_grabs: 3,
            stuck_ticks: nn::STALL_TICKS,
            // As much as a mask grab.
            stuck_penalty: 30.0,
            jump_starts: 0.25,
            run_up: 1200,
            turned_starts: 0.15,
            grab_starts: 0.15,
            lip_starts: 0.2,
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


/// A ramp lip of a track: (the level's id, the bot path, the lip's frame).
pub type LipKey = (u32, usize, usize);

/// How the flights from one lip went.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LipStats {
    pub takeoffs: u32,
    /// With a hop (`Step::gap_hop`).
    pub hopped: u32,
    pub made: u32,
    pub short: u32,
    pub fell: u32,
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
    /// The sum of `Step::boost_timing` over them: how late in the window each was tapped.
    pub boost_timing: f32,
    /// R1 pressed (a hop, when the kart can).
    pub hops: u32,
    /// Ticks spent drifting, and with R1 held.
    pub drifting: u32,
    pub r1: u32,
    /// Ticks in a flight from a gap's lip, the sum of `Step::air_misalign` over them (the turns
    /// away from the landing's heading), and the ones of them more than 45° off it.
    pub air_ticks: u32,
    pub air_misalign: f32,
    /// The sum of `Step::air_steer` over those ticks.
    pub air_steer: f32,
    pub air_askew: u32,
    /// Takeoffs from a gap's lip, and the ones that made it (`Step::gap_takeoff`, `gap_cleared`).
    pub gap_flights: u32,
    pub gaps_cleared: u32,
    /// The takeoffs from a gap's lip that came off a hop (`Step::gap_hop`).
    pub gap_hops: u32,
    /// The sum of `Step::gap_apex` (level units, by the takeoff's aim): one a flight.
    pub gap_apex: f32,
    /// How the other flights from a gap's lip ended (`Step::gap_short`, `gap_fell`), and the sums
    /// of the takeoff speeds, as shares of the top speed, of the ones that made it, fell short,
    /// and fell.
    pub gaps_short: u32,
    pub gaps_fell: u32,
    pub speed_made: f32,
    pub speed_short: f32,
    pub speed_fell: f32,
    /// It started before a jump ([`Rules::jump_starts`]), or a spot of a grab
    /// ([`Rules::grab_starts`]).
    pub jump_start: bool,
    pub grab_start: bool,
    /// It started before a ramp lip ([`Rules::lip_starts`]).
    pub lip_start: bool,
    /// Times a stuck kart tried another path (`Progress::switch_due`).
    pub switches: u32,
    /// Ticks against a wall, and braking (`Step::braking`).
    pub wall: u32,
    pub braking: u32,
    /// The progress made in the air that mask grabs took back (`Step::taken_back`).
    pub taken_back: i64,
    /// The reserves lost (`Step::reserves_lost`), in milliseconds.
    pub reserves_lost: i64,
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
    /// The gas while the engine revs (after a mask grab), not the policy's: as on [`REV_SKILL`].
    rev: Rev,
    progress: Progress,
    episode: Episode,
    /// Where the kart was grabbed lately, as (track, path, frame): the frame it last had the
    /// ground under it at, for a fall (the takeoff, not where it came down).
    spots: std::collections::VecDeque<(usize, usize, usize)>,
    /// The tracker's frame the last tick the kart touched the ground.
    ground_frame: usize,
    /// The ticks since R1 was last pressed (not held): a hop.
    since_hop: u32,
    /// How the flights from each lip went since [`Env::take_lips`].
    lips: std::collections::BTreeMap<LipKey, LipStats>,
    /// The lips [`Rules::lip_starts`] are before, and their weights ([`Env::set_lip_odds`]).
    lip_odds: std::sync::Arc<Vec<(LipKey, f32)>>,
}

/// How the kart revs as the mask lowers it, in training: as the bots on hard.
const REV_SKILL: Skill = Skill::of(vehicle::bot::difficulty_level::HARD);

/// How many grab spots an env keeps ([`Rules::grab_starts`]).
pub const GRAB_SPOTS: usize = 32;

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
            rev: Rev::new(REV_SKILL),
            progress: Progress::default(),
            episode: Episode::default(),
            spots: std::collections::VecDeque::with_capacity(GRAB_SPOTS),
            ground_frame: 0,
            since_hop: u32::MAX,
            lips: Default::default(),
            lip_odds: Default::default(),
        };
        env.reset();
        env
    }

    /// How the flights from each ramp lip went since the last call, and starts the count over.
    pub fn take_lips(&mut self) -> std::collections::BTreeMap<LipKey, LipStats> {
        std::mem::take(&mut self.lips)
    }

    /// Sets the lips [`Rules::lip_starts`] are before, each by its weight (any scale).
    pub fn set_lip_odds(&mut self, odds: std::sync::Arc<Vec<(LipKey, f32)>>) {
        self.lip_odds = odds;
    }

    /// A lip of [`Env::lip_odds`], by their weights, as (track, path, frame): None if there are
    /// none, or the one picked is not on a track or path the env drives.
    fn pick_lip(&mut self) -> Option<(usize, usize, usize)> {
        let total: f32 = self.lip_odds.iter().map(|(_, w)| w.max(0.0)).sum();
        if total <= 0.0 {
            return None;
        }
        let mut u = self.rng.unit() * total;
        let &((level, path, lip), _) = self
            .lip_odds
            .iter()
            .find(|(_, w)| {
                u -= w.max(0.0);
                u < 0.0
            })
            .unwrap_or(self.lip_odds.last()?);
        let track = self.tracks.iter().position(|t| t.id.0 == level)?;
        self.tracks[track].paths.contains(&path).then_some((track, path, lip))
    }

    /// A track, by their weights (`Track::weight`).
    fn pick_track(&mut self) -> usize {
        let total: f32 = self.tracks.iter().map(|t| t.weight.max(0.0)).sum();
        if total <= 0.0 {
            return self.rng.below(self.tracks.len());
        }
        let mut u = self.rng.unit() * total;
        for (i, t) in self.tracks.iter().enumerate() {
            u -= t.weight.max(0.0);
            if u < 0.0 {
                return i;
            }
        }
        self.tracks.len() - 1
    }

    fn reset(&mut self) {
        let any_jumps = self.tracks.iter().any(|t| !t.jumps.is_empty());
        loop {
            let u = self.rng.unit();
            let r = self.rules;
            let lip_share = u < r.lip_starts;
            let lip = if lip_share { self.pick_lip() } else { None };
            let lip_start = lip.is_some();
            // (A lip start without a lip known is a jump start.)
            let jump_start = !lip_start
                && any_jumps
                && (lip_share || (r.lip_starts..r.lip_starts + r.jump_starts).contains(&u));
            let grab_start = !lip_start
                && !jump_start
                && !self.spots.is_empty()
                && u >= r.lip_starts + r.jump_starts
                && u < r.lip_starts + r.jump_starts + r.grab_starts;
            let spot = if grab_start {
                Some(self.spots[self.rng.below(self.spots.len())])
            } else {
                lip
            };
            self.track = match spot {
                Some((track, _, _)) => track,
                None => self.pick_track(),
            };
            let track = &self.tracks[self.track];
            if track.paths.is_empty() || (jump_start && track.jumps.is_empty()) {
                continue;
            }
            let frame = if let Some((_, path, at)) = spot {
                self.path = path;
                run_up_frame(&track.lengths[path], at, self.rules.run_up)
            } else if jump_start {
                let (path, lip) = track.jumps[self.rng.below(track.jumps.len())];
                self.path = path;
                run_up_frame(&track.lengths[path], lip, self.rules.run_up)
            } else {
                self.path = track.paths[self.rng.below(track.paths.len())];
                self.rng.below(track.level.nav_paths[self.path].frames.len())
            };
            let frames = &track.level.nav_paths[self.path].frames;
            let spec = spec(self.rng.below(vehicle::stats::CHARACTER_ENGINES.len()));
            let mut race = Race::new(&track.level, track.objects.clone(), &[spec], false);
            race.laps = race::FREE_DRIVE_LAPS;
            // (Not before a jump, a lip or a grab's spot: those are for getting over it.)
            let turn = if !jump_start && !grab_start && !lip_start && self.rng.unit() < self.rules.turned_starts
            {
                self.rng.below(0x1000) as i32 - 0x800
            } else {
                0
            };
            if !race.place_on_path(&track.level, 0, &spec, self.path, frame, turn) {
                continue;
            }
            self.race = race;
            let driver = &self.race.karts[0].driver;
            self.tracker = Tracker::find(frames, driver.pos_curr);
            self.progress = Progress::new(frames, &track.lengths[self.path], &self.tracker, driver);
            self.latch = Latch::default();
            self.rev = Rev::new(REV_SKILL);
            self.since_hop = u32::MAX;
            self.ground_frame = self.tracker.frame as usize;
            self.episode = Episode {
                jump_start,
                grab_start,
                lip_start,
                ..Episode::default()
            };
            return;
        }
    }

    pub fn observe(&self) -> Obs {
        let frames = &self.tracks[self.track].level.nav_paths[self.path].frames;
        nn::observe(frames, &self.tracker, &self.progress, &self.race.karts[0], &self.latch)
    }

    pub fn step(&mut self, action: Action) -> Step {
        let tracks = self.tracks;
        let track = &tracks[self.track];
        let mut reward = 0.0;
        self.episode.choices += 1;
        for _ in 0..self.rules.repeat {
            // (The path may change from tick to tick: a stuck kart tries another.)
            let frames = &track.level.nav_paths[self.path].frames;
            let lengths = &track.lengths[self.path];
            let pressed = self.latch.r1;
            let (was_drifting, attempted, fast) = {
                let d = &self.race.karts[0].driver;
                (
                    d.kart_state == vehicle::kart_state::DRIFTING,
                    d.num_boosts_attempted,
                    (d.speed_approx as i32) * 4 > d.stats.speed_class_stat as i32,
                )
            };
            let air_steer = nn::air_steer(&self.progress, &self.race.karts[0].driver);
            let mut pad: Pad = self.latch.pad(action, was_drifting, attempted > 0, air_steer);
            if let Some(gas) = self.rev.gas(&self.race, 0, &mut self.rng) {
                pad = race::rev::with_gas(pad, gas);
            }
            let braking =
                fast && pad.held & (vehicle::buttons::SQUARE | vehicle::buttons::DOWN) != 0;
            self.since_hop = if !pressed && pad.held & vehicle::buttons::R1 != 0 {
                0
            } else {
                self.since_hop.saturating_add(1)
            };
            self.race.tick(&track.level, &[pad]);
            let driver = &self.race.karts[0].driver;
            let drifting = driver.kart_state == vehicle::kart_state::DRIFTING;
            let relocated = nn::track(&mut self.tracker, frames, driver);
            let mut step = nn::Step {
                // (The script's stick in a gap flight is none of the policy's doing.)
                steer_change: if air_steer.is_some() { 0 } else { self.latch.steer_change },
                braking,
                ..self.progress.step(frames, lengths, &self.tracker, relocated, driver)
            };
            step.gap_hop = step.gap_takeoff && !drifting && self.since_hop <= nn::GAP_HOP_TICKS;
            if let Some(lip) = step.gap_lip {
                let s = self.lips.entry((track.id.0, self.path, lip)).or_default();
                s.takeoffs += step.gap_takeoff as u32;
                s.hopped += step.gap_hop as u32;
                s.made += step.gap_cleared as u32;
                s.short += step.gap_short as u32;
                s.fell += step.gap_fell as u32;
            }
            let r = self.rules.weights.reward(&step);
            reward += r;
            let e = &mut self.episode;
            e.reward += r;
            e.ticks += 1;
            e.progress += step.progress;
            e.grabs += step.grabbed as u32;
            if driver.actions_flag_set & vehicle::actions::TOUCH_GROUND != 0 {
                self.ground_frame = self.tracker.frame as usize;
            }
            if step.grabbed {
                if self.spots.len() == GRAB_SPOTS {
                    self.spots.pop_front();
                }
                self.spots.push_back((self.track, self.path, self.ground_frame));
            }
            e.boosts += step.boosts as u32;
            e.boost_timing += step.boost_timing;
            e.wall += step.wall as u32;
            e.braking += step.braking as u32;
            e.taken_back += step.taken_back;
            e.reserves_lost += step.reserves_lost as i64;
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
            e.air_ticks += (self.progress.airborne > 0 && self.progress.flight_heading.is_some()) as u32;
            e.air_misalign += step.air_misalign;
            // (The stick in a gap flight is scripted (`nn::air_steer`): no cost to the policy,
            // only how much it took, while not yet closing on the heading.)
            if self.progress.in_gap_flight() && !step.air_closing {
                e.air_steer += self.latch.stick();
            }
            // (0x200 of 0x1000 to a turn: 45°.)
            e.air_askew +=
                (self.progress.flight_heading.is_some() && self.progress.air_error > 0x200) as u32;
            e.gap_flights += step.gap_takeoff as u32;
            e.gaps_cleared += step.gap_cleared as u32;
            e.gap_hops += step.gap_hop as u32;
            e.gap_apex += step.gap_apex;
            e.gaps_short += step.gap_short as u32;
            e.gaps_fell += step.gap_fell as u32;
            let share = step.gap_speed as f32 / (driver.stats.speed_class_stat as f32).max(1.0);
            if step.gap_cleared {
                e.speed_made += share;
            } else if step.gap_short {
                e.speed_short += share;
            } else if step.gap_fell {
                e.speed_fell += share;
            }
            e.steer_change += step.steer_change as u32;
            if self.progress.switch_due(driver)
                && let Some((path, tracker)) =
                    nn::next_path_of(&track.level, self.path, driver.pos_curr, &track.paths)
            {
                self.path = path;
                self.tracker = tracker;
                let frames = &track.level.nav_paths[path].frames;
                self.progress.switched(frames, &track.lengths[path], &self.tracker, driver);
                e.switches += 1;
            }
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

/// The frame `run_up` (level units, along the path) before frame `lip`, going back round the
/// loop.
fn run_up_frame(lengths: &race::bot::PathLength, lip: usize, run_up: i64) -> usize {
    let n = lengths.start.len();
    let mut frame = lip;
    for _ in 0..n {
        if lengths.delta(lengths.start[frame], lengths.start[lip]) >= run_up {
            break;
        }
        frame = (frame + n - 1) % n;
    }
    frame
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
