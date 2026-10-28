//! The Tiger Temple door (`STATIC_TEETH`) as the collision sees it: `sdata->doorAccessFlags`, and
//! what the door's `LInC` (`RB_Teeth_LInC`) says to a thing that reaches its hitbox. The door
//! itself, which opens and closes, is `instance::hazard::teeth`; this is the part of it that the
//! karts and the weapons both look at.

use math::mips;

/// `doorAccessFlags & 1`: the door quad blocks of the temple are passable.
pub const TEETH_ACCESS: u32 = 1;
/// `STATIC_TEETH`
pub const TEETH_MODEL: i32 = 0x70;

/// How far from the door's plane (4.12 units, in whole units) a thing is stopped by its hitbox.
const BLOCK_DISTANCE: i32 = 0x81;

/// A door as `RB_Teeth_LInC` reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Door {
    /// An index into `Level::instances`.
    pub instance: usize,
    /// `matrix.t`, in level units.
    pub pos: [i32; 3],
    /// `matrix.m[0][2]` and `matrix.m[2][2]`: the normal of the door's plane.
    pub facing: [i32; 2],
    /// `timeOpen != 0`: fully open, so that nothing is stopped by it.
    pub open: bool,
}

impl Door {
    /// The distance of `at` from the door's plane, in the CPU's wrapping words, before the shift
    /// (the absolute value).
    pub fn plane_distance(&self, at: [i32; 3]) -> i32 {
        let x = mips::mul_lo(mips::sub_lo(at[0], self.pos[0]), self.facing[0]);
        let z = mips::mul_lo(mips::sub_lo(at[2], self.pos[2]), self.facing[1]);
        let distance = mips::add_lo(x, z);
        if distance < 0 {
            mips::neg_lo(distance)
        } else {
            distance
        }
    }

    /// `RB_Teeth_LInC` for a thing at `at` that is not a driver: whether it is stopped (1), and
    /// not when it is 2.
    pub fn stops(&self, at: [i32; 3]) -> bool {
        !self.open && (self.plane_distance(at) >> 12) < BLOCK_DISTANCE
    }
}

/// The doors of a level and `doorAccessFlags`, as of the start of a tick.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Doors {
    pub access: u32,
    pub teeth: Vec<Door>,
}

impl Doors {
    /// The door on `instance`.
    pub fn door(&self, instance: usize) -> Option<&Door> {
        self.teeth.iter().find(|d| d.instance == instance)
    }
}
