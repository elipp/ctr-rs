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
