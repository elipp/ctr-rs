//! The channels of the sound engine (`game/HOWL/HOWL_Channel.c`): one for each voice of the SPU,
//! in a pool that a sound takes one from as it starts and gives back as it ends. With all 24
//! taken, a new sound does not play. What a sound asks of its voice (its sample, envelope, pitch,
//! echo and volume) is written to its channel and handed to the SPU at the next vertical blank
//! (`Channel_UpdateChannels`), only where it changed.

use std::collections::VecDeque;

use crate::spu::{Spu, VOICES};
use crate::tables::VOLUME_LR;

pub const CHANNELS: usize = VOICES;

/// The envelope of the sound effects and the engines: full at once, held, and let go within 8
/// samples ([`crate::spu`] has the layout).
pub const AD: u16 = 0x80ff;
pub const SR: u16 = 0x1fc2;

/// What a channel's voice is told at the next vertical blank (`HOWL_CHANNEL_UPDATE_*`).
pub mod update {
    pub const OFF: u32 = 0x1;
    pub const KEY_ON: u32 = 0x2;
    pub const SAMPLE: u32 = 0x4;
    pub const ADSR: u32 = 0x8;
    pub const PITCH: u32 = 0x10;
    pub const REVERB: u32 = 0x20;
    pub const VOLUME: u32 = 0x40;
    pub const ALL_ATTRS: u32 = SAMPLE | ADSR | PITCH | REVERB | VOLUME;
    /// What a sound changes as it plays.
    pub const DYNAMIC_ATTRS: u32 = PITCH | REVERB | VOLUME;
    pub const RESUME: u32 = KEY_ON | ALL_ATTRS;
}

/// The bits of [`Stats::flags`].
pub mod flags {
    pub const PLAYING: u8 = 0x1;
    /// The sound plays until it is stopped: its time does not run out.
    pub const LOOPS: u8 = 0x4;
}

/// What a channel plays (`HOWL_CHANNEL_TYPE_*`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    #[default]
    EngineFx,
    OtherFx,
    Music,
}

/// `struct ChannelAttr`: what a channel's voice plays.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Attr {
    /// The sample (`spuStartAddr`).
    pub sample: u16,
    /// The envelope, as the SPU's two registers.
    pub ad: u16,
    pub sr: u16,
    pub pitch: u16,
    pub reverb: u16,
    /// Left, right (`audioL`, `audioR`).
    pub volume: [i16; 2],
}

/// `struct ChannelStats`: the sound on a channel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub flags: u8,
    pub kind: Kind,
    /// What it was started with: its echo, volume, distortion and pan.
    pub echo: u8,
    pub vol: u8,
    pub distort: u8,
    pub lr: u8,
    /// A note of the music: its pitch, or which drum (`drumIndex_pitchIndex`).
    pub note: u8,
    /// How much longer it plays, in 300ths of a second.
    pub time_left: i16,
    /// The sound's number, and for a sound effect which start of it in the high half
    /// (`soundID`).
    pub sound: u32,
    /// The game's frame it started on.
    pub start_frame: u32,
}

/// The channels: the sounds on them, the free and the taken (each in its order), and what their
/// voices are to be told.
#[derive(Clone, Debug)]
pub struct Channels {
    stats: [Stats; CHANNELS],
    free: VecDeque<usize>,
    taken: Vec<usize>,
    updates: [u32; CHANNELS],
    /// What the sounds ask of the voices (`channelAttrNew`), and what the voices were told
    /// (`channelAttrCur`).
    new: [Attr; CHANNELS],
    current: [Attr; CHANNELS],
}

impl Default for Channels {
    /// `Voiceline_PoolInit`: all free, and the voices told nothing yet.
    fn default() -> Self {
        let told = Attr {
            sample: u16::MAX,
            ad: AD,
            sr: SR,
            pitch: u16::MAX,
            reverb: u16::MAX,
            volume: [-1; 2],
        };
        Channels {
            stats: [Stats::default(); CHANNELS],
            free: (0..CHANNELS).collect(),
            taken: Vec::with_capacity(CHANNELS),
            updates: [0; CHANNELS],
            new: [Attr::default(); CHANNELS],
            current: [told; CHANNELS],
        }
    }
}

impl Channels {
    pub fn stats(&self, channel: usize) -> &Stats {
        &self.stats[channel]
    }

    pub fn stats_mut(&mut self, channel: usize) -> &mut Stats {
        &mut self.stats[channel]
    }

    /// The taken channels, the first taken first.
    pub fn taken(&self) -> &[usize] {
        &self.taken
    }

    /// `Channel_FindSound`: whether sound effect `id` plays.
    pub fn find_sound(&self, id: u16) -> bool {
        self.taken.iter().any(|&c| {
            let stats = &self.stats[c];
            stats.kind == Kind::OtherFx && stats.sound as u16 == id
        })
    }

    /// `Channel_DestroySelf`: the channel's voice lets go, and the channel is free again.
    pub fn destroy(&mut self, channel: usize) {
        self.updates[channel] |= update::OFF;
        self.updates[channel] &= !update::KEY_ON;
        self.stats[channel].flags &= !flags::PLAYING;
        self.taken.retain(|&c| c != channel);
        self.free.push_back(channel);
    }

    /// `Channel_AllocSlot`: the first free channel, its voice to start playing `attr` and to be
    /// told what `flags` say. `None` if all are taken.
    pub fn alloc(&mut self, flags: u32, attr: &Attr) -> Option<usize> {
        let channel = self.free.pop_front()?;
        self.taken.push(channel);
        self.updates[channel] |= flags | update::KEY_ON;
        self.new[channel] = *attr;
        self.stats[channel].flags = flags::PLAYING;
        Some(channel)
    }

    /// `Channel_AllocSlot_AntiSpam`: [`Channels::alloc`], and with `anti_spam` first the end of
    /// sound effect `id` where it started less than 10 of the game's frames before `frame`.
    pub fn alloc_anti_spam(
        &mut self,
        id: u16,
        anti_spam: bool,
        flags: u32,
        attr: &Attr,
        frame: u32,
    ) -> Option<usize> {
        if anti_spam {
            for channel in self.taken.clone() {
                let stats = &self.stats[channel];
                let recent = frame.wrapping_sub(stats.start_frame) < 10;
                if stats.kind == Kind::OtherFx && stats.sound as u16 == id && recent {
                    self.destroy(channel);
                }
            }
        }
        self.alloc(flags, attr)
    }

    /// What the voice of `channel` plays from the next vertical blank on (`channelAttrNew`).
    pub fn new_attr(&self, channel: usize) -> &Attr {
        &self.new[channel]
    }

    /// What the voice of `channel` is to be told at the next vertical blank, which is to tell
    /// it what `updates` say.
    pub fn attr_mut(&mut self, channel: usize, updates: u32) -> &mut Attr {
        self.updates[channel] |= updates;
        &mut self.new[channel]
    }

    /// `Channel_SearchFX_EditAttr`: the first channel of `kind` playing `sound` takes what
    /// `flags` say of `attr`. Returns the channel.
    pub fn edit(&mut self, kind: Kind, sound: u32, flags: u32, attr: &Attr) -> Option<usize> {
        let channel = *self.taken.iter().find(|&&c| {
            let stats = &self.stats[c];
            stats.kind == kind && stats.sound == sound
        })?;
        self.updates[channel] |= flags;
        let new = &mut self.new[channel];
        if flags & update::SAMPLE != 0 {
            new.sample = attr.sample;
        }
        if flags & update::ADSR != 0 {
            (new.ad, new.sr) = (attr.ad, attr.sr);
        }
        if flags & update::PITCH != 0 {
            new.pitch = attr.pitch;
        }
        if flags & update::REVERB != 0 {
            new.reverb = attr.reverb;
        }
        if flags & update::VOLUME != 0 {
            new.volume = attr.volume;
        }
        Some(channel)
    }

    /// `Channel_SearchFX_Destroy`: the end of the first channel of `kind` whose sound matches
    /// `sound` in the bits of `mask` (all of them for one start of a sound, the low half for any).
    pub fn destroy_matching(&mut self, kind: Kind, sound: u32, mask: u32) -> Option<usize> {
        let channel = *self.taken.iter().find(|&&c| {
            let stats = &self.stats[c];
            stats.kind == kind && stats.sound & mask == sound & mask
        })?;
        self.destroy(channel);
        Some(channel)
    }

    /// `Channel_DestroyAll_LowLevel`: the end of every channel but the music's if `keep_music`,
    /// and but the menu's sound effects (0 to 5, which ring out) unless `all_fx`.
    pub fn destroy_all(&mut self, all_fx: bool, keep_music: bool) {
        for channel in self.taken.clone() {
            let stats = &self.stats[channel];
            let music = stats.kind == Kind::Music;
            let menu = stats.kind == Kind::OtherFx && stats.sound as u16 <= 5;
            if !(keep_music && music) && (all_fx || !menu) {
                self.destroy(channel);
            }
        }
    }

    /// The timers of `howl_PlayAudio_Update`: a vertical blank takes 5 off every sound that does
    /// not loop, and ends the ones whose time is up.
    pub fn run_timers(&mut self) {
        for channel in self.taken.clone() {
            let stats = &mut self.stats[channel];
            if stats.flags & flags::LOOPS != 0 {
                continue;
            }
            stats.time_left = stats.time_left.wrapping_sub(5);
            if stats.time_left <= 0 {
                self.destroy(channel);
            }
        }
    }

    /// `Channel_UpdateChannels`: the voices of the ended channels let go, the others are told
    /// what changed, and the new ones start.
    pub fn update(&mut self, spu: &mut Spu) {
        let mut off = 0;
        for (voice, updates) in self.updates.iter_mut().enumerate() {
            if *updates & update::OFF != 0 {
                off |= 1 << voice;
                // A channel ended and taken again in the same blank starts again.
                *updates = if *updates & update::KEY_ON != 0 {
                    *updates & !update::OFF
                } else {
                    0
                };
            }
        }
        spu.key_off(off);

        let mut on = 0;
        for voice in 0..CHANNELS {
            let updates = std::mem::take(&mut self.updates[voice]);
            if updates == 0 {
                continue;
            }
            let (current, new) = (&mut self.current[voice], &self.new[voice]);
            if updates & update::KEY_ON != 0 {
                on |= 1 << voice;
            }
            if updates & update::SAMPLE != 0 && current.sample != new.sample {
                current.sample = new.sample;
                spu.set_start(voice, new.sample);
            }
            if updates & update::ADSR != 0 && (current.ad, current.sr) != (new.ad, new.sr) {
                (current.ad, current.sr) = (new.ad, new.sr);
                spu.set_adsr(voice, new.ad, new.sr);
            }
            if updates & update::PITCH != 0 && current.pitch != new.pitch {
                current.pitch = new.pitch;
                spu.set_pitch(voice, new.pitch);
            }
            if updates & update::REVERB != 0 && current.reverb != new.reverb {
                current.reverb = new.reverb;
                spu.set_reverb(voice, new.reverb != 0);
            }
            if updates & update::VOLUME != 0 && current.volume != new.volume {
                current.volume = new.volume;
                spu.set_volume(voice, new.volume);
            }
        }
        spu.key_on(on);
    }
}

/// `Channel_SetVolume`: the volume of each side, left and right, of a voice at `volume` (at most
/// 0x3fff, in the SPU's 32768ths) and `pan` (0 all left, 0x80 the middle, 255 all right). In mono
/// both sides are at `volume`.
pub fn volume(volume: u32, pan: u8, stereo: bool) -> [i16; 2] {
    let volume = volume.min(0x3fff);
    if !stereo {
        return [volume as i16; 2];
    }
    let side = |towards: u8| ((volume * VOLUME_LR[towards as usize] as u32) >> 8) as i16;
    [side(u8::MAX - pan), side(pan)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attr(sample: u16) -> Attr {
        Attr {
            sample,
            ad: AD,
            sr: SR,
            pitch: 0x1000,
            reverb: 0,
            volume: [0x1000; 2],
        }
    }

    fn sound(channels: &mut Channels, id: u16, frame: u32, anti_spam: bool) -> Option<usize> {
        let channel =
            channels.alloc_anti_spam(id, anti_spam, update::ALL_ATTRS, &attr(0), frame)?;
        let stats = channels.stats_mut(channel);
        stats.kind = Kind::OtherFx;
        stats.sound = id as u32;
        stats.start_frame = frame;
        stats.time_left = 10;
        Some(channel)
    }

    #[test]
    fn the_channels_go_in_order_and_come_back_at_the_end() {
        let mut channels = Channels::default();
        assert_eq!(sound(&mut channels, 1, 0, false), Some(0));
        assert_eq!(sound(&mut channels, 2, 0, false), Some(1));
        channels.destroy(0);
        assert_eq!(sound(&mut channels, 3, 0, false), Some(2));
        for _ in 3..CHANNELS {
            sound(&mut channels, 4, 0, false);
        }
        // The freed one is last in line.
        assert_eq!(sound(&mut channels, 5, 0, false), Some(0));
        assert_eq!(sound(&mut channels, 6, 0, false), None);
    }

    #[test]
    fn anti_spam_cuts_the_same_sound_started_within_10_frames() {
        let mut channels = Channels::default();
        sound(&mut channels, 7, 100, true);
        sound(&mut channels, 7, 109, true);
        assert_eq!(channels.taken(), [1]);
        sound(&mut channels, 7, 119, true);
        assert_eq!(channels.taken(), [1, 2]);
        // Without it, they overlap.
        sound(&mut channels, 7, 119, false);
        assert_eq!(channels.taken().len(), 3);
    }

    #[test]
    fn a_sound_ends_when_its_time_is_up_unless_it_loops() {
        let mut channels = Channels::default();
        sound(&mut channels, 1, 0, false);
        let looped = sound(&mut channels, 2, 0, false).unwrap();
        channels.stats_mut(looped).flags |= flags::LOOPS;
        channels.run_timers();
        assert_eq!(channels.taken(), [0, 1]);
        channels.run_timers();
        assert_eq!(channels.taken(), [1]);
    }

    #[test]
    fn the_voices_are_told_only_what_changed() {
        let samples = vec![Some(vec![0u8; 32].into_boxed_slice())];
        let mut spu = Spu::new(samples);
        let mut channels = Channels::default();
        let channel = sound(&mut channels, 1, 0, false).unwrap();
        channels.update(&mut spu);
        assert!(spu.playing(channel));
        assert_eq!(channels.current[channel], attr(0));
        let louder = Attr {
            volume: [0x2000; 2],
            ..attr(0)
        };
        channels.edit(Kind::OtherFx, 1, update::DYNAMIC_ATTRS, &louder);
        channels.update(&mut spu);
        assert_eq!(channels.current[channel].volume, [0x2000; 2]);
        channels.destroy(channel);
        channels.update(&mut spu);
        assert_eq!(channels.updates, [0; CHANNELS]);
    }

    #[test]
    fn the_menu_sounds_ring_out_unless_all_go() {
        let mut channels = Channels::default();
        sound(&mut channels, 1, 0, false);
        sound(&mut channels, 0x45, 0, false);
        channels.destroy_all(false, true);
        assert_eq!(channels.taken(), [0]);
        channels.destroy_all(true, true);
        assert!(channels.taken().is_empty());
    }

    #[test]
    fn the_middle_is_a_little_louder_on_the_right() {
        let share = |of: i32| ((0x3fff * of) >> 8) as i16;
        assert_eq!(volume(0x3fff, 0x80, true), [share(179), share(181)]);
        assert_eq!(volume(0x5000, 0, true), [share(255), 0]);
        assert_eq!(volume(0x1000, 0, false), [0x1000; 2]);
    }
}
