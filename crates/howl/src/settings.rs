//! The volume options (`game/HOWL/HOWL_Settings.c`): the effects', the music's and the voices'
//! volumes, which the sounds that play follow at once, and the sound the options menu plays while
//! a volume is picked, to hear it by.

use crate::channel::{Kind, update};
use crate::fx::{AntiSpam, Sound};
use crate::music::{AKU_SONG, UKA_SONG};
use crate::voiceline::GRUNTS;
use crate::{Howl, Sfx};

/// A volume of the options (`HOWL_VOLUME_TYPE_*`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slider {
    Fx,
    Music,
    Voice,
}

/// The sound the effects' volume is heard by: the bowling bomb's roll, which loops.
const FX_TEST_SFX: u16 = 0x48;
/// The game's frames between two of the voices' test grunts, which take turns.
const VOICE_TEST_FRAMES: u32 = 25;

/// What [`Howl::test_sound`] plays (`HOWL_SLIDER_ROW`, `HOWL_SLIDER_PLAYING`, `HOWL_SLIDER_SOUND`).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Test {
    slider: Option<Slider>,
    /// The mask's song the music's volume is heard by.
    song: Option<u16>,
    /// The voices' last grunt, and the game's frame it was asked at.
    grunt: Option<Sound>,
    frame: u32,
}

impl Howl {
    /// `UpdateChannelVol_EngineFX_All` and `UpdateChannelVol_OtherFX_All`: every sound effect and
    /// engine that plays at the volumes there are now.
    pub(crate) fn set_fx_volumes(&mut self) {
        for channel in self.channels.taken().to_vec() {
            let stats = *self.channels.stats(channel);
            let attr = match stats.kind {
                Kind::OtherFx => match self.other_fx.get(stats.sound as u16 as usize) {
                    Some(fx) => self.other_fx_attr(fx, stats.vol, stats.lr, stats.distort),
                    None => continue,
                },
                Kind::EngineFx => match self.engine_fx.get(stats.sound as u16 as usize) {
                    Some(fx) => self.engine_fx_attr(fx, stats.vol, stats.lr, stats.distort),
                    None => continue,
                },
                Kind::Music => continue,
            };
            self.channels.attr_mut(channel, update::VOLUME).volume = attr.volume;
        }
    }

    /// `OptionsMenu_TestSound`, every frame of the options menu: while `slider` is picked its
    /// sound plays, and none once it is not. The effects loop the bomb's roll, the music plays a
    /// mask's song (Uka Uka's if Aku Aku's is the race's, else Aku Aku's), and the voices grunt
    /// as `character` does, every [`VOICE_TEST_FRAMES`] of the game's `frame`.
    ///
    /// DIVERGENCE: retail grunts on the frames that are a multiple of 25. The caller's frames
    /// may skip one, so this grunts when the frame passes one instead.
    pub fn test_sound(&mut self, slider: Option<Slider>, character: u8, frame: u32) {
        let test = self.test;
        if test.slider != slider {
            match test.slider {
                Some(Slider::Fx) => self.stop_id(FX_TEST_SFX),
                Some(Slider::Music) => {
                    if let Some(song) = test.song {
                        self.stop_song(song);
                    }
                }
                Some(Slider::Voice) => {
                    if let Some(grunt) = test.grunt {
                        self.stop(grunt);
                    }
                }
                None => {}
            }
            self.test = Test {
                slider,
                frame,
                ..Test::default()
            };
            match slider {
                Some(Slider::Fx) => {
                    self.play(FX_TEST_SFX, AntiSpam::Off, Sfx::FULL);
                }
                Some(Slider::Music) => {
                    let song = if self.music.playing == Some(AKU_SONG) {
                        UKA_SONG
                    } else {
                        AKU_SONG
                    };
                    if self.start_song(song, 0, true) {
                        self.test.song = Some(song);
                    }
                }
                Some(Slider::Voice) | None => {}
            }
        }
        if slider == Some(Slider::Voice)
            && frame / VOICE_TEST_FRAMES != self.test.frame / VOICE_TEST_FRAMES
        {
            let turn = (frame / VOICE_TEST_FRAMES % 2) as usize;
            self.test.grunt = self.play(GRUNTS[turn] + character as u16, AntiSpam::Off, Sfx::FULL);
        }
        self.test.frame = frame;
    }
}

#[cfg(test)]
mod tests {
    use loader::howl::OtherFx;
    use loader::howl::other_fx_flags;

    use super::*;
    use crate::music::test_howl;

    fn howl() -> Howl {
        let mut howl = test_howl();
        let fx = OtherFx {
            flags: other_fx_flags::LOOPS,
            volume: 255,
            pitch: 0x1000,
            sample: 0,
            duration: 300,
        };
        howl.other_fx = vec![fx; 0x50];
        howl
    }

    fn sounds(howl: &Howl, kind: Kind) -> Vec<u16> {
        let taken = howl.channels.taken().iter();
        taken
            .map(|&c| howl.channels.stats(c))
            .filter(|s| s.kind == kind)
            .map(|s| s.sound as u16)
            .collect()
    }

    #[test]
    fn the_effects_loop_the_bombs_roll_while_picked() {
        let mut howl = howl();
        howl.test_sound(Some(Slider::Fx), 0, 0);
        howl.test_sound(Some(Slider::Fx), 0, 1);
        assert_eq!(sounds(&howl, Kind::OtherFx), [FX_TEST_SFX]);
        howl.test_sound(None, 0, 2);
        assert!(sounds(&howl, Kind::OtherFx).is_empty());
    }

    #[test]
    fn the_music_plays_the_mask_song_the_race_does_not() {
        let mut howl = howl();
        howl.test_sound(Some(Slider::Music), 0, 0);
        assert_eq!(howl.test.song, Some(AKU_SONG));
        howl.test_sound(None, 0, 1);
        assert_eq!(howl.test.song, None);

        howl.play_music(AKU_SONG, 0);
        howl.test_sound(Some(Slider::Music), 0, 2);
        assert_eq!(howl.test.song, Some(UKA_SONG));
        howl.test_sound(None, 0, 3);
        assert_eq!(howl.music_playing().map(|(id, _)| id), Some(AKU_SONG));
    }

    #[test]
    fn the_voices_grunt_every_25_frames_one_grunt_then_the_other() {
        let mut howl = howl();
        howl.test_sound(Some(Slider::Voice), 3, 10);
        assert!(sounds(&howl, Kind::OtherFx).is_empty());
        howl.test_sound(Some(Slider::Voice), 3, 24);
        assert!(sounds(&howl, Kind::OtherFx).is_empty());
        howl.test_sound(Some(Slider::Voice), 3, 26);
        assert_eq!(sounds(&howl, Kind::OtherFx), [GRUNTS[1] + 3]);
        howl.test_sound(Some(Slider::Voice), 3, 50);
        assert_eq!(sounds(&howl, Kind::OtherFx), [GRUNTS[1] + 3, GRUNTS[0] + 3]);
    }

    #[test]
    fn the_effects_that_play_follow_their_volume() {
        let mut howl = howl();
        howl.play(0, AntiSpam::Off, Sfx::FULL);
        let channel = howl.channels.taken()[0];
        let loud = howl.channels.new_attr(channel).volume;
        howl.set_volumes(50, 100, 100);
        let quiet = howl.channels.new_attr(channel).volume;
        assert!(quiet[0] > 0 && quiet[0] < loud[0], "{quiet:?} {loud:?}");
    }
}
