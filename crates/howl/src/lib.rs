//! The game's sound engine (`game/HOWL`) on the SPU it plays through, as `ctr-native` emulates it
//! (`platform/native_audio.c`). Plain state, like the rest of the port: [`Howl`] is all of it,
//! and its output is samples, which the viewer hands to the speakers.
//!
//! The game starts, changes and stops sounds ([`fx`]); each takes a channel ([`channel`]), whose
//! voice of the [`spu`] plays it. The SPU's vertical blank, 60 a second of its output, runs out
//! the sounds' time and passes what changed on to the voices (`howl_PlayAudio_Update`), so
//! [`Howl::render`] is the clock of the sounds as the console's is.
//!
//! The voice lines ([`voiceline`]) grunt, or queue lines to speak, which the CD drive plays
//! ([`xa`]) as it does the jingles of the race.
//!
//! The [`music`] is the songs of the level's song pack, which play their notes on the channels
//! too; the states of a race ([`state`]) start, fade and stop them.
//!
//! The sounds that echo (a kart in a tunnel, [`Sfx::echo`]) are heard through the SPU's
//! [`reverb`] as well, which each level sets ([`Howl::set_level_reverb`]).
//!
//! Not ported yet: the banks (every sample a sound effect, an engine or the music plays is
//! loaded), and the volume options.

pub mod channel;
pub mod fx;
pub mod music;
pub mod reverb;
pub mod spu;
pub mod state;
pub mod tables;
pub mod voiceline;
pub mod xa;

use std::io;
use std::path::Path;

use loader::howl::{Cseq, EngineFx, OtherFx};
use loader::xa::Xnf;

use channel::{Channels, Kind, Stats, update};
pub use fx::{AntiSpam, NO_DISTORTION, Sound, crash_sound};
use music::Music;
use spu::{Samples, Spu};
use tables::{BOSS_REVERBS, LEVEL_REVERBS, OTHER_LEVEL_REVERB, REVERBS};

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

/// The sound engine: the sounds and songs of `KART.HWL`, the channels they play on, and the SPU.
pub struct Howl {
    pub(crate) spu: Spu,
    pub(crate) channels: Channels,
    pub(crate) other_fx: Vec<OtherFx>,
    pub(crate) engine_fx: Vec<EngineFx>,
    pub(crate) music: Music,
    /// `vol_FX`, `vol_Music` and `vol_Voice`.
    pub volume_fx: u8,
    pub volume_music: u8,
    pub volume_voice: u8,
    /// `boolStereoEnabled`: in mono both sides play the same.
    pub stereo: bool,
    /// `countSounds`.
    pub(crate) count: u32,
    /// The game's frame (`frameTimer_MainFrame_ResetDB`), which the anti-spam measures by.
    pub frame: u32,
    /// What `howl_PauseAudio` took off the channels (`channelStatsCurr`).
    paused: Vec<Stats>,
    pub(crate) voicelines: voiceline::Voicelines,
    /// `audioRNG`, which the voice lines draw from.
    pub(crate) rng: u32,
    /// `curReverb`: one of [`REVERBS`], or their number for none.
    reverb: u16,
    /// `audioState`: where the race is, none before it starts.
    pub(crate) state: Option<state::State>,
    /// `desiredXA_RaceEndIndex`: the jingle of the race's end.
    pub(crate) race_end_xa: u16,
    /// `desiredXA_RaceIntroIndex`: the jingle of the next fly-in, 0 to 3, one after the other.
    pub(crate) race_intro_xa: u16,
    /// The tracks of XA (`XA/ENG.XNF`), and what of them plays.
    pub(crate) xnf: Xnf,
    pub(crate) xa: xa::Xa,
    /// The output samples until the next vertical blank.
    until_vsync: u32,
}

impl Howl {
    /// `howl_InitGlobals`, with the song packs `packs` (`howl_songOffsets`).
    pub fn new(
        other_fx: Vec<OtherFx>,
        engine_fx: Vec<EngineFx>,
        packs: Vec<Cseq>,
        samples: Samples,
    ) -> Howl {
        Howl {
            spu: Spu::new(samples),
            channels: Channels::default(),
            other_fx,
            engine_fx,
            music: Music::new(packs),
            volume_fx: 215,
            volume_music: 175,
            volume_voice: 255,
            stereo: true,
            count: 0,
            frame: 0,
            paused: Vec::new(),
            voicelines: voiceline::Voicelines::default(),
            rng: 0x15a3_7e63,
            // `SetReverbMode(5)`, which the SPU's reverb already is: off, and not heard.
            reverb: REVERBS.len() as u16,
            state: None,
            race_end_xa: 0,
            race_intro_xa: 0,
            xnf: Xnf::default(),
            xa: xa::Xa::default(),
            until_vsync: 0,
        }
    }

    /// Reads `SOUNDS/KART.HWL` from the disc image at `path`: its sound effects, engines and
    /// song packs, and the ADPCM of their samples (3 MB); and the index of its XA (whose tracks
    /// whoever plays them reads, [`xa`]). A disc without XA plays none.
    pub fn read(path: &Path) -> io::Result<Howl> {
        let (mut disc, header) = loader::disc::open_howl(path)?;
        let mut packs = Vec::with_capacity(header.song_offsets.len());
        for song in 0..header.song_offsets.len() as u16 {
            packs.extend(header.cseq(&mut disc, song)?);
        }
        let mut samples: Samples = vec![None; header.sample_sizes.len()];
        let used = header.other_fx.iter().map(|fx| fx.sample);
        let used = used.chain(header.engine_fx.iter().map(|fx| fx.sample));
        for sample in used.chain(packs.iter().flat_map(Cseq::samples)) {
            let Some(slot @ None) = samples.get_mut(sample as usize) else {
                continue;
            };
            *slot = header.adpcm(&mut disc, sample)?.map(Vec::into_boxed_slice);
        }
        let mut howl = Howl::new(header.other_fx, header.engine_fx, packs, samples);
        howl.xnf = loader::disc::open_xa(path).map(|(xnf, _)| xnf).unwrap_or_default();
        Ok(howl)
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

    /// `howl_PlayAudio_Update`: the sounds' time runs, the songs play on, and the voices hear
    /// what changed.
    pub fn vsync(&mut self) {
        self.channels.run_timers();
        self.run_songs();
        self.channels.update(&mut self.spu);
    }

    /// `SetReverbMode`: the reverb is the game's `reverb` ([`REVERBS`]); a number past them turns
    /// it off. Another reverb starts silent.
    pub fn set_reverb(&mut self, reverb: u16) {
        let reverb = reverb.min(REVERBS.len() as u16);
        if reverb == self.reverb {
            return;
        }
        self.reverb = reverb;
        let unit = &mut self.spu.reverb;
        unit.depth = [0; 2];
        unit.on = false;
        if let Some(&(mode, depth)) = REVERBS.get(reverb as usize) {
            unit.on = true;
            unit.set_mode(mode);
            unit.depth = depth;
        }
    }

    /// `Audio_SetReverbMode`: the reverb of the level `level` (a `LevelId`), or in a boss race
    /// that of the boss `boss` (a boss past the game's sets none). A level past
    /// [`LEVEL_REVERBS`], a map's too, has [`OTHER_LEVEL_REVERB`].
    pub fn set_level_reverb(&mut self, level: u32, boss: Option<usize>) {
        let reverb = match boss {
            Some(boss) => match BOSS_REVERBS.get(boss) {
                Some(&reverb) => reverb,
                None => return,
            },
            None => LEVEL_REVERBS
                .get(level as usize)
                .copied()
                .unwrap_or(OTHER_LEVEL_REVERB),
        };
        self.set_reverb(reverb as u16);
    }

    /// `howl_PauseAudio`: every sound and song stops, and is kept to start again with
    /// [`Howl::unpause`]. The XA fades out, for good.
    pub fn pause(&mut self) {
        self.pause_xa();
        if !self.paused.is_empty() {
            return;
        }
        self.pause_songs(true);
        for &channel in &self.channels.taken().to_vec() {
            self.paused.push(*self.channels.stats(channel));
            self.channels.destroy(channel);
        }
    }

    /// `howl_UnPauseAudio`: what [`Howl::pause`] stopped starts again, each sound and note
    /// from its start, on the free channels, and the songs go on. As in retail, after a pause
    /// that stopped nothing the songs stay paused (in a race the engines always play).
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
                // Its own echo, the sequence's.
                Kind::Music => match self.music_attr(stats.sound as usize, stats.note, stats.vol) {
                    Some(attr) => attr,
                    None => continue,
                },
            };
            let Some(channel) = self.channels.alloc(update::RESUME, &attr) else {
                break;
            };
            *self.channels.stats_mut(channel) = stats;
        }
        self.pause_songs(false);
    }

    /// `howl_StopAudio`: every sound stops but, unless `all_fx`, the menu's; and with
    /// `erase_music` the songs. With `erase_paused`, what [`Howl::pause`] kept is gone too.
    pub fn stop_all(&mut self, erase_paused: bool, erase_music: bool, all_fx: bool) {
        if erase_music {
            self.stop_songs();
        }
        self.channels.destroy_all(all_fx, !erase_music);
        if erase_paused {
            self.paused.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use loader::howl::other_fx_flags;
    use loader::level::LevelId;
    use reverb::Mode;

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
        Howl::new(
            vec![fx],
            vec![],
            vec![],
            vec![Some(adpcm.into_boxed_slice())],
        )
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
    fn a_sound_that_echoes_is_heard_after_it_stops_where_the_level_has_a_reverb() {
        // What is heard from a tenth of a second after the sound stops, for another tenth.
        let tail = |level: Option<u32>, echo: bool| {
            let mut howl = howl();
            if let Some(level) = level {
                howl.set_level_reverb(level, None);
            }
            let sound = howl.play(0, AntiSpam::Off, Sfx { echo, ..Sfx::FULL });
            howl.render(&mut [[0; 2]; 4_410]);
            howl.stop(sound.unwrap());
            let mut out = [[0; 2]; 8_820];
            howl.render(&mut out);
            out[4_410..].iter().flatten().filter(|&&s| s != 0).count()
        };
        // Dingo Canyon's studio.
        assert!(tail(Some(0), true) > 1_000);
        assert_eq!(tail(Some(0), false), 0);
        assert_eq!(tail(None, true), 0);
    }

    #[test]
    fn each_level_has_its_reverb() {
        let mut howl = howl();
        howl.set_level_reverb(LevelId::BLIZZARD_BLUFF.0, None);
        assert_eq!(howl.spu.reverb.mode(), Mode::Pipe);
        assert_eq!(howl.spu.reverb.depth, [0x2dff; 2]);
        // The levels past the table, and the maps' (the top bit set).
        howl.set_level_reverb(LevelId::MAIN_MENU_LEVEL.0, None);
        assert_eq!(howl.spu.reverb.mode(), Mode::StudioA);
        howl.set_level_reverb(LevelId::DRAGON_MINES.0, None);
        howl.set_level_reverb(0x8000_1234, None);
        assert_eq!(howl.spu.reverb.mode(), Mode::StudioA);
        // Pinstripe's, then a boss there is none of.
        howl.set_level_reverb(0, Some(3));
        assert_eq!(howl.spu.reverb.mode(), Mode::Pipe);
        howl.set_level_reverb(0, Some(6));
        assert_eq!(howl.spu.reverb.mode(), Mode::Pipe);
        howl.set_reverb(5);
        assert!(!howl.spu.reverb.on);
        assert_eq!(howl.spu.reverb.depth, [0; 2]);
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
