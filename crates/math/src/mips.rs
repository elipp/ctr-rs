//! MIPS R3000 integer helpers (`CTR_Mips*` in `include/ctr_math.h`), with the
//! native build's behaviour: wrapping arithmetic and shift counts taken mod 32.
//! Division traps, as on hardware, are panics.

pub const fn sll(value: i32, shift: u32) -> i32 {
    ((value as u32) << (shift & 0x1f)) as i32
}

pub const fn sra(value: i32, shift: u32) -> i32 {
    value >> (shift & 0x1f)
}

pub const fn srl(value: i32, shift: u32) -> u32 {
    (value as u32) >> (shift & 0x1f)
}

pub const fn mul_lo(lhs: i32, rhs: i32) -> i32 {
    lhs.wrapping_mul(rhs)
}

pub const fn mul_hi_u(lhs: u32, rhs: u32) -> u32 {
    ((lhs as u64 * rhs as u64) >> 32) as u32
}

pub const fn add_lo(lhs: i32, rhs: i32) -> i32 {
    lhs.wrapping_add(rhs)
}

pub const fn sub_lo(lhs: i32, rhs: i32) -> i32 {
    lhs.wrapping_sub(rhs)
}

pub const fn neg_lo(value: i32) -> i32 {
    value.wrapping_neg()
}

/// `CTR_PackS16Pair`: low halves of `lo` and `hi` as one word.
pub const fn pack_s16_pair(lo: i32, hi: i32) -> u32 {
    lo as u16 as u32 | (hi as u16 as u32) << 16
}

/// `CTR_MipsDiv`; panics on divide-by-zero and `i32::MIN / -1`, which trap in the reference.
pub fn div(dividend: i32, divisor: i32) -> i32 {
    assert!(
        divisor != 0 && !(divisor == -1 && dividend == i32::MIN),
        "MIPS divide trap: {dividend} / {divisor}"
    );
    dividend / divisor
}

/// `CTR_MipsDivU`; panics on divide-by-zero.
pub fn div_u(dividend: u32, divisor: u32) -> u32 {
    assert!(divisor != 0, "MIPS divide trap: {dividend} / 0");
    dividend / divisor
}
