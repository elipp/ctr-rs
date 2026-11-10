//! What a learned bot (the `brain` crate) sees of the race, and how its choices become a pad: the
//! one definition of both, for the trainer and the host that runs it alike.
//!
//! Unlike [`crate::bot`], none of this is part of the sim. A learned bot's pad is made on the host
//! alone and goes to the others like a player's, so it may be floats: nothing here has to come out
//! the same on another machine.
//!
//! - [`observe`]: the kart and the path ahead of it, in the kart's own frame, about ±1. Only what
//!   a player sees: where the road goes and the kart's own state. Nothing of how to drive it (the
//!   path's turbo, ramp and drift flags, the hop's timers), for the bot to find out for itself.
//! - [`Action`] and [`Latch::pad`]: a choice from each of [`HEADS`], and the pad of it (a long
//!   hold of R1 goes on over the choices after it).
//! - [`Progress`]: how far along its path a kart got in a tick, and whether it fell off, for the
//!   reward ([`Weights::reward`]). Being on the line is worth nothing in itself: the line is only
//!   the way round.

use loader::lev::{Level, NavFrame};
use math::Vec3;
use vehicle::{Driver, actions, buttons, kart_state};

use crate::bot::{PathLength, Tracker};
use crate::{Kart, Pad};

/// How far ahead along the path (level units, on the ground plane) the points the bot sees are.
/// A kart at its top speed does about 50 a tick.
pub const AHEAD: [i32; 8] = [100, 200, 350, 550, 800, 1100, 1500, 2000];

/// The `kart_state`s told apart in [`observe`]; any other is the last entry.
const STATES: [u8; 6] = [
    kart_state::NORMAL,
    kart_state::DRIFTING,
    kart_state::SPINNING,
    kart_state::CRASHING,
    kart_state::MASK_GRABBED,
    kart_state::BLASTED,
];

/// The length of an observation.
pub const OBS_LEN: usize = AHEAD.len() * 3 // the points ahead
    + 3 // off the line: sideways, and the path's way against the kart's (sin, cos)
    + 4 // speed, forward and sideways velocity, turning rate
    + 1 // airborne
    + STATES.len() + 1
    + 4 // drift: its turn, the boost meter, boost tries left, reserves
    + 4 // the buttons held last tick: gas, reverse, hop, boost
    + 1; // the ticks a long hold of R1 has left ([`Latch`])

pub type Obs = [f32; OBS_LEN];

/// The units (level units) the distances of an observation are in.
const DISTANCE_SCALE: f32 = 1000.0;

/// `Driver::angle` units in radians.
const ANGLE_TO_RAD: f32 = std::f32::consts::TAU / 4096.0;

/// The kart's frame: `pos` (in `Driver::pos_curr` units) heading `angle`.
struct Frame {
    x: f32,
    y: f32,
    z: f32,
    sin: f32,
    cos: f32,
}

impl Frame {
    fn of(pos: Vec3, angle: i32) -> Frame {
        let a = angle as f32 * ANGLE_TO_RAD;
        Frame {
            x: pos.x as f32 / 256.0,
            y: pos.y as f32 / 256.0,
            z: pos.z as f32 / 256.0,
            sin: a.sin(),
            cos: a.cos(),
        }
    }

    /// `v` (a direction, in any units) as (forward, left). The kart goes toward (sin, cos) in x
    /// and z, and its left (a quarter turn up) is (cos, -sin).
    fn turn(&self, x: f32, z: f32) -> (f32, f32) {
        (x * self.sin + z * self.cos, x * self.cos - z * self.sin)
    }

    /// The point `p` (in `Driver::pos_curr` units) as (forward, left, up), in level units.
    fn local(&self, p: Vec3) -> (f32, f32, f32) {
        let (dx, dz) = (p.x as f32 / 256.0 - self.x, p.z as f32 / 256.0 - self.z);
        let (f, l) = self.turn(dx, dz);
        (f, l, p.y as f32 / 256.0 - self.y)
    }
}

/// Fills an observation in order, so that [`observe`] can check it filled all of it.
struct Fill {
    obs: Obs,
    at: usize,
}

impl Fill {
    fn push(&mut self, v: f32) {
        self.obs[self.at] = v;
        self.at += 1;
    }

    fn flag(&mut self, b: bool) {
        self.push(b as u8 as f32);
    }
}

/// What the bot of `kart` sees, it steering by `path` and being at `tracker` on it (kept up with
/// [`track`]), and its pads coming from `latch`.
pub fn observe(path: &[NavFrame], tracker: &Tracker, kart: &Kart, latch: &Latch) -> Obs {
    let d = &kart.driver;
    let pos = d.pos_curr;
    let frame = Frame::of(pos, d.angle as i32);
    let mut o = Fill {
        obs: [0.0; OBS_LEN],
        at: 0,
    };

    for distance in AHEAD {
        let (point, _) = tracker.ahead(path, pos, distance);
        let (f, l, u) = frame.local(point);
        o.push(f / DISTANCE_SCALE);
        o.push(l / DISTANCE_SCALE);
        o.push(u / DISTANCE_SCALE);
    }

    let (offset, heading) = tracker.offset(path, pos);
    o.push(offset as f32 / DISTANCE_SCALE);
    let error = (heading - d.angle as i32) as f32 * ANGLE_TO_RAD;
    o.push(error.sin());
    o.push(error.cos());

    // Speeds against the class's top speed (`speed_approx`, and `velocity` a tick in
    // `pos_curr` units are of a size).
    let top = (d.stats.speed_class_stat as f32).max(1.0);
    o.push(d.speed_approx as f32 / top);
    let (forward, left) = frame.turn(d.velocity.x as f32, d.velocity.z as f32);
    o.push(forward / top);
    o.push(left / top);
    o.push(d.amp_turn_state as f32 / 4096.0);

    o.flag(d.actions_flag_set & actions::TOUCH_GROUND == 0);
    let state = STATES.iter().position(|&s| s == d.kart_state);
    for i in 0..STATES.len() {
        o.flag(state == Some(i));
    }
    o.flag(state.is_none());

    // The boost meter fills from `turbo_max_room << 5` (`powerslide`'s FRAME_TIME_SHIFT) down.
    let room = ((d.stats.turbo_max_room as i32) << 5).max(1) as f32;
    o.push(d.mult_drift as f32 / 4096.0);
    let drifting = d.kart_state == kart_state::DRIFTING;
    o.push(if drifting {
        d.turbo_meter_room_left as f32 / room
    } else {
        0.0
    });
    // A drift has three tries at a boost (`DRIFT_MAX_BOOSTS`), a failed one too, and the meter
    // running out takes all that are left.
    o.push(if drifting {
        (3 - d.num_boosts_attempted as i32).clamp(0, 3) as f32 / 3.0
    } else {
        0.0
    });
    o.push((d.reserves as f32 / 2000.0).min(2.0));

    o.flag(kart.held & buttons::CROSS != 0);
    o.flag(kart.held & buttons::SQUARE != 0);
    o.flag(kart.held & buttons::R1 != 0);
    o.flag(kart.held & buttons::L1 != 0);
    o.push(latch.hold_left as f32 / HOLD_TICKS as f32);

    debug_assert_eq!(o.at, OBS_LEN, "an observation is filled in full");
    o.obs
}

/// Moves `tracker` along with the kart of `driver` as [`crate::bot::Bot::pad`] does: in the air
/// only stepping along (it arcs high over the path), on the ground searching for it when lost.
/// Returns whether it was searched for (a respawn, knocked off): the progress of that tick is no
/// driving.
pub fn track(tracker: &mut Tracker, path: &[NavFrame], driver: &Driver) -> bool {
    if driver.actions_flag_set & actions::TOUCH_GROUND == 0 {
        tracker.follow(path, driver.pos_curr);
        false
    } else {
        tracker.update(path, driver.pos_curr)
    }
}

/// How many choices each head of the policy has: the steering, the throttle, the hop, the boost.
pub const HEADS: [usize; N_HEADS] = [STEER_BINS, 3, 3, 2];

pub const N_HEADS: usize = 4;

/// The positions of the stick: full left to full right, the middle one centred.
pub const STEER_BINS: usize = 9;

/// The steering's strength (out of [`STEER_STEPS`]) a bin [`STEER_BINS`] / 2 away from the
/// middle has: the stick is the least that gives it, as `Steering::even_strength` picks.
const STEER_STEPS: i32 = (STEER_BINS / 2) as i32;

/// What a learned bot does for a tick.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Action {
    /// `0..STEER_BINS`: below the middle is left (raising `Driver::angle`).
    pub steer: u8,
    pub throttle: Throttle,
    /// R1: the hop, and held, the drift.
    pub hop: Hop,
    /// L1: also a hop, but in a drift held with R1, its tap turns the drift's meter into a boost
    /// (`vehicle::powerslide::update`).
    pub boost: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Throttle {
    /// Cross.
    #[default]
    Gas,
    Coast,
    /// Square: brakes, then backs up.
    Reverse,
}

impl Throttle {
    const ALL: [Throttle; 3] = [Throttle::Gas, Throttle::Coast, Throttle::Reverse];
}

/// What R1 does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Hop {
    #[default]
    Release,
    /// Held for this choice: a choice's held and then released is a tap, a hop.
    Hold,
    /// A fresh press (let go for a tick first, if it was held), and then held for
    /// [`HOLD_TICKS`] whatever the choices after it ([`Latch`]): a hop into a drift, held long
    /// enough for its meter to fill, which a hold choice after choice by chance hardly ever is.
    /// (A hold only ever starts a drift with the press: one held on and on starts none.)
    HoldLong,
}

impl Hop {
    const ALL: [Hop; 3] = [Hop::Release, Hop::Hold, Hop::HoldLong];
}

/// The ticks R1 stays held for after [`Hop::HoldLong`]. A drift's meter (`turbo_max_room`, 30
/// ticks) can be turned into a boost once under half (`turbo_low_room_warning`).
pub const HOLD_TICKS: u32 = 16;

/// What goes on of a bot's choices from tick to tick: a long hold of R1.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Latch {
    /// The ticks R1 is still held for, whatever is chosen.
    pub hold_left: u32,
    /// R1 was held on the last pad.
    pub r1: bool,
}

impl Latch {
    /// The pad of this tick, the bot having chosen `action` (this tick, or one before that it
    /// still holds).
    pub fn pad(&mut self, action: Action) -> Pad {
        let mut pad = action.pad();
        if action.hop == Hop::HoldLong && self.hold_left == 0 {
            self.hold_left = HOLD_TICKS;
            if self.r1 {
                // Let go for this tick, for the next to be a press.
                pad.held &= !buttons::R1;
                self.r1 = false;
                return pad;
            }
        }
        if self.hold_left > 0 {
            pad.held |= buttons::R1;
            self.hold_left -= 1;
        }
        self.r1 = pad.held & buttons::R1 != 0;
        pad
    }
}

const STICK_CENTER: i32 = 0x80;
const STICK_RANGE: i32 = 0x7f;
const STRENGTH_ONE: i32 = 0x1000;

/// How far from the centre the stick of each step of strength (`1..=STEER_STEPS`) is.
fn stick_off(step: i32) -> i32 {
    let want = step * STRENGTH_ONE / STEER_STEPS;
    (0..STICK_RANGE)
        .find(|&off| vehicle::joystick::strength(off, STRENGTH_ONE) >= want)
        .unwrap_or(STICK_RANGE)
}

impl Action {
    /// The choices of the heads ([`HEADS`] order).
    pub fn from_heads(choice: [usize; N_HEADS]) -> Action {
        Action {
            steer: choice[0].min(STEER_BINS - 1) as u8,
            throttle: Throttle::ALL[choice[1].min(2)],
            hop: Hop::ALL[choice[2].min(2)],
            boost: choice[3] != 0,
        }
    }

    pub fn heads(self) -> [usize; N_HEADS] {
        let throttle = Throttle::ALL.iter().position(|&t| t == self.throttle).unwrap_or(0);
        let hop = Hop::ALL.iter().position(|&h| h == self.hop).unwrap_or(0);
        [self.steer as usize, throttle, hop, self.boost as usize]
    }

    /// The pad of this choice alone: [`Hop::HoldLong`] holds R1 only for it (see [`Latch`]).
    pub fn pad(self) -> Pad {
        let signed = STEER_STEPS - self.steer as i32;
        let lx = STICK_CENTER - signed.signum() * stick_off(signed.abs());
        let throttle = match self.throttle {
            Throttle::Gas => buttons::CROSS,
            Throttle::Coast => 0,
            Throttle::Reverse => buttons::SQUARE,
        };
        Pad {
            held: throttle
                | if self.hop != Hop::Release { buttons::R1 } else { 0 }
                | if self.boost { buttons::L1 } else { 0 },
            lx: lx.clamp(0, 0xff) as u8,
            ..Pad::default()
        }
    }
}

/// The most progress a tick counts (level units): a kart at its top speed on a turbo pad does
/// about 75. More is a teleport.
pub const MAX_PROGRESS: i64 = 128;

/// What a tick did for a kart's progress along its path ([`Progress::step`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Step {
    /// How far along the path it went (level units, negative backwards), within
    /// ±[`MAX_PROGRESS`]; 0 in a tick it was searched for.
    pub progress: i64,
    /// It was grabbed by the mask this tick.
    pub grabbed: bool,
    /// The reserves (`Driver::reserves`, milliseconds of boost) it gained this tick: a drift's
    /// boost, a turbo pad, the start's.
    pub reserves_gained: i32,
}

/// Where a kart was along its path, to tell how far it went in a tick.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    pub arc: i64,
    pub kart_state: u8,
    pub reserves: i16,
}

impl Progress {
    pub fn new(path: &[NavFrame], lengths: &PathLength, tracker: &Tracker, driver: &Driver) -> Progress {
        Progress {
            arc: tracker.arc(path, lengths, driver.pos_curr),
            kart_state: driver.kart_state,
            reserves: driver.reserves,
        }
    }

    /// The tick just played, the tracker moved along (`relocated`: [`track`] searched for it).
    pub fn step(
        &mut self,
        path: &[NavFrame],
        lengths: &PathLength,
        tracker: &Tracker,
        relocated: bool,
        driver: &Driver,
    ) -> Step {
        let arc = tracker.arc(path, lengths, driver.pos_curr);
        let progress = if relocated {
            0
        } else {
            lengths
                .delta(self.arc, arc)
                .clamp(-MAX_PROGRESS, MAX_PROGRESS)
        };
        let grabbed = driver.kart_state == kart_state::MASK_GRABBED
            && self.kart_state != kart_state::MASK_GRABBED;
        self.arc = arc;
        self.kart_state = driver.kart_state;
        let reserves_gained = (driver.reserves as i32 - self.reserves as i32).max(0);
        self.reserves = driver.reserves;
        Step {
            progress,
            grabbed,
            reserves_gained,
        }
    }
}

/// How much each part of a [`Step`] is worth.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Weights {
    /// For each level unit of progress.
    pub progress: f32,
    /// For a mask grab (taken off), on top of whatever progress the respawn takes back (none if
    /// the tracker had to search for the kart, [`Step::progress`]).
    pub grabbed: f32,
    /// Each tick, whatever: a lap sooner is worth more.
    pub time: f32,
    /// For each millisecond of reserves gained: speed in the bank, which the progress pays for
    /// only once spent (and a drift without its boost costs some first).
    pub reserves: f32,
}

impl Weights {
    /// A tick at top speed (~50 units) along the path is about 1.
    pub const DEFAULT: Weights = Weights {
        progress: 1.0 / 50.0,
        grabbed: 30.0,
        time: 0.01,
        // A drift's best boost (`turbo_full_bar_reserve_gain << 5`, 1920 ms) is about 19:
        // as much as 19 ticks at top speed.
        reserves: 0.01,
    };

    pub fn reward(&self, step: &Step) -> f32 {
        step.progress as f32 * self.progress
            - if step.grabbed { self.grabbed } else { 0.0 }
            - self.time
            + step.reserves_gained as f32 * self.reserves
    }
}

impl Default for Weights {
    fn default() -> Weights {
        Weights::DEFAULT
    }
}

/// The lengths of each of the level's bot paths ([`PathLength::of`]), by `Level::nav_paths`.
pub fn path_lengths(level: &Level) -> Vec<PathLength> {
    level.nav_paths.iter().map(|p| PathLength::of(&p.frames)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_round_trip_through_heads() {
        for steer in 0..STEER_BINS as u8 {
            for throttle in Throttle::ALL {
                for (hop, boost) in Hop::ALL.into_iter().flat_map(|h| [(h, false), (h, true)]) {
                    let a = Action { steer, throttle, hop, boost };
                    let pad = a.pad();
                    assert_eq!(pad.held & buttons::CROSS != 0, throttle == Throttle::Gas);
                    assert_eq!(pad.held & buttons::SQUARE != 0, throttle == Throttle::Reverse);
                    assert_eq!(pad.held & buttons::R1 != 0, hop != Hop::Release);
                    assert_eq!(pad.held & buttons::L1 != 0, boost);
                    assert_eq!(Action::from_heads(a.heads()), a);
                }
            }
        }
    }

    #[test]
    fn a_long_hold_goes_on_over_the_choices_after_it() {
        let mut latch = Latch::default();
        let long = Action { hop: Hop::HoldLong, ..Action::default() };
        let release = Action::default();
        assert!(latch.pad(long).held & buttons::R1 != 0);
        for _ in 1..HOLD_TICKS {
            assert!(latch.pad(release).held & buttons::R1 != 0);
        }
        assert_eq!(latch.hold_left, 0);
        assert_eq!(latch.pad(release).held & buttons::R1, 0);
    }

    #[test]
    fn a_long_hold_of_a_held_button_starts_with_a_press() {
        let mut latch = Latch::default();
        let hold = Action { hop: Hop::Hold, ..Action::default() };
        let long = Action { hop: Hop::HoldLong, ..Action::default() };
        assert!(latch.pad(hold).held & buttons::R1 != 0);
        // Let go, then pressed and held on (the choice repeated, as the env holds it).
        assert_eq!(latch.pad(long).held & buttons::R1, 0);
        for _ in 0..HOLD_TICKS {
            assert!(latch.pad(long).held & buttons::R1 != 0);
        }
        assert_eq!(latch.hold_left, 0);
    }

    #[test]
    fn middle_bin_is_centred_and_low_bins_steer_left() {
        let pad = |steer| Action { steer, ..Action::default() }.pad();
        assert_eq!(pad(STEER_BINS as u8 / 2).lx, 0x80);
        assert!(pad(0).lx < 0x80);
        assert!(pad(STEER_BINS as u8 - 1).lx > 0x80);
        assert_eq!(pad(0).lx, 0x80 - stick_off(STEER_STEPS) as u8);
    }

    #[test]
    fn path_length_wraps_the_shorter_way() {
        let l = PathLength {
            start: vec![0, 100, 200, 300],
            total: 400,
        };
        assert_eq!(l.delta(390, 10), 20);
        assert_eq!(l.delta(10, 390), -20);
        assert_eq!(l.delta(100, 150), 50);
    }
}
