//! The sparks of `VehEmitter_DriverMain`: a burst when the kart lands hard on a
//! surface that throws them (`VehEmitter_Sparks_Ground`), and a trickle while it
//! scrapes a wall (`VehEmitter_Sparks_Wall`). Where they start and in which direction
//! is here; the particles themselves are the renderer's.
//!
//! Not ported: the multiplayer thinning and the rumble that goes with the wall.

use crate::exhaust::rotate;
use crate::terrain::{TERRAINS, terrain_flags};
use crate::{Driver, actions, kart_state};

const LANDING_MIN_SPEED: i32 = 6 * 256;
const LANDING_MIN_JUMP: i32 = 22 * 256;
const WALL_MIN_SPEED: i32 = 2 * 256;
const WALL_RUB_TIMER_FULL: i16 = 0xf0;
const WALL_LEFT_X: i32 = -0x2200;
const WALL_RIGHT_X: i32 = 0x2200;
const WALL_Y: i32 = 0xa00;
const WALL_REVERSE_Z: i32 = -0x1400;
const WALL_FORWARD_Z: i32 = 0x2800;

/// A landing burst (ten sparks): three vectors in the world's axes (1/256 game units)
/// that the sparks' positions and velocities are made from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Landing {
    /// The kart's back, 24 units: where the sparks start from.
    pub back: [i32; 3],
    /// The kart's side, 24 units: the sparks are thrown to either side.
    pub side: [i32; 3],
    /// Pushes the velocities down and back.
    pub push: [i32; 3],
}

/// The sparks of a landing, if the kart landed fast and hard enough on a surface that throws them.
pub fn landing(d: &Driver) -> Option<Landing> {
    let flags = TERRAINS[d.terrain_meta1 as usize].flags;
    if flags & terrain_flags::LANDING_SPARKS == 0 || d.actions_flag_set & actions::STARTED_TOUCH_GROUND == 0 {
        return None;
    }
    if (d.speed_approx as i32).abs() <= LANDING_MIN_SPEED || (d.jump_height_prev as i32).abs() <= LANDING_MIN_JUMP {
        return None;
    }
    let (m, _) = crate::baked::inst_matrix(d);
    Some(Landing { back: rotate(&m, [0, 0, -0x1800]), side: rotate(&m, [0x1800, 0, 0]), push: rotate(&m, [0, 0, -0x200]) })
}

/// A spark from the kart's tire at the wall: `tire` is where it starts (1/256 game
/// units from the kart), and `matrix` turns its velocity into the world's axes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Wall {
    pub tire: [i32; 3],
    pub matrix: crate::Matrix,
}

/// The spark of this tick of a scrape along a wall, from the front tire (the rear one
/// when reversing) on the side of the wall.
pub fn wall(d: &Driver) -> Option<Wall> {
    if d.wall_rub_timer != WALL_RUB_TIMER_FULL || d.kart_state == kart_state::MASK_GRABBED {
        return None;
    }
    let speed = d.speed_approx as i32;
    let z = if speed > WALL_MIN_SPEED { WALL_FORWARD_Z } else if speed < -WALL_MIN_SPEED { WALL_REVERSE_Z } else { return None };
    let (m, _) = crate::baked::inst_matrix(d);
    // The vectors go through 16 bits, as on the GTE.
    let short = |v: [i32; 3]| v.map(|c| c as i16 as i32);
    let left = short(rotate(&m, [WALL_LEFT_X, WALL_Y, z]));
    let right = short(rotate(&m, [WALL_RIGHT_X, WALL_Y, z]));

    // The tire that is farther along the way to the wall.
    let to_wall = [
        (d.pos_wall_coll.x as i32) << 8,
        (d.pos_wall_coll.y as i32) << 8,
        (d.pos_wall_coll.z as i32) << 8,
    ];
    let to_wall: [i32; 3] = std::array::from_fn(|a| (to_wall[a] - [d.pos_curr.x, d.pos_curr.y, d.pos_curr.z][a]) as i16 as i32);
    let along = |tire: [i32; 3]| (0..3).map(|a| tire[a] * to_wall[a]).sum::<i32>() >> 12;
    let tire = if along(left) < along(right) { right } else { left };
    Some(Wall { tire, matrix: m })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Matrix, terrain::terrain_type};

    fn kart() -> Driver {
        let mut d = Driver::new(0, 0);
        d.matrix_facing_dir = Matrix { m: [[0x1000, 0, 0], [0, 0x1000, 0], [0, 0, 0x1000]] };
        d
    }

    #[test]
    fn a_hard_landing_on_stone_throws_sparks_but_a_soft_one_does_not() {
        let mut d = kart();
        d.terrain_meta1 = terrain_type::STONE;
        d.actions_flag_set = actions::STARTED_TOUCH_GROUND;
        d.speed_approx = 0x900;
        d.jump_height_prev = 0x1800;
        let l = landing(&d).expect("sparks");
        assert_eq!(l.back, [0, 0, -0x1800]);
        assert_eq!(l.side, [0x1800, 0, 0]);
        d.jump_height_prev = 0x400;
        assert!(landing(&d).is_none());
    }

    #[test]
    fn a_scrape_sparks_from_the_tire_by_the_wall() {
        let mut d = kart();
        d.wall_rub_timer = WALL_RUB_TIMER_FULL;
        d.speed_approx = 0x900;
        // The wall is to the right.
        d.pos_wall_coll = loader::lev::SVec3 { x: 100, y: 0, z: 0 };
        assert_eq!(wall(&d).unwrap().tire, [WALL_RIGHT_X, WALL_Y, WALL_FORWARD_Z]);
        d.pos_wall_coll.x = -100;
        assert_eq!(wall(&d).unwrap().tire, [WALL_LEFT_X, WALL_Y, WALL_FORWARD_Z]);
        d.speed_approx = 0;
        assert!(wall(&d).is_none());
    }
}
