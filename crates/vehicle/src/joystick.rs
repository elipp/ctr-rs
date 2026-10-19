//! `game/Vehicle/VehPhysJoystick.c`, for the default gamepad (the racing wheel
//! variants take a `RacingWheelData`; there is no wheel here).

use math::mips;

use crate::calc::map_to_range;

const CENTER: i32 = 0x80;
const DEAD_ZONE: i32 = 0x30;
const RANGE: i32 = 0x7f;
/// Retail's no-wheel strength path uses 0x5e, not `RANGE - DEAD_ZONE`.
const STRENGTH_DISTANCE: i32 = 0x5e;
const STRENGTH_CURVE_SEGMENTS: i32 = 5;

/// `VehPhysJoystick_ReturnToRest`: a stick value (0x80 is centred) as
/// `-half..=half`, with the dead zone removed.
pub fn return_to_rest(stick: i32, half: i32) -> i32 {
    let stick = mips::sub_lo(stick, CENTER);
    if stick < 0 {
        mips::neg_lo(map_to_range(mips::neg_lo(stick), DEAD_ZONE, RANGE, 0, half))
    } else {
        map_to_range(stick, DEAD_ZONE, RANGE, 0, half)
    }
}

/// `VehPhysJoystick_GetStrength`: a stick's distance from the centre as
/// `0..=max`, on a curve that is flatter at first.
pub fn strength(val: i32, max: i32) -> i32 {
    let dist = STRENGTH_DISTANCE;
    if val < DEAD_ZONE {
        return 0;
    }
    if RANGE <= val {
        return max;
    }

    let val = mips::sub_lo(val, DEAD_ZONE);
    let half_dist = mips::sra(mips::add_lo(dist, (dist as u32 >> 31) as i32), 1);

    if val < half_dist {
        let max = max / STRENGTH_CURVE_SEGMENTS;
        let product = mips::sll(mips::mul_lo(val, max), 1);
        return mips::div(product, dist);
    }

    let max_fifth = max / STRENGTH_CURVE_SEGMENTS;
    let val = mips::sub_lo(val, half_dist);
    let max = mips::sub_lo(max, max_fifth);
    let product = mips::sll(mips::mul_lo(val, max), 1);
    mips::add_lo(mips::div(product, dist), max_fifth)
}

/// `VehPhysJoystick_GetStrengthAbsolute`: [`strength`] with the stick's side
/// as the sign (positive is right of centre).
pub fn strength_absolute(stick: i32, max_steer: i32) -> i32 {
    let dist_from_center = mips::sub_lo(stick, CENTER);
    if dist_from_center < 0 {
        mips::neg_lo(strength(mips::neg_lo(dist_from_center), max_steer))
    } else {
        strength(dist_from_center, max_steer)
    }
}
