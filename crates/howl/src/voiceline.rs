//! The voice lines (`game/HOWL/HOWL_Voiceline.c`): what a driver says when something happens to
//! it. A request (`Voiceline_RequestPlay`) either grunts at once, a sound effect of the
//! character's, or is queued to be spoken, a line of XA (`Voiceline_Update`,
//! `Voiceline_StartPlay`). A character says nothing for two seconds after its last, and a line
//! waits for the one before it to end.
//!
//! XA is not played yet: a line taken from the queue says nothing and holds the next back for a
//! second, as when the CD cannot play it. Nor are the wrong-way lines of Aku Aku and Uka Uka
//! here, or the lines of a boss race.

use std::collections::VecDeque;

use crate::{AntiSpam, Howl, Sfx};

/// `data.voiceID`: the set of lines a voice line picks from, by its number. Sets 0 and 1 have a
/// grunt that may stand in for the line.
const VOICE_SETS: [u8; 24] = [
    4, 1, 5, 2, 1, 1, 1, 3, 6, 6, 7, 7, 7, 7, 7, 4, 0, 0, 0, 0, 0, 8, 8, 8,
];
/// The lines in each set of a character (`data.voiceData`).
const SET_SIZES: [u32; 11] = [2, 2, 2, 2, 2, 2, 2, 2, 1, 1, 1];
/// The grunts of sets 0 and 1: the sound effect of the first character, the others after it.
const GRUNTS: [u16; 2] = [0x1c, 0x2c];

const CHARACTERS: u8 = 16;
/// The lines that wait at most (`voicelinePool`).
const QUEUE: usize = 8;
/// A character says nothing within this many of the game's frames after its last.
const QUIET_FRAMES: u32 = 0x3c;
/// The lines from this number on are often left unsaid: three times in four, seven in eight once
/// the character has said one since its last lap.
const OFTEN_UNSAID: u8 = 8;
/// A line that does not play holds the next back for this many of the game's frames.
const COOLDOWN_UNPLAYED: i16 = 30;

/// A line waiting to be spoken (`struct VoicelineItem`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Line {
    voice: u8,
    character: u8,
    /// The other character it is about (`secondaryCharacterID`), 16 for none.
    other: u8,
}

/// What the voice lines keep.
#[derive(Clone, Debug, Default)]
pub struct Voicelines {
    /// `boolCanPlayVoicelines`: lines are queued and spoken (while racing).
    pub(crate) enabled: bool,
    /// `voicelineCooldown`: the game's frames until the next line may start.
    cooldown: i16,
    /// `timeSet1`: by character, the lines queued since its last lap.
    queued: [u32; CHARACTERS as usize],
    /// `timeSet2`: by character, the frame it last grunted on.
    grunted: [u32; CHARACTERS as usize],
    /// The newest first.
    lines: VecDeque<Line>,
    /// The character of the line started last.
    speaking: u8,
}

impl Howl {
    /// `audioRNG`'s next draw.
    pub(crate) fn next_rng(&mut self) -> u32 {
        self.rng = ((self.rng >> 3).wrapping_add(self.rng.wrapping_mul(0x2000_0000)))
            .wrapping_mul(5)
            .wrapping_add(1);
        self.rng
    }

    /// `Voiceline_RequestPlay`: `character` would say line `voice` (about `other`, 16 for no
    /// one). Nothing is said once the race is over (`end_of_race`).
    pub fn request_voiceline(&mut self, voice: u8, character: u8, other: u8, end_of_race: bool) {
        if voice >= VOICE_SETS.len() as u8
            || character >= CHARACTERS
            || other > CHARACTERS
            || end_of_race
        {
            return;
        }
        let c = character as usize;
        let set = VOICE_SETS[voice as usize];
        if voice >= OFTEN_UNSAID {
            let said = self.voicelines.queued[c] & 1 << voice != 0;
            let odds = if said { 7 } else { 3 };
            if self.next_rng() & odds != 0 {
                return;
            }
        }
        let lines = &self.voicelines;
        let elapsed = self.frame.wrapping_sub(lines.grunted[c]);
        let mut grunt = elapsed > QUIET_FRAMES && set < 2;
        let busy = lines.cooldown != 0 && lines.speaking == character;
        let queue = lines.enabled && !busy && elapsed >= QUIET_FRAMES;
        if queue && grunt {
            grunt = self.next_rng() & 1 != 0;
        } else if !queue && !grunt {
            return;
        }
        if grunt {
            self.play(
                GRUNTS[set as usize] + character as u16,
                AntiSpam::Reject,
                Sfx::FULL,
            );
            self.voicelines.grunted[c] = self.frame;
            return;
        }

        let lines = &mut self.voicelines;
        lines.queued[c] |= 1 << voice;
        let waiting = lines
            .lines
            .iter()
            .any(|line| line.voice == voice && line.character == character);
        if waiting {
            return;
        }
        // With the queue full the oldest gives way.
        if lines.lines.len() == QUEUE {
            lines.lines.pop_back();
        }
        lines.lines.push_front(Line {
            voice,
            character,
            other,
        });
    }

    /// `Voiceline_Update`, once a frame of the game while racing: the newest line waiting starts
    /// once the one before is over.
    pub fn update_voicelines(&mut self) {
        let lines = &mut self.voicelines;
        if !lines.enabled {
            return;
        }
        if lines.cooldown != 0 {
            lines.cooldown -= 1;
            if lines.cooldown != 0 {
                return;
            }
        }
        if let Some(line) = lines.lines.pop_front() {
            self.start_voiceline(line);
        }
    }

    /// `Voiceline_StartPlay`: one of the lines of the set the line is in, picked at random.
    fn start_voiceline(&mut self, line: Line) {
        self.voicelines.speaking = line.character;
        let set = VOICE_SETS[line.voice as usize] as usize;
        // Which of the set's lines: the XA to play, once it is played.
        let _pick = self.next_rng() % SET_SIZES[set];
        self.voicelines.cooldown = COOLDOWN_UNPLAYED;
    }

    /// `Voiceline_ToggleEnable`: lines are queued and spoken, or not, and those waiting go.
    pub fn enable_voicelines(&mut self, enabled: bool) {
        if !enabled {
            self.voicelines.cooldown = 0;
            self.voicelines.lines.clear();
        }
        self.voicelines.enabled = enabled;
    }

    /// `Voiceline_ClearTimeStamp`, as the leading player starts a lap: every character may say
    /// anything again.
    pub fn clear_voiceline_times(&mut self) {
        self.voicelines.queued = [0; CHARACTERS as usize];
        self.voicelines.grunted = [0; CHARACTERS as usize];
    }

    /// `Voiceline_PoolClear`, for a new race.
    pub fn clear_voicelines(&mut self) {
        self.voicelines = Voicelines::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use loader::howl::OtherFx;

    /// The grunts of the first two characters, of sets 0 and 1.
    fn howl() -> Howl {
        let fx = OtherFx {
            flags: 0,
            volume: 255,
            pitch: 0x1000,
            sample: 0,
            duration: 300,
        };
        let samples = vec![Some(vec![0u8; 64].into_boxed_slice())];
        let mut howl = Howl::new(vec![fx; 0x2e], Vec::new(), Vec::new(), samples);
        howl.frame = 1000;
        howl
    }

    /// Voice line 1 is in set 1, which has a grunt, and 0 in set 4, which has none.
    const GRUNTS_TOO: u8 = 1;
    const NO_GRUNT: u8 = 0;

    #[test]
    fn a_driver_grunts_when_lines_are_off_and_then_keeps_quiet() {
        let mut howl = howl();
        howl.request_voiceline(GRUNTS_TOO, 1, 16, false);
        assert_eq!(howl.channels.taken().len(), 1);
        assert_eq!(howl.channels.stats(0).sound as u16, GRUNTS[1] + 1);
        howl.frame += QUIET_FRAMES;
        howl.request_voiceline(GRUNTS_TOO, 1, 16, false);
        assert_eq!(howl.channels.taken().len(), 1);
        // Another character is not held back by it.
        howl.request_voiceline(GRUNTS_TOO, 0, 16, false);
        assert_eq!(howl.channels.taken().len(), 2);
    }

    #[test]
    fn a_line_without_a_grunt_waits_in_the_queue_newest_first() {
        let mut howl = howl();
        howl.request_voiceline(NO_GRUNT, 3, 16, false);
        assert!(howl.voicelines.lines.is_empty());
        howl.enable_voicelines(true);
        howl.request_voiceline(NO_GRUNT, 3, 16, false);
        howl.request_voiceline(NO_GRUNT, 3, 16, false);
        howl.request_voiceline(NO_GRUNT, 4, 16, false);
        let waiting: Vec<u8> = howl.voicelines.lines.iter().map(|l| l.character).collect();
        assert_eq!(waiting, [4, 3]);
        // The newest starts; the next waits for its second.
        howl.update_voicelines();
        assert_eq!(howl.voicelines.speaking, 4);
        for _ in 0..COOLDOWN_UNPLAYED - 1 {
            howl.update_voicelines();
        }
        assert_eq!(howl.voicelines.lines.len(), 1);
        howl.update_voicelines();
        assert!(howl.voicelines.lines.is_empty());
        assert_eq!(howl.voicelines.speaking, 3);
    }

    #[test]
    fn nothing_is_said_after_the_race() {
        let mut howl = howl();
        howl.enable_voicelines(true);
        howl.request_voiceline(GRUNTS_TOO, 1, 16, true);
        howl.request_voiceline(NO_GRUNT, 1, 16, true);
        assert!(howl.channels.taken().is_empty());
        assert!(howl.voicelines.lines.is_empty());
    }

    #[test]
    fn the_rng_is_the_games() {
        let mut howl = howl();
        assert_eq!(howl.next_rng(), 0xed86_2efd);
    }
}
