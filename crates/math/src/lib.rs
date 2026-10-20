//! Fixed-point math layer, ported from the reference `ctr-native`
//! (`game/MATH/`, `platform/native_libgte.c`, `include/ctr_math.h`).
//!
//! Angles are `u32`/`i32` in 1/4096 turns (`ANG_TWO_PI == 0x1000`); trig
//! results are 4.12 fixed point. Integer behaviour follows the MIPS R3000
//! helpers in the reference: wrapping arithmetic, shifts masked to five bits.
//!
//! `tests/reference.rs` checks this port against the C code.

mod tables;

use gte::{Gte, cmd, ctrl, data};
use tables::{RATAN, RCOSSIN, SQRT, TRIG_APPROX};

pub mod mips;
pub mod wrap;

pub use wrap::{Bits, Wrap};

// Angles.
pub const ANG_TWO_PI: u32 = 0x1000;
pub const ANG_PI: u32 = ANG_TWO_PI / 2;
pub const ANG_HALF_PI: u32 = ANG_PI / 2;
pub const ANG_QUADRANT_MASK: u32 = ANG_HALF_PI - 1;
pub const ANG_QUADRANT_BIT: u32 = ANG_HALF_PI;
pub const ANG_SIGN_BIT: u32 = ANG_PI;
pub const ANG_QUADRANT_BITS: u32 = ANG_QUADRANT_BIT | ANG_SIGN_BIT;

pub const fn ang_modulo_two_pi(angle: u32) -> u32 {
    angle & (ANG_TWO_PI - 1)
}
pub const fn ang_modulo_pi(angle: u32) -> u32 {
    angle & (ANG_PI - 1)
}
pub const fn ang_modulo_half_pi(angle: u32) -> u32 {
    angle & ANG_QUADRANT_MASK
}

// Fixed point.
pub const FRACTIONAL_BITS_8: u32 = 8;
pub const FP8_ONE: i32 = 1 << FRACTIONAL_BITS_8;
pub const FRACTIONAL_BITS: u32 = 12;
pub const FP_ONE: i32 = 1 << FRACTIONAL_BITS;

/// `FP_Mult`: the product is formed in 32 bits (wrapping) before the shift.
pub const fn fp_mult(x: i32, y: i32) -> i32 {
    x.wrapping_mul(y) >> FRACTIONAL_BITS
}

pub const fn fp8_mult(x: i32, y: i32) -> i32 {
    x.wrapping_mul(y) >> FRACTIONAL_BITS_8
}

/// `sdata->randomNumber` at the start of the game.
pub const RANDOM_NUMBER_INIT: i32 = 100;

/// `MixRNG_GetValue`: the generator's step, a 16-bit linear congruential generator.
pub fn mix_rng_value(seed: i32) -> u32 {
    (seed as u32).wrapping_mul(0x6255).wrapping_add(0x3619) & 0xffff
}

/// `MixRNG_Scramble`: advances `random_number` (`sdata->randomNumber`) and returns it, 0..=0xffff.
pub fn mix_rng_scramble(random_number: &mut i32) -> i32 {
    *random_number = mix_rng_value(*random_number) as i32;
    *random_number
}

/// `clamp` / `max` / `min` from the reference.
pub fn clamp(n: i32, lo: i32, hi: i32) -> i32 {
    if n < lo {
        lo
    } else if n > hi {
        hi
    } else {
        n
    }
}

/// A vector of three `s16` (`SVec3`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SVec3 {
    pub x: i16,
    pub y: i16,
    pub z: i16,
}

/// A vector of three `s32` (`Vec3`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Vec3 {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

/// `TRIG_AngleSinCos`: `(sin, cos)` of an angle, from the quarter-turn table.
pub fn sin_cos(angle: u32) -> (i32, i32) {
    let (s, c) = TRIG_APPROX[ang_modulo_half_pi(angle) as usize];
    let (s, c) = (s as i32, c as i32);
    let (mut sin, mut cos);
    if angle & ANG_QUADRANT_BIT == 0 {
        (sin, cos) = (s, c);
        if angle & ANG_SIGN_BIT != 0 {
            (sin, cos) = (-sin, -cos);
        }
    } else {
        (sin, cos) = (c, s);
        if angle & ANG_SIGN_BIT == 0 {
            cos = -cos;
        } else {
            sin = -sin;
        }
    }
    (sin, cos)
}

/// `MATH_Sin`.
pub fn sin(angle: u32) -> i32 {
    sin_cos(angle).0
}

/// `MATH_Cos`.
pub fn cos(angle: u32) -> i32 {
    sin_cos(angle).1
}

/// `rsin` (libgte): table lookup on the low 12 bits of `|a|`, sign from `a`.
pub fn rsin(a: i32) -> i32 {
    if a < 0 {
        -(RCOSSIN[((a.wrapping_neg() as u32 & 0xfff) * 2) as usize] as i32)
    } else {
        RCOSSIN[((a as u32 & 0xfff) * 2) as usize] as i32
    }
}

/// `rcos` (libgte). Even in `a`, unlike [`rsin`].
pub fn rcos(a: i32) -> i32 {
    let a = if a < 0 { a.wrapping_neg() } else { a };
    RCOSSIN[((a as u32 & 0xfff) * 2 + 1) as usize] as i32
}

/// `ratan2` (libgte): angle of `(x, y)` in 1/4096 turns, range `-2048..=2048`.
/// Returns 0 for the origin.
pub fn ratan2(y: i32, x: i32) -> i32 {
    if x == 0 && y == 0 {
        return 0;
    }
    let (xlt0, ylt0) = (x < 0, y < 0);
    let (x, y) = (x.wrapping_abs(), y.wrapping_abs());

    // `ang` is `lo / hi` scaled to 0..=1024; large operands are pre-shifted
    // so the numerator does not overflow.
    let ratio = |lo: i32, hi: i32| -> usize {
        let ang = if lo as u32 & 0x7fe0_0000 == 0 {
            (lo << 10) / hi
        } else {
            lo / (hi >> 10)
        };
        ang as usize
    };

    let mut v = if y < x {
        RATAN[ratio(y, x)] as i32
    } else {
        1024 - RATAN[ratio(x, y)] as i32
    };
    if xlt0 {
        v = 2048 - v;
    }
    if ylt0 {
        v = -v;
    }
    v
}

/// Leading sign bits, as the GTE's LZCR reports them (32 for 0 and -1).
fn leading_count(value: i32) -> u32 {
    (if value < 0 { !value } else { value } as u32).leading_zeros()
}

/// `SquareRoot0` (libgte): table-driven square root.
///
/// The reference indexes `SQRT[idx - 64]` unchecked; negative inputs would
/// read out of range there, so they panic here.
pub fn square_root0(a: i32) -> i32 {
    let lzcs = leading_count(a);
    if lzcs == 32 {
        return 0;
    }
    let lzcs = lzcs & !1;
    let idx = if lzcs < 24 {
        a >> (24 - lzcs)
    } else {
        a << (lzcs - 24)
    };
    ((SQRT[(idx - 64) as usize] as i32) << ((31 - lzcs) >> 1)) >> 12
}

/// `SquareRoot0_stub`: exact bitwise integer square root of the value taken
/// as `s32` (negative inputs behave as the reference's loop does).
pub fn square_root0_stub(value: i32) -> i32 {
    if value == 0 {
        return 0;
    }
    let leading = leading_count(value) & 0x1e;
    let mut shifted = (value as u32) << leading;
    let mut bit = (leading ^ 0x1e) as i32;
    let (mut remainder, mut root) = (0u32, 0u32);
    loop {
        remainder |= shifted >> 0x1e;
        let trial = (root << 2).wrapping_add(1);
        root <<= 1;
        let next = remainder.wrapping_sub(trial);
        shifted <<= 2;
        if (next as i32) >= 0 {
            root = root.wrapping_add(1);
            remainder = next << 2;
        } else {
            remainder <<= 2;
        }
        bit -= 2;
        if bit < 0 {
            break;
        }
    }
    root as i32
}

/// `MATH_FastSqrt`.
pub fn fast_sqrt(mut value: u32, shift: i32) -> i32 {
    let (mut root, mut remainder) = (0i32, 0u32);
    let mut iterations = (shift >> 1).wrapping_add(0xf);
    loop {
        remainder = (remainder << 2) | (value >> 30);
        value <<= 2;
        root = root.wrapping_mul(2);
        let test = (root.wrapping_mul(2) as u32).wrapping_add(1);
        if test <= remainder {
            remainder -= test;
            root = root.wrapping_add(1);
        }
        let keep_going = iterations;
        iterations = iterations.wrapping_sub(1);
        if keep_going == 0 {
            break;
        }
    }
    root
}

/// `MATH_VectorLength`: sum of squares through the GTE (`mvmva` with the
/// vector as the first matrix row), then [`square_root0_stub`].
pub fn vector_length(v: SVec3) -> i32 {
    let xy = (v.x as u16 as u32) | (v.y as u16 as u32) << 16;
    let z = v.z as i32 as u32;
    let mut gte = Gte::new();
    gte.ctc2(xy, ctrl::R_MATRIX);
    gte.ctc2(z, ctrl::R13R21);
    gte.mtc2(xy, data::VXY0);
    gte.mtc2(z, data::VZ0);
    gte.cop2(0x0400012 | (cmd::CV_NONE as u32) << cmd::CV_SHIFT);
    square_root0_stub(gte.mfc2(data::MAC1) as i32)
}

/// `MATH_VectorNormalize`: scales to length 4096 in place, returns the old length.
pub fn vector_normalize(v: &mut SVec3) -> i32 {
    let length = vector_length(*v);
    if length != 0 {
        v.x = (((v.x as i32) << 12) / length) as i16;
        v.y = (((v.y as i32) << 12) / length) as i16;
        v.z = (((v.z as i32) << 12) / length) as i16;
    }
    length
}

#[cfg(test)]
mod rng_tests {
    use super::*;

    #[test]
    fn the_generator_steps_as_the_reference_formula() {
        let mut state = RANDOM_NUMBER_INIT;
        let first = mix_rng_scramble(&mut state);
        assert_eq!((first, state), (40781, 40781));
        assert_eq!(mix_rng_value(40781) as i32, mix_rng_scramble(&mut state));
        // It stays within 16 bits, and the state is the value.
        for _ in 0..1000 {
            let v = mix_rng_scramble(&mut state);
            assert!((0..=0xffff).contains(&v));
            assert_eq!(v, state);
        }
    }
}
