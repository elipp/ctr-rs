//! `ConvertRotToMatrix` and `MatrixRotate` (`game/MATH/MATH_7_MatrixStubs.c`):
//! a rotation (X, then Z, onto a rotation about Y) as a matrix, built the way
//! the game does, by multiplying rotation matrices on the GTE.

use math::{Bits, Wrap};
use gte::{Gte, ctrl, data};
use loader::lev::SVec3;

use crate::Matrix;
use crate::force::mvmva;

const FP_ONE: u32 = 0x1000;

fn neg_low_word(value: u32) -> u32 {
    0u32.wrapping_sub(value) & 0xffff
}

fn neg_high_word(value: u32) -> u32 {
    neg_low_word(value) << 16
}

/// `MATH_Matrix_TrigSinCos`: `(sin, cos)` as 16-bit words.
fn sin_cos_words(angle: i32) -> (u32, u32) {
    let (sin, cos) = math::sin_cos(angle as u32);
    (sin as u32 & 0xffff, cos as u32 & 0xffff)
}

fn load_rot_words(gte: &mut Gte, r: &[u32; 5]) {
    for (i, &word) in r.iter().enumerate() {
        gte.ctc2(word, ctrl::R_MATRIX + i);
    }
}

/// `MATH_Matrix_MulRotWords`: multiplies the matrix in the GTE by the one in
/// `r` (as the packed words of a `MATRIX`) and leaves the product in both.
fn mul_rot_words(gte: &mut Gte, r: &mut [u32; 5]) {
    let [t3, t4, t5, t6, t7] = *r;

    // The columns of `r` go in as the vectors.
    gte.mtc2((t3 & 0xffff) | (t4 & 0xffff_0000), data::VXY0);
    gte.mtc2(t6, data::VZ0);
    gte.mtc2((t3 >> 16) | (t5 << 16), data::VXY1);
    gte.mtc2(t6 >> 16, data::VZ1);
    gte.mtc2((t4 & 0xffff) | (t5 & 0xffff_0000), data::VXY2);
    gte.mtc2(t7, data::VZ2);

    let column = |gte: &mut Gte, v: u32| {
        gte.cop2(mvmva(1, 0, v, 3, 0));
        [gte.mfc2(data::IR1), gte.mfc2(data::IR2), gte.mfc2(data::IR3)]
    };
    let c0 = column(gte, 0);
    let c1 = column(gte, 1);
    let c2 = column(gte, 2);

    r[0] = (c0[0] & 0xffff) | (c1[0] << 16);
    r[1] = (c0[1] << 16) | (c2[0] & 0xffff);
    r[2] = (c1[1] & 0xffff) | (c2[1] << 16);
    r[3] = (c0[2] & 0xffff) | (c1[2] << 16);
    r[4] = c2[2];
    load_rot_words(gte, r);
}

/// `MATH_Matrix_MulIfNonZero`: turns the matrix about an axis (0: X, 1: Y,
/// 2: Z) by `angle`.
fn mul_if_nonzero(gte: &mut Gte, r: &mut [u32; 5], angle: i32, axis: u32) {
    if angle == 0 {
        return;
    }
    let (sine, cosine) = sin_cos_words(angle);
    *r = match axis {
        0 => [FP_ONE, 0, neg_high_word(sine) | cosine, sine << 16, cosine],
        1 => [cosine, sine, FP_ONE, neg_low_word(sine), cosine],
        _ => [neg_high_word(sine) | cosine, sine << 16, cosine, 0, FP_ONE],
    };
    mul_rot_words(gte, r);
}

/// `ConvertRotToMatrix`. Leaves the matrix in the GTE's rotation matrix too.
pub fn convert_rot_to_matrix(gte: &mut Gte, rot: SVec3) -> Matrix {
    let (sine, cosine) = sin_cos_words(rot.y as i32);
    let mut r = [cosine, sine, FP_ONE, neg_low_word(sine), cosine];
    load_rot_words(gte, &r);

    mul_if_nonzero(gte, &mut r, rot.x as i32, 0);
    mul_if_nonzero(gte, &mut r, rot.z as i32, 2);

    unpack(&r)
}

fn pack(m: &Matrix) -> [u32; 5] {
    let w = |a: i16, b: i16| a.zext16().bits() | (b.zext16().bits()) << 16;
    let m = &m.m;
    [w(m[0][0], m[0][1]), w(m[0][2], m[1][0]), w(m[1][1], m[1][2]), w(m[2][0], m[2][1]), m[2][2].zext16().bits()]
}

fn unpack(r: &[u32; 5]) -> Matrix {
    let lo = |w: u32| w.wrap_i16();
    let hi = |w: u32| (w >> 16).wrap_i16();
    Matrix { m: [[lo(r[0]), hi(r[0]), lo(r[1])], [hi(r[1]), lo(r[2]), hi(r[2])], [lo(r[3]), hi(r[3]), lo(r[4])]] }
}

/// `MatrixRotate(dst, src, rot)`: the product `src * rot`, built on the GTE.
pub fn matrix_rotate(gte: &mut Gte, src: &Matrix, rot: &Matrix) -> Matrix {
    load_rot_words(gte, &pack(src));
    let mut r = pack(rot);
    mul_rot_words(gte, &mut r);
    unpack(&r)
}
