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
use loader::level::LevelId;
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
    + 1 // the ticks it has got no further along ([`Progress::stalled`])
    + 1; // in a flight from a gap's lip ([`Progress::flight_heading`]), not a hop or a fall

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
    // (Against the way the kart goes, not its nose, for a gap: the nose does not decide where a
    // flight goes.)
    let (heading, facing) = match progress
        .flight_heading
        .or_else(|| gap_heading_ahead(path, tracker.frame as usize))
    {
        Some(heading) => (heading, travel_angle(d)),
        None => (path_heading(path, tracker, pos), d.angle as i32),
    };
    o.push(offset as f32 / DISTANCE_SCALE);
    let error = (heading - facing) as f32 * ANGLE_TO_RAD;
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
    o.flag(kart.held & (buttons::SQUARE | buttons::DOWN) != 0);
    o.flag(kart.held & buttons::R1 != 0);
    o.flag(kart.held & buttons::L1 != 0);
    o.push(latch.hold_left as f32 / HOLD_TICKS as f32);
    o.push((latch.steer as f32 - STEER_STEPS as f32) / STEER_STEPS as f32);
    o.push((progress.stalled as f32 / STALL_TICKS as f32).min(1.0));
    // (What the heading error above is against: the way to a gap's landing in the flight, the
    // lip-to-landing heading on the run-up to it, not the path's.)
    o.flag(progress.in_gap_flight());

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

/// The actor's outputs: all of the heads' logits, one after the other.
pub const LOGITS: usize = HEAD_START[N_HEADS - 1] + HEADS[N_HEADS - 1];

/// Where each head's logits start among the [`LOGITS`].
pub const HEAD_START: [usize; N_HEADS] = {
    let mut start = [0; N_HEADS];
    let mut h = 1;
    while h < N_HEADS {
        start[h] = start[h - 1] + HEADS[h - 1];
        h += 1;
    }
    start
};

/// A learned bot makes a choice every this many ticks, and holds it in between.
pub const REPEAT: u32 = 2;

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
    /// D-pad down (the stick all the way down, as the client's D-pad sends it), alone: backs up at
    /// the full reverse speed (`vehicle::linear`'s reverse from neutral), slowing the kart first
    /// if it is going forward. Not Square: alone that only brakes, to a stop (a kart against a
    /// wall could not get out), and with or without Down it throws the reserves away and ends a
    /// drift.
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
    /// How far the stick of the last pad is from the centre, 0 to 1 (full lock).
    pub fn stick(&self) -> f32 {
        (self.steer as f32 - STEER_STEPS as f32).abs() / STEER_STEPS as f32
    }


    /// The pad of this tick, the bot having chosen `action` (this tick, or one before that it
    /// still holds), its kart `drifting` as the tick starts.
    ///
    /// L1 is let through in a drift only. Out of one, a tap of L1 alone (even mid-hop, R1 held)
    /// makes L1 the button the next drift is held with (`vehicle::linear`), and that drift ends
    /// as soon as L1 is let go: by chance, a few ticks in, long before its meter can boost. A hop
    /// with L1 is no different from one with R1, so nothing is lost.
    ///
    /// The long hold ([`Hop::HoldLong`]) is for getting a drift to its meter's window: once the
    /// drift has `tried` a boost (`Driver::num_boosts_attempted`: in time, too early, or the
    /// meter ran out), it is over, and the hop's choice holds R1 or lets go again. Held on
    /// regardless, the one way out of a drift whose corner ends sooner would be Square, which
    /// throws the reserves away.
    ///
    /// In a `flying` kart (a flight from a gap's lip, [`Progress::in_gap_flight`]) the stick is
    /// left at the centre, whatever is chosen: nothing a jump needs is steered for in the air.
    pub fn pad(&mut self, mut action: Action, drifting: bool, tried: bool, flying: bool) -> Pad {
        if flying {
            action.steer = STEER_STEPS as u8;
        }
        self.steer_change = self.steer.abs_diff(action.steer);
        self.steer = action.steer;
        let mut pad = action.pad();
        if !drifting {
            pad.held &= !buttons::L1;
        }
        let done = drifting && tried;
        if done {
            self.hold_left = 0;
        }
        if action.hop == Hop::HoldLong && self.hold_left == 0 && !done {
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
            Throttle::Reverse => buttons::DOWN,
        };
        Pad {
            held: throttle
                | if self.hop != Hop::Release { buttons::R1 } else { 0 }
                | if self.boost { buttons::L1 } else { 0 },
            lx: lx.clamp(0, 0xff) as u8,
            ly: if self.throttle == Throttle::Reverse { 0xff } else { STICK_CENTER as u8 },
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

/// A flight off a gap's lip that ends on the ground without the mask having had the kart counts as
/// a cleared gap ([`Step::gap_cleared`]) if it made at least this much progress in the air (level
/// units): the gap is at least `bot::MIN_GAP` (256) across, so a hop off the lip that lands short
/// does not.
pub const GAP_CLEARED_PROGRESS: i64 = 200;

/// A hop pressed this many ticks or fewer before a takeoff from a gap's lip is the takeoff's
/// ([`Step::gap_hop`]): a hop lasts about 10, and the takeoff is the first tick in the air.
pub const GAP_HOP_TICKS: u32 = 6;

/// The most height above its takeoff (level units) a flight from a gap's lip is paid for
/// ([`Step::gap_apex`]): a hop off a ramp at speed tops out some way under it, and what goes
/// higher is something else (a bounce off the geometry).
pub const MAX_APEX: i32 = 400;

/// The reserves a boost spends a tick (`vehicle::linear`'s count-down, by `Env::elapsed_time_ms`):
/// what they drop by more than this is lost ([`Step::reserves_lost`]).
pub const RESERVES_SPENT_PER_TICK: i32 = 32;

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
    /// The reserves it lost this tick (milliseconds), beyond the [`RESERVES_SPENT_PER_TICK`] a
    /// boost spends: thrown away by Square (`vehicle::linear`), a wall's scrub, a crash, a spin,
    /// the mask. The ways a drift ends badly (`brake`, `wall`) cost what they cost the boost.
    pub reserves_lost: i32,
    /// Reverse (Down, or Square) was held going forward faster than a quarter of the top speed:
    /// braking, not backing out. [`Progress::step`] knows nothing of the pads: the caller puts it
    /// in.
    pub braking: bool,
    /// It has got no further along for more than [`STALL_GRACE_TICKS`] ([`Progress::stalled`]).
    pub stalled: bool,
    /// In a flight from a gap's lip ([`Progress::flight_heading`]), how far off the way to the
    /// landing (from where it is) the kart goes this tick: `1 - cos` of the angle off it (0 straight on, 1 a quarter
    /// turn off, 2 backwards), 0 on the takeoff tick. 0 on the ground, where a drift faces away on purpose, and in any other flight
    /// (a hop, a fall): there is no telling a turn that is wanted from one that is not.
    pub air_misalign: f32,
    /// In a flight from a gap's lip, past its takeoff tick: the kart goes less far off
    /// [`Progress::flight_heading`] than it did the tick before. A turn that brings it round
    /// toward the landing is free of [`Weights::air_steer`].
    pub air_closing: bool,
    /// The kart took off from the lip of a gap (a ramp frame of the path, [`gap_heading`]) this
    /// tick.
    pub gap_takeoff: bool,
    /// The kart came down on the ground this tick after a flight from a gap's lip that made
    /// [`GAP_CLEARED_PROGRESS`] or more, without the mask: it made the jump.
    pub gap_cleared: bool,
    /// The kart came down after a flight from a gap's lip with less than
    /// [`GAP_CLEARED_PROGRESS`] made (not grabbed): short of the gap, or on something below it.
    pub gap_short: bool,
    /// The mask grabbed the kart in a flight from a gap's lip: it fell.
    pub gap_fell: bool,
    /// On the tick one of [`Step::gap_cleared`], [`Step::gap_short`] and [`Step::gap_fell`] is
    /// set, the speed the flight took off with (`Driver::speed_approx`); else 0.
    pub gap_speed: i32,
    /// On the tick of a [`Step::gap_takeoff`] and on the ones that set [`Step::gap_cleared`],
    /// [`Step::gap_short`] or [`Step::gap_fell`], the frame of the gap's lip.
    pub gap_lip: Option<usize>,
    /// It took off from a gap's lip ([`Step::gap_takeoff`]) with a hop it pressed R1 for within
    /// [`GAP_HOP_TICKS`] before, and not in a drift. [`Progress::step`] knows nothing of the
    /// pads: the caller puts it in.
    pub gap_hop: bool,
    /// On the tick of a [`Step::gap_takeoff`], how well the kart's way of travel matches the lip-to-landing heading:
    /// the cosine of the angle off it, 0 from a quarter turn off on (1 straight on); else 0.
    pub gap_aim: f32,
    /// On the tick a flight from a gap's lip tops out (the first it is lower than it has been),
    /// or ends before that, how high above its takeoff it went (level units, at most
    /// [`MAX_APEX`]), times how well it took off toward the landing (as [`Step::gap_aim`]); else 0.
    /// The ramp, the speed and a hop at the lip all lift it, and the higher it goes the further
    /// it flies.
    pub gap_apex: f32,
    /// In a flight from a gap's lip, on a tick that is not [`Step::air_closing`], how far the
    /// stick was from the centre ([`Latch::stick`]); else 0. [`Progress::step`] knows nothing of
    /// the pads: the caller puts it in.
    pub air_steer: f32,
    /// The kart scraped a wall (`DRIVING_AGAINST_WALL`).
    pub wall: bool,
    /// Drift boosts fired this tick: taps of L1 in time, each of a drift's three
    /// (`Driver::num_boosts_success`).
    pub boosts: u8,
    /// Of the drift's boosts, the number of the last fired this tick (1 for its first; with
    /// [`Step::boosts`], for [`Weights::boost_growth`]).
    pub boost_number: u8,
    /// On a tick a drift boost fired, how late in its meter's window the tap came: the reserves
    /// it gained as a share of a full bar's (`turbo_full_bar_reserve_gain << 5`, which the last
    /// moment before the meter runs out all but gets), 0 to 1; else 0. (On a turbo pad too,
    /// unlike [`Step::reserves_gained`].)
    pub boost_timing: f32,
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
    /// The ticks since the mask put the kart back (`u32::MAX` if it never has).
    pub since_respawn: u32,
    /// It was grabbed again soon after the last respawn ([`GRAB_LOOP_TICKS`]): the next
    /// [`Progress::switch_due`] on the ground is a switch.
    pub loop_switch: bool,
    /// In a flight off a gap's lip, the way from the lip to its landing ([`gap_heading`]): the
    /// way the path goes, for the whole flight, instead of [`path_heading`]. That one looks
    /// some way along the path from where the kart is, which past a landing with a corner right
    /// after it bends into the corner while the kart is
    /// still in the air: the cost and the observation would have it turn mid-air.
    ///
    /// Taken at the takeoff, then each tick of the flight the way from where the kart is to
    /// [`Progress::flight_target`]: a kart that took off askew or has drifted aside is to turn
    /// toward the landing from where it is, not back to the lip's heading, which would carry it
    /// past alongside the landing.
    pub flight_heading: Option<i32>,
    /// In a flight with a [`Progress::flight_heading`]: the lip it took off from and the point
    /// [`FLIGHT_AIM_PAST`] beyond the landing that it aims at (level units, `x`, `z`).
    pub flight_lip_pos: [i64; 2],
    pub flight_target: [i64; 2],
    /// In a flight with a [`Progress::flight_heading`], how far the kart faced off it last tick
    /// (1/0x1000 of a turn, 0 to 0x800).
    pub air_error: i32,
    /// The kart's `Driver::speed_approx` when it took off on a flight with a
    /// [`Progress::flight_heading`].
    pub takeoff_speed: i32,
    /// The frame of the lip the kart took off from on a flight with a
    /// [`Progress::flight_heading`].
    pub flight_lip: usize,
    /// In a flight with a [`Progress::flight_heading`]: the kart's height when it took off, the
    /// most it has been since (level units), whether [`Step::gap_apex`] has been paid for it, and
    /// how far off the heading it took off (as [`Progress::air_error`]).
    pub takeoff_y: i32,
    pub apex_y: i32,
    pub apex_paid: bool,
    pub takeoff_error: i32,
}

/// How far beyond a gap's landing (level units, on along the lip-to-landing way) a flight aims
/// ([`Progress::flight_target`]): the way to a point the kart passes over swings round as it
/// gets near, and one past the landing keeps it steady up to the touchdown.
pub const FLIGHT_AIM_PAST: i64 = 256;

/// Closer than this (level units, along the lip-to-landing way) to [`Progress::flight_target`],
/// [`Progress::flight_heading`] stays as it was: the way to the target swings round there.
const FLIGHT_AIM_HOLD: i64 = 128;

/// The way from the lip of the gap the kart at `frame` of `path` is taking off from (within a
/// couple of frames of it) to the gap's landing (`bot::gap_landing`), if it is at one.
pub fn gap_heading(path: &[NavFrame], frame: usize) -> Option<i32> {
    let (lip, landing) = gap_lip(path, frame)?;
    let (a, b) = (&path[lip].pos, &path[landing].pos);
    Some(math::ratan2(b.x as i32 - a.x as i32, b.z as i32 - a.z as i32))
}

/// The way `driver` goes on the ground plane (1/0x1000 of a turn), or the way it faces if it is
/// standing still. A flight goes the way the kart took off, whatever the nose does in the air.
pub fn travel_angle(driver: &Driver) -> i32 {
    let v = driver.velocity;
    if v.x == 0 && v.z == 0 {
        driver.angle as i32
    } else {
        math::ratan2(v.x, v.z)
    }
}

/// How far ahead along `path` (level units) [`gap_heading_ahead`] looks for a gap's lip: the
/// run-up the env starts its jump episodes with.
pub const GAP_LOOKAHEAD: i64 = 1200;

/// [`gap_heading`] for the kart at `frame`, or else for the first gap's lip within
/// [`GAP_LOOKAHEAD`] ahead of it: the heading to be at on the run-up, known before the takeoff.
pub fn gap_heading_ahead(path: &[NavFrame], frame: usize) -> Option<i32> {
    if let Some(heading) = gap_heading(path, frame) {
        return Some(heading);
    }
    let n = path.len();
    if n < 2 {
        return None;
    }
    let mut distance = 0.0f32;
    for k in 1..n {
        let (a, b) = (&path[(frame + k - 1) % n].pos, &path[(frame + k) % n].pos);
        let (dx, dz) = ((b.x as i32 - a.x as i32) as f32, (b.z as i32 - a.z as i32) as f32);
        distance += (dx * dx + dz * dz).sqrt();
        if distance > GAP_LOOKAHEAD as f32 {
            break;
        }
        let lip = (frame + k) % n;
        if let Some(landing) = crate::bot::gap_landing(path, lip) {
            let (a, b) = (&path[lip].pos, &path[landing].pos);
            return Some(math::ratan2(b.x as i32 - a.x as i32, b.z as i32 - a.z as i32));
        }
    }
    None
}

/// The lip and the landing (frames of `path`) of the gap the kart at `frame` is taking off from
/// (it is within a couple of frames of the lip), if it is at one.
pub fn gap_lip(path: &[NavFrame], frame: usize) -> Option<(usize, usize)> {
    let n = path.len();
    if n < 2 {
        return None;
    }
    (0..5).find_map(|k| {
        let lip = (frame + n + k - 2) % n;
        Some((lip, crate::bot::gap_landing(path, lip)?))
    })
}

/// After this many ticks without getting further along ([`Progress::stalled`]) ...
pub const SAMPLE_AFTER_STALLED: u32 = 30;
/// ... or for this many after a respawn, a bot that drives by the likeliest choices should draw
/// them from the policy's odds instead, as in training ([`draw_choices`]).
pub const SAMPLE_AFTER_RESPAWN: u32 = 90;

/// Every this many ticks of a stall ([`Progress::stalled`]), a bot tries another path
/// ([`Progress::switch_due`]) ...
pub const SWITCH_AFTER_STALLED: u32 = 60;
/// ... and when the mask grabs it again within this many ticks of putting it back: a loop, the
/// line it steers by leading off the same edge again.
pub const GRAB_LOOP_TICKS: u32 = 30 * 20;

/// A bot path the learned bots never drive: not in training, not in eval, not in a race (a kart
/// whose slot steers by it takes another, [`learned_paths`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BannedPath {
    pub level: LevelId,
    /// Into `Level::nav_paths`.
    pub path: usize,
    /// Why, for whoever reads the list.
    pub why: &'static str,
}

/// The bot paths the learned bots never drive ([`BannedPath`]): one more is one more line.
pub const BANNED_PATHS: &[BannedPath] = &[BannedPath {
    level: LevelId::HOT_AIR_SKYWAY,
    path: 2,
    why: "the gap at lip 142 is too long for all but the Speed class, and for that only with a \
          flawless run-up, takeoff and hop: what a policy learns there is mostly to fall",
}];

/// Whether `path` of level `level` is one of the [`BANNED_PATHS`].
pub fn path_banned(level: LevelId, path: usize) -> bool {
    BANNED_PATHS.iter().any(|b| b.level == level && b.path == path)
}

/// The paths of `level` (of id `id`) a learned bot may drive, in order: those of two frames or
/// more that are not [`BANNED_PATHS`].
pub fn learned_paths(level: &Level, id: LevelId) -> Vec<usize> {
    (0..level.nav_paths.len())
        .filter(|&p| level.nav_paths[p].frames.len() >= 2 && !path_banned(id, p))
        .collect()
}

/// The path a stuck bot at `pos` switches to: the level's next one after `path` (two frames or
/// more), round them in turn, and where on it the bot is. In turn, not the nearest: with three
/// lines side by side the nearest to one is often the one it came from, and the third would
/// never be tried. None if there is no other.
pub fn next_path(level: &Level, path: usize, pos: Vec3) -> Option<(usize, Tracker)> {
    let all: Vec<usize> = (0..level.nav_paths.len()).collect();
    next_path_of(level, path, pos, &all)
}

/// [`next_path`] among only the paths `of` (indices, in order) that are one of the level's.
pub fn next_path_of(
    level: &Level,
    path: usize,
    pos: Vec3,
    of: &[usize],
) -> Option<(usize, Tracker)> {
    let n = level.nav_paths.len();
    (1..n)
        .map(|k| (path + k) % n)
        .find(|i| of.contains(i) && level.nav_paths[*i].frames.len() >= 2)
        .map(|i| (i, Tracker::find(&level.nav_paths[i].frames, pos)))
}

/// Whether the bot at `progress` should draw its choices, not take the likeliest. The sim is the
/// same every time, and so is the likeliest choice from where it is: stuck, what the bot sees
/// hardly changes, and it would sit there for good; put back by the mask at the same spot,
/// standing still, it would drive off the same edge again, and again. Drawn, it gets out as it
/// learned to.
pub fn draw_choices(progress: &Progress) -> bool {
    progress.stalled >= SAMPLE_AFTER_STALLED || progress.since_respawn < SAMPLE_AFTER_RESPAWN
}

impl Progress {
    /// The kart is in a flight from a gap's lip ([`Progress::flight_heading`]), not a hop or a fall.
    pub fn in_gap_flight(&self) -> bool {
        self.airborne > 0 && self.flight_heading.is_some()
    }

    /// Whether the bot of `driver` should try another path now ([`next_path`]): on the
    /// ground, after each [`SWITCH_AFTER_STALLED`] ticks of a stall, or once put back by the mask
    /// after a loop of grabs ([`Progress::loop_switch`], which this clears). The line it steers
    /// by may lead past what it cannot get round (a wall, a drop it went down beside the road),
    /// or off an edge, and another may show a way.
    pub fn switch_due(&mut self, driver: &Driver) -> bool {
        let on_ground = driver.actions_flag_set & actions::TOUCH_GROUND != 0
            && driver.kart_state != kart_state::MASK_GRABBED;
        if !on_ground {
            return false;
        }
        let stalled = self.stalled > 0 && self.stalled % SWITCH_AFTER_STALLED == 0;
        let looped = std::mem::take(&mut self.loop_switch);
        stalled || looped
    }

    /// Goes on along another path, the kart being at `tracker` on it: the progress from here is
    /// along that one. The stall goes on (a switch is no progress), so that a bot still stuck
    /// is still cut and switches again.
    pub fn switched(&mut self, path: &[NavFrame], lengths: &PathLength, tracker: &Tracker, driver: &Driver) {
        self.arc = tracker.arc(path, lengths, driver.pos_curr);
        self.air_progress = 0;
        self.flight_heading = None;
    }

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
            since_respawn: u32::MAX,
            loop_switch: false,
            flight_heading: None,
            flight_lip_pos: [0; 2],
            flight_target: [0; 2],
            air_error: 0,
            takeoff_speed: 0,
            flight_lip: 0,
            takeoff_y: 0,
            apex_y: 0,
            apex_paid: false,
            takeoff_error: 0,
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
        if grabbed && self.since_respawn < GRAB_LOOP_TICKS {
            self.loop_switch = true;
        }
        self.since_respawn = if driver.kart_state == kart_state::MASK_GRABBED {
            0
        } else {
            self.since_respawn.saturating_add(1)
        };
        // (Before the ground resets them: the flight this tick may end.)
        let flown = self.air_progress + progress;
        let was_gap_flight = self.flight_heading.is_some();
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
        let reserves_lost =
            (self.reserves as i32 - driver.reserves as i32 - RESERVES_SPENT_PER_TICK).max(0);
        // (What a boost spends of them this tick too, back.)
        let boost_reserves = driver.reserves as i32 - self.reserves as i32 + RESERVES_SPENT_PER_TICK;
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
        match self.airborne {
            0 => self.flight_heading = None,
            1 => {
                self.flight_heading = gap_heading(path, tracker.frame as usize);
                self.takeoff_speed = driver.speed_approx as i32;
                if let Some((lip, landing)) = gap_lip(path, tracker.frame as usize) {
                    self.flight_lip = lip;
                    let (a, b) = (&path[lip].pos, &path[landing].pos);
                    let (a, b) = ([a.x as i64, a.z as i64], [b.x as i64, b.z as i64]);
                    let d = [b[0] - a[0], b[1] - a[1]];
                    let len = ((d[0] * d[0] + d[1] * d[1]) as f64).sqrt().max(1.0);
                    let past = |i: usize| b[i] + (d[i] as f64 * FLIGHT_AIM_PAST as f64 / len) as i64;
                    self.flight_lip_pos = a;
                    self.flight_target = [past(0), past(1)];
                }
            }
            _ if self.flight_heading.is_some() => {
                // The way to the target from here, short of where it swings round.
                let pos = crate::bot::level_pos(driver.pos_curr);
                let (t, a) = (self.flight_target, self.flight_lip_pos);
                let to = [t[0] - pos[0], t[1] - pos[2]];
                let axis = [t[0] - a[0], t[1] - a[1]];
                let len = ((axis[0] * axis[0] + axis[1] * axis[1]) as f64).sqrt().max(1.0);
                let ahead = (to[0] * axis[0] + to[1] * axis[1]) as f64 / len;
                if ahead > FLIGHT_AIM_HOLD as f64 {
                    self.flight_heading = Some(math::ratan2(to[0] as i32, to[1] as i32));
                }
            }
            _ => {}
        }
        let gap_takeoff = self.airborne == 1 && self.flight_heading.is_some();
        let landed = was_gap_flight
            && self.airborne == 0
            && !grabbed
            && !relocated
            && driver.kart_state != kart_state::MASK_GRABBED;
        let gap_cleared = landed && flown >= GAP_CLEARED_PROGRESS;
        let gap_short = landed && !gap_cleared;
        let gap_fell = was_gap_flight && grabbed;
        let gap_speed = if gap_cleared || gap_short || gap_fell { self.takeoff_speed } else { 0 };
        // (A drift that ends puts it back to 0: no boost.)
        let boosts = (driver.num_boosts_success as i32 - self.boosts_success as i32).max(0) as u8;
        self.boosts_success = driver.num_boosts_success;
        // (A flight from a gap's lip only: a hop is no jump, and the other flights are not told
        // apart from it by their length, which is what this once went by.) The turn the takeoff
        // itself makes is free, a bad one is for the run-up to answer for.
        let mut gap_aim = 0.0;
        let mut air_closing = false;
        let air_misalign = match self.flight_heading {
            Some(heading) if self.airborne > 0 => {
                let mut error = (heading - travel_angle(driver)) & 0xfff;
                if error > 0x800 {
                    error = 0x1000 - error;
                }
                let off = |e: i32| 1.0 - (e as f32 * ANGLE_TO_RAD).cos();
                air_closing = self.airborne > 1 && error < self.air_error;
                self.air_error = error;
                if self.airborne == 1 {
                    gap_aim = (1.0 - off(error)).max(0.0);
                }
                // (The takeoff tick is free; after it, every tick off the heading costs.)
                if self.airborne > 1 { off(error) } else { 0.0 }
            }
            _ => 0.0,
        };
        let y = driver.pos_curr.y >> 8;
        let mut gap_apex = 0.0;
        if gap_takeoff {
            self.takeoff_y = y;
            self.apex_y = y;
            self.apex_paid = false;
            self.takeoff_error = self.air_error;
        } else if self.in_gap_flight() && !self.apex_paid && y >= self.apex_y {
            self.apex_y = y;
        } else if was_gap_flight && !self.apex_paid && (self.airborne == 0 || y < self.apex_y) {
            // Topped out, or came down (or was grabbed) before: what this tick's height is then
            // does not count.
            self.apex_paid = true;
            let aim = (self.takeoff_error as f32 * ANGLE_TO_RAD).cos().max(0.0);
            gap_apex = (self.apex_y - self.takeoff_y).clamp(0, MAX_APEX) as f32 * aim;
        }
        Step {
            progress,
            gap_apex,
            grabbed,
            reserves_gained,
            air_misalign,
            air_closing,
            gap_aim,
            gap_takeoff,
            gap_cleared,
            gap_short,
            gap_fell,
            gap_speed,
            gap_lip: (gap_takeoff || gap_cleared || gap_short || gap_fell).then_some(self.flight_lip),
            gap_hop: false,
            air_steer: 0.0,
            stalled: self.stalled > STALL_GRACE_TICKS,
            braking: false,
            reserves_lost,
            taken_back,
            wall: driver.actions_flag_set & actions::DRIVING_AGAINST_WALL != 0,
            boosts,
            boost_timing: if boosts > 0 {
                let full = (driver.stats.turbo_full_bar_reserve_gain as i32) << 5;
                (boost_reserves as f32 / full.max(1) as f32).clamp(0.0, 1.0)
            } else {
                0.0
            },
            boost_number: driver.num_boosts_success.max(0) as u8,
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
    /// For each millisecond of reserves lost ([`Step::reserves_lost`]): as much as gaining one
    /// pays, so that thrown away they were worth nothing.
    pub reserves_lost: f32,
    /// Each tick of [`Step::braking`] (on top of the reserves it throws away). Square at low
    /// speed, backing out of a wall, is free.
    pub brake: f32,
    /// Each tick of [`Step::stalled`]: more than anything a kart standing still could earn.
    pub stall: f32,
    /// Each tick against a wall ([`Step::wall`]): the speed it loses the progress pays for, but
    /// late and among much else.
    pub wall: f32,
    /// For each drift boost fired ([`Step::boosts`]), whatever its size (the reserves pay for
    /// that): a weak one counts as well, for a policy that has yet to find out about them. Not
    /// for the reserves of turbo pads and the start, which the progress pays for already.
    pub drift_boost: f32,
    /// Each boost of a drift after its first is worth this many times the one before: the
    /// k-th pays `drift_boost * boost_growth^(k - 1)`, for a policy to find out about chaining
    /// them (they fire hotter as well, which the progress pays for, later). 1 for none.
    pub boost_growth: f32,
    /// For each drift boost fired, on top of [`Weights::drift_boost`] (and grown by
    /// [`Weights::boost_growth`] as it is), times [`Step::boost_timing`] to the power
    /// [`Weights::boost_timing_power`]: for tapping at the last moment of the meter's window,
    /// not the first. The flat [`Weights::drift_boost`] pays a tap anywhere in the window, more
    /// than the reserves of a late one make up for the risk of the meter running out (which
    /// ends the drift's boosts); it gets a policy drifting, and can then fade (`train`'s
    /// `boost_fade`), this one staying. 0 for none.
    pub boost_timed: f32,
    /// See [`Weights::boost_timed`]: 1 pays in proportion to the lateness, more pays the last
    /// ticks of the window most.
    pub boost_timing_power: f32,
    /// For each unit of [`Step::air_misalign`], each tick: a jump from a gap's lip flies toward
    /// its landing, and holding a heading off it costs for as long as it lasts. 30 ticks a
    /// quarter turn off is about as much as 6 ticks at top speed.
    pub air_heading: f32,
    /// For a gap cleared ([`Step::gap_cleared`]), on top of the progress its flight made: the
    /// payoff of a jump comes ticks after the run-up that made it, and a fall takes the flight's
    /// progress back, so without this a policy can do as well by never trying.
    pub gap_cleared: f32,
    /// For each level unit of a gap flight's apex ([`Step::gap_apex`]), cleared or not: a hop at
    /// the lip, and the speed, carry the kart higher and so much further, and a policy that has
    /// never made the jump would not find that out. Paid at the top of the flight, ticks after
    /// the takeoff, rather than for the hop itself: it pays for how well the hop was timed (a
    /// late one, off the lip, lifts it less), and for whatever else lifts it (the speed up the ramp).
    pub gap_apex: f32,
    /// Each tick in a flight from a gap's lip, for each unit of [`Step::air_steer`]: a turn in
    /// the air that does not bring the kart round toward the landing (one that does is free, and
    /// [`Weights::air_heading`] pays for it). Full lock for 40 ticks is about 12 ticks at top speed.
    pub air_steer: f32,
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
        boost_growth: 1.0,
        // A tap at the last choice of the window (some 0.9 of a full bar) gets about 0.8 of it;
        // half way through the window, a quarter.
        boost_timed: 10.0,
        boost_timing_power: 2.0,
        // A short scrape (a few ticks) is about as much as a tick or three at top speed.
        wall: 0.3,
        // Half what a tick at top speed earns, and more than a turbo pad tops up a tick (some
        // 33 ms of reserves, 0.33).
        stall: 0.5,
        reserves_lost: 0.01,
        // A second of braking is as much as half a second at top speed.
        brake: 0.5,
        // A flight of 30 ticks sideways is as much as 6 ticks at top speed; within 15° of the
        // path, next to nothing.
        air_heading: 0.2,
        // As much as the best drift boost (15) and a bit: a grab costs 30 and the flight's progress.
        gap_cleared: 20.0,
        // A hop alone tops out some 45 units up (`jump_force` 4596 against `gravity` 900): with a
        // ramp's lift at speed, a long jump's apex of 100 is about 15, most of what clearing it
        // pays.
        gap_apex: 0.15,
        air_steer: 0.3,
        // Swinging end to end at every choice is about 2.5% of what top speed earns.
        steer_change: 0.05,
    };

    /// What the drift boosts of `step` are worth, each by its number in the drift.
    fn drift_boosts(&self, step: &Step) -> f32 {
        let last = step.boost_number as i32;
        let first = last + 1 - step.boosts as i32;
        let timed = self.boost_timed * step.boost_timing.clamp(0.0, 1.0).powf(self.boost_timing_power);
        (first.max(1)..=last)
            .map(|k| self.boost_growth.powi(k - 1))
            .sum::<f32>()
            * (self.drift_boost + timed)
    }

    pub fn reward(&self, step: &Step) -> f32 {
        step.progress as f32 * self.progress
            - if step.grabbed { self.grabbed } else { 0.0 }
            - self.time
            + step.reserves_gained as f32 * self.reserves
            + self.drift_boosts(step)
            - step.air_misalign * self.air_heading
            + if step.gap_cleared { self.gap_cleared } else { 0.0 }
            + step.gap_apex * self.gap_apex
            - step.air_steer * self.air_steer
            - if step.wall { self.wall } else { 0.0 }
            - if step.stalled { self.stall } else { 0.0 }
            - step.reserves_lost as f32 * self.reserves_lost
            - if step.braking { self.brake } else { 0.0 }
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
                    assert_eq!(pad.held & buttons::DOWN != 0, throttle == Throttle::Reverse);
                    assert_eq!(pad.held & buttons::SQUARE, 0);
                    assert_eq!(pad.ly == 0xff, throttle == Throttle::Reverse);
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
        assert!(latch.pad(long, false, false, false).held & buttons::R1 != 0);
        for _ in 1..HOLD_TICKS {
            assert!(latch.pad(release, false, false, false).held & buttons::R1 != 0);
        }
        assert_eq!(latch.hold_left, 0);
        assert_eq!(latch.pad(release, false, false, false).held & buttons::R1, 0);
    }

    #[test]
    fn the_latch_counts_the_stick_moves_once_a_choice() {
        let mut latch = Latch::default();
        let left = Action { steer: 0, ..Action::default() };
        assert_eq!(latch.steer, STEER_STEPS as u8);
        latch.pad(left, false, false, false);
        assert_eq!(latch.steer_change, STEER_STEPS as u8);
        // Held for another tick: no move.
        latch.pad(left, false, false, false);
        assert_eq!(latch.steer_change, 0);
        latch.pad(Action { steer: STEER_BINS as u8 - 1, ..Action::default() }, false, false, false);
        assert_eq!(latch.steer_change, STEER_BINS as u8 - 1);
    }

    #[test]
    fn a_long_hold_in_a_drift_does_not_let_go() {
        let mut latch = Latch::default();
        let hold = Action { hop: Hop::Hold, ..Action::default() };
        let long = Action { hop: Hop::HoldLong, ..Action::default() };
        assert!(latch.pad(hold, true, false, false).held & buttons::R1 != 0);
        for _ in 0..HOLD_TICKS {
            assert!(latch.pad(long, true, false, false).held & buttons::R1 != 0);
        }
    }

    #[test]
    fn a_long_hold_ends_once_the_drift_has_tried_a_boost() {
        let mut latch = Latch::default();
        let long = Action { hop: Hop::HoldLong, ..Action::default() };
        let release = Action::default();
        assert!(latch.pad(long, false, false, false).held & buttons::R1 != 0);
        assert!(latch.pad(release, true, false, false).held & buttons::R1 != 0);
        // Tried: the choice lets go.
        assert_eq!(latch.pad(release, true, true, false).held & buttons::R1, 0);
        assert_eq!(latch.hold_left, 0);
        // And a long hold chosen then is a plain hold.
        assert!(latch.pad(long, true, true, false).held & buttons::R1 != 0);
        assert_eq!(latch.hold_left, 0);
    }

    #[test]
    fn l1_only_in_a_drift() {
        let mut latch = Latch::default();
        let boost = Action { boost: true, ..Action::default() };
        assert_eq!(latch.pad(boost, false, false, false).held & buttons::L1, 0);
        assert!(latch.pad(boost, true, false, false).held & buttons::L1 != 0);
    }

    #[test]
    fn a_long_hold_of_a_held_button_starts_with_a_press() {
        let mut latch = Latch::default();
        let hold = Action { hop: Hop::Hold, ..Action::default() };
        let long = Action { hop: Hop::HoldLong, ..Action::default() };
        assert!(latch.pad(hold, false, false, false).held & buttons::R1 != 0);
        // Let go, then pressed and held on (the choice repeated, as the env holds it).
        assert_eq!(latch.pad(long, false, false, false).held & buttons::R1, 0);
        for _ in 0..HOLD_TICKS {
            assert!(latch.pad(long, false, false, false).held & buttons::R1 != 0);
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
