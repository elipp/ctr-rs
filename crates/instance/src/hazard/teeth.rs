//! `RB_Teeth_*`: the secret door of Tiger Temple (`STATIC_TEETH`). A weapon that reaches it opens
//! it: the door plays its animation to the end, stays open for a while (and as long as a driver or
//! a crate is in the doorway), then closes the same way. A driver does not open it, except one
//! with the mask on.
//!
//! The door is a wall of two parts. The quad blocks of the temple's shortcut are `DOOR` blocks,
//! which are solid until `doorAccessFlags & 1` is set; it is set as soon as the door is told to
//! open and cleared when it has closed. The hitbox of the instance (`RB_Teeth_LInC`) stops the
//! weapons until the door is fully open. Both are [`coll::doors`]'s.
//!
//! Not ported: the relic race, which leaves the door open from the start.

use coll::doors::{Door, TEETH_ACCESS};
use loader::lev::InstDef;
use vehicle::rotmat::rot_to_matrix;
use vehicle::{Matrix, Sound3D, actions};

use crate::{Blocker, Context, Instance, Pose, Tick, Touch};

/// How long (`timeOpen`) the door stays open once it has opened.
const OPEN_MS: i32 = 0x780;
/// `elapsedTimeMS` of a tick.
const TICK_MS: i32 = 32;
const SFX: u8 = 0x75;
/// What `RB_Teeth_BSP_Callback` looks for, with the squared reach of the search.
const REACH_SQUARED: i32 = 0x90000;
/// How far from the door's plane (whole units) a thing keeps it open.
const KEEP_OPEN_DISTANCE: i32 = 0x100;
/// The models in the doorway that keep the door open: a driver, and the crates and beakers
/// (`DYNAMIC_PLAYER`, `PU_EXPLOSIVE_CRATE`, `DYNAMIC_POISON`, `STATIC_CRATE_TNT`).
const KEEP_OPEN_MODELS: [i16; 4] = [
    coll::model_id::PLAYER,
    coll::model_id::NITRO_CRATE,
    0x1d,
    coll::model_id::TNT_CRATE,
];

/// `TEETH_DIRECTION_*`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    Closing = -1,
    Idle = 0,
    Opening = 1,
}

/// `struct Teeth` and the parts of its `Instance` the code changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Teeth {
    /// `matrix.t`, in level units.
    pos: [i32; 3],
    matrix: Matrix,
    direction: Direction,
    /// Non-zero while it is fully open, counting down (`timeOpen`).
    time_open: i32,
    /// `animFrame` of the animation that opens it (animation 0).
    anim_frame: i16,
    /// `doorAccessFlags & 1`: the quad blocks of the door are passable.
    access: bool,
    /// `PlaySound3D` of `RB_Teeth_OpenDoor`, played by the next tick of the thread.
    open_sound: bool,
}

impl Teeth {
    /// The door as the level places it, shut.
    pub fn new(def: &InstDef) -> Teeth {
        Teeth {
            pos: [def.pos.x as i32, def.pos.y as i32, def.pos.z as i32],
            matrix: rot_to_matrix(def.rot),
            direction: Direction::Idle,
            time_open: 0,
            anim_frame: 0,
            access: false,
            open_sound: false,
        }
    }

    /// `RB_Teeth_OpenDoor`: the door starts to open, and its quad blocks are passable at once.
    pub fn open(&mut self) {
        self.open_sound = true;
        self.direction = Direction::Opening;
        self.access = true;
    }

    /// `doorAccessFlags` as this door leaves it.
    pub fn access_flags(&self) -> u32 {
        if self.access { TEETH_ACCESS } else { 0 }
    }

    /// The door as the hitbox tests of the karts and the weapons read it.
    pub fn door(&self, instance: usize) -> Door {
        Door {
            instance,
            pos: self.pos,
            facing: [self.matrix.m[0][2] as i32, self.matrix.m[2][2] as i32],
            open: self.time_open != 0,
        }
    }

    /// `RB_Teeth_ThTick`, one game tick. `frames` is the number of frames of the animation.
    fn tick(&mut self, frames: i32, blockers: &[Blocker], out: &mut Tick) {
        let sound = |teeth: &Teeth| Sound3D {
            id: SFX,
            pos: teeth.pos,
        };
        if std::mem::take(&mut self.open_sound) {
            out.sounds.push(sound(self));
        }
        if self.direction != Direction::Idle {
            self.anim_frame += self.direction as i16;
            if self.anim_frame as i32 >= frames {
                self.anim_frame = (frames - 1) as i16;
                self.direction = Direction::Idle;
                self.time_open = OPEN_MS;
            } else if self.anim_frame < 0 {
                self.anim_frame = 0;
                self.direction = Direction::Idle;
                self.time_open = 0;
                self.access = false;
            }
        } else if self.time_open != 0 {
            self.time_open -= TICK_MS;
            if self.time_open <= 0 {
                out.sounds.push(sound(self));
                self.time_open = 0;
                self.direction = Direction::Closing;
            }
        }
        // A door that is open or closing looks for what is in the doorway.
        if self.time_open != 0 || self.direction == Direction::Closing {
            self.keep_open(blockers);
        }
    }

    /// `RB_Teeth_BSP_Callback` over the players and then the mines: a driver or crate close to the
    /// door's plane opens it again.
    fn keep_open(&mut self, blockers: &[Blocker]) {
        let door = self.door(0);
        for blocker in blockers {
            if !KEEP_OPEN_MODELS.contains(&blocker.model) || !in_reach(self.pos, blocker.at) {
                continue;
            }
            if door.plane_distance(blocker.at) >> 12 <= KEEP_OPEN_DISTANCE {
                self.direction = Direction::Opening;
            }
        }
    }
}

/// `PROC_CollideHitboxWithBucket`'s test of an instance at `at` against the sphere at `pos`.
fn in_reach(pos: [i32; 3], at: [i32; 3]) -> bool {
    let mut dist = 0i32;
    for axis in 0..3 {
        let delta = math::mips::sub_lo(pos[axis], at[axis]);
        let square = math::mips::mul_lo(delta, delta);
        dist = math::mips::add_lo(dist, square);
        if square > 0x0fff_ffff {
            return false;
        }
    }
    dist < REACH_SQUARED
}

impl Instance for Teeth {
    fn th_tick(&mut self, cx: &Context, out: &mut Tick) {
        self.tick(cx.frames[0], cx.blockers, out);
    }

    /// `RB_Teeth_LInC`. A driver is never stopped by the hitbox (the quad blocks are its wall); one
    /// with the mask on opens the door. The weapons, which the hitbox does stop, are told by
    /// [`coll::doors::Door::stops`] instead: this is 1 for them while the door is not open.
    fn touch(&mut self, hit: &mut Touch) -> i32 {
        if hit.model == coll::model_id::PLAYER {
            if hit.driver.actions_flag_set & actions::MASK_WEAPON != 0 {
                self.open();
            }
            return 2;
        }
        if self.time_open != 0 { 2 } else { 1 }
    }

    fn pose(&self) -> Option<Pose> {
        Some(Pose {
            pos: self.pos,
            matrix: self.matrix,
            animation: (0, self.anim_frame),
            // Only the fully open door is hidden.
            hidden: self.time_open != 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use loader::lev::SVec3;

    fn teeth() -> Teeth {
        Teeth::new(&InstDef {
            name: "teeth".into(),
            model: 0,
            model_name: "teeth".into(),
            scale: Default::default(),
            color_rgba: 0,
            flags: 0,
            pos: SVec3 { x: 100, y: 0, z: 200 },
            rot: SVec3::default(),
            model_id: 0x70,
        })
    }

    fn run(t: &mut Teeth, ticks: usize, blockers: &[Blocker]) -> Tick {
        let mut out = Tick::default();
        for _ in 0..ticks {
            t.tick(4, blockers, &mut out);
        }
        out
    }

    #[test]
    fn a_shut_door_does_nothing() {
        let mut t = teeth();
        run(&mut t, 100, &[]);
        assert_eq!(t.anim_frame, 0);
        assert_eq!(t.access_flags(), 0);
        assert!(!t.door(0).open);
    }

    #[test]
    fn it_opens_waits_and_closes() {
        let mut t = teeth();
        t.open();
        // The quad blocks are passable at once, the hitbox only once it is open.
        assert_eq!(t.access_flags(), TEETH_ACCESS);
        assert!(!t.door(0).open);
        // 3 frames up, and it is open on the tick that runs past the last (`RB_Teeth_ThTick`).
        let out = run(&mut t, 3, &[]);
        assert_eq!(t.anim_frame, 3);
        assert!(!t.door(0).open);
        run(&mut t, 1, &[]);
        assert_eq!(t.anim_frame, 3);
        assert!(t.door(0).open);
        assert_eq!(out.sounds.len(), 1);
        // 0x780 ms is 60 ticks of waiting.
        run(&mut t, 59, &[]);
        assert!(t.door(0).open);
        let out = run(&mut t, 1, &[]);
        assert!(!t.door(0).open);
        assert_eq!(out.sounds.len(), 1);
        // Closing: 3 frames back, and the quad blocks are solid again after the last.
        run(&mut t, 3, &[]);
        assert_eq!(t.access_flags(), TEETH_ACCESS);
        run(&mut t, 1, &[]);
        assert_eq!(t.anim_frame, 0);
        assert_eq!(t.access_flags(), 0);
    }

    #[test]
    fn a_driver_in_the_doorway_keeps_it_open() {
        let mut t = teeth();
        t.open();
        let driver = Blocker {
            model: coll::model_id::PLAYER,
            at: [100, 0, 220],
        };
        run(&mut t, 200, &[driver]);
        assert!(t.door(0).open);
        // One far from it does not.
        let mut t = teeth();
        t.open();
        let far = Blocker {
            model: coll::model_id::PLAYER,
            at: [5000, 0, 5000],
        };
        run(&mut t, 70, &[far]);
        assert!(!t.door(0).open);
    }

    #[test]
    fn an_open_door_stops_no_weapon() {
        let mut t = teeth();
        assert!(t.door(0).stops([100, 0, 250]));
        t.open();
        run(&mut t, 4, &[]);
        assert!(!t.door(0).stops([100, 0, 250]));
    }
}
