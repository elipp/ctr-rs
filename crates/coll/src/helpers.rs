//! Small helpers for the searches: packing and projection, and the GTE commands the reference
//! issues.

use gte::{Gte, cmd, ctrl, data};
use loader::lev::SVec3;
use math::mips;

use crate::FRACTION_ONE;
use crate::types::normal_axis;

pub(crate) fn pack(lo: i32, hi: i32) -> u32 {
    mips::pack_s16_pair(lo, hi)
}

pub(crate) fn abs(value: i32) -> i32 {
    if value < 0 {
        mips::neg_lo(value)
    } else {
        value
    }
}

/// The two coordinates (x=0, y=1, z=2) a triangle is projected onto, for its
/// dominant normal axis.
pub(crate) fn projection_axes(axis: i16) -> (usize, usize) {
    match axis {
        normal_axis::Y => (2, 0),
        normal_axis::Z => (0, 1),
        _ => (1, 2),
    }
}

pub(crate) fn coords(p: SVec3) -> [i32; 3] {
    [p.x as i32, p.y as i32, p.z as i32]
}

pub(crate) fn sv(x: i32, y: i32, z: i32) -> SVec3 {
    SVec3 {
        x: x as i16,
        y: y as i16,
        z: z as i16,
    }
}

/// `COLL_FIXED_TRIANGL_Barycentrics`: the point on edge `v1`-`v2` closest to
/// `point`, as a 4.12 fraction of the edge.
pub fn triangle_barycentrics(gte: &mut Gte, v1: SVec3, v2: SVec3, point: SVec3) -> SVec3 {
    let (v1x, v1y, v1z) = (v1.x as i32, v1.y as i32, v1.z as i32);
    let edge = (v2.x as i32 - v1x, v2.y as i32 - v1y, v2.z as i32 - v1z);
    let p = (
        point.x as i32 - v1x,
        point.y as i32 - v1y,
        point.z as i32 - v1z,
    );

    gte.ctc2(pack(edge.0, edge.1), ctrl::R_MATRIX);
    gte.ctc2(pack(edge.2, p.0), ctrl::R13R21);
    gte.ctc2(pack(p.1, p.2), ctrl::R22R23);
    gte.mtc2(pack(edge.0, edge.1), data::VXY0);
    gte.mtc2(edge.2 as u32, data::VZ0);
    gte.cop2(MVMVA_R_V0);
    let mut point_dot = gte.mfc2(data::MAC2) as i32;
    let mut edge_dot = gte.mfc2(data::MAC1) as i32;

    gte.mtc2(point_dot as u32, data::LZCS);
    let shift = (gte.mfc2(data::LZCR) as i32 - 2).clamp(0, 12) as u32;
    point_dot = mips::sll(point_dot, shift);
    if shift < 12 {
        edge_dot = mips::sra(edge_dot, 12 - shift);
    }

    let mut factor = 0;
    if edge_dot != 0 {
        factor = mips::div(point_dot, edge_dot).clamp(0, FRACTION_ONE);
    }

    gte.mtc2(factor as u32, data::IR0);
    load_mac(gte, v1x, v1y, v1z);
    load_ir(gte, edge.0, edge.1, edge.2);
    gte.cop2(GPL_SF1);
    SVec3 {
        x: gte.mfc2(data::MAC1) as i16,
        y: gte.mfc2(data::MAC2) as i16,
        z: gte.mfc2(data::MAC3) as i16,
    }
}

// Command words the reference issues (`CollFixed_Gte*`).
/// `mvmva` with the rotation matrix, V0, no translation.
pub(crate) const MVMVA_R_V0: u32 = 0x0406012;
pub(crate) const GPL_SF1: u32 = 0x01a8003e;
/// `mvmva` with sf=1, the rotation matrix, V0, no translation.
pub(crate) const RTV0_SF1: u32 = 0x0486012;
pub(crate) const GPF_SF1: u32 = 0x0198003d;
pub(crate) const OP_SF0: u32 = 0x0170000c;
/// `mvmva` with sf=1, the rotation matrix, IR as the vector, no translation.
pub(crate) const RTIR: u32 = 0x049e012;
const _: () = assert!(MVMVA_R_V0 & cmd::FUNCT_MASK == cmd::MVMVA);

pub(crate) fn load_ir(gte: &mut Gte, x: i32, y: i32, z: i32) {
    gte.mtc2(x as u32, data::IR1);
    gte.mtc2(y as u32, data::IR2);
    gte.mtc2(z as u32, data::IR3);
}

pub(crate) fn load_mac(gte: &mut Gte, x: i32, y: i32, z: i32) {
    gte.mtc2(x as u32, data::MAC1);
    gte.mtc2(y as u32, data::MAC2);
    gte.mtc2(z as u32, data::MAC3);
}
