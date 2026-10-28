//! The game's sound engine (`game/HOWL`) on the SPU it plays through, as `ctr-native` emulates it
//! (`platform/native_audio.c`). Plain state, like the rest of the port: [`Howl`] is all of it,
//! and its output is samples, which the viewer hands to the speakers.
//!
//! The game starts, changes and stops sounds ([`fx`]); each takes a channel ([`channel`]), whose
//! voice of the [`spu`] plays it. The SPU's vertical blank, 60 a second of its output, runs out
//! the sounds' time and passes what changed on to the voices (`howl_PlayAudio_Update`), so
//! [`Howl::render`] is the clock of the sounds as the console's is.
//!
//! Not ported yet: the voice lines, the music, the banks (every sample a sound effect or an
//! engine plays is loaded), and the volume options.

pub mod channel;
pub mod fx;
pub mod reverb;
pub mod spu;
pub mod tables;

use std::io;
use std::path::Path;

use loader::howl::{EngineFx, OtherFx};

use channel::{Channels, Kind, Stats, update};
pub use fx::{AntiSpam, NO_DISTORTION, Sound, crash_sound};
use spu::{Samples, Spu};

/// The output samples between two vertical blanks: 60 a second.
const FRAMES_PER_VSYNC: u32 = spu::RATE / 60;

/// The pan of the middle: 0 is all left, 255 all right.
pub const CENTRE: u8 = 0x80;

/// How a sound plays (the flags `HowlSfx_Pack` packs): its volume, its distortion (its pitch,
/// [`NO_DISTORTION`] its own), where it is heard (0 all left, [`CENTRE`] the middle, 255 all
/// right), and whether it echoes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sfx {
    pub volume: u8,
    pub distort: u8,
    pub pan: u8,
    pub echo: bool,
}

impl Sfx {
    /// `HOWL_SFX_DEFAULT_FLAGS`: full volume, at its own pitch, in the middle.
    pub const FULL: Sfx = Sfx {
        volume: u8::MAX,
        distort: NO_DISTORTION,
        pan: CENTRE,
        echo: false,
    };
    /// `HOWL_SFX_CENTER_NO_DISTORTION`: [`Sfx::FULL`] at no volume.
    pub const SILENT: Sfx = Sfx {
        volume: 0,
        ..Sfx::FULL
    };
}

/// The sound engine: the sounds of `KART.HWL`, the channels they play on, and the SPU.
pub struct Howl {
    pub(crate) spu: Spu,
    pub(crate) channels: Channels,
    pub(crate) other_fx: Vec<OtherFx>,
    pub(crate) engine_fx: Vec<EngineFx>,
    /// `vol_FX` and `vol_Voice`.
    pub volume_fx: u8,
    pub volume_voice: u8,
    /// `boolStereoEnabled`: in mono both sides play the same.
    pub stereo: bool,
    /// `countSounds`.
    pub(crate) count: u32,
    /// The game's frame (`frameTimer_MainFrame_ResetDB`), which the anti-spam measures by.
    pub frame: u32,
    /// What `howl_PauseAudio` took off the channels (`channelStatsCurr`).
    paused: Vec<Stats>,
    /// The output samples until the next vertical blank.
    until_vsync: u32,
}

impl Howl {
    /// `howl_InitGlobals`.
    pub fn new(other_fx: Vec<OtherFx>, engine_fx: Vec<EngineFx>, samples: Samples) -> Howl {
        Howl {
            spu: Spu::new(samples),
            channels: Channels::default(),
            other_fx,
            engine_fx,
            volume_fx: 215,
            volume_voice: 255,
            stereo: true,
            count: 0,
            frame: 0,
            paused: Vec::new(),
            until_vsync: 0,
        }
    }

    /// Reads `SOUNDS/KART.HWL` from the disc image at `path`: its sound effects and engines, and
    /// the ADPCM of their samples.
    pub fn read(path: &Path) -> io::Result<Howl> {
        let (mut disc, header) = loader::disc::open_howl(path)?;
        let mut samples: Samples = vec![None; header.sample_sizes.len()];
        let used = header.other_fx.iter().map(|fx| fx.sample);
        for sample in used.chain(header.engine_fx.iter().map(|fx| fx.sample)) {
            let Some(slot @ None) = samples.get_mut(sample as usize) else {
                continue;
            };
            *slot = header.adpcm(&mut disc, sample)?.map(Vec::into_boxed_slice);
        }
        Ok(Howl::new(header.other_fx, header.engine_fx, samples))
    }

    /// The sound effects, by number.
    pub fn other_fx(&self) -> &[OtherFx] {
        &self.other_fx
    }

    /// The next output samples, left and right at [`spu::RATE`], with a vertical blank every
    /// sixtieth of a second of them.
    pub fn render(&mut self, frames: &mut [[i16; 2]]) {
        for frame in frames {
            if self.until_vsync == 0 {
                self.vsync();
                self.until_vsync = FRAMES_PER_VSYNC;
            }
            self.until_vsync -= 1;
            *frame = self.spu.mix();
        }
    }

    /// `howl_PlayAudio_Update`: the sounds' time runs, and the voices hear what changed.
    pub fn vsync(&mut self) {
        self.channels.run_timers();
        self.channels.update(&mut self.spu);
    }

    /// `howl_PauseAudio`: every sound stops, and is kept to start again with [`Howl::unpause`].
    pub fn pause(&mut self) {
        if !self.paused.is_empty() {
            return;
        }
        for &channel in &self.channels.taken().to_vec() {
            self.paused.push(*self.channels.stats(channel));
            self.channels.destroy(channel);
        }
    }

    /// `howl_UnPauseAudio`: what [`Howl::pause`] stopped starts again, each sound from its start,
    /// on the free channels.
    pub fn unpause(&mut self) {
        if self.paused.is_empty() {
            return;
        }
        for stats in std::mem::take(&mut self.paused) {
            let echo = |attr| channel::Attr {
                reverb: stats.echo as u16,
                ..attr
            };
            let attr = match stats.kind {
                Kind::OtherFx => match self.other_fx.get(stats.sound as u16 as usize) {
                    Some(fx) => echo(self.other_fx_attr(fx, stats.vol, stats.lr, stats.distort)),
                    None => continue,
                },
                Kind::EngineFx => match self.engine_fx.get(stats.sound as u16 as usize) {
                    Some(fx) => echo(self.engine_fx_attr(fx, stats.vol, stats.lr, stats.distort)),
                    None => continue,
                },
            };
            let Some(channel) = self.channels.alloc(update::RESUME, &attr) else {
                break;
            };
            *self.channels.stats_mut(channel) = stats;
        }
    }

    /// `howl_StopAudio`: every sound stops but, unless `all_fx`, the menu's. With `erase_paused`,
    /// what [`Howl::pause`] kept is gone too.
    pub fn stop_all(&mut self, erase_paused: bool, all_fx: bool) {
        self.channels.destroy_all(all_fx);
        if erase_paused {
            self.paused.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use loader::howl::other_fx_flags;

    fn howl() -> Howl {
        // A second of a loud square wave that loops.
        let mut adpcm = vec![];
        for i in 0..1575 {
            let mut block = [if i % 2 == 0 { 0x77 } else { 0x99 }; 16];
            block[0] = 0;
            block[1] = match i {
                0 => loader::howl::block_flags::LOOP_START,
                1574 => loader::howl::block_flags::END | loader::howl::block_flags::REPEAT,
                _ => 0,
            };
            adpcm.extend(block);
        }
        let fx = OtherFx {
            flags: other_fx_flags::LOOPS,
            volume: 255,
            pitch: 0x1000,
            sample: 0,
            duration: 0,
        };
        Howl::new(vec![fx], vec![], vec![Some(adpcm.into_boxed_slice())])
    }

    #[test]
    fn a_sound_is_heard_from_the_vertical_blank_after_it_starts() {
        let mut howl = howl();
        let mut out = [[0; 2]; 2000];
        howl.render(&mut out[..1]);
        howl.play(0, AntiSpam::Off, Sfx::FULL);
        howl.render(&mut out);
        let first = out.iter().position(|&[l, _]| l != 0).unwrap();
        assert!((FRAMES_PER_VSYNC as usize - 1..FRAMES_PER_VSYNC as usize + 4).contains(&first));
        // A sample of 28672 at full volume in the middle: the voice at 215/255 of 0x3fff, 179/256
        // of that on the left, both 32768ths, then the master's 0x3fff, so about 4160.
        let loudest = out.iter().map(|&[l, _]| l.unsigned_abs()).max().unwrap();
        assert!((4_000..=4_200).contains(&loudest), "{loudest}");
    }

    #[test]
    fn a_pause_stops_the_sounds_and_starts_them_again() {
        let mut howl = howl();
        let sound = howl.play(0, AntiSpam::Off, Sfx::FULL).unwrap();
        howl.vsync();
        howl.pause();
        howl.pause();
        assert!(howl.channels.taken().is_empty());
        // A vertical blank lets its voice go, which takes a few samples.
        howl.render(&mut [[0; 2]; 100]);
        assert!(!howl.spu.playing(0));
        howl.unpause();
        howl.vsync();
        assert_eq!(howl.channels.taken().len(), 1);
        // The same sound, which can still be stopped.
        howl.stop(sound);
        assert!(howl.channels.taken().is_empty());
    }
}
