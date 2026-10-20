//! The exhaust part of `VehEmitter_DriverMain` (`DRIVER_FUNC_PARTICLES`, run
//! after the physics): whether the kart puffs smoke from its two pipes this
//! tick, and where and how fast. The particles themselves are the renderer's.
//!
//! Not ported: the stop while the boost flame burns, the multiplayer thinning (`numPlyrCurrGame`), bots, the engine
//! revving at the start (`revEngineState`), invisibility and the underwater
//! bubbles.

use crate::{Driver, Matrix, kart_state};

/// One puff from one pipe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Puff {
    /// `exhaustPos`: the pipe, rotated into the world's axes (1/256 game units).
    pub position: [i32; 3],
    /// `exhaustVel`: 1/256 game units per tick, back and up from the kart.
    pub velocity: [i32; 3],
    /// Drawn subtractive (dark) instead of additive: the turbo meter is in
    /// the window where a tap of the hop button gives a boost.
    pub dark: bool,
}

const VELOCITY: [i32; 3] = [0, 0x400, -0x400];
const POSITION_Y: i32 = 7;
const POSITION_Z: i32 = -0x38;

/// `turbo_MeterRoomLeft` below this is out of the boost window.
const TURBO_METER_COLOR_MIN: i32 = 129;
const TURBO_ROOM_WARNING_PAD: i32 = 2;
const TURBO_ROOM_WARNING_SCALE: i32 = 32;

/// `gte_rtv0`: `m * v` with 4.12 fixed point.
pub fn rotate(m: &Matrix, v: [i32; 3]) -> [i32; 3] {
    std::array::from_fn(|row| (0..3).map(|col| m.m[row][col] as i32 * v[col]).sum::<i32>() >> 12)
}

/// Whether the turbo meter is in the window where a tap gives a boost.
fn in_boost_window(d: &Driver) -> bool {
    let top = (d.stats.turbo_low_room_warning as i32 + TURBO_ROOM_WARNING_PAD) * TURBO_ROOM_WARNING_SCALE;
    (TURBO_METER_COLOR_MIN..=top).contains(&(d.turbo_meter_room_left as i32))
}

/// The puffs this tick. Counts down `failed_boost_exhaust_timer`.
pub fn exhaust(d: &mut Driver) -> Vec<Puff> {
    // Retail stops the exhaust while the boost flame burns outside the window;
    // here it always shows (the flame's lifetime is not simulated), only dark in the window.
    if d.failed_boost_exhaust_timer != 0 {
        d.failed_boost_exhaust_timer -= 1;
    }

    // The pipes ride on the kart model, wheelie and all.
    let (m, _) = crate::baked::inst_matrix(d);
    let m = &m;
    let velocity = rotate(m, VELOCITY);
    let [sx, sy, sz] = [d.inst_scale_x, d.inst_scale_y, d.inst_scale_z].map(i32::from);
    let y = (sy * POSITION_Y) >> 1;
    let z = (sz * POSITION_Z) >> 4;
    let pipes = [(sx * 9) >> 3, (sx * -0x12) >> 4];
    // Revving at the start never runs here, so the window decides.
    let dark = d.kart_state != kart_state::ENGINE_REVVING && in_boost_window(d);
    pipes.map(|x| Puff { position: rotate(m, [x, y, z]), velocity, dark }).to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn driver() -> Driver {
        let mut d = Driver::new(0, 0);
        d.matrix_facing_dir = Matrix { m: [[0x1000, 0, 0], [0, 0x1000, 0], [0, 0, 0x1000]] };
        d.stats.turbo_low_room_warning = 15;
        d
    }

    #[test]
    fn two_pipes_behind_and_above_the_kart() {
        let mut d = driver();
        let puffs = exhaust(&mut d);
        assert_eq!(puffs.len(), 2);
        // Left and right of the middle, at the back, 1/256 game units.
        assert_eq!(puffs[0].position, [3685, 11466, -11466]);
        assert_eq!(puffs[1].position, [-3686, 11466, -11466]);
        assert_eq!(puffs[0].velocity, [0, 0x400, -0x400]);
        assert!(!puffs[0].dark);
    }

    #[test]
    fn the_boost_window_makes_the_smoke_dark() {
        let mut d = driver();
        d.turbo_meter_room_left = 200;
        assert!(exhaust(&mut d).iter().all(|p| p.dark));
        // 15 + 2 = 17 * 32 = 544 is the top of the window.
        d.turbo_meter_room_left = 545;
        assert!(exhaust(&mut d).iter().all(|p| !p.dark));
        d.reserves = 100;
        assert_eq!(exhaust(&mut d).len(), 2);
        d.failed_boost_exhaust_timer = 2;
        assert_eq!(exhaust(&mut d).len(), 2);
        assert_eq!(d.failed_boost_exhaust_timer, 1);
    }

    #[test]
    fn the_puffs_follow_the_kart_around() {
        let mut d = driver();
        // Facing the other way (a half turn around Y): back is +Z, pipes swap sides.
        d.matrix_facing_dir = Matrix { m: [[-0x1000, 0, 0], [0, 0x1000, 0], [0, 0, -0x1000]] };
        let p = exhaust(&mut d);
        assert_eq!(p[0].position, [-3685, 11466, 11466]);
        assert_eq!(p[0].velocity, [0, 0x400, 0x400]);
    }
}
