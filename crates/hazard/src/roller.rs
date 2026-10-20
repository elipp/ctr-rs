//! `RB_Snowball_ThTick`: the snowball of Blizzard Bluff and the barrels of Sewer Speedway roll
//! along a path recorded in the level file, one point a tick, forwards and then backwards.

use loader::lev::PosRot;

use crate::{Contact, Damage, Shape};

/// `RB_FindDriverCollision`'s radius: a squared distance, 256 units.
const HIT_RADIUS: u32 = 0x10000;

/// A snowball or a barrel: which of the level's recorded paths it follows (the digit its instance
/// name ends in).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Roller {
    pub path: usize,
}

/// The point of the path a roller is at `tick` ticks after it was born: `pointIndex` runs to twice
/// the last point, and the second half goes back. `None` for a path of a single point.
pub fn frame_index(points: usize, tick: u32) -> Option<usize> {
    let last = points.checked_sub(1).filter(|&last| last > 0)? as u32;
    let point = tick % (last * 2);
    Some(if point <= last { point } else { last * 2 - point } as usize)
}

/// Where a roller at `frame` of its path looks for drivers, and what it does to one.
pub fn contact(frame: &PosRot) -> Contact {
    Contact {
        pos: [frame.pos.x as i32, frame.pos.y as i32, frame.pos.z as i32],
        damage: Damage::Squish,
        hit_radius: HIT_RADIUS,
        sound: 0,
        shape: Shape::Sphere,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_path_is_walked_forwards_and_back() {
        let at: Vec<_> = (0..9).map(|t| frame_index(4, t).unwrap()).collect();
        assert_eq!(at, [0, 1, 2, 3, 2, 1, 0, 1, 2]);
        assert_eq!(frame_index(1, 5), None);
        assert_eq!(frame_index(0, 5), None);
    }
}
