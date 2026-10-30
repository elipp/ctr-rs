//! The PS1's sound processor, the SPU, as `platform/native_audio.c` plays it (after psx-spx): 24
//! voices, each of which decodes its sample's ADPCM a block at a time as it plays, steps through
//! it by its pitch with the Gaussian interpolation, and is shaped by an ADSR envelope, mixed at
//! 44.1 kHz.
//!
//! There is no sound RAM: a voice's start address is the number of a sample of `KART.HWL`, and
//! key on starts it at the sample's first block, where the loop goes back to as well until a block
//! marks the loop's start (the SPU keeps the repeat address of whatever played before). A voice
//! whose reverb bit is set is sent to the [`Reverb`] as well, at its volume. So is the CD's input
//! (the XA, [`Cd`]) when its reverb is on.

use loader::howl::{BLOCK_SIZE, SAMPLES_PER_BLOCK, block_flags, decode_block};

use crate::reverb::Reverb;
use crate::tables::GAUSS;
use crate::xa::Cd;

pub const VOICES: usize = 24;
/// The output rate, samples a second; a pitch of 0x1000 plays a sample at this rate.
pub const RATE: u32 = 44_100;

/// Where a voice is in its block, in 4096ths of a sample; its pitch, what it adds each output
/// sample, goes up to four samples.
const PITCH_SHIFT: u32 = 12;
const PITCH_MAX: u32 = 0x4000;
const BLOCK_SPAN: u32 = (SAMPLES_PER_BLOCK as u32) << PITCH_SHIFT;

/// The volume registers are in 32768ths (`NATIVE_AUDIO_DIRECT_VOL_MAX`), a voice's and then the
/// master's.
const UNIT_VOLUME: i64 = 0x8000;

/// An envelope at its loudest.
const LEVEL_MAX: i32 = 0x7fff;
/// An envelope steps when its counter reaches this.
const STEP_BIT: u32 = 0x8000;

/// The ADPCM of each sample, by number; `None` for the ones not loaded.
pub type Samples = Vec<Option<Box<[u8]>>>;

/// The phase of an envelope.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Phase {
    /// Silent: the voice does not play.
    #[default]
    Off,
    Attack,
    Decay,
    Sustain,
    Release,
}

/// The ADSR envelope of a voice. Its rates are the SPU's two registers (`adsr1`, `adsr2`):
///
/// ```text
/// ad  15     attack exponential       sr  15     sustain exponential
///     14-10  attack shift                 14     sustain decreasing
///     9-8    attack step                  12-8   sustain shift
///     7-4    decay shift                  7-6    sustain step
///     3-0    sustain level                5      release exponential
///                                         4-0    release shift
/// ```
#[derive(Clone, Copy, Debug, Default)]
struct Envelope {
    phase: Phase,
    /// 0 to 0x7fff.
    level: i32,
    counter: u32,
}

impl Envelope {
    fn start(&mut self) {
        *self = Envelope {
            phase: Phase::Attack,
            ..Envelope::default()
        };
    }

    fn release(&mut self) {
        match self.phase {
            Phase::Off => *self = Envelope::default(),
            _ => self.enter(Phase::Release),
        }
    }

    fn enter(&mut self, phase: Phase) {
        self.phase = phase;
        self.counter = 0;
    }

    /// `NativeAudio_ApplyAdsrEnvelope`.
    fn apply(&self, sample: i32) -> i32 {
        match self.level {
            0 => 0,
            LEVEL_MAX.. => sample,
            level => clamp16(sample * level / LEVEL_MAX),
        }
    }

    /// `NativeAudio_AdsrAdvance`: one output sample of the envelope.
    fn advance(&mut self, [ad, sr]: [u16; 2]) {
        match self.phase {
            Phase::Attack => {
                let rate = (ad >> 8) & 0x7f;
                self.run(rate >> 2, rate & 3, ad & 0x8000 != 0, false, rate == 0x7f);
                if self.level >= LEVEL_MAX {
                    self.level = LEVEL_MAX;
                    self.enter(Phase::Decay);
                }
            }
            Phase::Decay => {
                let target = (((ad & 0xf) as i32 + 1) * 0x800).min(LEVEL_MAX);
                if self.level > target {
                    self.run((ad >> 4) & 0xf, 0, true, true, false);
                }
                if self.level <= target {
                    self.level = target;
                    self.enter(Phase::Sustain);
                }
            }
            Phase::Sustain => {
                let rate = (sr >> 6) & 0x7f;
                let (exponential, decreasing) = (sr & 0x8000 != 0, sr & 0x4000 != 0);
                self.run(rate >> 2, rate & 3, exponential, decreasing, rate == 0x7f);
            }
            Phase::Release => {
                if self.level > 0 {
                    let shift = sr & 0x1f;
                    self.run(shift, 0, sr & 0x20 != 0, true, shift == 0x1f);
                }
                if self.level <= 0 {
                    *self = Envelope::default();
                }
            }
            Phase::Off => {}
        }
    }

    /// `NativeAudio_AdsrRunEnvelopeStep`: a step of `step` ("+7, +6, +5, +4" or "-8, -7, -6,
    /// -5") every `1 << (shift - 11)` samples, or a larger step every sample for a smaller shift.
    /// Exponential, an increase slows down above 0x6000 and a decrease is in proportion to the
    /// level. The fastest rate of all (`never`) does not move.
    fn run(&mut self, shift: u16, step: u16, exponential: bool, decreasing: bool, never: bool) {
        if never {
            return;
        }
        let shift = shift as u32;
        let mut step = 7 - step as i32;
        if decreasing {
            step = !step;
        }
        if shift < 11 {
            step <<= 11 - shift;
        }
        let mut increment = STEP_BIT >> shift.saturating_sub(11);
        if exponential && !decreasing && self.level > 0x6000 {
            match shift {
                ..10 => step >>= 2,
                10 => {
                    step >>= 1;
                    increment >>= 1;
                }
                _ => increment >>= 2,
            }
        } else if exponential && decreasing {
            step = (step as i64 * self.level as i64 / 0x8000) as i32;
            if self.phase == Phase::Release && step == 0 && self.level > 0 {
                step = -1;
            }
        }
        let counter = self.counter + increment.max(1);
        self.counter = counter & (STEP_BIT - 1);
        if counter & STEP_BIT == 0 {
            return;
        }
        self.level += step;
        self.level = if decreasing {
            self.level.max(0)
        } else {
            self.level.clamp(-0x8000, LEVEL_MAX)
        };
    }
}

/// A voice: its registers (`SpuVoiceAttr`), and where it is in its sample.
#[derive(Clone, Copy, Debug)]
struct Voice {
    /// The sample key on starts (the start address).
    start: u16,
    /// Left, right.
    volume: [i16; 2],
    pitch: u16,
    adsr: [u16; 2],
    reverb: bool,

    /// The sample it plays.
    sample: u16,
    /// The byte its block starts at in the sample, and the one the loop goes back to.
    at: usize,
    repeat: usize,
    /// Where it is in the block, in 4096ths of a sample.
    counter: u32,
    /// The block's samples and flags, and the last three of the block before.
    decoded: [i16; SAMPLES_PER_BLOCK],
    flags: u8,
    before: [i16; 3],
    /// The ADPCM filter's last two samples.
    history: (i32, i32),
    envelope: Envelope,
}

impl Default for Voice {
    fn default() -> Self {
        Voice {
            start: 0,
            volume: [0; 2],
            pitch: 0,
            // What `Voiceline_PoolInit` sets for every voice.
            adsr: [0x80ff, 0x1fc2],
            reverb: false,
            sample: 0,
            at: 0,
            repeat: 0,
            counter: 0,
            decoded: [0; SAMPLES_PER_BLOCK],
            flags: 0,
            before: [0; 3],
            history: (0, 0),
            envelope: Envelope::default(),
        }
    }
}

impl Voice {
    /// Key on: the voice starts its sample from the first block, and its envelope from silence.
    fn key_on(&mut self, samples: &Samples) {
        let Some(Some(adpcm)) = samples.get(self.start as usize) else {
            self.envelope = Envelope::default();
            return;
        };
        self.sample = self.start;
        self.at = 0;
        self.repeat = 0;
        self.counter = 0;
        self.before = [0; 3];
        self.history = (0, 0);
        self.decode(adpcm);
        self.envelope.start();
    }

    /// `NativeAudio_DecodeVoiceBlock`: the block at `at`. Past the end of the sample (which the
    /// SPU would go on into) it is silent and ends the voice.
    fn decode(&mut self, adpcm: &[u8]) {
        let block = adpcm
            .get(self.at..self.at + BLOCK_SIZE)
            .and_then(|block| block.try_into().ok());
        let Some(block) = block else {
            self.decoded = [0; SAMPLES_PER_BLOCK];
            self.flags = block_flags::END;
            return;
        };
        self.flags = decode_block(block, &mut self.history, &mut self.decoded);
        if self.flags & block_flags::LOOP_START != 0 {
            self.repeat = self.at;
        }
    }

    /// `NativeAudio_VoiceStreamAdvanceBlock`: on to the next block, or back to the loop's start
    /// after the last. Returns false when the voice ends: the last block does not repeat.
    fn next_block(&mut self, adpcm: &[u8]) -> bool {
        self.before
            .copy_from_slice(&self.decoded[SAMPLES_PER_BLOCK - 3..]);
        let ended = if self.flags & block_flags::END != 0 {
            self.at = self.repeat;
            self.flags & block_flags::REPEAT == 0
        } else {
            self.at += BLOCK_SIZE;
            false
        };
        self.decode(adpcm);
        !ended
    }

    /// `NativeAudio_InterpolateVoiceSample`: the four samples up to where the voice is, weighed
    /// by how far past the newest but one it is.
    fn interpolate(&self) -> i32 {
        let newest = (self.counter >> PITCH_SHIFT) as usize;
        let g = ((self.counter >> 4) & 0xff) as usize;
        // The three before the block's first come from the block before.
        let at = |back: usize| match (newest + 3).checked_sub(back + 3) {
            Some(i) => self.decoded[i] as i32,
            None => self.before[newest + 3 - back] as i32,
        };
        let weigh = |weight: i16, sample: i32| (weight as i32 * sample) >> 15;
        clamp16(
            weigh(GAUSS[0xff - g], at(3))
                + weigh(GAUSS[0x1ff - g], at(2))
                + weigh(GAUSS[0x100 + g], at(1))
                + weigh(GAUSS[g], at(0)),
        )
    }

    /// The voice's next output sample, before its volume, and on by its pitch. `None` when it
    /// does not play.
    fn play(&mut self, samples: &Samples) -> Option<i32> {
        if self.envelope.phase == Phase::Off {
            return None;
        }
        if self.pitch == 0 {
            self.envelope.advance(self.adsr);
            return None;
        }
        let sample = self.envelope.apply(self.interpolate());
        self.envelope.advance(self.adsr);
        self.counter += (self.pitch as u32).min(PITCH_MAX);
        let adpcm = samples
            .get(self.sample as usize)
            .and_then(Option::as_deref)
            .unwrap_or_default();
        while self.counter >= BLOCK_SPAN {
            self.counter -= BLOCK_SPAN;
            if !self.next_block(adpcm) {
                self.envelope = Envelope::default();
                break;
            }
        }
        Some(sample)
    }
}

/// The SPU: its voices, the samples they play, the reverb, and the master volume.
pub struct Spu {
    voices: [Voice; VOICES],
    samples: Samples,
    pub reverb: Reverb,
    /// The CD's input: the XA.
    pub cd: Cd,
    /// Left, right (`SpuSetCommonMasterVolume`).
    pub master: [i16; 2],
}

impl Spu {
    pub fn new(samples: Samples) -> Spu {
        Spu {
            voices: [Voice::default(); VOICES],
            samples,
            reverb: Reverb::default(),
            cd: Cd::default(),
            // `howl_InitGlobals`.
            master: [0x3fff; 2],
        }
    }

    /// Swaps the ADPCM of sample `sample` for `adpcm`, and gives the old (`None` if there is no
    /// such sample).
    pub fn replace_sample(
        &mut self,
        sample: u16,
        adpcm: Option<Box<[u8]>>,
    ) -> Option<Option<Box<[u8]>>> {
        let slot = self.samples.get_mut(sample as usize)?;
        Some(std::mem::replace(slot, adpcm))
    }

    /// Whether sample `sample` is loaded (`spuAddr != 0`).
    pub fn has(&self, sample: u16) -> bool {
        matches!(self.samples.get(sample as usize), Some(Some(_)))
    }

    /// `SpuSetVoiceStartAddr`: the sample the next key on starts.
    pub fn set_start(&mut self, voice: usize, sample: u16) {
        self.voices[voice].start = sample;
    }

    /// `SpuSetVoiceADSRAttr`, as the two registers ([`Envelope`]).
    pub fn set_adsr(&mut self, voice: usize, ad: u16, sr: u16) {
        self.voices[voice].adsr = [ad, sr];
    }

    /// `SpuSetVoicePitch`: 0x1000 plays the sample at [`RATE`].
    pub fn set_pitch(&mut self, voice: usize, pitch: u16) {
        self.voices[voice].pitch = pitch;
    }

    /// `SpuSetVoiceVolume`: left and right, in 32768ths.
    pub fn set_volume(&mut self, voice: usize, volume: [i16; 2]) {
        self.voices[voice].volume = volume;
    }

    /// `SpuSetReverbVoice`.
    pub fn set_reverb(&mut self, voice: usize, on: bool) {
        self.voices[voice].reverb = on;
    }

    /// `SpuSetKey(1, voices)`: the voices of the bits start their sample.
    pub fn key_on(&mut self, voices: u32) {
        for (i, voice) in self.voices.iter_mut().enumerate() {
            if voices & 1 << i != 0 {
                voice.key_on(&self.samples);
            }
        }
    }

    /// `SpuSetKey(0, voices)`: the voices of the bits let go (their envelope's release).
    pub fn key_off(&mut self, voices: u32) {
        for (i, voice) in self.voices.iter_mut().enumerate() {
            if voices & 1 << i != 0 {
                voice.envelope.release();
            }
        }
    }

    /// Whether voice `voice` plays.
    pub fn playing(&self, voice: usize) -> bool {
        self.voices[voice].envelope.phase != Phase::Off
    }

    /// `NativeAudio_MixFrame`: the next output sample, left and right: the CD's and the voices',
    /// and what of them the reverb sends back.
    pub fn mix(&mut self) -> [i16; 2] {
        let mut mix = [0i32; 2];
        let mut send = [0i32; 2];
        if let Some(frame) = self.cd.next() {
            for side in 0..2 {
                mix[side] += volume(frame[side], self.cd.volume[side], self.master[side]);
                if self.cd.reverb {
                    send[side] += master_volume(frame[side], self.cd.volume[side]);
                }
            }
        }
        for voice in &mut self.voices {
            let Some(sample) = voice.play(&self.samples) else {
                continue;
            };
            for side in 0..2 {
                mix[side] += volume(sample, voice.volume[side], self.master[side]);
                if voice.reverb {
                    send[side] += master_volume(sample, voice.volume[side]);
                }
            }
        }
        let wet = self.reverb.run(send);
        for side in 0..2 {
            mix[side] += master_volume(wet[side], self.master[side]);
        }
        mix.map(|side| clamp16(side) as i16)
    }
}

/// `NativeAudio_ApplyVolume`: `sample` at a voice's volume, and the master's.
fn volume(sample: i32, volume: i16, master: i16) -> i32 {
    let gain = |v: i16| (v as i64).max(0);
    (sample as i64 * gain(volume) * gain(master) / (UNIT_VOLUME * UNIT_VOLUME)) as i32
}

/// `NativeAudio_ApplyMasterVolume`: `sample` at one volume.
fn master_volume(sample: i32, volume: i16) -> i32 {
    (sample as i64 * (volume as i64).max(0) / UNIT_VOLUME) as i32
}

fn clamp16(value: i32) -> i32 {
    value.clamp(i16::MIN as i32, i16::MAX as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A block of 28 samples of `nibble`, at shift 12 (each nibble worth itself) and no filter.
    fn block(flags: u8, nibble: u8) -> [u8; BLOCK_SIZE] {
        let mut b = [nibble | nibble << 4; BLOCK_SIZE];
        b[0] = 12;
        b[1] = flags;
        b
    }

    fn spu(blocks: &[[u8; BLOCK_SIZE]]) -> Spu {
        let mut spu = Spu::new(vec![Some(blocks.concat().into_boxed_slice())]);
        spu.master = [0x7fff; 2];
        spu.set_volume(0, [0x7fff; 2]);
        spu.set_pitch(0, 0x1000);
        spu
    }

    #[test]
    fn the_default_envelope_is_full_at_once_and_lets_go_in_eight_samples() {
        let mut envelope = Envelope::default();
        envelope.start();
        let adsr = [0x80ff, 0x1fc2];
        let mut levels = vec![];
        for _ in 0..5 {
            envelope.advance(adsr);
            levels.push(envelope.level);
        }
        // +0x3800 a sample, a quarter of it above 0x6000.
        assert_eq!(levels, [0x3800, 0x7000, 0x7e00, LEVEL_MAX, LEVEL_MAX]);
        assert_eq!(envelope.phase, Phase::Sustain);
        envelope.release();
        let steps = std::iter::from_fn(|| {
            envelope.advance(adsr);
            (envelope.phase != Phase::Off).then_some(envelope.level)
        })
        .count();
        // -0x1000 a sample from 0x7fff.
        assert_eq!(steps, 7);
    }

    #[test]
    fn a_slow_linear_attack_steps_every_few_samples() {
        let mut envelope = Envelope::default();
        envelope.start();
        // Shift 13 (a step every 4 samples), step 0 (+7).
        let adsr = [(13 << 10) | 0xff, 0x1fc2];
        let levels: Vec<i32> = (0..8)
            .map(|_| {
                envelope.advance(adsr);
                envelope.level
            })
            .collect();
        assert_eq!(levels, [0, 0, 0, 7, 7, 7, 7, 14]);
    }

    #[test]
    fn a_one_shot_plays_its_blocks_and_ends() {
        let mut spu = spu(&[block(0, 1), block(block_flags::END, 2)]);
        spu.key_on(1);
        let out: Vec<i16> = std::iter::from_fn(|| {
            let [left, _] = spu.mix();
            spu.playing(0).then_some(left)
        })
        .collect();
        // Two blocks of 28 samples at the pitch of the rate, then the end.
        assert_eq!(out.len(), 2 * SAMPLES_PER_BLOCK - 1);
        assert!(!spu.playing(0));
    }

    #[test]
    fn a_loop_goes_back_to_its_start_and_key_off_lets_it_go() {
        let blocks = [
            block(0, 1),
            block(block_flags::LOOP_START, 2),
            block(block_flags::END | block_flags::REPEAT, 3),
        ];
        let mut spu = spu(&blocks);
        spu.key_on(1);
        for _ in 0..10 * SAMPLES_PER_BLOCK {
            spu.mix();
        }
        assert!(spu.playing(0));
        assert_eq!(spu.voices[0].repeat, BLOCK_SIZE);
        assert!(spu.voices[0].at >= BLOCK_SIZE);
        spu.key_off(1);
        for _ in 0..8 {
            spu.mix();
        }
        assert!(!spu.playing(0));
    }

    #[test]
    fn the_interpolation_of_a_steady_sample_is_nearly_the_sample() {
        let mut voice = Voice {
            decoded: [10_000; SAMPLES_PER_BLOCK],
            before: [10_000; 3],
            ..Voice::default()
        };
        for counter in (0..BLOCK_SPAN).step_by(0x123) {
            voice.counter = counter;
            // The four weights add up to a little under 1.
            let sample = voice.interpolate();
            assert!((9_900..=10_000).contains(&sample), "{counter:#x}: {sample}");
        }
        // It lags: nearly at sample 4, sample 3 counts for most of it.
        voice.decoded[3] = 0;
        voice.counter = (4 << PITCH_SHIFT) | 0xff0;
        assert!(voice.interpolate() < 4_000);
    }

    #[test]
    fn the_volumes_are_32768ths() {
        assert_eq!(volume(0x4000, 0x4000, 0x4000), 0x1000);
        assert_eq!(volume(0x4000, -1, 0x4000), 0);
    }
}
