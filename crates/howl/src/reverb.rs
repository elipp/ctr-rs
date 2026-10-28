//! The SPU's reverb unit, as `platform/native_audio.c` runs it (after psx-spx): the voices whose
//! reverb bit is set send their samples to it, and what it gives back is mixed in with them.
//!
//! It runs at half the output rate, on a work area of samples that it reads from and writes to
//! at offsets from a cursor that goes round it: the reflections off the same side and off the
//! other (each damped as it goes back in), four combs that read them, and two all-pass filters.
//! A filter of 39 taps halves the rate on the way in and doubles it on the way out. Each mode
//! ([`Mode`]) is the size of the work area and the 32 registers that the reverb runs by.
//!
//! Off ([`Reverb::on`]), it no longer writes its work area, but what is in it is still heard.

/// The modes of `libspu` (`SPU_REV_MODE_*`), in its order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Off,
    Room,
    StudioA,
    StudioB,
    StudioC,
    Hall,
    Space,
    Echo,
    Delay,
    Pipe,
}

impl Mode {
    const ALL: [Mode; 10] = [
        Mode::Off,
        Mode::Room,
        Mode::StudioA,
        Mode::StudioB,
        Mode::StudioC,
        Mode::Hall,
        Mode::Space,
        Mode::Echo,
        Mode::Delay,
        Mode::Pipe,
    ];

    /// `NativeAudio_ReverbModeFromRaw`: the mode of `SpuReverbAttr.mode` (its low byte), off for
    /// any number that is not one.
    pub fn from_raw(raw: u16) -> Mode {
        Mode::ALL
            .get((raw & 0xff) as usize)
            .copied()
            .unwrap_or_default()
    }
}

/// `SPU_REV_MODE_CLEAR_WA`: with the mode, the work area is cleared even if it stays the same.
pub const CLEAR_WORK_AREA: u16 = 0x100;

/// The registers of a mode (`NativeAudioReverbRegister`), in the SPU's order: the offsets in the
/// work area (`d`, `m`) are in 8 bytes, 4 samples; the volumes (`v`) are in 32768ths, and signed.
mod reg {
    pub const DAPF1: usize = 0;
    pub const DAPF2: usize = 1;
    pub const VIIR: usize = 2;
    pub const VCOMB1: usize = 3;
    pub const VCOMB2: usize = 4;
    pub const VCOMB3: usize = 5;
    pub const VCOMB4: usize = 6;
    pub const VWALL: usize = 7;
    pub const VAPF1: usize = 8;
    pub const VAPF2: usize = 9;
    pub const MLSAME: usize = 10;
    pub const MRSAME: usize = 11;
    pub const MLCOMB1: usize = 12;
    pub const MRCOMB1: usize = 13;
    pub const DLSAME: usize = 16;
    pub const DRSAME: usize = 17;
    pub const MLDIFF: usize = 18;
    pub const MRDIFF: usize = 19;
    pub const DLDIFF: usize = 24;
    pub const DRDIFF: usize = 25;
    pub const MLAPF1: usize = 26;
    pub const MRAPF1: usize = 27;
    pub const MLAPF2: usize = 28;
    pub const MRAPF2: usize = 29;
    pub const VLIN: usize = 30;
    pub const VRIN: usize = 31;
    pub const COUNT: usize = 32;
    /// From a comb's first register, its second, third and fourth: they alternate left and right
    /// (`mLCOMB1`, `mRCOMB1`, `mLCOMB2`, ...), with the reflections' between the second and
    /// third.
    pub const COMBS: [usize; 4] = [0, 2, 8, 10];
}

/// `NativeAudioReverbPreset`: the size of a mode's work area, in bytes, and its registers.
struct Preset {
    bytes: usize,
    reg: [u16; reg::COUNT],
}

/// `s_reverbPresets`, by [`Mode`].
#[rustfmt::skip]
const PRESETS: [Preset; 10] = [
    Preset { bytes: 0x10, reg: [
        0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0001, 0x0001,
        0x0001, 0x0001, 0x0001, 0x0001, 0x0000, 0x0000, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001, 0x0001,
        0x0000, 0x0000, 0x0001, 0x0001, 0x0001, 0x0001, 0x0000, 0x0000,
    ] },
    Preset { bytes: 0x26C0, reg: [
        0x007D, 0x005B, 0x6D80, 0x54B8, 0xBED0, 0x0000, 0x0000, 0xBA80, 0x5800, 0x5300, 0x04D6, 0x0333,
        0x03F0, 0x0227, 0x0374, 0x01EF, 0x0334, 0x01B5, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000,
        0x0000, 0x0000, 0x01B4, 0x0136, 0x00B8, 0x005C, 0x8000, 0x8000,
    ] },
    Preset { bytes: 0x1F40, reg: [
        0x0033, 0x0025, 0x70F0, 0x4FA8, 0xBCE0, 0x4410, 0xC0F0, 0x9C00, 0x5280, 0x4EC0, 0x03E4, 0x031B,
        0x03A4, 0x02AF, 0x0372, 0x0266, 0x031C, 0x025D, 0x025C, 0x018E, 0x022F, 0x0135, 0x01D2, 0x00B7,
        0x018F, 0x00B5, 0x00B4, 0x0080, 0x004C, 0x0026, 0x8000, 0x8000,
    ] },
    Preset { bytes: 0x4840, reg: [
        0x00B1, 0x007F, 0x70F0, 0x4FA8, 0xBCE0, 0x4510, 0xBEF0, 0xB4C0, 0x5280, 0x4EC0, 0x0904, 0x076B,
        0x0824, 0x065F, 0x07A2, 0x0616, 0x076C, 0x05ED, 0x05EC, 0x042E, 0x050F, 0x0305, 0x0462, 0x02B7,
        0x042F, 0x0265, 0x0264, 0x01B2, 0x0100, 0x0080, 0x8000, 0x8000,
    ] },
    Preset { bytes: 0x6FE0, reg: [
        0x00E3, 0x00A9, 0x6F60, 0x4FA8, 0xBCE0, 0x4510, 0xBEF0, 0xA680, 0x5680, 0x52C0, 0x0DFB, 0x0B58,
        0x0D09, 0x0A3C, 0x0BD9, 0x0973, 0x0B59, 0x08DA, 0x08D9, 0x05E9, 0x07EC, 0x04B0, 0x06EF, 0x03D2,
        0x05EA, 0x031D, 0x031C, 0x0238, 0x0154, 0x00AA, 0x8000, 0x8000,
    ] },
    Preset { bytes: 0xADE0, reg: [
        0x01A5, 0x0139, 0x6000, 0x5000, 0x4C00, 0xB800, 0xBC00, 0xC000, 0x6000, 0x5C00, 0x15BA, 0x11BB,
        0x14C2, 0x10BD, 0x11BC, 0x0DC1, 0x11C0, 0x0DC3, 0x0DC0, 0x09C1, 0x0BC4, 0x07C1, 0x0A00, 0x06CD,
        0x09C2, 0x05C1, 0x05C0, 0x041A, 0x0274, 0x013A, 0x8000, 0x8000,
    ] },
    Preset { bytes: 0xF6C0, reg: [
        0x033D, 0x0231, 0x7E00, 0x5000, 0xB400, 0xB000, 0x4C00, 0xB000, 0x6000, 0x5400, 0x1ED6, 0x1A31,
        0x1D14, 0x183B, 0x1BC2, 0x16B2, 0x1A32, 0x15EF, 0x15EE, 0x1055, 0x1334, 0x0F2D, 0x11F6, 0x0C5D,
        0x1056, 0x0AE1, 0x0AE0, 0x07A2, 0x0464, 0x0232, 0x8000, 0x8000,
    ] },
    Preset { bytes: 0x18040, reg: [
        0x0001, 0x0001, 0x7FFF, 0x7FFF, 0x0000, 0x0000, 0x0000, 0x8100, 0x0000, 0x0000, 0x1FFF, 0x0FFF,
        0x1005, 0x0005, 0x0000, 0x0000, 0x1005, 0x0005, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000,
        0x0000, 0x0000, 0x1004, 0x1002, 0x0004, 0x0002, 0x8000, 0x8000,
    ] },
    Preset { bytes: 0x18040, reg: [
        0x0001, 0x0001, 0x7FFF, 0x7FFF, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x1FFF, 0x0FFF,
        0x1005, 0x0005, 0x0000, 0x0000, 0x1005, 0x0005, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000,
        0x0000, 0x0000, 0x1004, 0x1002, 0x0004, 0x0002, 0x8000, 0x8000,
    ] },
    Preset { bytes: 0x3C00, reg: [
        0x0017, 0x0013, 0x70F0, 0x4FA8, 0xBCE0, 0x4510, 0xBEF0, 0x8500, 0x5F80, 0x54C0, 0x0371, 0x02AF,
        0x02E5, 0x01DF, 0x02B0, 0x01D7, 0x0358, 0x026A, 0x01D6, 0x011E, 0x012D, 0x00B1, 0x011F, 0x0059,
        0x01A0, 0x00E3, 0x0058, 0x0040, 0x0028, 0x0014, 0x8000, 0x8000,
    ] },
];

/// The largest work area, in samples (`NATIVE_AUDIO_REVERB_MAX_SAMPLES`): the echo's and the
/// delay's.
const MAX_SAMPLES: usize = 0x18040 / 2;

/// `s_reverbFirCoeffs`: the filter that halves the rate going in and doubles it coming out, the
/// oldest sample's weight first, in 32768ths.
const TAPS: usize = 39;
#[rustfmt::skip]
const FIR: [i16; TAPS] = [
    -0x0001, 0x0000, 0x0002, 0x0000, -0x000A, 0x0000, 0x0023, 0x0000, -0x0067, 0x0000, 0x010A,
    0x0000, -0x0268, 0x0000, 0x0534, 0x0000, -0x0B90, 0x0000, 0x2806, 0x4000, 0x2806, 0x0000,
    -0x0B90, 0x0000, 0x0534, 0x0000, -0x0268, 0x0000, 0x010A, 0x0000, -0x0067, 0x0000, 0x0023,
    0x0000, -0x000A, 0x0000, 0x0002, 0x0000, -0x0001,
];

/// The last [`TAPS`] samples of the two sides, and where the next goes (the oldest).
#[derive(Clone, Copy)]
struct History {
    sides: [[i16; TAPS]; 2],
    at: usize,
}

impl History {
    const EMPTY: History = History {
        sides: [[0; TAPS]; 2],
        at: 0,
    };

    fn push(&mut self, sample: [i32; 2]) {
        for (side, sample) in self.sides.iter_mut().zip(sample) {
            side[self.at] = clamp16(sample) as i16;
        }
        self.at = (self.at + 1) % TAPS;
    }

    /// `NativeAudio_ReverbFirApply`.
    fn filter(&self, side: usize) -> i32 {
        let sum: i64 = (0..TAPS)
            .map(|i| FIR[i] as i64 * self.sides[side][(self.at + i) % TAPS] as i64)
            .sum();
        clamp16((sum / 0x8000) as i32)
    }
}

/// `NativeAudioReverbState`, and the reverb's own registers.
pub struct Reverb {
    /// `SpuSetReverb`: whether it writes its work area.
    pub on: bool,
    /// `SpuSetReverbModeDepth`: how much of it is heard, left and right, in 32768ths.
    pub depth: [i16; 2],
    mode: Mode,
    /// The work area, of which the mode's size is used; none at all before a mode is set.
    area: Box<[i16]>,
    size: usize,
    cursor: usize,
    input: History,
    output: History,
    /// Whether the output sample is the first of a pair, which the reverb does not run on.
    first: bool,
}

impl Default for Reverb {
    fn default() -> Self {
        Reverb {
            on: false,
            depth: [0; 2],
            mode: Mode::Off,
            area: vec![0; MAX_SAMPLES].into_boxed_slice(),
            size: 0,
            cursor: 0,
            input: History::EMPTY,
            output: History::EMPTY,
            first: false,
        }
    }
}

impl Reverb {
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// `NativeAudio_ReverbConfigureModeNoLock`: what `SpuSetReverbModeParam` does with a mode. A
    /// work area of another mode or size, or with [`CLEAR_WORK_AREA`], starts silent.
    pub fn set_mode(&mut self, raw: u16) {
        let mode = Mode::from_raw(raw);
        let size = PRESETS[mode as usize].bytes / 2;
        let changed = mode != self.mode || size != self.size;
        self.mode = mode;
        self.size = size;
        if raw & CLEAR_WORK_AREA != 0 || changed {
            self.clear();
        }
    }

    /// `NativeAudio_ReverbClearBufferNoLock`.
    fn clear(&mut self) {
        self.area.fill(0);
        self.cursor = 0;
        self.input = History::EMPTY;
        self.output = History::EMPTY;
        self.first = false;
    }

    fn reg(&self, reg: usize) -> u16 {
        PRESETS[self.mode as usize].reg[reg]
    }

    fn volume(&self, reg: usize) -> i16 {
        self.reg(reg) as i16
    }

    /// `NativeAudio_ReverbOffsetSamples`.
    fn offset(&self, reg: usize) -> isize {
        self.reg(reg) as isize * 4
    }

    fn index(&self, offset: isize) -> usize {
        (self.cursor as isize + offset).rem_euclid(self.size as isize) as usize
    }

    /// `NativeAudio_ReverbRead`: at the register's offset and `delta` samples more.
    fn read(&self, reg: usize, delta: isize) -> i32 {
        self.read_at(self.offset(reg) + delta)
    }

    /// `NativeAudio_ReverbReadAtOffset`.
    fn read_at(&self, offset: isize) -> i32 {
        match self.size {
            0 => 0,
            _ => self.area[self.index(offset)] as i32,
        }
    }

    /// `NativeAudio_ReverbWrite`, if it is on.
    fn write(&mut self, reg: usize, value: i32) {
        if self.on && self.size != 0 {
            let at = self.index(self.offset(reg));
            self.area[at] = clamp16(value) as i16;
        }
    }

    /// `NativeAudio_ReverbRunReflectionStage`: the input and what comes back off a wall,
    /// smoothed with what the reflection was a sample before.
    fn reflect(&self, input: i32, from: usize, to: usize) -> i32 {
        let previous = self.read(to, -1);
        let wall = mul(self.read(from, 0), self.volume(reg::VWALL));
        mul(input + wall - previous, self.volume(reg::VIIR)) + previous
    }

    /// `NativeAudio_ReverbRunCombStage`: the four combs of a side, from its first.
    fn combs(&self, first: usize) -> i32 {
        let volumes = [reg::VCOMB1, reg::VCOMB2, reg::VCOMB3, reg::VCOMB4];
        reg::COMBS
            .iter()
            .zip(volumes)
            .map(|(&comb, volume)| mul(self.read(first + comb, 0), self.volume(volume)))
            .sum()
    }

    /// `NativeAudio_ReverbRunApfStage`: an all-pass filter, which keeps what goes through it at
    /// `apf` and reads it back `delta` later.
    fn all_pass(&mut self, input: i32, apf: usize, delta: usize, volume: usize) -> i32 {
        let back = self.offset(apf) - self.offset(delta);
        let volume = self.volume(volume);
        let stored = input - mul(self.read_at(back), volume);
        self.write(apf, stored);
        mul(stored, volume) + self.read_at(back)
    }

    /// `NativeAudio_ReverbProcessNoLock`: what the voices send to the reverb this output sample,
    /// left and right, and what it gives back (before the master volume).
    pub fn run(&mut self, send: [i32; 2]) -> [i32; 2] {
        self.input.push(send);
        self.first = !self.first;
        if self.first || self.size == 0 {
            self.output.push([0; 2]);
        } else {
            let input = [reg::VLIN, reg::VRIN].map(|v| self.volume(v));
            let [lin, rin] = [0, 1].map(|side| mul(self.input.filter(side), input[side]));
            let same_left = self.reflect(lin, reg::DLSAME, reg::MLSAME);
            let same_right = self.reflect(rin, reg::DRSAME, reg::MRSAME);
            let diff_left = self.reflect(lin, reg::DRDIFF, reg::MLDIFF);
            let diff_right = self.reflect(rin, reg::DLDIFF, reg::MRDIFF);
            self.write(reg::MLSAME, same_left);
            self.write(reg::MRSAME, same_right);
            self.write(reg::MLDIFF, diff_left);
            self.write(reg::MRDIFF, diff_right);

            let (left, right) = (self.combs(reg::MLCOMB1), self.combs(reg::MRCOMB1));
            let left = self.all_pass(left, reg::MLAPF1, reg::DAPF1, reg::VAPF1);
            let right = self.all_pass(right, reg::MRAPF1, reg::DAPF1, reg::VAPF1);
            let left = self.all_pass(left, reg::MLAPF2, reg::DAPF2, reg::VAPF2);
            let right = self.all_pass(right, reg::MRAPF2, reg::DAPF2, reg::VAPF2);
            self.output.push([left, right]);
            self.cursor = (self.cursor + 1) % self.size;
        }
        // `NativeAudio_ReverbFirApplyUpsampled`: half the samples are the zeros between.
        [0, 1].map(|side| {
            let wet = clamp16(self.output.filter(side) * 2);
            clamp16(mul(wet, self.depth[side]))
        })
    }
}

/// `NativeAudio_ReverbMul`: `sample` at a signed volume, in 32768ths.
fn mul(sample: i32, volume: i16) -> i32 {
    (sample as i64 * volume as i64 / 0x8000) as i32
}

fn clamp16(value: i32) -> i32 {
    value.clamp(i16::MIN as i32, i16::MAX as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reverb of `mode` at full depth, and a short burst sent to it.
    fn tail(mode: Mode, on: bool, samples: usize) -> Vec<[i32; 2]> {
        let mut reverb = Reverb::default();
        reverb.set_mode(CLEAR_WORK_AREA | mode as u16);
        reverb.on = on;
        reverb.depth = [0x7fff; 2];
        (0..samples)
            .map(|i| reverb.run(if i < 64 { [0x4000, 0] } else { [0; 2] }))
            .collect()
    }

    #[test]
    fn the_modes_are_numbered_as_libspu_numbers_them() {
        assert_eq!(Mode::from_raw(CLEAR_WORK_AREA | 9), Mode::Pipe);
        assert_eq!(Mode::from_raw(1), Mode::Room);
        assert_eq!(Mode::from_raw(10), Mode::Off);
        // Each mode's work area fits in the largest.
        assert!(PRESETS.iter().all(|p| p.bytes / 2 <= MAX_SAMPLES));
    }

    #[test]
    fn a_reverb_before_its_mode_is_set_is_silent() {
        let mut reverb = Reverb {
            on: true,
            depth: [0x7fff; 2],
            ..Reverb::default()
        };
        assert!((0..1000).all(|_| reverb.run([0x4000; 2]) == [0; 2]));
    }

    #[test]
    fn a_studio_echoes_a_burst_on_both_sides_and_dies_away() {
        let out = tail(Mode::StudioA, true, 44_100);
        // What comes back is late: the reflections are a few hundred samples away at the least.
        let first = out.iter().position(|&s| s != [0; 2]).unwrap();
        assert!((64..2_000).contains(&first), "{first}");
        // A burst on the left alone comes back on the right too (as it does not in a room, whose
        // reflections off the other side are all at the cursor).
        assert!(out.iter().any(|&[_, right]| right != 0));
        let loudest = |from: usize, to: usize| {
            out[from..to]
                .iter()
                .flatten()
                .map(|s| s.unsigned_abs())
                .max()
                .unwrap()
        };
        assert!(loudest(0, 4_410) > 100 * loudest(40_000, 44_100).max(1));
    }

    #[test]
    fn a_reverb_that_is_off_keeps_nothing_new() {
        // Nothing goes into its work area: its combs read silence.
        assert!(tail(Mode::Room, false, 10_000).iter().all(|&s| s == [0; 2]));
    }

    #[test]
    fn the_same_mode_again_keeps_its_work_area_unless_it_is_cleared() {
        let mut reverb = Reverb::default();
        reverb.set_mode(Mode::Hall as u16);
        reverb.on = true;
        for _ in 0..1000 {
            reverb.run([0x4000; 2]);
        }
        reverb.set_mode(Mode::Hall as u16);
        assert!(reverb.area.iter().any(|&s| s != 0));
        reverb.set_mode(CLEAR_WORK_AREA | Mode::Hall as u16);
        assert!(reverb.area.iter().all(|&s| s == 0));
    }
}
