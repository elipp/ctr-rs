//! `RB_Seal_ThTick_*`: the seals of Blizzard Bluff slide along a line between the two points of
//! their path (45 ticks each way), turn half way round at either end, and slide back. A kart
//! one touches is spun out.

use loader::lev::{InstDef, SVec3};
use vehicle::rotmat::rot_to_matrix;
use vehicle::{Matrix, Sound3D};

use crate::{Contact, Context, Damage, Instance, Pose, Tick, interpolate};

/// Ticks to slide from one end of the path to the other (`distFromSpawn` runs to this).
const SLIDE_TICKS: i32 = 0x2d;
/// The animation advances two frames a tick.
const ANIM_STEP: i16 = 2;
/// How the facing turns (`RB_Hazard_InterpolateValue`'s speed with 32 ms ticks: this many angle
/// units a tick), and how the slant of its body turns over.
const TURN_SPEED: i32 = 0x80;
const SLANT_SPEED: i32 = 0x14;
const HALF_TURN: i32 = 0x800;
const FULL_TURN: i32 = 0x1000;
/// `RB_CheckHazardCollisions(sealInst, t, 1, 0x4000, 0x78)`: a squared distance of 128 units.
const HIT_RADIUS: u32 = 0x4000;
const HIT_SOUND: i32 = 0x78;
/// The bark of a seal turning round, each time its animation starts over.
const TURN_SFX: u8 = 0x77;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// `RB_Seal_ThTick_Move`
    Moving,
    /// `RB_Seal_ThTick_TurnAround`
    Turning,
}

/// `struct Seal` and the parts of its `Instance` the code changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seal {
    /// `matrix.t`: where it is, in level units.
    pub pos: [i32; 3],
    /// The two ends of its path: `spawnPos`, and the difference to the other end (`vel`).
    start: [i16; 3],
    vel: [i16; 3],
    /// How far along the path it is, in ticks (`distFromSpawn`).
    dist: i32,
    /// Going away from the start (`direction`).
    away: bool,
    rot_curr: SVec3,
    rot_desired: SVec3,
    turn_around_y: i16,
    phase: Phase,
    pub anim_frame: i16,
    matrix: Matrix,
}

impl Seal {
    /// `RB_Seal_LInB`: at the definition's place, facing as it says, about to slide from `start`
    /// towards `end`, the first two points of the path its name's digit picks.
    pub fn new(def: &InstDef, start: SVec3, end: SVec3) -> Seal {
        Seal {
            pos: [def.pos.x as i32, def.pos.y as i32, def.pos.z as i32],
            start: [start.x, start.y, start.z],
            vel: [
                start.x.wrapping_sub(end.x),
                start.y.wrapping_sub(end.y),
                start.z.wrapping_sub(end.z),
            ],
            dist: 0,
            away: true,
            rot_curr: def.rot,
            rot_desired: def.rot,
            turn_around_y: 0,
            phase: Phase::Moving,
            anim_frame: 0,
            matrix: rot_to_matrix(def.rot),
        }
    }

    /// One game tick. `frames` is the number of frames of its animation. Where it looks for
    /// drivers, and the bark of a turn, go in `out`.
    pub fn tick(&mut self, frames: i32, out: &mut Tick) {
        // A thread that switches procedures (`ThTick_SetAndExec`) runs the new one at once, which
        // steps the animation again.
        loop {
            let wrapped = self.step_animation(frames);
            match self.phase {
                Phase::Moving => {
                    for a in 0..3 {
                        self.pos[a] = self.start[a] as i32
                            - self.dist * self.vel[a] as i32 / SLIDE_TICKS;
                    }
                    let sliding = if self.away {
                        self.dist < SLIDE_TICKS
                    } else {
                        self.dist > 0
                    };
                    if sliding {
                        self.dist += if self.away { 1 } else { -1 };
                    } else {
                        // Reached an end of the path: turn half way round, then go back.
                        self.away = !self.away;
                        self.turn_around_y =
                            ((self.rot_curr.y as i32 + HALF_TURN) % FULL_TURN) as i16;
                        self.phase = Phase::Turning;
                        continue;
                    }
                }
                Phase::Turning => {
                    if wrapped {
                        out.sounds.push(Sound3D {
                            id: TURN_SFX,
                            pos: self.pos,
                        });
                    }
                    if self.rot_curr.y != self.turn_around_y {
                        let slant = |curr: i16, desired: i16| {
                            interpolate(curr, desired.wrapping_neg(), SLANT_SPEED)
                        };
                        self.rot_curr.y = interpolate(self.rot_curr.y, self.turn_around_y, TURN_SPEED);
                        self.rot_curr.x = slant(self.rot_curr.x, self.rot_desired.x);
                        self.rot_curr.z = slant(self.rot_curr.z, self.rot_desired.z);
                        self.matrix = rot_to_matrix(self.rot_curr);
                    } else {
                        // The new slant is where the next turn starts from.
                        self.rot_desired = self.rot_curr;
                        self.matrix = rot_to_matrix(self.rot_curr);
                        self.phase = Phase::Moving;
                        continue;
                    }
                }
            }
            out.contacts
                .push(Contact::sphere(self.pos, Damage::Spin, HIT_RADIUS, HIT_SOUND));
            return;
        }
    }

    /// Two frames on, or back to the first. Whether it started over.
    fn step_animation(&mut self, frames: i32) -> bool {
        if (self.anim_frame as i32 + ANIM_STEP as i32) < frames {
            self.anim_frame += ANIM_STEP;
            false
        } else {
            self.anim_frame = 0;
            true
        }
    }
}

impl Instance for Seal {
    fn th_tick(&mut self, cx: &Context, out: &mut Tick) {
        self.tick(cx.frames[0], out);
    }

    fn pose(&self) -> Option<Pose> {
        Some(Pose {
            pos: self.pos,
            matrix: self.matrix,
            animation: (0, self.anim_frame),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAMES: i32 = 20;

    fn def() -> InstDef {
        InstDef {
            name: "seal#0".into(),
            model: 0,
            model_name: "seal".into(),
            scale: Default::default(),
            color_rgba: 0,
            flags: 0,
            pos: SVec3 {
                x: 7,
                y: 8,
                z: 9,
            },
            rot: SVec3 {
                x: 20,
                y: 0x400,
                z: 0,
            },
            model_id: 0,
        }
    }

    /// A path of 450 units along -x.
    fn seal() -> Seal {
        let end = SVec3 {
            x: -450,
            y: 90,
            z: 0,
        };
        Seal::new(&def(), SVec3::default(), end)
    }

    #[test]
    fn it_slides_to_the_other_end_in_forty_five_ticks() {
        let mut seal = seal();
        let mut out = Tick::default();
        // The first tick puts it at the start of the path.
        seal.tick(FRAMES, &mut out);
        assert_eq!(seal.pos, [0, 0, 0]);
        seal.tick(FRAMES, &mut out);
        assert_eq!(seal.pos, [-10, 2, 0]);
        for _ in 0..44 {
            seal.tick(FRAMES, &mut out);
        }
        // The 46th tick is at the end, and turns.
        assert_eq!(seal.pos, [-450, 90, 0]);
        assert_eq!(seal.phase, Phase::Turning);
        assert_eq!(out.contacts.len(), 46);
    }

    #[test]
    fn it_turns_half_way_round_and_slides_back() {
        let mut seal = seal();
        let mut out = Tick::default();
        for _ in 0..46 {
            seal.tick(FRAMES, &mut out);
        }
        let end = seal.pos;
        let mut turning = 0;
        while seal.phase == Phase::Turning {
            seal.tick(FRAMES, &mut out);
            turning += 1;
            assert!(turning < 100);
            assert!(seal.pos == end || seal.phase == Phase::Moving);
        }
        // Half a turn in steps of 0x80 (the 46th tick made the first), and the tick that ends it
        // already slides.
        assert_eq!(turning, 16);
        assert_eq!(seal.rot_curr.y, 0xc00);
        assert!(!seal.away);
        // The slant is the opposite one now.
        assert_eq!(seal.rot_curr.x, (-20i16).rem_euclid(0x1000));
        // It goes back the way it came: the tick that ended the turn already slid.
        assert_eq!(seal.pos, end);
        seal.tick(FRAMES, &mut out);
        assert_eq!(seal.pos, [-440, 88, 0]);
        for _ in 0..44 {
            seal.tick(FRAMES, &mut out);
        }
        // Back at the start, and turning again to go away.
        assert_eq!(seal.pos, [0, 0, 0]);
        assert!(seal.away);
    }

    #[test]
    fn it_barks_as_its_animation_starts_over_in_a_turn_only() {
        let mut seal = seal();
        let mut out = Tick::default();
        for _ in 0..40 {
            seal.tick(FRAMES, &mut out);
        }
        // (The animation wraps while it slides, silently.)
        assert!(out.sounds.is_empty());
        for _ in 0..40 {
            seal.tick(FRAMES, &mut out);
        }
        assert!(!out.sounds.is_empty());
        assert!(out.sounds.iter().all(|s| s.id == TURN_SFX));
    }

    #[test]
    fn it_spins_out_what_it_touches() {
        let mut seal = seal();
        let mut out = Tick::default();
        seal.tick(FRAMES, &mut out);
        let contact = out.contacts[0];
        assert_eq!(
            (contact.damage, contact.hit_radius, contact.sound),
            (Damage::Spin, 0x4000, 0x78)
        );
        assert!(contact.hits([100, 0, 0]));
        assert!(!contact.hits([128, 0, 0]));
    }
}
