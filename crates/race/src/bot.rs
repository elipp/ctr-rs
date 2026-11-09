//! The bots: karts the race drives itself, with a pad of its own making, along the level's bot
//! paths ([`loader::lev::NavPath`]). Unlike retail's bots (`BOTS_Driver_Convert`), which move along
//! the path and skip the physics, these drive the same kart a player does.
//!
//! So far: where on its path a bot is ([`Tracker`]), where it should head for
//! ([`Tracker::ahead`]), and the pad that takes it there ([`Bot::pad`]): the gas held, and the
//! stick turned toward a point ahead on the path (pure pursuit). All of it is integers, so every
//! client drives the bots the same.

use loader::lev::{Level, NavFrame};
use math::Vec3;
use vehicle::{Driver, actions, buttons, kart_state};

use crate::Pad;

/// `Driver::pos_curr` is in 1/256 of the units of the level (and of the path).
const WORLD_POS_SHIFT: u32 = 8;

/// The most frames a tracker moves in a tick before it gives up and searches. A kart does less
/// than a frame a tick, so more than this is a teleport (a respawn, a mask grab).
const MAX_STEPS: usize = 16;

/// Further than this from the segment it is on (in level units), a kart has left the path, and the
/// tracker searches for it again.
pub const LOST_DISTANCE: i64 = 1500;

/// How many frames either side of the last one the tracker searches first, so that a kart is not
/// put on another part of a track that passes close (a bridge over it, the other side of a
/// hairpin).
const SEARCH_WINDOW: usize = 48;

type P = [i64; 3];

fn frame_pos(f: &NavFrame) -> P {
    [f.pos.x as i64, f.pos.y as i64, f.pos.z as i64]
}

fn level_pos(pos: Vec3) -> P {
    [pos.x, pos.y, pos.z].map(|v| (v >> WORLD_POS_SHIFT) as i64)
}

fn sub(a: P, b: P) -> P {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn sq(v: i64) -> i64 {
    v * v
}

fn dot(a: P, b: P) -> i64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// The square of the distance from `p` to the segment `a`..`b`.
fn segment_distance2(p: P, a: P, b: P) -> i64 {
    let d = sub(b, a);
    let len2 = dot(d, d);
    let ap = sub(p, a);
    if len2 == 0 {
        return dot(ap, ap);
    }
    let t = dot(ap, d).clamp(0, len2);
    // The closest point is a + d * t / len2.
    let off = [0, 1, 2].map(|k| ap[k] - d[k] * t / len2);
    dot(off, off)
}

/// Where a kart is on a bot path: between [`Tracker::frame`] and the frame after it. The paths are
/// loops, the last frame leading to the first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tracker {
    /// The last frame the kart has passed.
    pub frame: u16,
    /// How many times the kart has been searched for on the path (after a respawn, or being
    /// knocked off it), for the steering to tell that the path is not where it was.
    pub relocations: u16,
}

impl Tracker {
    /// A tracker for a kart at `pos` (in the units of `Driver::pos_curr`), searched for on the
    /// whole of `path`.
    pub fn find(path: &[NavFrame], pos: Vec3) -> Tracker {
        let mut t = Tracker::default();
        if path.len() >= 2 {
            let p = level_pos(pos);
            t.frame = nearest(path, p, 0..path.len()).0 as u16;
            t.settle(path, p);
        }
        t
    }

    /// Moves the tracker along with the kart, now at `pos`, only stepping along the path: never
    /// searching for it, however far off it is. For a kart in the air, high over the path (a jump
    /// arcs well past [`LOST_DISTANCE`]), which a search would put on some other part of the
    /// track below.
    pub fn follow(&mut self, path: &[NavFrame], pos: Vec3) {
        if path.len() >= 2 && (self.frame as usize) < path.len() {
            self.settle(path, level_pos(pos));
        }
    }

    /// Moves the tracker along with the kart, now at `pos`. Returns whether it had to search for
    /// the kart again.
    pub fn update(&mut self, path: &[NavFrame], pos: Vec3) -> bool {
        let n = path.len();
        if n < 2 {
            return false;
        }
        if self.frame as usize >= n {
            *self = Tracker::find(path, pos);
            return true;
        }
        let p = level_pos(pos);
        if self.settle(path, p) && self.distance2(path, p) <= LOST_DISTANCE * LOST_DISTANCE {
            return false;
        }

        // Lost: near where it was, or anywhere.
        let here = self.frame as usize;
        let (mut best, best_d2) = if n > 2 * SEARCH_WINDOW + 1 {
            let start = here + n - SEARCH_WINDOW;
            nearest(path, p, (start..start + 2 * SEARCH_WINDOW + 1).map(|i| i % n))
        } else {
            (here, i64::MAX)
        };
        if best_d2 > LOST_DISTANCE * LOST_DISTANCE {
            best = nearest(path, p, 0..n).0;
        }
        self.frame = best as u16;
        self.settle(path, p);
        self.relocations = self.relocations.wrapping_add(1);
        true
    }

    /// Steps forward over the frames the kart has passed, or back over the ones it has not
    /// reached. Returns false when that took more than [`MAX_STEPS`].
    fn settle(&mut self, path: &[NavFrame], p: P) -> bool {
        let n = path.len();
        let mut i = self.frame as usize;
        let mut settled = false;
        for _ in 0..MAX_STEPS {
            let next = (i + 1) % n;
            if !passed(path, next, p) {
                settled = true;
                break;
            }
            i = next;
        }
        if settled && i == self.frame as usize {
            // Not forward: maybe back. The same test, so the two never undo each other.
            settled = false;
            for _ in 0..MAX_STEPS {
                if passed(path, i, p) {
                    settled = true;
                    break;
                }
                i = (i + n - 1) % n;
            }
        }
        self.frame = i as u16;
        settled
    }

    /// The square of the kart's distance from the segment it is on.
    fn distance2(&self, path: &[NavFrame], p: P) -> i64 {
        let i = self.frame as usize;
        let a = frame_pos(&path[i]);
        let b = frame_pos(&path[(i + 1) % path.len()]);
        segment_distance2(p, a, b)
    }

    /// The point of the path `distance` (in level units) ahead of the kart at `pos`, measured
    /// along the path on the ground plane from where the kart is level with it; in the units of
    /// `Driver::pos_curr`. With it, the frame the point is past (for its flags).
    ///
    /// On the ground plane, as the steering only looks at that, and a path falls straight down
    /// off a ramp's lip into the kill plane under the gap (retail's bots fly it): measured in 3D,
    /// the point would sink under the lip there, right under a kart about to take off.
    pub fn ahead(&self, path: &[NavFrame], pos: Vec3, distance: i32) -> (Vec3, usize) {
        let n = path.len();
        if n == 0 {
            return (pos, 0);
        }
        let to_world = |q: P| Vec3 {
            x: (q[0] << WORLD_POS_SHIFT) as i32,
            y: (q[1] << WORLD_POS_SHIFT) as i32,
            z: (q[2] << WORLD_POS_SHIFT) as i32,
        };
        let mut i = self.frame as usize % n;
        if n == 1 {
            return (to_world(frame_pos(&path[0])), 0);
        }
        let p = level_pos(pos);
        let mut a = frame_pos(&path[i]);
        let mut b = frame_pos(&path[(i + 1) % n]);
        let flat = |v: P| [v[0], 0, v[2]];
        let mut d = sub(b, a);
        let mut len = dot(flat(d), flat(d)).isqrt();
        // How far along this segment the kart is.
        let along = if len == 0 {
            0
        } else {
            (dot(flat(sub(p, a)), flat(d)) / len).clamp(0, len)
        };
        let mut left = along + distance.max(0) as i64;
        for _ in 0..n {
            if left <= len {
                break;
            }
            left -= len;
            i = (i + 1) % n;
            a = b;
            b = frame_pos(&path[(i + 1) % n]);
            d = sub(b, a);
            len = dot(flat(d), flat(d)).isqrt();
        }
        let left = left.min(len);
        let point = if len == 0 {
            a
        } else {
            [0, 1, 2].map(|k| a[k] + d[k] * left / len)
        };
        (to_world(point), i)
    }
}

/// How a bot steers: its settings, to tune by (`tests/bots.rs` tries some against each other).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Steering {
    /// How far ahead on the path a bot aims (in level units): this much standing still ...
    pub lookahead_min: i32,
    /// ... and as far as it goes in this many ticks on top. (`speed_approx >> 8` is level units
    /// a tick.)
    pub lookahead_ticks: i32,
    /// A second, far point to aim at, this many ticks of travel past the near one: it turns as
    /// soon as a corner comes. (Aiming at it cuts the corner, though: pure pursuit heads
    /// straight for its point.)
    pub far_ticks: i32,
    /// How much the far point counts in the heading error, out of 256 (the near one has the
    /// rest).
    pub far_weight: i32,
    /// The kart turns only some ticks after the stick does, and goes on turning some after it
    /// is straightened: `PhysAngular` builds the turning rate up over a couple of ticks, and then
    /// holds back up to 3072 of it (half of a Speed kart's whole rate) from the 4th tick of a
    /// steer, giving it back over two seconds (`steer_accel`). So the bot steers from where the
    /// kart will be this many ticks on, going and turning as it is now: into a corner before it
    /// is there, and out of it before it has turned too far. (On a steady curve the two cancel.)
    pub predict_ticks: i32,
    /// From this heading error (in 1/0x1000 of a turn) on, the stick is turned all the way: the
    /// steering's gain.
    pub full_error: i32,
    /// Whether the steering's strength is in proportion to the error, rather than the stick's
    /// travel: the joystick's curve (`VehPhysJoystick_GetStrength`) gives only a fifth of the
    /// strength over the first half of the stick past the dead zone, so the stick's travel in
    /// proportion steers small errors very softly.
    pub even_strength: bool,
    /// Whether it hops off the ramps of its path: the hop's jump goes on top of the ramp's
    /// (`VehPhysGeneral_JumpAndFriction`), which the long ones need (retail's bots are launched
    /// over them by `rampPhys` instead).
    pub hop_ramps: bool,
    /// Whether the bot takes a gap of its path as straight from the lip to the landing: the kart
    /// flies straight over it, so it has to leave the lip lined up with the landing, and the
    /// path under a gap (in the kill plane, where retail's bots fly) can bend: Sewer Speedway's
    /// turns about 30 degrees from the ramp. Only the gap: up to the lip the path is as ever (an
    /// aim at the landing from further back cuts the corners before a ramp).
    pub straight_gaps: bool,
    /// The [`Steering::full_error`] on the run-up to a gap (while its straight is aimed at), with
    /// no dead zone; 0 steers as anywhere else. The kart flies straight off the lip, so a heading
    /// error that the ground would shrug off (a couple of degrees, within or just past the dead
    /// zone, which the joystick's curve makes all but nothing) lands it far to the side after a
    /// long jump: about 170 units for 2.4 degrees over Hot Air Skyway's second gap.
    pub gap_full_error: i32,
    /// The most a bot's way of travel is turned in a tick in the air, toward where it should come
    /// down (in 1/0x1000 of a turn); 0 leaves its flight to the physics. A cheat, as retail's: its
    /// bots fly along the path ([`Bot::fly`]).
    pub air_turn: i32,
}

impl Steering {
    pub const DEFAULT: Steering = Steering {
        lookahead_min: 300,
        lookahead_ticks: 12,
        far_ticks: 16,
        far_weight: 0,
        // The best of `tests/bots.rs`'s tries: of 72 laps (4 bots on 18 tracks) it gets round 65,
        // and is grabbed by the mask 110 times (most of them a few bots falling at the same
        // place over and over), against 64 and 144 of the stick's travel in proportion (full at
        // 0x100, no prediction). Sewer Speedway, Cortex Castle, N. Gin Labs, Polar Pass and
        // Oxide Station give every setting tried trouble.
        predict_ticks: 6,
        full_error: 0xc0,
        even_strength: true,
        hop_ramps: true,
        straight_gaps: true,
        // The only one tried that gets every bot over Hot Air Skyway's gaps (0x60 gets one more
        // lap elsewhere, and loses those; 0x30 and 0 do worse).
        gap_full_error: 0x40,
        air_turn: 0x40,
    };
}

impl Default for Steering {
    fn default() -> Steering {
        Steering::DEFAULT
    }
}

/// A ramp is this many frames of the path ahead of the kart, or fewer.
const RAMP_FRAMES: usize = 4;

/// `NavFrame::special_bits`: the frame launches retail's bots by `rampPhys`.
const SPECIAL_RAMP: u8 = 0x10;

/// How many frames back from the kart's frame [`Bot::fly`] looks for the lip of a gap it is over.
const GAP_FRAMES: usize = 64;

/// Off from where it should come down by more than this (1/0x1000 of a turn), a bot in the air is
/// not turned at all ([`Bot::fly`]).
const AIR_TURN_LIMIT: i32 = 0x400;

/// The least distance on the ground plane from a gap's lip to its landing (level units): the
/// paths have frames out of the kill plane a few units past some lips (Hot Air Skyway's last
/// gap: 384 is the lip, 385 a kill plane frame at the lip's height, 386 out of it, 8 units on,
/// and only then the drop), which are no landing.
const MIN_GAP: i64 = 256;

/// The landing of the gap whose lip is frame `lip` of `path`, if it is one: a frame that launches
/// retail's bots, with the path dropping into a kill plane after it. The landing is the first
/// frame out of the kill plane at least [`MIN_GAP`] from the lip on the ground plane.
pub fn gap_landing(path: &[NavFrame], lip: usize) -> Option<usize> {
    let n = path.len();
    let next = (lip + 1) % n;
    if path[lip].special_bits & SPECIAL_RAMP == 0 || path[next].flags & NavFrame::KILL_PLANE == 0
    {
        return None;
    }
    let a = frame_pos(&path[lip]);
    (1..n).map(|k| (lip + k) % n).find(|&j| {
        let b = frame_pos(&path[j]);
        path[j].flags & NavFrame::KILL_PLANE == 0 && sq(b[0] - a[0]) + sq(b[2] - a[2]) >= sq(MIN_GAP)
    })
}

/// Heading errors (in 1/0x1000 of a turn) up to this leave the stick centred.
const STEER_DEAD_ERROR: i32 = 0x10;

/// The strength the stick is turned to, out of this.
const STRENGTH_ONE: i32 = 0x1000;

/// `stick_lx` at the centre, and how far from it the stick starts to steer (the joystick's dead
/// zone) and steers its most.
const STICK_CENTER: i32 = 0x80;
const STICK_DEAD_ZONE: i32 = 0x30;
const STICK_RANGE: i32 = 0x7f;

/// Slower than this (`speed_approx`) while it should be going, a bot is stuck ...
const STUCK_SPEED: i32 = 0x200;
/// ... once it has been for this many ticks; it then backs out for as many.
const STUCK_TICKS: u16 = 30;
const REVERSE_TICKS: u16 = 30;

/// A kart the race drives: which of the level's paths it follows and where on it it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bot {
    /// Into `Level::nav_paths`.
    pub path: u8,
    pub tracker: Tracker,
    /// The ticks it has been stuck for.
    pub stuck: u16,
    /// The ticks it still backs out for.
    pub reversing: u16,
    pub steering: Steering,
}

impl Bot {
    /// A bot for the kart of `driver`, in place `slot`: on the path of the slot's number (as
    /// retail spreads its bots over the three), or the first the level has.
    pub fn new(level: &Level, slot: usize, driver: &Driver) -> Bot {
        let paths = &level.nav_paths;
        let path = (0..paths.len())
            .map(|k| (slot + k) % paths.len())
            .find(|&i| paths[i].frames.len() >= 2)
            .unwrap_or(0);
        Bot {
            path: path as u8,
            tracker: Tracker::find(&paths[path].frames, driver.pos_curr),
            stuck: 0,
            reversing: 0,
            steering: Steering::DEFAULT,
        }
    }

    /// The bot's pad for this tick, its kart being `driver`. `racing` is whether the lights have
    /// gone green (before that, a kart standing still is not stuck).
    pub fn pad(&mut self, level: &Level, driver: &Driver, racing: bool) -> Pad {
        let path = &level.nav_paths[self.path as usize].frames;
        if path.len() < 2 {
            return Pad {
                held: buttons::CROSS,
                ..Pad::default()
            };
        }
        let airborne = driver.actions_flag_set & actions::TOUCH_GROUND == 0;
        if airborne {
            self.tracker.follow(path, driver.pos_curr);
        } else {
            self.tracker.update(path, driver.pos_curr);
        }

        let speed = (driver.speed_approx as i32).abs();
        // Stuck on a wall (or another kart): back out of it, the other way round.
        if self.reversing > 0 {
            self.reversing -= 1;
        } else if racing && speed < STUCK_SPEED && driver.kart_state == kart_state::NORMAL {
            self.stuck += 1;
            if self.stuck >= STUCK_TICKS {
                self.stuck = 0;
                self.reversing = REVERSE_TICKS;
            }
        } else {
            self.stuck = 0;
        }

        let pos = driver.pos_curr;
        let st = self.steering;
        let lookahead = st.lookahead_min + (speed >> 8) * st.lookahead_ticks;
        if self.reversing > 0 {
            let (near, _frame) = self.tracker.ahead(path, pos, lookahead);
            // Backing up, the stick turns the kart the other way: this points its nose at
            // the path.
            return Pad {
                held: buttons::SQUARE,
                lx: stick_for(-heading_error(driver.angle as i32, pos, near), st),
                ..Pad::default()
            };
        }

        // In the air the stick turns the kart's nose, but not where it goes: steering for the path
        // would only land it askew. So it keeps the nose on the way it flies, to land straight.
        if airborne {
            let v = driver.velocity;
            let ahead = Vec3 {
                x: pos.x.wrapping_add(v.x.wrapping_mul(16)),
                y: pos.y,
                z: pos.z.wrapping_add(v.z.wrapping_mul(16)),
            };
            // Just off a ramp, while a hop still counts (the coyote time): hop. Held until it
            // has (the cooldown starts), and not after, or it would drift on landing.
            let hop = st.hop_ramps
                && driver.kart_state == kart_state::NORMAL
                && driver.jump_coyote_timer_ms > 0
                && driver.jump_cooldown_ms == 0
                && self.ramp_ahead(path);
            return Pad {
                held: buttons::CROSS | if hop { buttons::R1 } else { 0 },
                lx: stick_for(heading_error(driver.angle as i32, pos, ahead), st),
                ..Pad::default()
            };
        }

        // Where the kart will be and which way it will face: `velocity` moves `pos_curr` by
        // itself a tick, `amp_turn_state` turns the angle by itself / 256 (`PhysAngular`).
        // Spinning out or the like, the turn is not the steering's: it goes on from the angle.
        let v = driver.velocity;
        let then = Vec3 {
            x: pos.x.wrapping_add(v.x.wrapping_mul(st.predict_ticks)),
            y: pos.y.wrapping_add(v.y.wrapping_mul(st.predict_ticks)),
            z: pos.z.wrapping_add(v.z.wrapping_mul(st.predict_ticks)),
        };
        let mut angle = driver.angle as i32;
        if driver.kart_state == kart_state::NORMAL {
            angle += (driver.amp_turn_state as i32 * st.predict_ticks) >> 8;
        }
        // The points ahead of it then: as far again past what it goes in the meantime.
        let travel = (speed >> 8) * st.predict_ticks;
        let gap = st
            .straight_gaps
            .then(|| self.over_gap(path, pos, travel + lookahead))
            .flatten();
        let near = gap.unwrap_or_else(|| self.tracker.ahead(path, pos, travel + lookahead).0);
        let mut error = heading_error(angle, then, near);
        if st.far_weight != 0 {
            let far_distance = travel + lookahead + (speed >> 8) * st.far_ticks;
            let (far, _frame) = self.tracker.ahead(path, pos, far_distance);
            let far_error = heading_error(angle, then, far);
            error = (error * (256 - st.far_weight) + far_error * st.far_weight) >> 8;
        }
        let lx = if gap.is_some() && st.gap_full_error > 0 {
            stick_with(error, 0, st.gap_full_error, st.even_strength)
        } else {
            stick_for(error, st)
        };
        Pad {
            held: buttons::CROSS,
            lx,
            ..Pad::default()
        }
    }
}

impl Bot {
    /// After the kart's tick: in the air, its way of travel turns toward where it should come down
    /// (up to [`Steering::air_turn`] a tick), its speed kept. Over a gap that is the gap's landing,
    /// elsewhere the path ahead. Retail's bots fly the path itself (`BOTS_Driver_Convert`); this
    /// is that, only gentler, and the kart still flies by its physics.
    ///
    /// The way it goes is `Driver::axis_rotation_x`: the next tick makes the velocity from it and
    /// the speed (`VehPhysForce_ConvertSpeedToVec`). The velocity turns too, so that the nose,
    /// which [`Bot::pad`] keeps on it in the air, follows.
    pub fn fly(&self, level: &Level, driver: &mut Driver) {
        let st = self.steering;
        let path = &level.nav_paths[self.path as usize].frames;
        if st.air_turn <= 0
            || path.len() < 2
            || driver.actions_flag_set & actions::TOUCH_GROUND != 0
            || driver.kart_state != kart_state::NORMAL
        {
            return;
        }
        let v = driver.velocity;
        let going_speed = (sq(v.x as i64) + sq(v.z as i64)).isqrt();
        if going_speed == 0 {
            return;
        }
        let pos = driver.pos_curr;
        let target = match self.gap_landing_around(path) {
            Some(landing) => {
                let f = frame_pos(&path[landing]);
                Vec3 {
                    x: (f[0] << WORLD_POS_SHIFT) as i32,
                    y: (f[1] << WORLD_POS_SHIFT) as i32,
                    z: (f[2] << WORLD_POS_SHIFT) as i32,
                }
            }
            None => {
                let speed = (driver.speed_approx as i32).abs();
                let lookahead = st.lookahead_min + (speed >> 8) * st.lookahead_ticks;
                self.tracker.ahead(path, pos, lookahead).0
            }
        };
        let going = math::ratan2(v.x, v.z);
        let error = heading_error(going, pos, target);
        // A target behind it is the tracker's mistake, not a way to fly.
        if error.abs() > AIR_TURN_LIMIT {
            return;
        }
        let way = (going + error.clamp(-st.air_turn, st.air_turn)) & 0xfff;
        let (sin, cos) = math::sin_cos(way as u32);
        driver.velocity.x = ((going_speed * sin as i64) >> 12) as i32;
        driver.velocity.z = ((going_speed * cos as i64) >> 12) as i32;
        driver.axis_rotation_x = way as i16;
    }

    /// The landing of the gap the kart is over: one whose lip is up to [`RAMP_FRAMES`] ahead of
    /// its frame (it has just taken off), or behind it with the landing still ahead.
    fn gap_landing_around(&self, path: &[NavFrame]) -> Option<usize> {
        let n = path.len();
        let frame = self.tracker.frame as usize % n;
        // How many frames on from `a` frame `b` is.
        let on = |a: usize, b: usize| (b + n - a) % n;
        (0..=RAMP_FRAMES)
            .map(|k| ((frame + k) % n, true))
            .chain((1..=GAP_FRAMES.min(n - 1)).map(|k| ((frame + n - k) % n, false)))
            .find_map(|(lip, ahead)| {
                let landing = gap_landing(path, lip)?;
                (ahead || on(lip, frame) < on(lip, landing)).then_some(landing)
            })
    }

    /// The point `distance` (on the ground plane) ahead of the kart at `pos`, if that is past
    /// the lip of a gap: on the straight line from the lip to the landing. A gap is a lip (a
    /// frame that launches retail's bots) with the path dropping into a kill plane after it; the
    /// landing is the first frame out of the kill plane.
    fn over_gap(&self, path: &[NavFrame], pos: Vec3, distance: i32) -> Option<Vec3> {
        let n = path.len();
        let ground = |a: P, b: P| (sq(b[0] - a[0]) + sq(b[2] - a[2])).isqrt();
        // The frame the kart has passed first (it is on the lip just before it takes off), as 0
        // away; then on from the next.
        let mut i = self.tracker.frame as usize % n;
        // How far it is to frame `i`.
        let mut gone = 0;
        for step in 0..n {
            if step == 1 {
                gone = ground(level_pos(pos), frame_pos(&path[i]));
            }
            if gone > distance as i64 {
                return None;
            }
            let next = (i + 1) % n;
            if let Some(landing) = gap_landing(path, i) {
                let (a, b) = (frame_pos(&path[i]), frame_pos(&path[landing]));
                let d = sub(b, a);
                let len = ground(a, b);
                let left = (distance as i64 - gone).min(len);
                let q = if len == 0 {
                    a
                } else {
                    [0, 1, 2].map(|k| a[k] + d[k] * left / len)
                };
                return Some(Vec3 {
                    x: (q[0] << WORLD_POS_SHIFT) as i32,
                    y: (q[1] << WORLD_POS_SHIFT) as i32,
                    z: (q[2] << WORLD_POS_SHIFT) as i32,
                });
            }
            if step > 0 {
                gone += ground(frame_pos(&path[i]), frame_pos(&path[next]));
            }
            i = next;
        }
        None
    }

    /// Whether one of the next few frames of the path is a ramp, retail's (it launches its bots
    /// there) or a jump.
    fn ramp_ahead(&self, path: &[NavFrame]) -> bool {
        (0..=RAMP_FRAMES).any(|k| {
            let f = &path[(self.tracker.frame as usize + k) % path.len()];
            f.special_bits & SPECIAL_RAMP != 0 || f.flags & NavFrame::JUMP != 0
        })
    }
}

/// How far the kart heading `angle` (`Driver::angle`, 0x1000 a turn) has to turn to face
/// `target` from `pos`, as `-0x800..0x800`: positive is a turn that raises the angle (left). On
/// the ground plane, the kart goes toward `(sin angle, cos angle)` in x and z.
fn heading_error(angle: i32, pos: Vec3, target: Vec3) -> i32 {
    let dx = (target.x - pos.x) >> WORLD_POS_SHIFT;
    let dz = (target.z - pos.z) >> WORLD_POS_SHIFT;
    if dx == 0 && dz == 0 {
        return 0;
    }
    let want = math::ratan2(dx, dz);
    ((want - angle + 0x800) & 0xfff) - 0x800
}

/// The `stick_lx` that turns by a heading error of `error` (see [`heading_error`]): a stick to
/// the left (below the centre) raises the angle. In proportion between [`STEER_DEAD_ERROR`] and
/// [`Steering::full_error`]: the stick's travel past its dead zone, or with
/// [`Steering::even_strength`] the strength (the least stick that gives it).
fn stick_for(error: i32, st: Steering) -> u8 {
    stick_with(error, STEER_DEAD_ERROR, st.full_error, st.even_strength)
}

/// [`stick_for`] with errors up to `dead` leaving the stick centred, and the stick all the way
/// from `full` on.
fn stick_with(error: i32, dead: i32, full: i32, even_strength: bool) -> u8 {
    let size = error.abs();
    if size <= dead {
        return STICK_CENTER as u8;
    }
    let part = |one: i32| (size - dead).min(full - dead) * one / (full - dead);
    let off = if even_strength {
        let want = part(STRENGTH_ONE);
        (STICK_DEAD_ZONE..STICK_RANGE)
            .find(|&off| vehicle::joystick::strength(off, STRENGTH_ONE) >= want)
            .unwrap_or(STICK_RANGE)
    } else {
        STICK_DEAD_ZONE + part(STICK_RANGE - STICK_DEAD_ZONE)
    };
    (STICK_CENTER - error.signum() * off) as u8
}

/// The direction from `a` to `b`, 4096 long (zero if they are the same point).
fn unit(a: P, b: P) -> P {
    let d = sub(b, a);
    let len = dot(d, d).isqrt();
    if len == 0 {
        return [0; 3];
    }
    d.map(|v| v * 4096 / len)
}

/// Whether `p` is past frame `j` of `path`: on the far side of the plane through it that halves
/// the turn there. (Past it along the segment to it would not do: a kart that cuts the inside of a
/// corner never gets there.)
fn passed(path: &[NavFrame], j: usize, p: P) -> bool {
    let n = path.len();
    let here = frame_pos(&path[j]);
    let before = frame_pos(&path[(j + n - 1) % n]);
    let after = frame_pos(&path[(j + 1) % n]);
    let a = unit(before, here);
    let b = unit(here, after);
    dot(sub(p, here), [a[0] + b[0], a[1] + b[1], a[2] + b[2]]) >= 0
}

/// The frame of `path` among `frames` that is nearest to `p`, and the square of its distance.
fn nearest(path: &[NavFrame], p: P, frames: impl Iterator<Item = usize>) -> (usize, i64) {
    let mut best = (0, i64::MAX);
    for i in frames {
        let off = sub(p, frame_pos(&path[i]));
        let d2 = dot(off, off);
        if d2 < best.1 {
            best = (i, d2);
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use loader::lev::SVec3;

    /// A square loop of side `side` in the ground plane, a frame every 100 units, going +x, +z,
    /// -x, -z from the origin.
    fn square_of(side: i32) -> Vec<NavFrame> {
        let mut frames = Vec::new();
        let corners = [(0, 0), (side, 0), (side, side), (0, side)];
        let steps = side / 100;
        for k in 0..4 {
            let (x0, z0) = corners[k];
            let (x1, z1) = corners[(k + 1) % 4];
            for s in 0..steps {
                frames.push(NavFrame {
                    pos: SVec3 {
                        x: (x0 + (x1 - x0) * s / steps) as i16,
                        y: 0,
                        z: (z0 + (z1 - z0) * s / steps) as i16,
                    },
                    ..Default::default()
                });
            }
        }
        frames
    }

    fn square() -> Vec<NavFrame> {
        square_of(1000)
    }

    fn world(x: i32, y: i32, z: i32) -> Vec3 {
        Vec3 {
            x: x << 8,
            y: y << 8,
            z: z << 8,
        }
    }

    #[test]
    fn finds_and_follows() {
        let path = square();
        let mut t = Tracker::find(&path, world(150, 0, 20));
        assert_eq!(t.frame, 1);
        // Driving along: a frame at a time.
        for x in (150..960).step_by(30) {
            assert!(!t.update(&path, world(x, 0, 20)));
            assert_eq!(t.frame as i32, x / 100);
        }
        // Cutting the corner, and on up the second side.
        assert!(!t.update(&path, world(990, 0, 20)));
        assert_eq!(t.frame, 10);
        assert!(!t.update(&path, world(990, 0, 120)));
        assert_eq!(t.frame, 11);
        assert_eq!(t.relocations, 0);
    }

    #[test]
    fn wraps_and_backs_up() {
        let path = square();
        let mut t = Tracker::find(&path, world(10, 0, 50));
        assert_eq!(t.frame, 39);
        assert!(!t.update(&path, world(50, 0, -10)));
        assert_eq!(t.frame, 0);
        // Reversing over the line.
        assert!(!t.update(&path, world(-10, 0, 150)));
        assert_eq!(t.frame, 38);
    }

    #[test]
    fn searches_after_a_teleport() {
        // Big enough that the far side is further than LOST_DISTANCE.
        let path = square_of(4000);
        let mut t = Tracker::find(&path, world(150, 0, 0));
        assert_eq!(t.frame, 1);
        // Put on the far side: too many frames on to step over.
        assert!(t.update(&path, world(1950, 0, 4000)));
        assert_eq!(t.frame, 100);
        assert_eq!(t.relocations, 1);
        // Knocked off the track, far.
        assert!(t.update(&path, world(2050, 0, 6000)));
        assert_eq!(t.frame, 99);
        assert_eq!(t.relocations, 2);
    }

    #[test]
    fn looks_ahead_over_a_drop() {
        // Up a ramp to a lip, straight down into the kill plane, and on under the gap.
        let at = |x: i16, y: i16| NavFrame {
            pos: SVec3 { x, y, z: 0 },
            ..Default::default()
        };
        let path = vec![
            at(0, 0),
            at(200, 50),
            at(400, 100),
            at(400, -16000),
            at(1200, -16000),
            at(2000, -16000),
        ];
        let t = Tracker::find(&path, world(390, 98, 0));
        assert_eq!(t.frame, 1);
        // Measured on the ground, the point is past the drop, on the way over the gap: not
        // under the lip.
        let (point, frame) = t.ahead(&path, world(390, 98, 0), 300);
        assert_eq!((point.x >> 8, frame), (690, 3));
    }

    #[test]
    fn aims_straight_over_a_gap() {
        // Up to a lip at x 400, into the kill plane, and out of it at (1200, 600): the gap
        // turns off the ramp's line.
        let at = |x: i16, y: i16, z: i16, flags: u16, special_bits: u8| NavFrame {
            pos: SVec3 { x, y, z },
            flags,
            special_bits,
            ..Default::default()
        };
        let kill = NavFrame::KILL_PLANE;
        let path = vec![
            at(0, 0, 0, 0, 0),
            at(200, 0, 0, 0, 0),
            at(400, 0, 0, 0, SPECIAL_RAMP),
            at(400, -16000, 0, kill, 0),
            at(800, -16000, 300, kill, 0),
            at(1200, 0, 600, 0, 0),
            at(1200, 0, 1200, 0, 0),
            at(0, 0, 1200, 0, 0),
        ];
        let bot = Bot {
            path: 0,
            tracker: Tracker::find(&path, world(150, 0, 0)),
            stuck: 0,
            reversing: 0,
            steering: Steering::DEFAULT,
        };
        // 250 to the lip, and 1000 on the straight from it to the landing (800 by 600).
        assert_eq!(
            bot.over_gap(&path, world(150, 0, 0), 750),
            Some(world(800, 0, 300))
        );
        // No further than the landing.
        assert_eq!(
            bot.over_gap(&path, world(150, 0, 0), 5000),
            Some(world(1200, 0, 600))
        );
        // Short of the lip: the path as ever.
        assert_eq!(bot.over_gap(&path, world(150, 0, 0), 200), None);
    }

    #[test]
    fn a_landing_is_past_the_lip() {
        // As Hot Air Skyway's last gap: the lip, a kill plane frame at its height and one out
        // of it a few units on, then the drop, and the landing.
        let at = |z: i16, y: i16, flags: u16, special_bits: u8| NavFrame {
            pos: SVec3 { x: 0, y, z },
            flags,
            special_bits,
            ..Default::default()
        };
        let kill = NavFrame::KILL_PLANE;
        let path = vec![
            at(0, 0, 0, 0),
            at(300, 0, 0, SPECIAL_RAMP | 1),
            at(307, -120, kill, 0),
            at(308, -60, 0, 0),
            at(318, -16000, kill, 0),
            at(1500, -16000, kill, 0),
            at(2400, -700, 0, 0),
            at(3000, -700, 0, 0),
        ];
        assert_eq!(gap_landing(&path, 1), Some(6));
        // Not a lip.
        assert_eq!(gap_landing(&path, 0), None);
        assert_eq!(gap_landing(&path, 3), None);
    }

    #[test]
    fn follows_through_the_air() {
        let path = square_of(4000);
        let mut t = Tracker::find(&path, world(150, 0, 0));
        // High over the path, well past LOST_DISTANCE: it steps along, and does not search.
        t.follow(&path, world(450, 3000, 0));
        assert_eq!((t.frame, t.relocations), (4, 0));
        // Back on the ground it would have.
        assert!(t.update(&path, world(1950, 0, 4000)));
    }

    #[test]
    fn heading_errors() {
        let at = world(0, 0, 0);
        // Facing +z (angle 0) with the target ahead, to +x (a quarter turn up), and behind.
        assert_eq!(heading_error(0, at, world(0, 0, 500)), 0);
        assert_eq!(heading_error(0, at, world(500, 0, 0)), 0x400);
        assert_eq!(heading_error(0x400, at, world(0, 0, 500)), -0x400);
        // Over the wrap: from just under a full turn to just over 0.
        assert_eq!(heading_error(0xff0, at, world(0, 0, 500)), 0x10);
        assert_eq!(heading_error(0x10, at, world(0, 0, 500)), -0x10);
    }

    #[test]
    fn stick_for_errors() {
        const FULL: i32 = 0xc0;
        let st = Steering {
            full_error: FULL,
            even_strength: true,
            ..Steering::DEFAULT
        };
        let stick_for = |error| stick_for(error, st);
        assert_eq!(stick_for(0), 0x80);
        assert_eq!(stick_for(STEER_DEAD_ERROR), 0x80);
        assert_eq!(stick_for(-STEER_DEAD_ERROR), 0x80);
        // Just past the dead error, just past the stick's dead zone; left for a raise.
        assert!((0x80 - 0x34..0x80 - 0x30).contains(&stick_for(STEER_DEAD_ERROR + 1)));
        assert!((0x80 + 0x30..0x80 + 0x34).contains(&stick_for(-STEER_DEAD_ERROR - 1)));
        // The strength the stick gives goes with the error.
        let strength = |error| {
            vehicle::joystick::strength_absolute(stick_for(error) as i32, STRENGTH_ONE).abs()
        };
        let mut last = 0;
        for size in STEER_DEAD_ERROR + 1..=FULL {
            let want = (size - STEER_DEAD_ERROR) * STRENGTH_ONE
                / (FULL - STEER_DEAD_ERROR);
            let got = strength(size);
            assert!(got >= want && got >= last, "{size:#x}: {got} for {want}");
            // Near enough: a step of the stick is under 1/50 of it. (Bar full lock: the curve
            // leaps from about 3/4 to all of it at the stick's end.)
            if got < STRENGTH_ONE {
                assert!(got - want <= STRENGTH_ONE / 16, "{size:#x}: {got} for {want}");
            }
            last = got;
        }
        // All the way, and no further.
        assert_eq!(stick_for(FULL), 0x01);
        assert_eq!(stick_for(-0x800), 0xff);
        assert_eq!(stick_for(0x7ff), 0x01);
        // The stick's travel in proportion, as it was first.
        let st = Steering {
            full_error: 0x100,
            even_strength: false,
            ..Steering::DEFAULT
        };
        assert_eq!(super::stick_for(STEER_DEAD_ERROR + 1, st), 0x80 - 0x30);
        assert_eq!(super::stick_for(-STEER_DEAD_ERROR - 1, st), 0x80 + 0x30);
        assert_eq!(super::stick_for(0x100, st), 0x01);
        assert_eq!(super::stick_for(-0x800, st), 0xff);
    }

    #[test]
    fn looks_ahead_along_the_path() {
        let path = square();
        let t = Tracker::find(&path, world(950, 0, 30));
        let (point, frame) = t.ahead(&path, world(950, 0, 30), 250);
        assert_eq!(point, world(1000, 0, 200));
        assert_eq!(frame, 11);
        // Over the end of the loop.
        let t = Tracker::find(&path, world(0, 0, 150));
        let (point, frame) = t.ahead(&path, world(0, 0, 150), 300);
        assert_eq!(point, world(150, 0, 0));
        assert_eq!(frame, 1);
    }
}
