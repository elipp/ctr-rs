//! The sound effects (`game/HOWL/HOWL_OtherFX.c`, `HOWL_OtherFXRecycle.c`) and the engines'
//! loops (`EngineAudio_*` of `HOWL_Engine.c`): what starts them on a channel, changes them as
//! they play and stops them.

use loader::howl::{EngineFx, OtherFx, other_fx_flags};

use crate::channel::{self, AD, Attr, Kind, SR, flags, update};
use crate::tables::{DISTORT_ENGINE, DISTORT_OTHER_FX};
use crate::{Howl, Sfx};

/// The distortion that keeps a sound at its own pitch (`HOWL_SFX_DISTORTION_NONE`).
pub const NO_DISTORTION: u8 = 0x80;

/// What [`Howl::play`] does when the same sound effect plays already (`boolAntiSpam`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AntiSpam {
    /// Both play.
    Off,
    /// The one started less than 10 of the game's frames before ends.
    CutRecent,
    /// The new one does not play.
    Reject,
}

/// One start of a sound effect: its number, and which start of it in the high half
/// (`soundID_count`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sound(u32);

impl Sound {
    pub fn id(self) -> u16 {
        self.0 as u16
    }
}

/// `OtherFX_DriverCrashing`: the bump of two karts, hard, medium and soft.
const CRASH_HARD: u16 = 10;
const CRASH_MEDIUM: u16 = 12;
const CRASH_SOFT: u16 = 11;

/// `OtherFX_DriverCrashing`: the bump of two karts by its volume, harder the louder.
pub fn crash_sound(volume: u8) -> u16 {
    match volume {
        221.. => CRASH_HARD,
        161..=220 => CRASH_MEDIUM,
        _ => CRASH_SOFT,
    }
}

/// A pitch at a distortion of `table`.
fn distorted(pitch: u16, distort: u8, table: &[u32; 256]) -> u16 {
    if distort == NO_DISTORTION {
        pitch
    } else {
        ((pitch as u32 * table[distort as usize]) >> 16) as u16
    }
}

impl Howl {
    /// `OtherFX_Play_LowLevel`: sound effect `id` on a channel of its own, until its time is up
    /// or, for one that loops, until it is stopped. `None` if it is not loaded, all channels are
    /// taken, or `anti_spam` rejects it.
    pub fn play(&mut self, id: u16, anti_spam: AntiSpam, sfx: Sfx) -> Option<Sound> {
        let fx = *self.other_fx.get(id as usize)?;
        if !self.spu.has(fx.sample) {
            return None;
        }
        let attr = Attr {
            reverb: sfx.echo as u16,
            ..self.other_fx_attr(&fx, sfx.volume, sfx.pan, sfx.distort)
        };
        if anti_spam == AntiSpam::Reject && self.channels.find_sound(id) {
            return None;
        }
        let cut = anti_spam == AntiSpam::CutRecent;
        let channel =
            self.channels
                .alloc_anti_spam(id, cut, update::ALL_ATTRS, &attr, self.frame)?;
        let count = self.count_sounds();
        let stats = self.channels.stats_mut(channel);
        if fx.flags & other_fx_flags::LOOPS != 0 {
            stats.flags |= flags::LOOPS;
        }
        stats.kind = Kind::OtherFx;
        stats.echo = sfx.echo as u8;
        stats.vol = sfx.volume;
        stats.distort = sfx.distort;
        stats.lr = sfx.pan;
        stats.time_left = fx.duration as i16;
        stats.sound = count << 16 | id as u32;
        stats.start_frame = self.frame;
        Some(Sound(stats.sound))
    }

    /// `OtherFX_Modify`: `sound` plays on at `sfx`, if it still plays.
    pub fn modify(&mut self, sound: Sound, sfx: Sfx) {
        let Some(&fx) = self.other_fx.get(sound.id() as usize) else {
            return;
        };
        let volume = self.fx_volume(&fx) * fx.volume as u32 * sfx.volume as u32;
        let attr = Attr {
            volume: channel::volume(volume >> 10, sfx.pan, self.stereo),
            pitch: distorted(fx.pitch, sfx.distort, &DISTORT_OTHER_FX),
            reverb: sfx.echo as u16,
            ..Attr::default()
        };
        let edited = self
            .channels
            .edit(Kind::OtherFx, sound.0, update::DYNAMIC_ATTRS, &attr);
        if let Some(channel) = edited {
            let stats = self.channels.stats_mut(channel);
            stats.echo = sfx.echo as u8;
            stats.vol = sfx.volume;
            stats.distort = sfx.distort;
            stats.lr = sfx.pan;
        }
    }

    /// `OtherFX_Stop1`: the end of `sound`.
    pub fn stop(&mut self, sound: Sound) {
        self.channels
            .destroy_matching(Kind::OtherFx, sound.0, u32::MAX);
    }

    /// `OtherFX_Stop2`: the end of sound effect `id`, the first found of its starts.
    pub fn stop_id(&mut self, id: u16) {
        self.channels
            .destroy_matching(Kind::OtherFx, id as u32, 0xffff);
    }

    /// `OtherFX_RecycleNew`: what plays in `slot` becomes sound effect `id` at `sfx`. The same
    /// sound plays on changed, another one starts in its place, and `None` stops it.
    pub fn recycle(&mut self, slot: &mut Option<Sound>, id: Option<u16>, sfx: Sfx) {
        if let Some(playing) = *slot
            && Some(playing.id()) != id
        {
            self.stop(playing);
            *slot = None;
        }
        let Some(id) = id else {
            return;
        };
        match *slot {
            None => *slot = self.play(id, AntiSpam::Off, sfx),
            Some(playing) => self.modify(playing, sfx),
        }
    }

    /// `OtherFX_RecycleMute`: what plays in `slot` stops.
    pub fn recycle_mute(&mut self, slot: &mut Option<Sound>) {
        if let Some(playing) = slot.take() {
            self.stop(playing);
        }
    }

    /// `EngineAudio_InitOnce`: engine loop `id` starts on a channel of its own, at `sfx`. False
    /// if it is not loaded or all channels are taken.
    pub fn start_engine(&mut self, id: u16, sfx: Sfx) -> bool {
        let Some(&fx) = self.engine_fx.get(id as usize) else {
            return false;
        };
        if !self.spu.has(fx.sample) {
            return false;
        }
        let attr = Attr {
            reverb: sfx.echo as u16,
            ..self.engine_fx_attr(&fx, sfx.volume, sfx.pan, sfx.distort)
        };
        let Some(channel) = self.channels.alloc(update::ALL_ATTRS, &attr) else {
            return false;
        };
        let stats = self.channels.stats_mut(channel);
        stats.kind = Kind::EngineFx;
        stats.echo = sfx.echo as u8;
        stats.vol = sfx.volume;
        stats.distort = sfx.distort;
        stats.lr = sfx.pan;
        stats.time_left = 0;
        stats.sound = id as u32;
        stats.flags |= flags::LOOPS;
        true
    }

    /// `EngineAudio_Recalculate`: engine loop `id` plays on at `sfx`. (Retail turns it down
    /// for two or more players on one screen; there is one here.)
    pub fn set_engine(&mut self, id: u16, sfx: Sfx) {
        let Some(&fx) = self.engine_fx.get(id as usize) else {
            return;
        };
        let volume = self.volume_fx as u32 * fx.volume as u32 * sfx.volume as u32;
        let attr = Attr {
            volume: channel::volume(volume >> 10, sfx.pan, self.stereo),
            pitch: distorted(fx.pitch, sfx.distort, &DISTORT_ENGINE),
            reverb: sfx.echo as u16,
            ..Attr::default()
        };
        let edited = self
            .channels
            .edit(Kind::EngineFx, id as u32, update::DYNAMIC_ATTRS, &attr);
        if let Some(channel) = edited {
            let stats = self.channels.stats_mut(channel);
            stats.echo = sfx.echo as u8;
            stats.vol = sfx.volume;
            stats.distort = sfx.distort;
            stats.lr = sfx.pan;
        }
    }

    /// `EngineAudio_Stop`.
    pub fn stop_engine(&mut self, id: u16) {
        self.channels
            .destroy_matching(Kind::EngineFx, id as u32, u32::MAX);
    }

    /// `howl_InitChannelAttr_OtherFX`, without the echo.
    pub(crate) fn other_fx_attr(&self, fx: &OtherFx, vol: u8, pan: u8, distort: u8) -> Attr {
        let volume = self.fx_volume(fx) * fx.volume as u32 * vol as u32;
        Attr {
            sample: fx.sample,
            ad: AD,
            sr: SR,
            pitch: distorted(fx.pitch, distort, &DISTORT_OTHER_FX),
            reverb: 0,
            volume: channel::volume(volume >> 10, pan, self.stereo),
        }
    }

    /// `howl_InitChannelAttr_EngineFX`, without the echo.
    pub(crate) fn engine_fx_attr(&self, fx: &EngineFx, vol: u8, pan: u8, distort: u8) -> Attr {
        let volume = self.volume_fx as u32 * fx.volume as u32 * vol as u32;
        Attr {
            sample: fx.sample,
            ad: AD,
            sr: SR,
            pitch: distorted(fx.pitch, distort, &DISTORT_ENGINE),
            reverb: 0,
            volume: channel::volume(volume >> 10, pan, self.stereo),
        }
    }

    /// The volume a sound effect plays at: a voice clip's, or the effects'.
    fn fx_volume(&self, fx: &OtherFx) -> u32 {
        if fx.flags & other_fx_flags::VOICE != 0 {
            self.volume_voice as u32
        } else {
            self.volume_fx as u32
        }
    }

    /// `CountSounds`: the number of the next start of a sound effect, never 0.
    fn count_sounds(&mut self) -> u32 {
        self.count = self.count.wrapping_add(1).max(1);
        self.count
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spu::Samples;

    /// Sound effect 0 is a one-shot held 10/300 s, 1 loops; engine 0 loops.
    fn howl() -> Howl {
        let samples: Samples = vec![Some(vec![0u8; 64].into_boxed_slice())];
        let fx = |flags| OtherFx {
            flags,
            volume: 255,
            pitch: 0x1000,
            sample: 0,
            duration: 10,
        };
        let engine = EngineFx {
            flags: 0,
            volume: 200,
            pitch: 0x800,
            sample: 0,
        };
        Howl::new(
            vec![fx(0), fx(other_fx_flags::LOOPS)],
            vec![engine],
            samples,
        )
    }

    #[test]
    fn a_one_shot_ends_with_its_time_and_a_loop_when_stopped() {
        let mut howl = howl();
        let shot = howl.play(0, AntiSpam::Off, Sfx::FULL).unwrap();
        let looped = howl.play(1, AntiSpam::Off, Sfx::FULL).unwrap();
        assert_eq!(shot.id(), 0);
        assert_ne!(shot, looped);
        howl.vsync();
        howl.vsync();
        assert_eq!(howl.channels.taken(), [1]);
        howl.stop(looped);
        assert!(howl.channels.taken().is_empty());
    }

    #[test]
    fn reject_does_not_start_a_sound_that_plays() {
        let mut howl = howl();
        assert!(howl.play(1, AntiSpam::Reject, Sfx::FULL).is_some());
        assert!(howl.play(1, AntiSpam::Reject, Sfx::FULL).is_none());
        assert!(howl.play(1, AntiSpam::Off, Sfx::FULL).is_some());
    }

    #[test]
    fn recycle_changes_the_sound_in_place_or_starts_another() {
        let mut howl = howl();
        let mut slot = None;
        howl.recycle(&mut slot, Some(1), Sfx::FULL);
        let first = slot.unwrap();
        let quiet = Sfx {
            volume: 10,
            ..Sfx::FULL
        };
        howl.recycle(&mut slot, Some(1), quiet);
        assert_eq!(slot, Some(first));
        assert_eq!(howl.channels.stats(0).vol, 10);
        howl.recycle(&mut slot, Some(0), Sfx::FULL);
        assert_ne!(slot, Some(first));
        assert_eq!(howl.channels.taken(), [1]);
        howl.recycle(&mut slot, None, Sfx::FULL);
        assert_eq!(slot, None);
        assert!(howl.channels.taken().is_empty());
    }

    #[test]
    fn an_engine_loops_at_what_it_is_set_to() {
        let mut howl = howl();
        assert!(howl.start_engine(0, Sfx::SILENT));
        howl.vsync();
        let sfx = Sfx {
            volume: 0x80,
            distort: 0x80 + 64,
            ..Sfx::SILENT
        };
        howl.set_engine(0, sfx);
        howl.vsync();
        // 64 steps of distortion is an octave up.
        assert_eq!(howl.channels.stats(0).distort, 0x80 + 64);
        for _ in 0..100 {
            howl.vsync();
        }
        assert_eq!(howl.channels.taken(), [0]);
        howl.stop_engine(0);
        assert!(howl.channels.taken().is_empty());
    }

    #[test]
    fn the_distortions_are_an_octave_in_128_and_in_64() {
        assert_eq!(distorted(0x1000, 0x80, &DISTORT_OTHER_FX), 0x1000);
        assert_eq!(distorted(0x1000, 0xff, &DISTORT_OTHER_FX), 0x1fd3);
        assert_eq!(distorted(0x1000, 0x00, &DISTORT_OTHER_FX), 0x800);
        assert_eq!(distorted(0x1000, 0xc0, &DISTORT_ENGINE), 0x2000);
    }

    #[test]
    fn the_harder_the_bump_the_harder_the_sound() {
        assert_eq!(crash_sound(255), CRASH_HARD);
        assert_eq!(crash_sound(221), CRASH_HARD);
        assert_eq!(crash_sound(220), CRASH_MEDIUM);
        assert_eq!(crash_sound(161), CRASH_MEDIUM);
        assert_eq!(crash_sound(160), CRASH_SOFT);
    }
}
