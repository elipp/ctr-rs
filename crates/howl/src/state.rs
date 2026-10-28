//! The states of a race as the sounds follow them (`game/HOWL/HOWL_AudioState.c`): what
//! `Audio_SetState` does as a race comes to each, and what `Audio_Update1` does in each, once a
//! frame of the game.
//!
//! The music is silent at the traffic lights and starts after the green, the level's song or,
//! while a mask is out, the mask's. It fades down for "FINAL LAP!" (a jingle of XA) and up again
//! a second into it, and plays 20 beats a minute faster from then on; at the end of the race it
//! fades out for the race's end, a jingle of a win or of a loss, and plays again from its start
//! once that is over. The voice lines are on while racing, but not for the start of the last lap
//! and the end of the race.
//!
//! A race starts at the lights, as the fly-in's state (`AUDIO_RACE_INTRO`) waits for its jingle
//! and nothing else: the fly-in's jingle is not played.

use loader::xa::Category;

use crate::Howl;
use crate::music::LEVEL_SONG;
use crate::xa::XaState;

/// How near the finish line the leader is for the start of the last lap to be near, in the
/// units of `distanceToFinish`.
const NEAR_FINISH: i32 = 9000;
/// How much faster the music plays from "FINAL LAP!" on, in beats a minute.
const LAST_LAP_TEMPO: i32 = 20;
/// The music's XA: "FINAL LAP!", and the race's end when the driver wins and when it does not.
const FINAL_LAP_XA: u16 = 6;
const WIN_XA: u16 = 4;
const LOSS_XA: u16 = 5;
/// The music comes up again once "FINAL LAP!" has played this many blocks of the XA's time (a
/// second).
const FINAL_LAP_HEARD: i32 = 0xe1;

/// `AUDIO_*`: where a race is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// `AUDIO_TRAFFIC`: the traffic lights count down.
    Traffic,
    /// `AUDIO_RACING`.
    Racing,
    /// `AUDIO_PRE_LAST_LAP`: the leader nears the line of the last lap but one.
    PreLastLap,
    /// `AUDIO_FINAL_LAP`: it is over it.
    FinalLap,
    /// `AUDIO_POST_LAST_LAP`: on the last lap.
    PostLastLap,
    /// `AUDIO_LAST_LAP`: near its end.
    LastLap,
    /// `AUDIO_RACE_END`: the leader has finished.
    RaceEnd,
}

/// What [`Howl::update_state`] hears of a race: of the race, and of the human driver in the
/// lead.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RaceAudio {
    /// The traffic lights are out (`trafficLightsTimer < 1`).
    pub lights_out: bool,
    /// The laps of the race (`numLaps`).
    pub laps: u8,
    /// The driver's lap, from 0 (`lapIndex`), how far it is from the finish line
    /// (`distanceToFinish_curr`), and whether it has finished.
    pub lap: u8,
    pub to_finish: i32,
    pub finished: bool,
    /// It is first (`driversInRaceOrder[0]`).
    pub first: bool,
    /// The song of the mask that is out (`ACTION_MASK_WEAPON`, with `AKU_SONG` or `UKA_SONG`):
    /// [`crate::music::AKU_SONG`] or [`crate::music::UKA_SONG`].
    pub mask_song: Option<u16>,
}

impl Howl {
    /// `Audio_SetState_Safe`: the race comes to `state`, if it was not there.
    pub fn set_state(&mut self, state: State) {
        if self.state == Some(state) {
            return;
        }
        // `Audio_SetState`.
        match state {
            State::Traffic => {
                self.stop_music();
                self.stop_songs();
            }
            State::Racing => self.enable_voicelines(true),
            State::PreLastLap | State::LastLap => self.enable_voicelines(false),
            State::FinalLap => {
                self.lower_music();
                self.play_xa(Category::Music, FINAL_LAP_XA);
            }
            State::PostLastLap => {
                self.raise_music();
                self.enable_voicelines(true);
            }
            State::RaceEnd => {
                self.fade_out_music();
                self.play_xa(Category::Music, self.race_end_xa);
            }
        }
        self.state = Some(state);
    }

    /// `Audio_Update1`: a frame of the game in the state the race is in. (The win's sound, which
    /// it plays as the leader finishes first, is the listener's.)
    pub fn update_state(&mut self, race: &RaceAudio) {
        let Some(state) = self.state else {
            return;
        };
        let (lap, laps) = (race.lap as i32, race.laps as i32);
        let near = race.to_finish < NEAR_FINISH;
        match state {
            State::Traffic => {
                if race.lights_out {
                    self.set_state(State::Racing);
                }
            }
            State::Racing => {
                self.set_mask_song(race, 0);
                self.update_voicelines();
                if laps > 2 && lap == laps - 2 && near {
                    self.set_state(State::PreLastLap);
                }
            }
            State::PreLastLap => {
                self.set_mask_song(race, 0);
                if lap == laps - 1 {
                    self.set_state(State::FinalLap);
                }
            }
            // Up again a second into "FINAL LAP!", and on as soon as it is over.
            State::FinalLap => {
                let mut tempo = 0;
                if self.xa.offset > FINAL_LAP_HEARD {
                    self.raise_music();
                    tempo = LAST_LAP_TEMPO;
                }
                self.set_mask_song(race, tempo);
                if self.xa_state() == XaState::Idle {
                    self.set_state(State::PostLastLap);
                }
            }
            State::PostLastLap => {
                self.set_mask_song(race, LAST_LAP_TEMPO);
                self.update_voicelines();
                if lap == laps - 1 && near {
                    self.set_state(State::LastLap);
                }
            }
            State::LastLap => {
                if race.finished {
                    self.race_end_xa = if race.first { WIN_XA } else { LOSS_XA };
                    self.set_state(State::RaceEnd);
                }
            }
            // As soon as the race's end is over.
            State::RaceEnd => {
                if self.xa_state() == XaState::Idle {
                    self.set_mask_song(race, 0);
                }
            }
        }
    }

    /// `Audio_SetMaskSong`: the song of the mask that is out plays, or the level's, at `tempo`
    /// beats a minute faster than its own.
    fn set_mask_song(&mut self, race: &RaceAudio, tempo: i32) {
        self.play_music(race.mask_song.unwrap_or(LEVEL_SONG), tempo);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::music::{AKU_SONG, test_howl};

    /// A race of three laps, our kart far from the line on lap `lap`.
    fn on_lap(lap: u8) -> RaceAudio {
        RaceAudio {
            lights_out: true,
            laps: 3,
            lap,
            to_finish: 50_000,
            ..RaceAudio::default()
        }
    }

    fn racing() -> Howl {
        let mut howl = test_howl();
        howl.set_state(State::Traffic);
        howl.update_state(&on_lap(0));
        howl.update_state(&on_lap(0));
        howl
    }

    #[test]
    fn the_music_is_silent_at_the_lights_and_starts_after_the_green() {
        let mut howl = test_howl();
        howl.play_music(LEVEL_SONG, 0);
        howl.set_state(State::Traffic);
        assert_eq!(howl.music_playing(), None);
        let lights = RaceAudio {
            lights_out: false,
            ..on_lap(0)
        };
        howl.update_state(&lights);
        assert_eq!(howl.state, Some(State::Traffic));
        assert!(!howl.voicelines.enabled);
        // The green: the drivers speak, and from the next frame the level's song plays.
        howl.update_state(&on_lap(0));
        assert_eq!(howl.state, Some(State::Racing));
        assert!(howl.voicelines.enabled);
        assert_eq!(howl.music_playing(), None);
        howl.update_state(&on_lap(0));
        assert_eq!(howl.music_playing(), Some((LEVEL_SONG, 60)));
    }

    #[test]
    fn the_last_lap_fades_the_music_down_and_up_and_plays_it_faster() {
        let mut howl = racing();
        let near = |lap| RaceAudio {
            to_finish: 8_000,
            ..on_lap(lap)
        };
        howl.update_state(&on_lap(1));
        assert_eq!(howl.state, Some(State::Racing));
        howl.update_state(&near(1));
        assert_eq!(howl.state, Some(State::PreLastLap));
        assert!(!howl.voicelines.enabled);
        howl.update_state(&on_lap(2));
        assert_eq!(howl.state, Some(State::FinalLap));
        assert_eq!(howl.music_volume(), Some((190, 90)));
        // Without the XA, "FINAL LAP!" is not heard: up again at once, and 20 beats a minute
        // faster.
        howl.update_state(&on_lap(2));
        assert_eq!(howl.state, Some(State::PostLastLap));
        assert_eq!(howl.music_volume(), Some((190, 190)));
        assert!(howl.voicelines.enabled);
        howl.update_state(&on_lap(2));
        assert_eq!(howl.music_playing(), Some((LEVEL_SONG, 80)));
        howl.update_state(&near(2));
        assert_eq!(howl.state, Some(State::LastLap));
        assert!(!howl.voicelines.enabled);
    }

    /// Renders `frames` of output, and a frame of the game after them.
    fn play(howl: &mut Howl, frames: usize) {
        howl.render(&mut vec![[0; 2]; frames]);
        howl.update_xa();
    }

    /// The XA asked for, read: `seconds` of a tone.
    fn read(howl: &mut Howl, seconds: usize) {
        let request = howl.take_xa_request().unwrap();
        howl.start_xa(request.serial, Some(crate::xa::tests::tone(seconds)));
    }

    #[test]
    fn final_lap_is_heard_and_the_last_lap_waits_for_it() {
        let mut howl = racing();
        howl.xnf = crate::xa::tests::xnf(100);
        howl.update_state(&RaceAudio {
            to_finish: 8_000,
            ..on_lap(1)
        });
        howl.update_state(&on_lap(2));
        assert_eq!(howl.state, Some(State::FinalLap));
        assert_eq!(howl.xa_playing(), Some((Category::Music, FINAL_LAP_XA)));
        read(&mut howl, 2);
        howl.update_state(&on_lap(2));
        assert_eq!(howl.music_volume(), Some((190, 90)));
        // Down while it plays; a little over a second into it the music comes up again, and
        // plays faster.
        play(&mut howl, (FINAL_LAP_HEARD as usize + 1) * 256);
        howl.update_state(&on_lap(2));
        assert_eq!(howl.state, Some(State::FinalLap));
        assert_eq!(howl.music_volume(), Some((90, 190)));
        howl.update_state(&on_lap(2));
        assert_eq!(howl.music_playing(), Some((LEVEL_SONG, 80)));
        // Once it is over, the last lap.
        play(&mut howl, 2 * 44_100 - (FINAL_LAP_HEARD as usize + 1) * 256);
        howl.update_state(&on_lap(2));
        assert_eq!(howl.state, Some(State::PostLastLap));
    }

    #[test]
    fn the_end_of_the_race_is_a_win_or_a_loss_and_the_music_waits_for_it() {
        for (first, jingle) in [(true, WIN_XA), (false, LOSS_XA)] {
            let mut howl = racing();
            howl.xnf = crate::xa::tests::xnf(100);
            howl.state = Some(State::LastLap);
            let finished = RaceAudio {
                finished: true,
                first,
                ..on_lap(2)
            };
            howl.update_state(&finished);
            assert_eq!(howl.xa_playing(), Some((Category::Music, jingle)));
            read(&mut howl, 1);
            play(&mut howl, 40_000);
            howl.update_state(&finished);
            assert_eq!(howl.music_playing(), None);
            play(&mut howl, 4_100);
            howl.update_state(&finished);
            assert_eq!(howl.music_playing(), Some((LEVEL_SONG, 60)));
        }
    }

    #[test]
    fn a_mask_out_plays_its_song_and_then_the_levels_again() {
        let mut howl = racing();
        let masked = RaceAudio {
            mask_song: Some(AKU_SONG),
            ..on_lap(0)
        };
        howl.update_state(&masked);
        assert_eq!(howl.music_playing(), Some((AKU_SONG, 60)));
        howl.update_state(&on_lap(0));
        assert_eq!(howl.music_playing(), Some((LEVEL_SONG, 60)));
    }

    #[test]
    fn the_end_of_the_race_fades_the_music_out_and_plays_it_again() {
        let mut howl = racing();
        howl.state = Some(State::LastLap);
        let finished = RaceAudio {
            finished: true,
            ..on_lap(2)
        };
        howl.update_state(&finished);
        assert_eq!(howl.state, Some(State::RaceEnd));
        assert_eq!(howl.music_volume(), Some((190, 0)));
        // Once it is out (24 blanks), the level's song plays from its start.
        for _ in 0..24 {
            howl.vsync();
        }
        assert_eq!(howl.music_playing(), None);
        howl.update_state(&finished);
        assert_eq!(howl.music_playing(), Some((LEVEL_SONG, 60)));
    }
}
