//! Who a bomb or a missile goes for (`VehPickupItem_MissileGetTargetDriver`).
//!
//! DIVERGENCE: retail projects every kart onto the shooter's own screen and takes the nearest one
//! that lands inside it (`pushBuffer`, so it depends on the camera, which the sim does not have and
//! which two clients would not share). This takes the nearest kart inside a cone ahead of the
//! shooter instead. The cone is [`HALF_ANGLE_TAN`], about what a one-player screen sees.

use math::mips;

use crate::Body;

/// `MISSILE_TARGET_POS_SHIFT`: distances are of the positions in level units.
const POS_SHIFT: u32 = 8;

/// The tangent of the cone's half angle, as a 4.12 fraction: 1.0 is 45 degrees to each side.
pub const HALF_ANGLE_TAN: i32 = 0x1000;

/// The kart `shooter` (facing `rot_y`) aims at, if any: the nearest of the others that is ahead of
/// it and inside the cone. Karts held by the mask and invisible ones are never aimed at.
pub fn target(shooter: usize, bodies: &[Body], rot_y: i16) -> Option<usize> {
    let me = bodies.get(shooter)?;
    let (sin, cos) = math::sin_cos(rot_y as u32);
    let mut best: Option<(usize, i32)> = None;
    for (i, body) in bodies.iter().enumerate() {
        if i == shooter || body.mask_grabbed || body.invisible {
            continue;
        }
        let dx = mips::sra(mips::sub_lo(body.pos_curr[0], me.pos_curr[0]), POS_SHIFT);
        let dz = mips::sra(mips::sub_lo(body.pos_curr[2], me.pos_curr[2]), POS_SHIFT);
        // How far ahead and how far to the side, along the way the shooter faces.
        let ahead = mips::sra(
            mips::add_lo(mips::mul_lo(dx, sin), mips::mul_lo(dz, cos)),
            12,
        );
        let side = mips::sra(
            mips::sub_lo(mips::mul_lo(dx, cos), mips::mul_lo(dz, sin)),
            12,
        );
        if ahead <= 0 || mips::sra(mips::mul_lo(ahead, HALF_ANGLE_TAN), 12) < side.abs() {
            continue;
        }
        let distance = mips::add_lo(mips::mul_lo(dx, dx), mips::mul_lo(dz, dz));
        if best.is_none_or(|(_, closest)| distance < closest) {
            best = Some((i, distance));
        }
    }
    best.map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(x: i32, z: i32) -> Body {
        Body {
            at: [x, 0, z],
            pos_curr: [x << 8, 0, z << 8],
            mask_grabbed: false,
            invisible: false,
        }
    }

    #[test]
    fn the_nearest_kart_ahead_is_the_target() {
        // Facing +z (angle 0).
        let bodies = [body(0, 0), body(10, 900), body(0, 400), body(0, -100)];
        assert_eq!(target(0, &bodies, 0), Some(2));
    }

    #[test]
    fn karts_outside_the_cone_and_behind_are_not() {
        let bodies = [body(0, 0), body(800, 400), body(0, -400)];
        assert_eq!(target(0, &bodies, 0), None);
        // Turned to face +x, the one that was to the side is ahead.
        assert_eq!(target(0, &bodies, 0x400), Some(1));
    }

    #[test]
    fn karts_that_cannot_be_hit_are_skipped() {
        let mut bodies = [body(0, 0), body(0, 300), body(0, 600)];
        bodies[1].mask_grabbed = true;
        assert_eq!(target(0, &bodies, 0), Some(2));
        bodies[2].invisible = true;
        assert_eq!(target(0, &bodies, 0), None);
    }
}
