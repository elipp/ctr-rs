//! `game/Vehicle/VehCalc.c`

use math::mips;

/// `VehCalc_FastSqrt`: a bitwise square root of `n`, scaled by `shift`
/// (the result has `shift / 2` fractional bits). Not the same routine as
/// `math::fast_sqrt` (`MATH_FastSqrt`).
pub fn fast_sqrt(n: u32, shift: u32) -> u32 {
    let mut result = 0u32;
    let mut test_bit = mips::sll(1, shift) as u32;
    let mut root_bit_index = 1i32;
    let mut last_approximation = 0u32;

    while test_bit < n && (test_bit << 2) != 0 {
        root_bit_index += 1;
        test_bit <<= 2;
    }

    let mut add_bit = mips::sll(1, mips::add_lo(root_bit_index, mips::sub_lo(shift as i32, 1)) as u32) as u32;

    while test_bit != 0 {
        let mut shifted_result = mips::sll(result as i32, root_bit_index as u32) as u32;
        if root_bit_index < 0 {
            shifted_result = mips::srl(result as i32, mips::neg_lo(root_bit_index) as u32);
        }

        let approximation = shifted_result.wrapping_add(last_approximation.wrapping_add(test_bit));
        if approximation <= n {
            last_approximation = approximation;
            result = result.wrapping_add(add_bit);
        }

        add_bit >>= 1;
        test_bit >>= 2;
        root_bit_index -= 1;
    }
    result
}

/// `VehCalc_InterpBySpeed`: moves `val` towards `desired` by at most `speed`.
pub fn interp_by_speed(val: i32, speed: i32, desired: i32) -> i32 {
    if val > desired {
        let val = mips::sub_lo(val, speed);
        if val < desired {
            return desired;
        }
        val
    } else if val < desired {
        let val = mips::add_lo(val, speed);
        if val > desired {
            return desired;
        }
        val
    } else {
        val
    }
}

/// `VehCalc_MapToRange`: maps `val` from `old_min..old_max` to `new_min..new_max`
/// linearly, clamped.
pub fn map_to_range(val: i32, old_min: i32, old_max: i32, new_min: i32, new_max: i32) -> i32 {
    if val <= old_min {
        return new_min;
    }
    if val >= old_max {
        return new_max;
    }
    let dist_from_bottom = mips::sub_lo(val, old_min);
    let new_range = mips::sub_lo(new_max, new_min);
    let old_range = mips::sub_lo(old_max, old_min);
    let scaled = mips::div(mips::mul_lo(dist_from_bottom, new_range), old_range);
    mips::add_lo(new_min, scaled)
}

/// `VehCalc_SteerAccel`: how hard the steering accelerates, by the number of
/// frames spent steering.
pub fn steer_accel(
    steering_frame_count: i32,
    stage2_first_frame: i32,
    stage2_frame_length: i32,
    stage4_first_frame: i32,
    stage1_min_steer: i32,
    stage1_max_steer: i32,
) -> i32 {
    // Stage 1: the first frames, steering accelerates harder as time passes.
    if steering_frame_count < stage2_first_frame {
        return map_to_range(steering_frame_count, 0, stage2_first_frame, stage1_min_steer, stage1_max_steer);
    }
    // Stage 3: from there on it eases off (and from stage 4 on, it is zero).
    let stage3_first_frame = mips::add_lo(stage2_first_frame, stage2_frame_length);
    if stage3_first_frame < steering_frame_count {
        return map_to_range(steering_frame_count, stage3_first_frame, stage4_first_frame, stage1_max_steer, 0);
    }
    // Stage 2: the maximum.
    stage1_max_steer
}
