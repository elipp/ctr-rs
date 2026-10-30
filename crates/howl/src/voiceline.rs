//! The voice lines (`game/HOWL/HOWL_Voiceline.c`): what a driver says when something happens to
//! it. A request (`Voiceline_RequestPlay`) either grunts at once, a sound effect of the
//! character's, or is queued to be spoken, a line of XA (`Voiceline_Update`,
//! `Voiceline_StartPlay`). A character says nothing for two seconds after its last, and a line
//! waits for the one before it to end, and for the CD to be free.
//!
//! A driver that goes the wrong way for a second is told so by its mask, Aku Aku or Uka Uka,
//! once until it has gone the right way for a second again.
//!
//! Not ported: the lines of a boss race.

use std::collections::VecDeque;

use loader::xa::Category;

use crate::xa::XaState;
use crate::{AntiSpam, Howl, Sfx};

/// `data.voiceID`: the set of lines a voice line picks from, by its number. Sets 0 and 1 have a
/// grunt that may stand in for the line.
const VOICE_SETS: [u8; 24] = [
    4, 1, 5, 2, 1, 1, 1, 3, 6, 6, 7, 7, 7, 7, 7, 4, 0, 0, 0, 0, 0, 8, 8, 8,
];
/// The lines in each set of a character (`data.voiceData`).
const SET_SIZES: [u32; 11] = [2, 2, 2, 2, 2, 2, 2, 2, 1, 1, 1];
/// The lines of a character, of every set: its XA are this many after the character's before it.
const LINES: u16 = 19;
/// The character whose lines are out of order: its first set is its sixth again.
const OUT_OF_ORDER: u8 = 10;
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
/// A line holds the next back for this many of the game's frames, and a fifth of its sectors
/// more if it plays.
const COOLDOWN: i16 = 30;
/// The XA of [`Category::Extra`] in which Aku Aku, and Uka Uka, say the driver goes the wrong
/// way.
const WRONG_WAY_AKU: u16 = 30;
const WRONG_WAY_UKA: u16 = 61;
/// How many of the game's frames the driver goes one way before it counts.
const SAME_WAY_FRAMES: u32 = 30;

/// A line waiting to be spoken (`struct VoicelineItem`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Line {
    voice: u8,
    character: u8,
    /// The other character it is about (`secondaryCharacterID`), 16 for none.
    other: u8,
    /// How loud it is said, 255 for the driver's own.
    volume: u8,
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
    /// `WrongWayDirection_bool`: the driver goes the wrong way, and `framesDrivingSameDirection`:
    /// the game's frames it has since it went the other.
    wrong_way: bool,
    same_way: u32,
    /// `boolCanPlayWrongWaySFX`: the mask may say it, the driver having gone the right way.
    can_say_wrong_way: bool,
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
    /// one). Nothing is said once the race is over (`end_of_race`). A grunt plays at `sfx`, and
    /// a line of XA at its volume (it has no pan: the CD's input is both sides alike); a driver
    /// heard from far away is quiet (this is not retail's, which says every driver's the same).
    pub fn request_voiceline(
        &mut self,
        voice: u8,
        character: u8,
        other: u8,
        end_of_race: bool,
        sfx: Sfx,
    ) {
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
                sfx,
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
            volume: sfx.volume,
        });
    }

    /// `UI_RenderFrame_Racing`'s part, once a frame of the game: which way the driver goes
    /// (`vehicle::lap::wrong_way`), and for how long.
    pub(crate) fn track_direction(&mut self, wrong_way: bool) {
        let lines = &mut self.voicelines;
        if lines.wrong_way != wrong_way {
            lines.same_way = 0;
            lines.wrong_way = wrong_way;
        }
        lines.same_way += 1;
    }

    /// The racing begins, or goes on after "FINAL LAP!" (`Audio_SetState`): the driver has gone
    /// no way yet.
    pub(crate) fn reset_direction(&mut self) {
        self.voicelines.wrong_way = false;
        self.voicelines.same_way = 0;
    }

    /// `Voiceline_Update`, once a frame of the game while racing: the mask says the driver goes
    /// the wrong way, or the newest line waiting starts, once the one before is over and the CD
    /// plays nothing. `good_guy` is for the driver's mask (`VehPickupItem_MaskBoolGoodGuy`).
    pub fn update_voicelines(&mut self, good_guy: bool) {
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
        if self.xa.state != XaState::Idle {
            return;
        }
        let lines = &mut self.voicelines;
        let settled = lines.same_way > SAME_WAY_FRAMES;
        if lines.can_say_wrong_way {
            if lines.wrong_way && settled {
                lines.can_say_wrong_way = false;
                let id = if good_guy {
                    WRONG_WAY_AKU
                } else {
                    WRONG_WAY_UKA
                };
                self.voicelines.cooldown = COOLDOWN;
                if self.play_xa(Category::Extra, id) {
                    self.voicelines.cooldown += (self.xa_length(Category::Extra, id) / 5) as i16;
                }
                return;
            }
        } else if !lines.wrong_way && settled {
            lines.can_say_wrong_way = true;
        }
        if let Some(line) = self.voicelines.lines.pop_front() {
            self.start_voiceline(line);
        }
    }

    /// `Voiceline_StartPlay`: one of the lines of the set the line is in, picked at random, plays.
    fn start_voiceline(&mut self, line: Line) {
        self.voicelines.speaking = line.character;
        let set = VOICE_SETS[line.voice as usize] as usize;
        let pick = self.next_rng() % SET_SIZES[set];
        let id = line_xa(line.character, set, pick as u16);
        self.voicelines.cooldown = COOLDOWN;
        if self.play_xa_at(Category::Game, id, line.volume) {
            self.voicelines.cooldown += (self.xa_length(Category::Game, id) / 5) as i16;
        }
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

/// `data.voiceData`: the XA of [`Category::Game`] of line `pick` of set `set` of `character`.
/// The first 8 sets have two lines, the others one.
fn line_xa(character: u8, set: usize, pick: u16) -> u16 {
    let line = if set < 8 {
        set as u16 * 2 + pick
    } else {
        16 + set as u16 - 8
    };
    match (character, line) {
        (OUT_OF_ORDER, 0 | 1) => line_xa(character, 5, line),
        _ => character as u16 * LINES + line,
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
        howl.xnf = crate::xa::tests::xnf(50);
        howl
    }

    /// Voice line 1 is in set 1, which has a grunt, and 0 in set 4, which has none.
    const GRUNTS_TOO: u8 = 1;
    const NO_GRUNT: u8 = 0;

    #[test]
    fn a_driver_grunts_when_lines_are_off_and_then_keeps_quiet() {
        let mut howl = howl();
        howl.request_voiceline(GRUNTS_TOO, 1, 16, false, Sfx::FULL);
        assert_eq!(howl.channels.taken().len(), 1);
        assert_eq!(howl.channels.stats(0).sound as u16, GRUNTS[1] + 1);
        howl.frame += QUIET_FRAMES;
        howl.request_voiceline(GRUNTS_TOO, 1, 16, false, Sfx::FULL);
        assert_eq!(howl.channels.taken().len(), 1);
        // Another character is not held back by it.
        howl.request_voiceline(GRUNTS_TOO, 0, 16, false, Sfx::FULL);
        assert_eq!(howl.channels.taken().len(), 2);
    }

    #[test]
    fn a_line_without_a_grunt_waits_in_the_queue_newest_first() {
        let mut howl = howl();
        howl.request_voiceline(NO_GRUNT, 3, 16, false, Sfx::FULL);
        assert!(howl.voicelines.lines.is_empty());
        howl.enable_voicelines(true);
        howl.request_voiceline(NO_GRUNT, 3, 16, false, Sfx::FULL);
        howl.request_voiceline(NO_GRUNT, 3, 16, false, Sfx::FULL);
        howl.request_voiceline(NO_GRUNT, 4, 16, false, Sfx::FULL);
        let waiting: Vec<u8> = howl.voicelines.lines.iter().map(|l| l.character).collect();
        assert_eq!(waiting, [4, 3]);
        // The newest starts, one of the two lines of set 4 of its character; the next waits for
        // its second and a fifth of its 50 sectors, and for the CD to be free.
        howl.update_voicelines(true);
        assert_eq!(howl.voicelines.speaking, 4);
        let (category, id) = howl.xa_playing().unwrap();
        assert_eq!(category, Category::Game);
        assert!([4 * 19 + 8, 4 * 19 + 9].contains(&id), "{id}");
        assert_eq!(howl.voicelines.cooldown, COOLDOWN + 10);
        for _ in 0..COOLDOWN + 10 {
            howl.update_voicelines(true);
        }
        assert_eq!(howl.voicelines.lines.len(), 1);
        howl.stop_xa();
        howl.update_voicelines(true);
        assert!(howl.voicelines.lines.is_empty());
        assert_eq!(howl.voicelines.speaking, 3);
    }

    /// `frames` of the game, the driver going one way.
    fn drive(howl: &mut Howl, wrong_way: bool, frames: u32, good_guy: bool) {
        for _ in 0..frames {
            howl.track_direction(wrong_way);
            howl.update_voicelines(good_guy);
        }
    }

    #[test]
    fn the_mask_says_the_driver_goes_the_wrong_way_once_until_it_turns_round() {
        let mut howl = howl();
        howl.enable_voicelines(true);
        // Not before the driver has gone the right way for a second.
        drive(&mut howl, true, 100, true);
        assert_eq!(howl.xa_playing(), None);
        drive(&mut howl, false, SAME_WAY_FRAMES + 1, true);
        drive(&mut howl, true, SAME_WAY_FRAMES, true);
        assert_eq!(howl.xa_playing(), None);
        drive(&mut howl, true, 1, true);
        assert_eq!(howl.xa_playing(), Some((Category::Extra, WRONG_WAY_AKU)));
        assert_eq!(howl.voicelines.cooldown, COOLDOWN + 10);
        // Once: the driver still going the wrong way, nothing more is said.
        howl.stop_xa();
        drive(&mut howl, true, 200, true);
        assert_eq!(howl.xa_playing(), None);
        // Uka Uka tells the others, after they have turned round and back.
        drive(&mut howl, false, SAME_WAY_FRAMES + 1, false);
        drive(&mut howl, true, SAME_WAY_FRAMES + 1, false);
        assert_eq!(howl.xa_playing(), Some((Category::Extra, WRONG_WAY_UKA)));
    }

    #[test]
    fn each_character_has_its_own_lines() {
        assert_eq!(line_xa(0, 0, 1), 1);
        assert_eq!(line_xa(1, 8, 0), 19 + 16);
        assert_eq!(line_xa(15, 10, 0), 303);
        // Character 10's first set is its sixth's lines again.
        assert_eq!(line_xa(10, 0, 0), 0xc8);
        assert_eq!(line_xa(10, 1, 0), 0xc0);
        assert_eq!(line_xa(10, 5, 1), 0xc9);
        assert_eq!(line_xa(10, 10, 0), 0xd0);
    }

    #[test]
    fn nothing_is_said_after_the_race() {
        let mut howl = howl();
        howl.enable_voicelines(true);
        howl.request_voiceline(GRUNTS_TOO, 1, 16, true, Sfx::FULL);
        howl.request_voiceline(NO_GRUNT, 1, 16, true, Sfx::FULL);
        assert!(howl.channels.taken().is_empty());
        assert!(howl.voicelines.lines.is_empty());
    }

    #[test]
    fn the_rng_is_the_games() {
        let mut howl = howl();
        assert_eq!(howl.next_rng(), 0xed86_2efd);
    }
}
