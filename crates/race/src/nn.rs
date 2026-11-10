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
//!   the way round. Two costs keep the driving tidy: facing away from the path in the air, and
//!   moving the stick from choice to choice.

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
    + 1 // the ticks a long hold of R1 has left ([`Latch`])
    + 1 // the stick of the last choice ([`Latch::steer`]), for what moving it costs
    + 1; // the ticks it has got no further along ([`Progress::stalled`])

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
/// [`track`]) and `progress` along it, and its pads coming from `latch`.
pub fn observe(
    path: &[NavFrame],
    tracker: &Tracker,
    progress: &Progress,
    kart: &Kart,
    latch: &Latch,
) -> Obs {
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

    // (The way the path goes over a stretch, not the segment's: see `path_heading`.)
    let (offset, _) = tracker.offset(path, pos);
    let heading = path_heading(path, tracker, pos);
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
    o.push((latch.steer as f32 - STEER_STEPS as f32) / STEER_STEPS as f32);
    o.push((progress.stalled as f32 / STALL_TICKS as f32).min(1.0));

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
    /// L1, in a drift only ([`Latch::pad`]): its tap turns the drift's meter into a boost
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

/// The ticks R1 stays held for after [`Hop::HoldLong`]: the hop (some 10 ticks in the air; the
/// drift starts as it lands) and then the whole of the drift's meter (`turbo_max_room`, 30
/// ticks), which can be turned into a boost once under half (`turbo_low_room_warning`): from
/// some 15 ticks into the drift to 30.
pub const HOLD_TICKS: u32 = 40;

/// What goes on of a bot's choices from tick to tick: a long hold of R1, and where the stick was.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Latch {
    /// The ticks R1 is still held for, whatever is chosen.
    pub hold_left: u32,
    /// R1 was held on the last pad.
    pub r1: bool,
    /// The steering bin of the last pad (`Action::steer`); centred to begin with.
    pub steer: u8,
    /// How many bins the stick moved for the last pad: on the first tick of a choice, the move
    /// from the one before; 0 while a choice is held.
    pub steer_change: u8,
}

impl Default for Latch {
    fn default() -> Latch {
        Latch {
            hold_left: 0,
            r1: false,
            steer: STEER_STEPS as u8,
            steer_change: 0,
        }
    }
}

impl Latch {
    /// The pad of this tick, the bot having chosen `action` (this tick, or one before that it
    /// still holds), its kart `drifting` as the tick starts.
    ///
    /// L1 is let through in a drift only. Out of one, a tap of L1 alone (even mid-hop, R1 held)
    /// makes L1 the button the next drift is held with (`vehicle::linear`), and that drift ends
    /// as soon as L1 is let go: by chance, a few ticks in, long before its meter can boost. A hop
    /// with L1 is no different from one with R1, so nothing is lost.
    pub fn pad(&mut self, action: Action, drifting: bool) -> Pad {
        self.steer_change = self.steer.abs_diff(action.steer);
        self.steer = action.steer;
        let mut pad = action.pad();
        if !drifting {
            pad.held &= !buttons::L1;
        }
        if action.hop == Hop::HoldLong && self.hold_left == 0 {
            self.hold_left = HOLD_TICKS;
            // In a drift, R1 holds it: letting go would end it. Out of one, let go for this
            // tick, for the next to be a press.
            if self.r1 && !drifting {
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

/// After this many ticks without getting further along its path ([`Progress::stalled`]), a kart
/// is stuck: [`observe`] sees it as 1 from here on, and the trainer gives up on it.
pub const STALL_TICKS: u32 = 30 * 5;

/// How far along the path (level units, on the ground plane) [`path_heading`] takes the way it
/// goes over.
const HEADING_SPAN: i32 = 500;

/// The way the path goes where the kart at `pos` is level with it (as `Driver::angle`): from that
/// point to the one [`HEADING_SPAN`] further on, on the ground plane. Not the way of the segment
/// it is on (`Tracker::offset`): over a gap the path falls straight down off the lip into the
/// kill plane, and the segments of the drop are a few units long on the ground plane, their way
/// any; an airborne kart's tracker only steps along (`track`), and sits on them a good part of
/// the flight. [`Tracker::ahead`] steps over them, and so this goes from the lip to the landing.
pub fn path_heading(path: &[NavFrame], tracker: &Tracker, pos: Vec3) -> i32 {
    let (a, _) = tracker.ahead(path, pos, 0);
    let (b, _) = tracker.ahead(path, pos, HEADING_SPAN);
    math::ratan2((b.x - a.x) >> 8, (b.z - a.z) >> 8)
}

/// Ticks in the air before [`Step::air_misalign`] counts: a hop (about 10) is free, as a drift
/// starts with one, steered, which turns the kart in the air; a long jump (30 to 60) is not. A
/// turn made in these ticks still costs for the rest of the flight.
pub const AIR_GRACE_TICKS: u32 = 15;

/// Ticks without getting further along before [`Step::stalled`]: a back-up or a turn round is
/// free.
pub const STALL_GRACE_TICKS: u32 = 30;

/// What a tick did for a kart's progress along its path ([`Progress::step`]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Step {
    /// How far along the path it went (level units, negative backwards), within
    /// ±[`MAX_PROGRESS`]; 0 in a tick it was searched for, or carried by the mask; and at a mask
    /// grab, less what it made in the air before ([`Step::taken_back`]).
    pub progress: i64,
    /// It was grabbed by the mask this tick.
    pub grabbed: bool,
    /// The reserves (`Driver::reserves`, milliseconds of boost) it gained this tick: a drift's
    /// boost, the start's. Not a turbo pad's, which pays in speed (the progress) already, and
    /// tops the reserves up each tick a kart stays on it: a bonus for those pays it to park
    /// there.
    pub reserves_gained: i32,
    /// The progress made in the air that a mask grab took back (level units, out of
    /// [`Step::progress`]): see [`Progress::air_progress`]. 0 but at a grab.
    pub taken_back: i64,
    /// It has got no further along for more than [`STALL_GRACE_TICKS`] ([`Progress::stalled`]).
    pub stalled: bool,
    /// In the air (and not carried by the mask) for more than [`AIR_GRACE_TICKS`], how far the
    /// kart faces away from the path: `1 - cos` of the angle, 0 straight along it, 1 sideways, 2
    /// backwards. 0 on the ground, where a drift faces away on purpose, and in a hop.
    pub air_misalign: f32,
    /// The kart scraped a wall (`DRIVING_AGAINST_WALL`).
    pub wall: bool,
    /// Drift boosts fired this tick: taps of L1 in time, each of a drift's three
    /// (`Driver::num_boosts_success`).
    pub boosts: u8,
    /// How many steering bins the stick moved for this tick's pad ([`Latch::steer_change`]).
    /// [`Progress::step`] knows nothing of the pads: the caller puts it in.
    pub steer_change: u8,
}

/// Where a kart was along its path, to tell how far it went in a tick.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    pub arc: i64,
    pub kart_state: u8,
    pub reserves: i16,
    /// The progress since [`Progress::new`] (the sum of [`Step::progress`]), and the most it has
    /// been.
    pub total: i64,
    pub furthest: i64,
    /// The ticks since `total` last went past `furthest`: getting nowhere (stuck, or the wrong
    /// way). A mask grab or a relocation starts it over from where the kart is.
    pub stalled: u32,
    /// The ticks it has been in the air for, on end (not counting the mask's carry).
    pub airborne: u32,
    /// `Driver::num_boosts_success` (the drift's boosts so far; 0 out of a drift).
    pub boosts_success: i8,
    /// The progress made since the kart last touched the ground. The progress is measured on the
    /// ground plane (`Tracker::arc`), so a kart falling off the edge at speed, far below the
    /// road, goes on making it; the mask then puts it back behind, which is a search (no
    /// progress, [`track`]). A grab takes this back, so that a fall is worth none of the way it
    /// flew (a jump that lands keeps it).
    pub air_progress: i64,
}

impl Progress {
    pub fn new(path: &[NavFrame], lengths: &PathLength, tracker: &Tracker, driver: &Driver) -> Progress {
        Progress {
            arc: tracker.arc(path, lengths, driver.pos_curr),
            kart_state: driver.kart_state,
            reserves: driver.reserves,
            total: 0,
            furthest: 0,
            stalled: 0,
            airborne: 0,
            boosts_success: driver.num_boosts_success,
            air_progress: 0,
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
        let grabbed = driver.kart_state == kart_state::MASK_GRABBED
            && self.kart_state != kart_state::MASK_GRABBED;
        // Carried by the mask is no driving either (it takes the kart back over the way it fell).
        let carried = driver.kart_state == kart_state::MASK_GRABBED && !grabbed;
        let mut progress = if relocated || carried {
            0
        } else {
            lengths
                .delta(self.arc, arc)
                .clamp(-MAX_PROGRESS, MAX_PROGRESS)
        };
        let mut taken_back = 0;
        if grabbed {
            taken_back = self.air_progress;
            progress -= taken_back;
            self.air_progress = 0;
        } else if driver.actions_flag_set & actions::TOUCH_GROUND == 0 {
            self.air_progress += progress;
        } else {
            self.air_progress = 0;
        }
        self.arc = arc;
        self.kart_state = driver.kart_state;
        let on_pad = driver.step_flag_set & vehicle::step_triggers::TURBO_PAD_MASK != 0;
        let reserves_gained = if on_pad {
            0
        } else {
            (driver.reserves as i32 - self.reserves as i32).max(0)
        };
        self.reserves = driver.reserves;
        self.total += progress;
        // Carried by the mask, or put somewhere else: where it ends up is the new furthest, not
        // ground to make up.
        if relocated || driver.kart_state == kart_state::MASK_GRABBED {
            self.furthest = self.total;
            self.stalled = 0;
        } else if self.total > self.furthest {
            self.furthest = self.total;
            self.stalled = 0;
        } else {
            self.stalled += 1;
        }
        let airborne = driver.actions_flag_set & actions::TOUCH_GROUND == 0
            && driver.kart_state != kart_state::MASK_GRABBED;
        self.airborne = if airborne { self.airborne + 1 } else { 0 };
        // (A drift that ends puts it back to 0: no boost.)
        let boosts = (driver.num_boosts_success as i32 - self.boosts_success as i32).max(0) as u8;
        self.boosts_success = driver.num_boosts_success;
        let air_misalign = if self.airborne > AIR_GRACE_TICKS {
            let heading = path_heading(path, tracker, driver.pos_curr);
            1.0 - ((heading - driver.angle as i32) as f32 * ANGLE_TO_RAD).cos()
        } else {
            0.0
        };
        Step {
            progress,
            grabbed,
            reserves_gained,
            air_misalign,
            stalled: self.stalled > STALL_GRACE_TICKS,
            taken_back,
            wall: driver.actions_flag_set & actions::DRIVING_AGAINST_WALL != 0,
            boosts,
            steer_change: 0,
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
    /// Each tick of [`Step::stalled`]: more than anything a kart standing still could earn.
    pub stall: f32,
    /// Each tick against a wall ([`Step::wall`]): the speed it loses the progress pays for, but
    /// late and among much else.
    pub wall: f32,
    /// For each drift boost fired ([`Step::boosts`]), whatever its size (the reserves pay for
    /// that): a weak one counts as well, for a policy that has yet to find out about them. Not
    /// for the reserves of turbo pads and the start, which the progress pays for already.
    pub drift_boost: f32,
    /// Each tick in the air, for each unit of [`Step::air_misalign`]: a long jump flies
    /// straight, or turns with the path.
    pub air_heading: f32,
    /// For a move of the stick from one end to the other ([`Step::steer_change`], in
    /// `STEER_BINS - 1` bins), less for less: enough to settle the choice between near bins,
    /// which otherwise flips from choice to choice (the wiggles), not enough to stop a turn.
    pub steer_change: f32,
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
        // As much as 15 ticks at top speed; with a full bar's reserves, about 34.
        drift_boost: 15.0,
        // A short scrape (a few ticks) is about as much as a tick or three at top speed.
        wall: 0.3,
        // Half what a tick at top speed earns, and more than a turbo pad tops up a tick (some
        // 33 ms of reserves, 0.33).
        stall: 0.5,
        // A flight of 30 ticks sideways is as much as 6 ticks at top speed; within 15° of the
        // path, next to nothing.
        air_heading: 0.2,
        // Swinging end to end at every choice is about 2.5% of what top speed earns.
        steer_change: 0.05,
    };

    pub fn reward(&self, step: &Step) -> f32 {
        step.progress as f32 * self.progress
            - if step.grabbed { self.grabbed } else { 0.0 }
            - self.time
            + step.reserves_gained as f32 * self.reserves
            + step.boosts as f32 * self.drift_boost
            - step.air_misalign * self.air_heading
            - if step.wall { self.wall } else { 0.0 }
            - if step.stalled { self.stall } else { 0.0 }
            - step.steer_change as f32 / (STEER_BINS - 1) as f32 * self.steer_change
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
        assert!(latch.pad(long, false).held & buttons::R1 != 0);
        for _ in 1..HOLD_TICKS {
            assert!(latch.pad(release, false).held & buttons::R1 != 0);
        }
        assert_eq!(latch.hold_left, 0);
        assert_eq!(latch.pad(release, false).held & buttons::R1, 0);
    }

    #[test]
    fn the_latch_counts_the_stick_moves_once_a_choice() {
        let mut latch = Latch::default();
        let left = Action { steer: 0, ..Action::default() };
        assert_eq!(latch.steer, STEER_STEPS as u8);
        latch.pad(left, false);
        assert_eq!(latch.steer_change, STEER_STEPS as u8);
        // Held for another tick: no move.
        latch.pad(left, false);
        assert_eq!(latch.steer_change, 0);
        latch.pad(Action { steer: STEER_BINS as u8 - 1, ..Action::default() }, false);
        assert_eq!(latch.steer_change, STEER_BINS as u8 - 1);
    }

    #[test]
    fn a_long_hold_in_a_drift_does_not_let_go() {
        let mut latch = Latch::default();
        let hold = Action { hop: Hop::Hold, ..Action::default() };
        let long = Action { hop: Hop::HoldLong, ..Action::default() };
        assert!(latch.pad(hold, true).held & buttons::R1 != 0);
        for _ in 0..HOLD_TICKS {
            assert!(latch.pad(long, true).held & buttons::R1 != 0);
        }
    }

    #[test]
    fn l1_only_in_a_drift() {
        let mut latch = Latch::default();
        let boost = Action { boost: true, ..Action::default() };
        assert_eq!(latch.pad(boost, false).held & buttons::L1, 0);
        assert!(latch.pad(boost, true).held & buttons::L1 != 0);
    }

    #[test]
    fn a_long_hold_of_a_held_button_starts_with_a_press() {
        let mut latch = Latch::default();
        let hold = Action { hop: Hop::Hold, ..Action::default() };
        let long = Action { hop: Hop::HoldLong, ..Action::default() };
        assert!(latch.pad(hold, false).held & buttons::R1 != 0);
        // Let go, then pressed and held on (the choice repeated, as the env holds it).
        assert_eq!(latch.pad(long, false).held & buttons::R1, 0);
        for _ in 0..HOLD_TICKS {
            assert!(latch.pad(long, false).held & buttons::R1 != 0);
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
