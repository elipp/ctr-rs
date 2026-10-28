//! `RB_Armadillo_*`: an armadillo rolls along its facing for a while, turns around on the spot,
//! and rolls back, over and over.

use loader::lev::{InstDef, SVec3};
use vehicle::{Matrix, Sound3D};

use crate::{Contact, Context, Damage, Instance, Pose, Tick, interpolate};
use vehicle::rotmat::rot_to_matrix;

/// Ticks of rolling are counted in milliseconds of `elapsedTimeMS`: 32 a tick, up to `ROLL_TIME`.
const ROLL_STEP: i16 = 0x20;
const ROLL_TIME: i16 = 0x500;
/// How the facing turns (`RB_Hazard_InterpolateValue`'s speed, with 32 ms ticks: this many angle
/// units a tick) and how far: half a turn.
const TURN_SPEED: i32 = 0x100;
const HALF_TURN: i16 = 0x800;
const FULL_TURN: i16 = 0x1000;
/// `RB_CheckHazardCollisions(armInst, t, 1, 0x2400, 0x71)`: a squared distance of 96 units.
const HIT_RADIUS: u32 = 0x2400;
const HIT_SOUND: i32 = 0x71;
/// The squeak of an armadillo turning back.
const TURN_SFX: u8 = 0x70;

/// The model's animations (`INSTANCE_GetNumAnimFrames`).
pub mod anim {
    pub const TURN: u8 = 0;
    pub const ROLL: u8 = 1;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// `RB_Armadillo_ThTick_Rolling`
    Rolling,
    /// `RB_Armadillo_ThTick_TurnAround`
    Turning,
}

/// `struct Armadillo` and the parts of its `Instance` the code changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Armadillo {
    /// `matrix.t`: where it is, in level units.
    pub pos: [i32; 3],
    /// The facing and its target (`rotCurr`, `rotDesired.y`).
    rot: SVec3,
    rot_desired_y: i16,
    /// How far it moves a tick (`velX`, `velZ`); negated at each turn.
    vel: [i16; 2],
    time_rolling: i16,
    /// Ticks to wait before the first roll, so that the armadillos do not move in step.
    time_at_edge: u16,
    phase: Phase,
    /// `animIndex` and `animFrame`
    pub anim_index: u8,
    pub anim_frame: i16,
    matrix: Matrix,
}

fn matrix_of(rot: SVec3) -> Matrix {
    rot_to_matrix(rot)
}

impl Armadillo {
    /// `RB_Armadillo_LInB`: at the definition's place, facing as it says. `delay` is the entry of
    /// the level's table (`ptrSpawnType1`) for the digit its name ends in.
    pub fn new(def: &InstDef, delay: u16) -> Armadillo {
        let matrix = matrix_of(def.rot);
        let rot = SVec3 {
            x: def.rot.x,
            y: def.rot.y.rem_euclid(FULL_TURN),
            z: def.rot.z,
        };
        Armadillo {
            pos: [def.pos.x as i32, def.pos.y as i32, def.pos.z as i32],
            rot,
            rot_desired_y: (rot.y + HALF_TURN) % FULL_TURN,
            vel: [matrix.m[0][2] >> 7, matrix.m[2][2] >> 7],
            time_rolling: 0,
            time_at_edge: delay,
            phase: Phase::Rolling,
            anim_index: anim::ROLL,
            anim_frame: 0,
            matrix,
        }
    }

    /// The orientation (`matrix`, without its translation).
    pub fn matrix(&self) -> Matrix {
        self.matrix
    }

    fn contact(&self) -> Contact {
        Contact::sphere(self.pos, Damage::Spin, HIT_RADIUS, HIT_SOUND)
    }

    /// One game tick. `frames` is the number of frames of its turn and roll animations. Where it
    /// looks for drivers, unless it is still waiting at the edge.
    pub fn tick(&mut self, frames: [i32; 2]) -> Option<Contact> {
        // A thread that switches procedures (`ThTick_SetAndExec`) runs the new one at once.
        loop {
            match self.phase {
                Phase::Rolling => {
                    if self.time_at_edge != 0 {
                        self.time_at_edge -= 1;
                        return None;
                    } else if self.time_rolling < ROLL_TIME {
                        self.time_rolling += ROLL_STEP;
                        self.pos[0] += self.vel[0] as i32;
                        self.pos[2] += self.vel[1] as i32;
                        self.anim_frame =
                            if (self.anim_frame as i32 + 1) < frames[anim::ROLL as usize] {
                                self.anim_frame + 1
                            } else {
                                0
                            };
                    } else {
                        self.time_rolling = 0;
                        self.anim_index = anim::TURN;
                        self.anim_frame = 0;
                        self.rot_desired_y = (self.rot.y + HALF_TURN) % FULL_TURN;
                        self.phase = Phase::Turning;
                        continue;
                    }
                }
                Phase::Turning => {
                    if self.rot.y != self.rot_desired_y {
                        self.rot.y = interpolate(self.rot.y, self.rot_desired_y, TURN_SPEED);
                        self.matrix = matrix_of(self.rot);
                        self.anim_frame += 1;
                    } else if (self.anim_frame as i32 + 1) < frames[anim::TURN as usize] {
                        self.anim_frame += 1;
                    } else {
                        // Reverse direction when the turn animation finishes.
                        self.vel = self.vel.map(|v| -v);
                        self.anim_index = anim::ROLL;
                        self.anim_frame = 0;
                        self.phase = Phase::Rolling;
                        continue;
                    }
                }
            }
            return Some(self.contact());
        }
    }
}

impl Instance for Armadillo {
    fn pose(&self) -> Option<Pose> {
        Some(Pose {
            pos: self.pos,
            matrix: self.matrix,
            animation: (self.anim_index, self.anim_frame),
            hidden: false,
        })
    }

    fn th_tick(&mut self, cx: &Context, out: &mut Tick) {
        let vel = self.vel;
        out.contacts.extend(self.tick([cx.frames[0], cx.frames[1]]));
        // It squeaks as it turns back.
        if self.vel != vel {
            out.sounds.push(Sound3D {
                id: TURN_SFX,
                pos: self.pos,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAMES: [i32; 2] = [32, 10];

    #[test]
    fn it_squeaks_as_it_turns_back() {
        let level = [];
        let cx = Context {
            instance: 3,
            frames: [FRAMES[0], FRAMES[1], 0, 0, 0, 0, 0, 0],
            ticks: 0,
            position_path: &level,
            roll_paths: &[],
            blockers: &[],
        };
        let mut a = Armadillo::new(&def(0x400), 0);
        let vel = a.vel;
        let mut out = Tick::default();
        let mut ticks = 0;
        while out.sounds.is_empty() {
            a.th_tick(&cx, &mut out);
            ticks += 1;
            assert!(ticks < 1000);
        }
        assert_eq!(a.vel, vel.map(|v| -v));
        assert_eq!(
            out.sounds,
            [Sound3D {
                id: TURN_SFX,
                pos: a.pos
            }]
        );
    }

    fn def(yaw: i16) -> InstDef {
        InstDef {
            name: "armadillo#0".into(),
            model: 0,
            model_name: String::new(),
            scale: SVec3 {
                x: 4096,
                y: 4096,
                z: 4096,
            },
            color_rgba: 0,
            flags: 0,
            pos: SVec3 {
                x: 1000,
                y: 50,
                z: -2000,
            },
            rot: SVec3 { x: 0, y: yaw, z: 0 },
            model_id: 0,
        }
    }

    #[test]
    fn it_rolls_along_its_facing_for_forty_ticks() {
        // Facing +x: a yaw of a quarter turn makes the matrix's third column point along x.
        let mut a = Armadillo::new(&def(0x400), 0);
        let [vx, vz] = a.vel;
        assert!(vx.abs() > 0 && vz == 0, "{:?}", a.vel);
        for tick in 1..=40 {
            a.tick(FRAMES);
            assert_eq!(a.pos, [1000 + vx as i32 * tick, 50, -2000], "tick {tick}");
            assert_eq!(a.anim_index, anim::ROLL);
            assert_eq!(a.anim_frame as i32, (tick) % 10);
        }
    }

    #[test]
    fn it_turns_around_and_rolls_back() {
        let mut a = Armadillo::new(&def(0x400), 0);
        let vx = a.vel[0] as i32;
        for _ in 0..40 {
            a.tick(FRAMES);
        }
        let end = a.pos;
        // The next tick starts the turn at once; half a turn is 8 steps of 0x100.
        a.tick(FRAMES);
        assert_eq!((a.anim_index, a.pos), (anim::TURN, end));
        let mut turning = 1;
        while a.anim_index == anim::TURN {
            a.tick(FRAMES);
            turning += 1;
            assert!(turning < 100);
        }
        // It faces the other way and goes back (the tick that ends the turn already rolls).
        assert_eq!(a.rot.y, 0x400 + 0x800);
        assert_eq!(a.pos[0], end[0] - vx);
        for _ in 0..39 {
            a.tick(FRAMES);
        }
        assert_eq!(a.pos, [1000, 50, -2000]);
    }

    #[test]
    fn it_looks_for_drivers_where_it_is_unless_it_waits() {
        let mut a = Armadillo::new(&def(0x400), 1);
        // Waiting at the edge: no check.
        assert_eq!(a.tick(FRAMES), None);
        let contact = a.tick(FRAMES).unwrap();
        assert_eq!(
            (contact.pos, contact.damage, contact.hit_radius),
            (a.pos, Damage::Spin, 0x2400)
        );
        // A driver on its path is hit, one a few car lengths off is not.
        assert!(contact.hits([a.pos[0] + 50, a.pos[1], a.pos[2]]));
        assert!(!contact.hits([a.pos[0], a.pos[1], a.pos[2] + 200]));
    }

    #[test]
    fn it_waits_before_it_first_moves() {
        let mut a = Armadillo::new(&def(0), 72);
        for _ in 0..72 {
            a.tick(FRAMES);
            assert_eq!(a.pos, [1000, 50, -2000]);
        }
        a.tick(FRAMES);
        assert_ne!(a.pos, [1000, 50, -2000]);
    }
}
