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
//! `tests/oracle.rs` checks this port against the C code on random inputs.

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
    pub const R_MATRIX: usize = 0; // R11R12 .. R33 (5 words)
    pub const TRX: usize = 5;
    pub const TRY: usize = 6;
    pub const TRZ: usize = 7;
    pub const L_MATRIX: usize = 8; // L11L12 .. L33 (5 words)
    pub const RBK: usize = 13;
    pub const GBK: usize = 14;
    pub const BBK: usize = 15;
    pub const LC_MATRIX: usize = 16; // LR1LR2 .. LB3 (5 words)
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
}

const F31: u32 = 1 << 31;

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
fn set_byte(w: &mut u32, byte: u32, v: i32) {
    let shift = byte * 8;
    *w = (*w & !(0xff << shift)) | ((v as u8 as u32) << shift);
}
fn byte(w: u32, byte: u32) -> i32 {
    ((w >> (byte * 8)) & 0xff) as i32
}

/// `gte_shift`: apply the `sf` flag (positive: `>> 12`, negative: `* 4096`).
fn shift(a: i64, sf: i32) -> i64 {
    match sf.signum() {
        1 => a >> 12,
        -1 => a.wrapping_mul(4096),
        _ => a,
    }
}

/// Number of leading bits equal to the sign bit (the GTE's LZCR).
fn leading_count(lzcs: u32) -> u32 {
    (if (lzcs as i32) < 0 { !lzcs } else { lzcs }).leading_zeros()
}

/// The GTE's 17-bit reciprocal divide (UNR), returning `0xffffffff` on overflow.
fn divide(numerator: u16, denominator: u16) -> u32 {
    let (n, d) = (numerator as i32, denominator as i32);
    if n < d * 2 {
        let shift = leading_count(denominator as u32).wrapping_sub(16) as i32;
        let r1 = (d << shift) & 0x7fff;
        let r2 = UNR_TABLE[((r1 + 0x40) >> 7) as usize] as i32 + 0x101;
        let r3 = ((0x80 - r2 * (r1 + 0x8000)) >> 8) & 0x1ffff;
        let reciprocal = ((r2 * r3 + 0x80) >> 8) as u32;
        (((reciprocal as u64) * ((n << shift) as u64) + 0x8000) >> 16) as u32
    } else {
        0xffff_ffff
    }
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
            15 => {
                self.d[12] = self.d[13];
                self.d[13] = self.d[14];
                self.d[14] = value;
            }
            28 => {
                set_lo(&mut self.d[9], ((value & 0x1f) << 7) as i32);
                set_lo(&mut self.d[10], ((value & 0x3e0) << 2) as i32);
                set_lo(&mut self.d[11], ((value & 0x7c00) >> 3) as i32);
            }
            30 => self.d[31] = leading_count(value),
            31 => return,
            _ => {}
        }
        self.d[reg] = value;
    }

    /// `MFC2`: read a data register (some reads normalise the stored value).
    pub fn mfc2(&mut self, reg: usize) -> u32 {
        match reg {
            1 | 3 | 5 | 8 | 9 | 10 | 11 => self.d[reg] = lo_s(self.d[reg]) as u32,
            7 | 16 | 17 | 18 | 19 => self.d[reg] = lo_u(self.d[reg]) as u32,
            15 => self.d[reg] = self.d[14],
            28 | 29 => {
                let q = |ir: i32| self_lim_nf(ir >> 7, 0x1f, 0) as u32;
                self.d[reg] = q(lo_s(self.d[9])) | (q(lo_s(self.d[10])) << 5) | (q(lo_s(self.d[11])) << 10);
            }
            _ => {}
        }
        self.d[reg]
    }

    /// `CTC2`: write a control register.
    pub fn ctc2(&mut self, mut value: u32, reg: usize) {
        match reg {
            4 | 12 | 20 | 26 | 27 | 29 | 30 => value = value as u16 as i16 as i32 as u32,
            31 => {
                value &= 0x7fff_f000;
                if value & 0x7f87_e000 != 0 {
                    value |= F31;
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

    fn flag(&mut self, bits: u32) {
        self.c[31] |= bits;
    }

    /// `LIM`
    fn lim(&mut self, value: i32, max: i32, min: i32, flag: u32) -> i32 {
        if value > max {
            self.flag(flag);
            max
        } else if value < min {
            self.flag(flag);
            min
        } else {
            value
        }
    }

    fn ir(&self, n: usize) -> i32 {
        lo_s(self.d[8 + n])
    }
    fn set_ir(&mut self, n: usize, v: i32) {
        set_lo(&mut self.d[8 + n], v);
    }
    fn mac(&self, n: usize) -> i32 {
        self.d[24 + n] as i32
    }
    fn set_mac(&mut self, n: usize, v: i32) {
        self.d[24 + n] = v as u32;
    }
    fn tr(&self, i: usize) -> i32 {
        self.c[5 + i] as i32
    }
    fn bk(&self, i: usize) -> i32 {
        self.c[13 + i] as i32
    }
    fn fc(&self, i: usize) -> i32 {
        self.c[21 + i] as i32
    }
    fn sz(&self, n: usize) -> i32 {
        lo_u(self.d[16 + n])
    }
    /// Matrix element `k` (row-major, 0..9) of the 5-word matrix at control register `base`.
    fn mat(&self, base: usize, k: usize) -> i32 {
        let w = self.c[base + k / 2];
        if k % 2 == 0 { lo_s(w) } else { hi_s(w) }
    }
    /// Input vector `v` (0..3 are V0..V2, 3 is IR1..IR3).
    fn vec(&self, v: usize) -> [i32; 3] {
        if v < 3 {
            [lo_s(self.d[v * 2]), hi_s(self.d[v * 2]), lo_s(self.d[v * 2 + 1])]
        } else {
            [self.ir(1), self.ir(2), self.ir(3)]
        }
    }
    fn rgb_byte(&self, i: usize) -> i32 {
        byte(self.d[6], i as u32)
    }

    // ---- limiters ---------------------------------------------------------

    /// `A1..A3` / `BOUNDS`: 44-bit overflow flags, then `sf` shift.
    fn a(&mut self, i: usize, v: i64) -> i32 {
        const MAX: [u32; 3] = [F31 | (1 << 30), F31 | (1 << 29), F31 | (1 << 28)];
        const MIN: [u32; 3] = [F31 | (1 << 27), F31 | (1 << 26), F31 | (1 << 25)];
        if i == 2 {
            self.mac3 = v;
        }
        if v > 0x7ff_ffff_ffff {
            self.flag(MAX[i]);
        }
        // Reference quirk: the threshold is 2^39, not 2^43.
        if v < -0x80_0000_0000 {
            self.flag(MIN[i]);
        }
        shift(v, self.sf) as i32
    }

    /// `Lm_B1..Lm_B3`
    fn lm_b(&mut self, i: usize, v: i32, lm: i32) -> i32 {
        const FLAG: [u32; 3] = [F31 | (1 << 24), F31 | (1 << 23), 1 << 22];
        self.lim(v, 0x7fff, -0x8000 * (lm == 0) as i32, FLAG[i])
    }

    /// `Lm_B3_sf`
    fn lm_b3_sf(&mut self, value: i64, lm: i32) -> i32 {
        let value_sf = shift(value, self.sf) as i32;
        let value_12 = shift(value, 1) as i32;
        let min = if lm == 0 { -0x8000 } else { 0 };
        if !(-0x8000..=0x7fff).contains(&value_12) {
            self.flag(1 << 22);
        }
        value_sf.clamp(min, 0x7fff)
    }

    /// `Lm_C1..Lm_C3`
    fn lm_c(&mut self, i: usize, v: i32) -> i32 {
        self.lim(v, 0xff, 0, 1 << (21 - i))
    }

    /// `Lm_D`
    fn lm_d(&mut self, a: i64, sf: i32) -> i32 {
        self.lim(shift(a, sf) as i32, 0xffff, 0, F31 | (1 << 18))
    }

    /// `Lm_E`
    fn lm_e(&mut self, result: u32) -> u32 {
        if result == 0xffff_ffff {
            self.flag(F31 | (1 << 17));
            0x1ffff
        } else {
            result.min(0x1ffff)
        }
    }

    /// `F`
    fn f(&mut self, a: i64) -> i64 {
        self.mac0 = a;
        if a > 0x7fff_ffff {
            self.flag(F31 | (1 << 16));
        }
        if a < -0x8000_0000 {
            self.flag(F31 | (1 << 15));
        }
        a
    }

    /// `Lm_G1` / `Lm_G2`
    fn lm_g(&mut self, i: usize, a: i64) -> i32 {
        if a > 0x3ff {
            self.flag(F31 | (1 << (14 - i)));
            0x3ff
        } else if a < -0x400 {
            self.flag(F31 | (1 << (14 - i)));
            -0x400
        } else {
            a as i32
        }
    }

    /// `Lm_H`
    fn lm_h(&mut self, value: i64, sf: i32) -> i32 {
        let value_sf = shift(value, sf);
        let value_12 = shift(value, 1) as i32;
        if !(0..=0x1000).contains(&value_sf) {
            self.flag(1 << 12);
        }
        value_12.clamp(0, 0x1000)
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
        self.d[20] = self.d[21];
        self.d[21] = self.d[22];
        let code = byte(self.d[6], 3);
        set_byte(&mut self.d[22], 3, code);
        for i in 0..3 {
            let m = self.mac(1 + i) >> 4;
            let v = self.lm_c(i, m);
            set_byte(&mut self.d[22], i as u32, v);
        }
    }

    /// `MAC = L * v`, `IR = limit(MAC)`
    fn light_vector(&mut self, v: [i32; 3], lm: i32) {
        for i in 0..3 {
            let sum: i64 = (0..3).map(|j| (self.mat(8, i * 3 + j) * v[j]) as i64).sum();
            let m = self.a(i, sum);
            self.set_mac(1 + i, m);
        }
        self.ir_from_mac(lm);
    }

    /// `MAC = BK<<12 + LC * IR`, `IR = limit(MAC)`
    fn light_color(&mut self, lm: i32) {
        for i in 0..3 {
            let sum: i64 = (0..3)
                .map(|j| (self.mat(16, i * 3 + j) * self.ir(1 + j)) as i64)
                .sum();
            let m = self.a(i, ((self.bk(i) as i64) << 12) + sum);
            self.set_mac(1 + i, m);
        }
        self.ir_from_mac(lm);
    }

    /// `MAC = base + IR0 * limit(FC<<12 - base)`: interpolation towards the far colour.
    fn far_color_blend(&mut self, i: usize, base: i32) {
        let inner = self.a(i, ((self.fc(i) as i64) << 12) - base as i64);
        let blended = self.lm_b(i, inner, 0);
        let m = self.a(i, base.wrapping_add(self.ir(0).wrapping_mul(blended)) as i64);
        self.set_mac(1 + i, m);
    }

    /// `MAC = (rgb << 4) * IR` per channel, optionally blended towards the far colour.
    fn color_times_ir(&mut self, lm: i32, blend: bool) {
        for i in 0..3 {
            let base = (self.rgb_byte(i) << 4).wrapping_mul(self.ir(1 + i));
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
            let sum: i64 = (0..3).map(|j| (self.mat(0, i * 3 + j) * v[j]) as i64).sum();
            let m = self.a(i, ((self.tr(i) as i64) << 12) + sum);
            self.set_mac(1 + i, m);
        }
        let ir1 = self.lm_b(0, self.mac(1), lm);
        self.set_ir(1, ir1);
        let ir2 = self.lm_b(1, self.mac(2), lm);
        self.set_ir(2, ir2);
        let ir3 = self.lm_b3_sf(self.mac3, lm);
        self.set_ir(3, ir3);

        self.d[16] = (self.d[16] & 0xffff_0000) | self.d[17] & 0xffff;
        self.d[17] = (self.d[17] & 0xffff_0000) | self.d[18] & 0xffff;
        self.d[18] = (self.d[18] & 0xffff_0000) | self.d[19] & 0xffff;
        let sz3 = self.lm_d(self.mac3, 1);
        set_lo(&mut self.d[19], sz3);

        let h = lo_s(self.c[26]) as u16;
        let h_over_sz3 = self.lm_e(divide(h, lo_u(self.d[19]) as u16)) as i32;
        self.d[12] = self.d[13];
        self.d[13] = self.d[14];
        let ofx = self.c[24] as i32 as i64;
        let ofy = self.c[25] as i32 as i64;
        let fx = self.f(ofx + self.ir(1) as i64 * h_over_sz3 as i64) >> 16;
        let sx = self.lm_g(0, fx);
        set_lo(&mut self.d[14], sx);
        let fy = self.f(ofy + self.ir(2) as i64 * h_over_sz3 as i64) >> 16;
        let sy = self.lm_g(1, fy);
        set_hi(&mut self.d[14], sy);
        h_over_sz3
    }

    fn depth_cue(&mut self, h_over_sz3: i32) {
        let dqa = lo_s(self.c[27]) as i64;
        let dqb = self.c[28] as i32 as i64;
        let m = self.f(dqb + dqa * h_over_sz3 as i64) as i32;
        self.set_mac(0, m);
        let ir0 = self.lm_h(self.mac0, 1);
        self.set_ir(0, ir0);
    }

    // ---- COP2 command -----------------------------------------------------

    /// Execute a COP2 command word. Returns `false` for unimplemented
    /// function codes (the flag register is still cleared, as in the reference).
    pub fn cop2(&mut self, op: u32) -> bool {
        let op = op & 0x1ff_ffff;
        let lm = ((op >> 10) & 1) as i32;
        self.sf = ((op >> 19) & 1) as i32;
        self.c[31] = 0;

        match op & 63 {
            0x00 | 0x01 => {
                let h = self.rot_trans_pers(0, lm);
                self.depth_cue(h);
            }
            // NCLIP
            0x06 => {
                let sx = |n: usize| lo_s(self.d[12 + n]) as i64;
                let sy = |n: usize| hi_s(self.d[12 + n]) as i64;
                let v = sx(0) * sy(1) + sx(1) * sy(2) + sx(2) * sy(0)
                    - sx(0) * sy(2)
                    - sx(1) * sy(0)
                    - sx(2) * sy(1);
                let m = self.f(v) as i32;
                self.set_mac(0, m);
                self.c[31] = 0;
            }
            // OP
            0x0c => {
                let (d1, d2, d3) = (self.mat(0, 0) as i64, self.mat(0, 4) as i64, self.mat(0, 8) as i64);
                let ir = [self.ir(1) as i64, self.ir(2) as i64, self.ir(3) as i64];
                let m1 = self.a(0, d2 * ir[2] - d3 * ir[1]);
                self.set_mac(1, m1);
                let m2 = self.a(1, d3 * ir[0] - d1 * ir[2]);
                self.set_mac(2, m2);
                let m3 = self.a(2, d1 * ir[1] - d2 * ir[0]);
                self.set_mac(3, m3);
                self.ir_from_mac(lm);
            }
            // DPCS
            0x10 => {
                for i in 0..3 {
                    self.far_color_blend(i, self.rgb_byte(i) << 16);
                }
                self.ir_from_mac(lm);
                self.push_color();
            }
            // INTPL
            0x11 => {
                for i in 0..3 {
                    self.far_color_blend(i, self.ir(1 + i) << 12);
                }
                self.ir_from_mac(lm);
                self.push_color();
            }
            // MVMVA
            0x12 => {
                let mx = ((op >> 17) & 3) as usize;
                let v = ((op >> 15) & 3) as usize;
                let cv = ((op >> 13) & 3) as usize;
                let vec = self.vec(v);
                // Matrix element (row, col); mx == 3 is the reference's "garbage matrix".
                let m = |g: &Self, k: usize| -> i32 {
                    if mx < 3 {
                        g.mat(mx * 8, k)
                    } else {
                        let r = byte(g.d[6], 0);
                        match k {
                            0 => (-r) << 4,
                            1 => r << 4,
                            2 => g.ir(0),
                            3..=5 => lo_s(g.c[1]),
                            _ => lo_s(g.c[2]),
                        }
                    }
                };
                let t = |g: &Self, i: usize| -> i64 {
                    if cv < 3 { g.c[cv * 8 + 5 + i] as i32 as i64 } else { 0 }
                };
                if cv == 2 {
                    for i in 0..3 {
                        let rest = (m(self, i * 3 + 1) * vec[1]) as i64 + (m(self, i * 3 + 2) * vec[2]) as i64;
                        let r = self.a(i, rest);
                        self.set_mac(1 + i, r);
                    }
                    // Discarded values; only the flag and `mac3` side effects remain.
                    for i in 0..3 {
                        let first = t(self, i) * 4096 + (m(self, i * 3) * vec[0]) as i64;
                        let a = self.a(i, first);
                        self.lm_b(i, a, 0);
                    }
                } else {
                    for i in 0..3 {
                        let sum: i64 = (0..3).map(|j| (m(self, i * 3 + j) * vec[j]) as i64).sum();
                        let r = self.a(i, t(self, i) * 4096 + sum);
                        self.set_mac(1 + i, r);
                    }
                }
                self.ir_from_mac(lm);
            }
            // NCDS
            0x13 => {
                self.light_vector(self.vec(0), lm);
                self.light_color(lm);
                self.color_times_ir(lm, true);
                self.push_color();
            }
            // CDP
            0x14 => {
                self.light_color(lm);
                self.color_times_ir(lm, true);
                self.push_color();
            }
            // NCDT
            0x16 => {
                for v in 0..3 {
                    self.light_vector(self.vec(v), lm);
                    self.light_color(lm);
                    self.color_times_ir(lm, true);
                    self.push_color();
                }
            }
            // NCCS
            0x1b => {
                self.light_vector(self.vec(0), lm);
                self.light_color(lm);
                self.color_times_ir(lm, false);
                self.push_color();
            }
            // CC
            0x1c => {
                self.light_color(lm);
                self.color_times_ir(lm, false);
                self.push_color();
            }
            // NCS
            0x1e => {
                self.light_vector(self.vec(0), lm);
                self.light_color(lm);
                self.push_color();
            }
            // NCT
            0x20 => {
                for v in 0..3 {
                    self.light_vector(self.vec(v), lm);
                    self.light_color(lm);
                    self.push_color();
                }
            }
            // SQR
            0x28 => {
                for i in 0..3 {
                    let ir = self.ir(1 + i);
                    let m = self.a(i, (ir * ir) as i64);
                    self.set_mac(1 + i, m);
                }
                self.ir_from_mac(lm);
            }
            // DCPL
            0x29 => {
                self.color_times_ir(lm, true);
                self.push_color();
            }
            // DPCT
            0x2a => {
                for _ in 0..3 {
                    for i in 0..3 {
                        self.far_color_blend(i, byte(self.d[20], i as u32) << 16);
                    }
                    self.ir_from_mac(lm);
                    self.push_color();
                }
            }
            // AVSZ3
            0x2d => {
                let zsf = lo_s(self.c[29]);
                let v = (zsf * self.sz(1)) as i64 + (zsf * self.sz(2)) as i64 + (zsf * self.sz(3)) as i64;
                let m = self.f(v) as i32;
                self.set_mac(0, m);
                let otz = self.lm_d(self.mac0, 1);
                set_lo(&mut self.d[7], otz);
            }
            // AVSZ4
            0x2e => {
                let zsf = lo_s(self.c[30]);
                let v = (zsf * self.sz(0)) as i64
                    + (zsf * self.sz(1)) as i64
                    + (zsf * self.sz(2)) as i64
                    + (zsf * self.sz(3)) as i64;
                let m = self.f(v) as i32;
                self.set_mac(0, m);
                let otz = self.lm_d(self.mac0, 1);
                set_lo(&mut self.d[7], otz);
            }
            // RTPT
            0x30 => {
                let mut h = 0;
                for v in 0..3 {
                    h = self.rot_trans_pers(v, lm);
                }
                self.depth_cue(h);
            }
            // GPF
            0x3d => {
                for i in 0..3 {
                    let m = self.a(i, (self.ir(0) * self.ir(1 + i)) as i64);
                    self.set_mac(1 + i, m);
                }
                self.ir_from_mac(lm);
                self.push_color();
            }
            // GPL
            0x3e => {
                for i in 0..3 {
                    let v = shift(self.mac(1 + i) as i64, -self.sf) + (self.ir(0) * self.ir(1 + i)) as i64;
                    let m = self.a(i, v);
                    self.set_mac(1 + i, m);
                }
                self.ir_from_mac(lm);
                self.push_color();
            }
            // NCCT
            0x3f => {
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

/// `LIM` with no flag side effect (used by the IRGB/ORGB read-back).
fn self_lim_nf(value: i32, max: i32, min: i32) -> i32 {
    value.clamp(min, max)
}
