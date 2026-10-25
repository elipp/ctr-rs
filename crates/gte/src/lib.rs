//! PlayStation GTE (Geometry Transformation Engine, COP2), ported from the
//! reference `ctr-native` implementation (`platform/native_gte_core.c` and
//! `native_inline_c.c`), which is derived from PsyCross / REDRIVER2 (MIT, see
//! `THIRD_PARTY_NOTICES.md`).
//!
//! The goal is bit-for-bit agreement with that reference, not with real
//! hardware, so its quirks are kept on purpose:
//!
//! * the 44-bit MAC negative-overflow threshold is `-0x8000000000`
//!   (2^39, not 2^43);
//! * registers are 32-bit words and fields such as `IR1` or `SX2` write only
//!   their own 16-bit half, leaving the other half as it was;
//! * `MFC2` normalises some registers *in place* when read;
//! * `m_mac0` / `m_mac3` persist between operations.
//!
//! `tests/reference.rs` checks this port against the C code on random inputs.

mod unr_table;

use unr_table::UNR_TABLE;

/// Data register indices (`mtc2` / `mfc2`).
pub mod data {
    pub const VXY0: usize = 0;
    pub const VZ0: usize = 1;
    pub const VXY1: usize = 2;
    pub const VZ1: usize = 3;
    pub const VXY2: usize = 4;
    pub const VZ2: usize = 5;
    /// Registers per input vertex (an XY word and a Z word).
    pub const VERTEX_STRIDE: usize = 2;
    pub const RGBC: usize = 6;
    pub const OTZ: usize = 7;
    pub const IR0: usize = 8;
    pub const IR1: usize = 9;
    pub const IR2: usize = 10;
    pub const IR3: usize = 11;
    pub const SXY0: usize = 12;
    pub const SXY1: usize = 13;
    pub const SXY2: usize = 14;
    pub const SXYP: usize = 15;
    pub const SZ0: usize = 16;
    pub const SZ1: usize = 17;
    pub const SZ2: usize = 18;
    pub const SZ3: usize = 19;
    pub const RGB0: usize = 20;
    pub const RGB1: usize = 21;
    pub const RGB2: usize = 22;
    pub const MAC0: usize = 24;
    pub const MAC1: usize = 25;
    pub const MAC2: usize = 26;
    pub const MAC3: usize = 27;
    pub const IRGB: usize = 28;
    pub const ORGB: usize = 29;
    pub const LZCS: usize = 30;
    pub const LZCR: usize = 31;
}

/// Control register indices (`ctc2` / `cfc2`).
pub mod ctrl {
    /// First word of the rotation matrix (`R11R12`); five words, two elements each.
    pub const R_MATRIX: usize = 0;
    pub const R13R21: usize = 1;
    pub const R22R23: usize = 2;
    pub const R33: usize = 4;
    pub const TRX: usize = 5;
    pub const TRY: usize = 6;
    pub const TRZ: usize = 7;
    /// First word of the light matrix (`L11L12`).
    pub const L_MATRIX: usize = 8;
    pub const L33: usize = 12;
    pub const RBK: usize = 13;
    pub const GBK: usize = 14;
    pub const BBK: usize = 15;
    /// First word of the light colour matrix (`LR1LR2`).
    pub const LC_MATRIX: usize = 16;
    pub const LB3: usize = 20;
    pub const RFC: usize = 21;
    pub const GFC: usize = 22;
    pub const BFC: usize = 23;
    pub const OFX: usize = 24;
    pub const OFY: usize = 25;
    pub const H: usize = 26;
    pub const DQA: usize = 27;
    pub const DQB: usize = 28;
    pub const ZSF3: usize = 29;
    pub const ZSF4: usize = 30;
    pub const FLAG: usize = 31;

    /// The three matrices, and the translation / background / far-colour
    /// vectors, are each eight registers apart.
    pub const GROUP_STRIDE: usize = 8;
}

/// COP2 function codes (the low 6 bits of a command word) and operand fields.
pub mod cmd {
    /// The reference treats code 0 as an alias of [`RTPS`].
    pub const RTPS_ALIAS: u32 = 0x00;
    pub const RTPS: u32 = 0x01;
    pub const NCLIP: u32 = 0x06;
    pub const OP: u32 = 0x0c;
    pub const DPCS: u32 = 0x10;
    pub const INTPL: u32 = 0x11;
    pub const MVMVA: u32 = 0x12;
    pub const NCDS: u32 = 0x13;
    pub const CDP: u32 = 0x14;
    pub const NCDT: u32 = 0x16;
    pub const NCCS: u32 = 0x1b;
    pub const CC: u32 = 0x1c;
    pub const NCS: u32 = 0x1e;
    pub const NCT: u32 = 0x20;
    pub const SQR: u32 = 0x28;
    pub const DCPL: u32 = 0x29;
    pub const DPCT: u32 = 0x2a;
    pub const AVSZ3: u32 = 0x2d;
    pub const AVSZ4: u32 = 0x2e;
    pub const RTPT: u32 = 0x30;
    pub const GPF: u32 = 0x3d;
    pub const GPL: u32 = 0x3e;
    pub const NCCT: u32 = 0x3f;

    /// Every function code the reference implements.
    pub const ALL: [u32; 23] = [
        RTPS_ALIAS, RTPS, NCLIP, OP, DPCS, INTPL, MVMVA, NCDS, CDP, NCDT, NCCS, CC, NCS, NCT, SQR,
        DCPL, DPCT, AVSZ3, AVSZ4, RTPT, GPF, GPL, NCCT,
    ];

    /// Mask for the function code within a command word.
    pub const FUNCT_MASK: u32 = 0x3f;
    /// Bits of a command word the GTE looks at (the rest select COP2 itself).
    pub const OPERAND_MASK: u32 = 0x1ff_ffff;

    // Operand fields of a command word.
    pub const SF_SHIFT: u32 = 19;
    pub const MX_SHIFT: u32 = 17;
    pub const V_SHIFT: u32 = 15;
    pub const CV_SHIFT: u32 = 13;
    pub const LM_SHIFT: u32 = 10;
    /// `mx`, `v` and `cv` are two-bit fields.
    pub const SELECTOR_MASK: u32 = 3;

    // MVMVA matrix selector. Values 0..=2 are the matrices at
    // `ctrl::R_MATRIX + mx * ctrl::GROUP_STRIDE`.
    /// Not a real matrix: the reference builds one from `RGBC.R`, `IR0` and
    /// two of the rotation elements.
    pub const MX_GARBAGE: usize = 3;

    // MVMVA vector selector: 0..=2 are V0..V2.
    pub const V_IR: usize = 3;

    // MVMVA translation selector. Values 0..=2 are the vectors at
    // `ctrl::TRX + cv * ctrl::GROUP_STRIDE` (translation, background, far colour).
    pub const CV_FAR_COLOR: usize = 2;
    pub const CV_NONE: usize = 3;
}

/// Bits of the FLAG control register.
///
/// Where the reference raises the summary [`ERROR`] bit alongside a cause,
/// the constant includes it; `IR3_SATURATED`, the colour bits and
/// `IR0_SATURATED` do not.
pub mod flag {
    pub const ERROR: u32 = 1 << 31;
    pub const MAC1_OVERFLOW_POS: u32 = ERROR | 1 << 30;
    pub const MAC2_OVERFLOW_POS: u32 = ERROR | 1 << 29;
    pub const MAC3_OVERFLOW_POS: u32 = ERROR | 1 << 28;
    pub const MAC1_OVERFLOW_NEG: u32 = ERROR | 1 << 27;
    pub const MAC2_OVERFLOW_NEG: u32 = ERROR | 1 << 26;
    pub const MAC3_OVERFLOW_NEG: u32 = ERROR | 1 << 25;
    pub const IR1_SATURATED: u32 = ERROR | 1 << 24;
    pub const IR2_SATURATED: u32 = ERROR | 1 << 23;
    pub const IR3_SATURATED: u32 = 1 << 22;
    pub const COLOR_R_SATURATED: u32 = 1 << 21;
    pub const COLOR_G_SATURATED: u32 = 1 << 20;
    pub const COLOR_B_SATURATED: u32 = 1 << 19;
    pub const SZ3_OTZ_SATURATED: u32 = ERROR | 1 << 18;
    pub const DIVIDE_OVERFLOW: u32 = ERROR | 1 << 17;
    pub const MAC0_OVERFLOW_POS: u32 = ERROR | 1 << 16;
    pub const MAC0_OVERFLOW_NEG: u32 = ERROR | 1 << 15;
    pub const SX2_SATURATED: u32 = ERROR | 1 << 14;
    pub const SY2_SATURATED: u32 = ERROR | 1 << 13;
    pub const IR0_SATURATED: u32 = 1 << 12;

    // The same bits, indexed by channel (MAC1..3, IR1..3, R/G/B, SX2/SY2).
    pub(crate) const MAC_OVERFLOW_POS: [u32; 3] =
        [MAC1_OVERFLOW_POS, MAC2_OVERFLOW_POS, MAC3_OVERFLOW_POS];
    pub(crate) const MAC_OVERFLOW_NEG: [u32; 3] =
        [MAC1_OVERFLOW_NEG, MAC2_OVERFLOW_NEG, MAC3_OVERFLOW_NEG];
    pub(crate) const IR_SATURATED: [u32; 3] = [IR1_SATURATED, IR2_SATURATED, IR3_SATURATED];
    pub(crate) const COLOR_SATURATED: [u32; 3] =
        [COLOR_R_SATURATED, COLOR_G_SATURATED, COLOR_B_SATURATED];
    pub(crate) const SXY_SATURATED: [u32; 2] = [SX2_SATURATED, SY2_SATURATED];

    /// `CTC2` keeps only these bits of a FLAG write.
    pub(crate) const WRITABLE_MASK: u32 = 0x7fff_f000;
    /// If any of these causes is set after a FLAG write, [`ERROR`] is set too.
    pub(crate) const ERROR_CAUSES_MASK: u32 = 0x7f87_e000;
}

// Fixed-point layout.
/// Fractional bits of the 4.12 format used for matrices and the `sf` shift.
const FRACTION_BITS: u32 = 12;
const ONE: i64 = 1 << FRACTION_BITS;
/// A colour byte is scaled by this to match IR's scale (`rgb << 4`), and MAC is
/// scaled back by it to produce a colour byte (`mac >> 4`).
const COLOR_IR_SHIFT: u32 = 4;
/// Colour bytes are shifted by this before blending towards the far colour.
const COLOR_BLEND_SHIFT: u32 = 16;
/// Screen coordinates are the offset-plus-product value shifted down by this.
const SXY_FRACTION_BITS: u32 = 16;
/// The `mx == MX_GARBAGE` matrix scales `RGBC.R` by this.
const GARBAGE_MATRIX_SHIFT: u32 = 4;

// Saturation ranges.
const IR_MAX: i32 = 0x7fff;
const IR_MIN: i32 = -0x8000;
const IR0_MAX: i32 = 0x1000;
const COLOR_MAX: i32 = 0xff;
/// Largest SZ3 / OTZ value.
const DEPTH_MAX: i32 = 0xffff;
/// Largest quotient of the perspective divide.
const DIVIDE_MAX: u32 = 0x1ffff;
const DIVIDE_OVERFLOW: u32 = 0xffff_ffff;
const SXY_MAX: i64 = 0x3ff;
const SXY_MIN: i64 = -0x400;
/// MAC1..3 are 44-bit accumulators; this is the positive limit, 2^43 - 1.
const MAC_MAX: i64 = 0x7ff_ffff_ffff;
/// Reference quirk: the negative limit is -2^39, not -2^43.
const MAC_MIN: i64 = -0x80_0000_0000;
const MAC0_MAX: i64 = i32::MAX as i64;
const MAC0_MIN: i64 = i32::MIN as i64;

// IRGB / ORGB: three 5-bit colour channels, each IR = channel << 7.
const IRGB_CHANNEL_BITS: u32 = 5;
const IRGB_CHANNEL_MASK: u32 = (1 << IRGB_CHANNEL_BITS) - 1;
const IRGB_TO_IR_SHIFT: u32 = 7;

// Byte lanes of RGBC / RGBn: R, G, B, then the command code.
const CODE_BYTE: u32 = 3;

// Newton-Raphson reciprocal (UNR) coefficients.
const UNR_MANTISSA_MASK: i32 = 0x7fff;
const UNR_TABLE_ROUND: i32 = 0x40;
const UNR_TABLE_SHIFT: u32 = 7;
const UNR_TABLE_BIAS: i32 = 0x101;
const UNR_STEP_BIAS: i32 = 0x80;
const UNR_IMPLICIT_ONE: i32 = 0x8000;
const UNR_STEP_MASK: i32 = 0x1ffff;
const UNR_ROUND: u64 = 0x8000;
const UNR_RESULT_SHIFT: u32 = 16;
const UNR_DENOMINATOR_BITS: u32 = 16;

/// Diagonal elements (11, 22, 33) of a row-major 3x3 matrix.
const MATRIX_DIAGONAL: [usize; 3] = [0, 4, 8];

// 16-bit halves of a register word.
fn lo_s(w: u32) -> i32 {
    w as u16 as i16 as i32
}
fn hi_s(w: u32) -> i32 {
    (w >> 16) as u16 as i16 as i32
}
fn lo_u(w: u32) -> i32 {
    (w & 0xffff) as i32
}
fn set_lo(w: &mut u32, v: i32) {
    *w = (*w & 0xffff_0000) | (v as u16 as u32);
}
fn set_hi(w: &mut u32, v: i32) {
    *w = (*w & 0x0000_ffff) | ((v as u16 as u32) << 16);
}
fn set_byte(w: &mut u32, lane: u32, v: i32) {
    let shift = lane * 8;
    *w = (*w & !(0xff << shift)) | ((v as u8 as u32) << shift);
}
fn byte(w: u32, lane: u32) -> i32 {
    ((w >> (lane * 8)) & 0xff) as i32
}

/// `gte_shift`: apply the `sf` flag (positive: `>> 12`, negative: `* 4096`).
fn shift(a: i64, sf: i32) -> i64 {
    match sf.signum() {
        1 => a >> FRACTION_BITS,
        -1 => a.wrapping_mul(ONE),
        _ => a,
    }
}

/// Lower bound of IR1..3 for the `lm` flag.
fn ir_min(lm: i32) -> i32 {
    if lm == 0 { IR_MIN } else { 0 }
}

/// Number of leading bits equal to the sign bit (the GTE's LZCR).
fn leading_count(lzcs: u32) -> u32 {
    (if (lzcs as i32) < 0 { !lzcs } else { lzcs }).leading_zeros()
}

/// The GTE's 17-bit reciprocal divide (UNR), returning [`DIVIDE_OVERFLOW`] on overflow.
fn divide(numerator: u16, denominator: u16) -> u32 {
    let (n, d) = (numerator as i32, denominator as i32);
    if n >= d * 2 {
        return DIVIDE_OVERFLOW;
    }
    let shift = leading_count(denominator as u32).wrapping_sub(UNR_DENOMINATOR_BITS) as i32;
    let r1 = (d << shift) & UNR_MANTISSA_MASK;
    let r2 =
        UNR_TABLE[((r1 + UNR_TABLE_ROUND) >> UNR_TABLE_SHIFT) as usize] as i32 + UNR_TABLE_BIAS;
    let r3 = ((UNR_STEP_BIAS - r2 * (r1 + UNR_IMPLICIT_ONE)) >> 8) & UNR_STEP_MASK;
    let reciprocal = ((r2 * r3 + UNR_STEP_BIAS) >> 8) as u32;
    (((reciprocal as u64) * ((n << shift) as u64) + UNR_ROUND) >> UNR_RESULT_SHIFT) as u32
}

/// Register file plus the two pieces of state the reference keeps in globals.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Gte {
    d: [u32; 32],
    c: [u32; 32],
    sf: i32,
    mac0: i64,
    mac3: i64,
}

impl Gte {
    pub fn new() -> Self {
        Self::default()
    }

    /// Raw data registers, for inspection.
    pub fn data_regs(&self) -> &[u32; 32] {
        &self.d
    }

    /// Raw control registers, for inspection.
    pub fn ctrl_regs(&self) -> &[u32; 32] {
        &self.c
    }

    // ---- register moves --------------------------------------------------

    /// `MTC2`: write a data register.
    pub fn mtc2(&mut self, value: u32, reg: usize) {
        match reg {
            data::SXYP => {
                self.d[data::SXY0] = self.d[data::SXY1];
                self.d[data::SXY1] = self.d[data::SXY2];
                self.d[data::SXY2] = value;
            }
            data::IRGB => {
                for i in 0..3 {
                    let channel = (value >> (IRGB_CHANNEL_BITS * i as u32)) & IRGB_CHANNEL_MASK;
                    set_lo(
                        &mut self.d[data::IR1 + i],
                        (channel << IRGB_TO_IR_SHIFT) as i32,
                    );
                }
            }
            data::LZCS => self.d[data::LZCR] = leading_count(value),
            data::LZCR => return,
            _ => {}
        }
        self.d[reg] = value;
    }

    /// `MFC2`: read a data register (some reads normalise the stored value).
    pub fn mfc2(&mut self, reg: usize) -> u32 {
        match reg {
            data::VZ0 | data::VZ1 | data::VZ2 | data::IR0 | data::IR1 | data::IR2 | data::IR3 => {
                self.d[reg] = lo_s(self.d[reg]) as u32
            }
            data::OTZ | data::SZ0 | data::SZ1 | data::SZ2 | data::SZ3 => {
                self.d[reg] = lo_u(self.d[reg]) as u32
            }
            data::SXYP => self.d[reg] = self.d[data::SXY2],
            data::IRGB | data::ORGB => {
                self.d[reg] = (0..3).fold(0, |acc, i| {
                    let channel =
                        (self.ir(1 + i) >> IRGB_TO_IR_SHIFT).clamp(0, IRGB_CHANNEL_MASK as i32);
                    acc | (channel as u32) << (IRGB_CHANNEL_BITS * i as u32)
                });
            }
            _ => {}
        }
        self.d[reg]
    }

    /// `CTC2`: write a control register.
    pub fn ctc2(&mut self, mut value: u32, reg: usize) {
        match reg {
            ctrl::R33 | ctrl::L33 | ctrl::LB3 | ctrl::H | ctrl::DQA | ctrl::ZSF3 | ctrl::ZSF4 => {
                value = value as u16 as i16 as i32 as u32
            }
            ctrl::FLAG => {
                value &= flag::WRITABLE_MASK;
                if value & flag::ERROR_CAUSES_MASK != 0 {
                    value |= flag::ERROR;
                }
            }
            _ => {}
        }
        self.c[reg] = value;
    }

    /// `CFC2`: read a control register.
    pub fn cfc2(&self, reg: usize) -> u32 {
        self.c[reg]
    }

    // ---- field access ----------------------------------------------------

    fn raise(&mut self, bits: u32) {
        self.c[ctrl::FLAG] |= bits;
    }

    /// `LIM`
    fn lim(&mut self, value: i32, max: i32, min: i32, flag: u32) -> i32 {
        if value > max {
            self.raise(flag);
            max
        } else if value < min {
            self.raise(flag);
            min
        } else {
            value
        }
    }

    /// `IR0..IR3`
    fn ir(&self, n: usize) -> i32 {
        lo_s(self.d[data::IR0 + n])
    }
    fn set_ir(&mut self, n: usize, v: i32) {
        set_lo(&mut self.d[data::IR0 + n], v);
    }
    /// `MAC0..MAC3`
    fn mac(&self, n: usize) -> i32 {
        self.d[data::MAC0 + n] as i32
    }
    fn set_mac(&mut self, n: usize, v: i32) {
        self.d[data::MAC0 + n] = v as u32;
    }
    fn translation(&self, i: usize) -> i32 {
        self.c[ctrl::TRX + i] as i32
    }
    fn background(&self, i: usize) -> i32 {
        self.c[ctrl::RBK + i] as i32
    }
    fn far_color(&self, i: usize) -> i32 {
        self.c[ctrl::RFC + i] as i32
    }
    /// `SZ0..SZ3`
    fn sz(&self, n: usize) -> i32 {
        lo_u(self.d[data::SZ0 + n])
    }
    /// Matrix element `k` (row-major, 0..9) of the 5-word matrix at control register `base`.
    fn mat(&self, base: usize, k: usize) -> i32 {
        let w = self.c[base + k / 2];
        if k.is_multiple_of(2) {
            lo_s(w)
        } else {
            hi_s(w)
        }
    }
    /// Input vector `v` (0..3 are V0..V2, [`cmd::V_IR`] is IR1..IR3).
    fn vec(&self, v: usize) -> [i32; 3] {
        if v != cmd::V_IR {
            let xy = self.d[data::VXY0 + v * data::VERTEX_STRIDE];
            let z = self.d[data::VZ0 + v * data::VERTEX_STRIDE];
            [lo_s(xy), hi_s(xy), lo_s(z)]
        } else {
            [self.ir(1), self.ir(2), self.ir(3)]
        }
    }
    /// Colour channel `i` (R, G, B) of `RGBC`.
    fn rgb_byte(&self, i: usize) -> i32 {
        byte(self.d[data::RGBC], i as u32)
    }

    // ---- limiters ---------------------------------------------------------

    /// `A1..A3` / `BOUNDS`: 44-bit overflow flags, then `sf` shift.
    fn a(&mut self, i: usize, v: i64) -> i32 {
        if i == 2 {
            self.mac3 = v;
        }
        if v > MAC_MAX {
            self.raise(flag::MAC_OVERFLOW_POS[i]);
        }
        if v < MAC_MIN {
            self.raise(flag::MAC_OVERFLOW_NEG[i]);
        }
        shift(v, self.sf) as i32
    }

    /// `Lm_B1..Lm_B3`
    fn lm_b(&mut self, i: usize, v: i32, lm: i32) -> i32 {
        self.lim(v, IR_MAX, ir_min(lm), flag::IR_SATURATED[i])
    }

    /// `Lm_B3_sf`
    fn lm_b3_sf(&mut self, value: i64, lm: i32) -> i32 {
        let value_sf = shift(value, self.sf) as i32;
        let value_12 = shift(value, 1) as i32;
        if !(IR_MIN..=IR_MAX).contains(&value_12) {
            self.raise(flag::IR3_SATURATED);
        }
        value_sf.clamp(ir_min(lm), IR_MAX)
    }

    /// `Lm_C1..Lm_C3`
    fn lm_c(&mut self, i: usize, v: i32) -> i32 {
        self.lim(v, COLOR_MAX, 0, flag::COLOR_SATURATED[i])
    }

    /// `Lm_D`
    fn lm_d(&mut self, a: i64, sf: i32) -> i32 {
        self.lim(shift(a, sf) as i32, DEPTH_MAX, 0, flag::SZ3_OTZ_SATURATED)
    }

    /// `Lm_E`
    fn lm_e(&mut self, result: u32) -> u32 {
        if result == DIVIDE_OVERFLOW {
            self.raise(flag::DIVIDE_OVERFLOW);
            DIVIDE_MAX
        } else {
            result.min(DIVIDE_MAX)
        }
    }

    /// `F`
    fn f(&mut self, a: i64) -> i64 {
        self.mac0 = a;
        if a > MAC0_MAX {
            self.raise(flag::MAC0_OVERFLOW_POS);
        }
        if a < MAC0_MIN {
            self.raise(flag::MAC0_OVERFLOW_NEG);
        }
        a
    }

    /// `Lm_G1` / `Lm_G2`
    fn lm_g(&mut self, i: usize, a: i64) -> i32 {
        if a > SXY_MAX {
            self.raise(flag::SXY_SATURATED[i]);
            SXY_MAX as i32
        } else if a < SXY_MIN {
            self.raise(flag::SXY_SATURATED[i]);
            SXY_MIN as i32
        } else {
            a as i32
        }
    }

    /// `Lm_H`
    fn lm_h(&mut self, value: i64, sf: i32) -> i32 {
        let value_sf = shift(value, sf);
        let value_12 = shift(value, 1) as i32;
        if !(0..=IR0_MAX as i64).contains(&value_sf) {
            self.raise(flag::IR0_SATURATED);
        }
        value_12.clamp(0, IR0_MAX)
    }

    // ---- shared pipeline stages ------------------------------------------

    /// `IR1..3 = Lm_B(MAC1..3, lm)`
    fn ir_from_mac(&mut self, lm: i32) {
        for i in 0..3 {
            let m = self.mac(1 + i);
            let v = self.lm_b(i, m, lm);
            self.set_ir(1 + i, v);
        }
    }

    /// Push the colour FIFO from `MAC1..3 >> 4`.
    fn push_color(&mut self) {
        self.d[data::RGB0] = self.d[data::RGB1];
        self.d[data::RGB1] = self.d[data::RGB2];
        let code = byte(self.d[data::RGBC], CODE_BYTE);
        set_byte(&mut self.d[data::RGB2], CODE_BYTE, code);
        for i in 0..3 {
            let m = self.mac(1 + i) >> COLOR_IR_SHIFT;
            let v = self.lm_c(i, m);
            set_byte(&mut self.d[data::RGB2], i as u32, v);
        }
    }

    /// `MAC = L * v`, `IR = limit(MAC)`
    fn light_vector(&mut self, v: [i32; 3], lm: i32) {
        for i in 0..3 {
            let sum: i64 = (0..3)
                .map(|j| (self.mat(ctrl::L_MATRIX, i * 3 + j) * v[j]) as i64)
                .sum();
            let m = self.a(i, sum);
            self.set_mac(1 + i, m);
        }
        self.ir_from_mac(lm);
    }

    /// `MAC = BK<<12 + LC * IR`, `IR = limit(MAC)`
    fn light_color(&mut self, lm: i32) {
        for i in 0..3 {
            let sum: i64 = (0..3)
                .map(|j| (self.mat(ctrl::LC_MATRIX, i * 3 + j) * self.ir(1 + j)) as i64)
                .sum();
            let m = self.a(i, ((self.background(i) as i64) << FRACTION_BITS) + sum);
            self.set_mac(1 + i, m);
        }
        self.ir_from_mac(lm);
    }

    /// `MAC = base + IR0 * limit(FC<<12 - base)`: interpolation towards the far colour.
    fn far_color_blend(&mut self, i: usize, base: i32) {
        let inner = self.a(
            i,
            ((self.far_color(i) as i64) << FRACTION_BITS) - base as i64,
        );
        let blended = self.lm_b(i, inner, 0);
        let m = self.a(
            i,
            base.wrapping_add(self.ir(0).wrapping_mul(blended)) as i64,
        );
        self.set_mac(1 + i, m);
    }

    /// `MAC = (rgb << 4) * IR` per channel, optionally blended towards the far colour.
    fn color_times_ir(&mut self, lm: i32, blend: bool) {
        for i in 0..3 {
            let base = (self.rgb_byte(i) << COLOR_IR_SHIFT).wrapping_mul(self.ir(1 + i));
            if blend {
                self.far_color_blend(i, base);
            } else {
                let m = self.a(i, base as i64);
                self.set_mac(1 + i, m);
            }
        }
        self.ir_from_mac(lm);
    }

    /// `GTE_RotTransPers`
    fn rot_trans_pers(&mut self, idx: usize, lm: i32) -> i32 {
        let v = self.vec(idx);
        for i in 0..3 {
            let sum: i64 = (0..3)
                .map(|j| (self.mat(ctrl::R_MATRIX, i * 3 + j) * v[j]) as i64)
                .sum();
            let m = self.a(i, ((self.translation(i) as i64) << FRACTION_BITS) + sum);
            self.set_mac(1 + i, m);
        }
        let ir1 = self.lm_b(0, self.mac(1), lm);
        self.set_ir(1, ir1);
        let ir2 = self.lm_b(1, self.mac(2), lm);
        self.set_ir(2, ir2);
        let ir3 = self.lm_b3_sf(self.mac3, lm);
        self.set_ir(3, ir3);

        // Depth FIFO: SZ0 <- SZ1 <- SZ2 <- SZ3 <- new depth.
        for n in 0..3 {
            let next = lo_u(self.d[data::SZ0 + n + 1]);
            set_lo(&mut self.d[data::SZ0 + n], next);
        }
        let sz3 = self.lm_d(self.mac3, 1);
        set_lo(&mut self.d[data::SZ3], sz3);

        let h = lo_s(self.c[ctrl::H]) as u16;
        let h_over_sz3 = self.lm_e(divide(h, lo_u(self.d[data::SZ3]) as u16)) as i32;
        self.d[data::SXY0] = self.d[data::SXY1];
        self.d[data::SXY1] = self.d[data::SXY2];
        let ofx = self.c[ctrl::OFX] as i32 as i64;
        let ofy = self.c[ctrl::OFY] as i32 as i64;
        let fx = self.f(ofx + self.ir(1) as i64 * h_over_sz3 as i64) >> SXY_FRACTION_BITS;
        let sx = self.lm_g(0, fx);
        set_lo(&mut self.d[data::SXY2], sx);
        let fy = self.f(ofy + self.ir(2) as i64 * h_over_sz3 as i64) >> SXY_FRACTION_BITS;
        let sy = self.lm_g(1, fy);
        set_hi(&mut self.d[data::SXY2], sy);
        h_over_sz3
    }

    fn depth_cue(&mut self, h_over_sz3: i32) {
        let dqa = lo_s(self.c[ctrl::DQA]) as i64;
        let dqb = self.c[ctrl::DQB] as i32 as i64;
        let m = self.f(dqb + dqa * h_over_sz3 as i64) as i32;
        self.set_mac(0, m);
        let ir0 = self.lm_h(self.mac0, 1);
        self.set_ir(0, ir0);
    }

    /// Average the depth FIFO entries `first..=3` weighted by `zsf` into `MAC0` and `OTZ`.
    fn average_depth(&mut self, zsf: i32, first: usize) {
        let sum: i64 = (first..4).map(|n| (zsf * self.sz(n)) as i64).sum();
        let m = self.f(sum) as i32;
        self.set_mac(0, m);
        let otz = self.lm_d(self.mac0, 1);
        set_lo(&mut self.d[data::OTZ], otz);
    }

    // ---- COP2 command -----------------------------------------------------

    /// Execute a COP2 command word. Returns `false` for unimplemented
    /// function codes (the flag register is still cleared, as in the reference).
    pub fn cop2(&mut self, op: u32) -> bool {
        let op = op & cmd::OPERAND_MASK;
        let lm = ((op >> cmd::LM_SHIFT) & 1) as i32;
        self.sf = ((op >> cmd::SF_SHIFT) & 1) as i32;
        self.c[ctrl::FLAG] = 0;

        match op & cmd::FUNCT_MASK {
            cmd::RTPS_ALIAS | cmd::RTPS => {
                let h = self.rot_trans_pers(0, lm);
                self.depth_cue(h);
            }
            cmd::NCLIP => {
                let sx = |n: usize| lo_s(self.d[data::SXY0 + n]) as i64;
                let sy = |n: usize| hi_s(self.d[data::SXY0 + n]) as i64;
                let v = sx(0) * sy(1) + sx(1) * sy(2) + sx(2) * sy(0)
                    - sx(0) * sy(2)
                    - sx(1) * sy(0)
                    - sx(2) * sy(1);
                let m = self.f(v) as i32;
                self.set_mac(0, m);
                // The reference discards any flags raised by the area computation.
                self.c[ctrl::FLAG] = 0;
            }
            cmd::OP => {
                let [d1, d2, d3] = MATRIX_DIAGONAL.map(|k| self.mat(ctrl::R_MATRIX, k) as i64);
                let ir = [self.ir(1) as i64, self.ir(2) as i64, self.ir(3) as i64];
                let m1 = self.a(0, d2 * ir[2] - d3 * ir[1]);
                self.set_mac(1, m1);
                let m2 = self.a(1, d3 * ir[0] - d1 * ir[2]);
                self.set_mac(2, m2);
                let m3 = self.a(2, d1 * ir[1] - d2 * ir[0]);
                self.set_mac(3, m3);
                self.ir_from_mac(lm);
            }
            cmd::DPCS => {
                for i in 0..3 {
                    self.far_color_blend(i, self.rgb_byte(i) << COLOR_BLEND_SHIFT);
                }
                self.ir_from_mac(lm);
                self.push_color();
            }
            cmd::INTPL => {
                for i in 0..3 {
                    self.far_color_blend(i, self.ir(1 + i) << FRACTION_BITS);
                }
                self.ir_from_mac(lm);
                self.push_color();
            }
            cmd::MVMVA => {
                let field = |shift| ((op >> shift) & cmd::SELECTOR_MASK) as usize;
                let (mx, v, cv) = (
                    field(cmd::MX_SHIFT),
                    field(cmd::V_SHIFT),
                    field(cmd::CV_SHIFT),
                );
                let vec = self.vec(v);
                let matrix = |g: &Self, k: usize| -> i32 {
                    if mx != cmd::MX_GARBAGE {
                        return g.mat(ctrl::R_MATRIX + mx * ctrl::GROUP_STRIDE, k);
                    }
                    let r = byte(g.d[data::RGBC], 0);
                    match k {
                        0 => (-r) << GARBAGE_MATRIX_SHIFT,
                        1 => r << GARBAGE_MATRIX_SHIFT,
                        2 => g.ir(0),
                        3..=5 => lo_s(g.c[ctrl::R13R21]),
                        _ => lo_s(g.c[ctrl::R22R23]),
                    }
                };
                let offset = |g: &Self, i: usize| -> i64 {
                    if cv == cmd::CV_NONE {
                        0
                    } else {
                        g.c[ctrl::TRX + cv * ctrl::GROUP_STRIDE + i] as i32 as i64
                    }
                };
                if cv == cmd::CV_FAR_COLOR {
                    // The reference's far-colour path skips the X column in the
                    // stored result...
                    for i in 0..3 {
                        let rest = (matrix(self, i * 3 + 1) * vec[1]) as i64
                            + (matrix(self, i * 3 + 2) * vec[2]) as i64;
                        let r = self.a(i, rest);
                        self.set_mac(1 + i, r);
                    }
                    // ...and evaluates it separately, keeping only the flag and `mac3` side effects.
                    for i in 0..3 {
                        let first = offset(self, i) * ONE + (matrix(self, i * 3) * vec[0]) as i64;
                        let a = self.a(i, first);
                        self.lm_b(i, a, 0);
                    }
                } else {
                    for i in 0..3 {
                        let sum: i64 = (0..3)
                            .map(|j| (matrix(self, i * 3 + j) * vec[j]) as i64)
                            .sum();
                        let r = self.a(i, offset(self, i) * ONE + sum);
                        self.set_mac(1 + i, r);
                    }
                }
                self.ir_from_mac(lm);
            }
            cmd::NCDS => {
                self.light_vector(self.vec(0), lm);
                self.light_color(lm);
                self.color_times_ir(lm, true);
                self.push_color();
            }
            cmd::CDP => {
                self.light_color(lm);
                self.color_times_ir(lm, true);
                self.push_color();
            }
            cmd::NCDT => {
                for v in 0..3 {
                    self.light_vector(self.vec(v), lm);
                    self.light_color(lm);
                    self.color_times_ir(lm, true);
                    self.push_color();
                }
            }
            cmd::NCCS => {
                self.light_vector(self.vec(0), lm);
                self.light_color(lm);
                self.color_times_ir(lm, false);
                self.push_color();
            }
            cmd::CC => {
                self.light_color(lm);
                self.color_times_ir(lm, false);
                self.push_color();
            }
            cmd::NCS => {
                self.light_vector(self.vec(0), lm);
                self.light_color(lm);
                self.push_color();
            }
            cmd::NCT => {
                for v in 0..3 {
                    self.light_vector(self.vec(v), lm);
                    self.light_color(lm);
                    self.push_color();
                }
            }
            cmd::SQR => {
                for i in 0..3 {
                    let ir = self.ir(1 + i);
                    let m = self.a(i, (ir * ir) as i64);
                    self.set_mac(1 + i, m);
                }
                self.ir_from_mac(lm);
            }
            cmd::DCPL => {
                self.color_times_ir(lm, true);
                self.push_color();
            }
            cmd::DPCT => {
                for _ in 0..3 {
                    for i in 0..3 {
                        self.far_color_blend(
                            i,
                            byte(self.d[data::RGB0], i as u32) << COLOR_BLEND_SHIFT,
                        );
                    }
                    self.ir_from_mac(lm);
                    self.push_color();
                }
            }
            cmd::AVSZ3 => self.average_depth(lo_s(self.c[ctrl::ZSF3]), 1),
            cmd::AVSZ4 => self.average_depth(lo_s(self.c[ctrl::ZSF4]), 0),
            cmd::RTPT => {
                let mut h = 0;
                for v in 0..3 {
                    h = self.rot_trans_pers(v, lm);
                }
                self.depth_cue(h);
            }
            cmd::GPF => {
                for i in 0..3 {
                    let m = self.a(i, (self.ir(0) * self.ir(1 + i)) as i64);
                    self.set_mac(1 + i, m);
                }
                self.ir_from_mac(lm);
                self.push_color();
            }
            cmd::GPL => {
                for i in 0..3 {
                    let v = shift(self.mac(1 + i) as i64, -self.sf)
                        + (self.ir(0) * self.ir(1 + i)) as i64;
                    let m = self.a(i, v);
                    self.set_mac(1 + i, m);
                }
                self.ir_from_mac(lm);
                self.push_color();
            }
            cmd::NCCT => {
                for v in 0..3 {
                    self.light_vector(self.vec(v), lm);
                    self.light_color(lm);
                    self.color_times_ir(lm, false);
                    self.push_color();
                }
            }
            _ => return false,
        }
        true
    }
}
