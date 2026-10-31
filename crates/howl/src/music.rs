//! The music (`game/HOWL/HOWL_CseqMusic.c`, `HOWL_SongPool.c`, `HOWL_CseqOpcode.c` and
//! `Channel_ParseSongToChannels`), and what the game asks of it (`HOWL_Music.c`).
//!
//! A level loads a song pack ([`Cseq`]): its own song, Aku Aku's and Uka Uka's. Up to two songs
//! play at once, each of sequences of notes (the two share 24 sequences). A note of a sequence
//! plays on a channel of its own until a note-off of its pitch (or drum) ends it, so the music
//! takes channels from the sound effects. Every vertical blank the songs go on by their tempo
//! (`bpm × tpqn / 60` ticks a second, in 65536ths of a tick a blank), each sequence plays the
//! notes that are due, and the volumes that fade take a step.

use loader::howl::Cseq;

use crate::channel::{self, AD, Attr, CHANNELS, Kind, SR, flags, update};
use crate::fx::NO_DISTORTION;
use crate::tables::{
    BOSS_SONG, CREDITS_SONG, DISTORT_MUSIC, DISTORT_OTHER_FX, INTRO_RACE_SONG, LEVEL_SONGS,
    MAIN_MENU_SONG, NAUGHTY_DOG_CRATE_SONG, NOTE_FREQUENCY, OXIDE_ENDING_SONG,
    OXIDE_TRUE_ENDING_SONG,
};
use crate::{CENTRE, Howl};

/// The songs of a pack (`CseqSong`): the level's, and the two of the masks.
pub const LEVEL_SONG: u16 = 0;
pub const AKU_SONG: u16 = 1;
pub const UKA_SONG: u16 = 2;

/// Songs that play at once (`songPool`), and the sequences they play (`songSeq`).
const SONGS: usize = 2;
const SEQUENCES: usize = CHANNELS;

/// The vertical blanks a second, which the tempo is a share of.
const BLANKS_PER_SECOND: u32 = 60;

/// The volume a song starts at: a mask's at full, the others' lower (`SongPool_Start`).
const MASK_SONG_VOLUME: u8 = 255;
const SONG_VOLUME: u8 = 190;
/// `Music_LowerVolume` and `Music_RaiseVolume`, for "FINAL LAP!": a song's volume down and up
/// again, a mask's and the others', 8 a vertical blank.
const LOWER_MASK_SONG: u8 = 150;
const LOWER_SONG: u8 = 90;
const FADE_STEP: u8 = 8;

/// The bits of [`Sequence::flags`].
mod seq_flags {
    pub const PLAYING: u8 = 0x1;
    pub const LOOPS: u8 = 0x2;
    pub const DRUMS: u8 = 0x4;
    /// The song ended: its next note is its first again.
    pub const RESTART: u8 = 0x8;
}

/// The bits of [`Song::flags`].
mod song_flags {
    pub const PLAYING: u8 = 0x1;
    pub const PAUSED: u8 = 0x2;
    /// It stops once its volume is down (`CseqMusic_Restart`).
    pub const STOPPING: u8 = 0x4;
}

/// The opcodes of the notes (`opcodeFunc`), and how many bytes each is with what it takes
/// (`opcodeOffset`). 0, 2 and 4 do nothing.
mod op {
    pub const NOTE_OFF: u8 = 1;
    pub const END: u8 = 3;
    pub const NOTE_ON: u8 = 5;
    pub const VOLUME: u8 = 6;
    pub const PAN: u8 = 7;
    pub const REVERB: u8 = 8;
    pub const INSTRUMENT: u8 = 9;
    pub const DISTORT: u8 = 10;
    pub const SIZES: [usize; 11] = [1, 2, 2, 1, 2, 3, 2, 2, 2, 2, 2];
}

/// A volume that goes towards another by a step each vertical blank (`vol_Curr`, `vol_New`,
/// `vol_StepRate`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Fade {
    now: u8,
    to: u8,
    step: u8,
}

impl Fade {
    fn at(volume: u8) -> Fade {
        Fade {
            now: volume,
            to: volume,
            step: 1,
        }
    }

    /// `SongPool_Volume`: towards `to` by `step`.
    fn set(&mut self, to: u8, step: u8) {
        self.to = to;
        self.step = step;
    }

    /// A step towards where it goes. `None` if it is there; whether the step went past it (and
    /// so stopped there), which a step that lands on it does not.
    fn run(&mut self) -> Option<bool> {
        if self.now == self.to {
            return None;
        }
        let (now, to, step) = (self.now as i32, self.to as i32, self.step as i32);
        let (stepped, past) = if now < to {
            (now + step, now + step > to)
        } else {
            (now - step, now - step < to)
        };
        self.now = if past { self.to } else { stepped as u8 };
        Some(past)
    }
}

/// `struct SongSeq`: a sequence of a song that plays.
#[derive(Clone, Copy, Debug, Default)]
struct Sequence {
    flags: u8,
    instrument: u8,
    /// What its notes start with; its opcode changes the notes that play, not this.
    reverb: u8,
    volume: Fade,
    distort: u8,
    pan: u8,
    /// Its song, in `songPool`.
    song: usize,
    /// Ticks from its last note to its next, and since its last (`NoteLength`,
    /// `NoteTimeElapsed`).
    length: u32,
    elapsed: u32,
    /// Where its notes start in the pack, and its next note's opcode.
    first: usize,
    at: usize,
}

/// `struct Song`: a song that plays.
#[derive(Clone, Debug, Default)]
struct Song {
    flags: u8,
    /// Its number in the pack.
    id: u16,
    tpqn: i16,
    bpm: i16,
    /// Ticks a vertical blank, in 65536ths, and the 65536ths left over from the last
    /// (`unk10`).
    tempo: u32,
    fraction: u32,
    volume: Fade,
    /// Its sequences, in `songSeq` (`CseqSequences`).
    sequences: Vec<usize>,
}

/// The music's state.
#[derive(Clone, Debug, Default)]
pub struct Music {
    /// Every song pack of `KART.HWL`, and the one the level loaded (`ptrCseqHeader`).
    packs: Vec<Cseq>,
    /// How many of them are the game's: a map's pack, if the level has one, is after them.
    retail: usize,
    pack: Option<usize>,
    songs: [Song; SONGS],
    sequences: [Sequence; SEQUENCES],
    /// What `HOWL_Music.c` plays (`cseqHighestIndex`, `cseqBoolPlay`), and the change of its
    /// tempo (`cseqTempo`).
    pub(crate) playing: Option<u16>,
    tempo: i32,
}

impl Music {
    pub fn new(packs: Vec<Cseq>) -> Music {
        Music {
            retail: packs.len(),
            packs,
            ..Music::default()
        }
    }

    fn cseq(&self) -> Option<&Cseq> {
        self.packs.get(self.pack?)
    }

    /// Whether the pack that is loaded has song `id`.
    fn has(&self, id: u16) -> bool {
        self.cseq()
            .is_some_and(|cseq| (id as usize) < cseq.songs.len())
    }

    /// The songs that play `id`, by their place in the pool.
    fn songs_of(&self, id: u16) -> Vec<usize> {
        (0..SONGS)
            .filter(|&i| self.songs[i].flags & song_flags::PLAYING != 0 && self.songs[i].id == id)
            .collect()
    }
}

/// `howl_GetNextNote`: the delay before the note at `at` (7 bits a byte, the top bit set on all
/// but the last), and where its opcode is.
fn next_note(data: &[u8], mut at: usize) -> (usize, u32) {
    let byte = |at: &mut usize| {
        let b = data.get(*at).copied().unwrap_or(0);
        *at += 1;
        b as u32
    };
    let first = byte(&mut at);
    let mut length = first;
    if first & 0x80 != 0 {
        length &= 0x7f;
        loop {
            let next = byte(&mut at);
            length = (length << 7).wrapping_add(next & 0x7f);
            if next & 0x80 == 0 {
                break;
            }
        }
    }
    (at, length)
}

/// `SongPool_CalculateTempo`: the ticks a vertical blank, in 65536ths.
fn tempo(tpqn: i16, bpm: i16) -> u32 {
    let per_second = (bpm as i32).wrapping_mul(tpqn as i32) as u32 / 60;
    (per_second << 16) / BLANKS_PER_SECOND
}

/// `howl_InstrumentPitch`: the pitch of note `note` of an instrument whose middle C is
/// `base`. A distortion's top two bits move the note by -2 to +1 semitones, the rest bend it up
/// within one.
pub fn instrument_pitch(base: u16, note: u8, distort: u8) -> u16 {
    let note = note as i32 + (distort >> 6) as i32 - 2;
    // A note past the table reads what follows it in retail.
    let frequency = usize::try_from(note)
        .ok()
        .and_then(|n| NOTE_FREQUENCY.get(n))
        .copied()
        .unwrap_or(0);
    let mut pitch = (frequency as u32).wrapping_mul(base as u32) >> 12;
    let bend = (distort & 0x3f) as usize;
    if bend != 0 {
        let scale = DISTORT_MUSIC[bend] + 0x10_0000;
        pitch = (pitch as u16 as u32).wrapping_mul(scale) >> 20;
    }
    pitch as u16
}

impl Howl {
    /// The song pack of the level `level` (a `LevelId`), or in a boss race the boss's (the
    /// part of `Music_AsyncParseBanks` that picks it, `howl_SetSong`, `howl_LoadSong`). What
    /// played stops. A level past [`LEVEL_SONGS`] without a pack of its own, a map's too, has
    /// none.
    pub fn set_level_music(&mut self, level: u32, boss: Option<usize>) {
        use loader::level::LevelId;
        let pack = match boss {
            Some(boss) => (boss < 6).then_some(BOSS_SONG),
            None => match LevelId(level) {
                LevelId(level) if (level as usize) < LEVEL_SONGS.len() => {
                    Some(LEVEL_SONGS[level as usize])
                }
                LevelId::MAIN_MENU_LEVEL => Some(MAIN_MENU_SONG),
                LevelId::INTRO_RACE_TODAY => Some(INTRO_RACE_SONG),
                LevelId::NAUGHTY_DOG_CRATE => Some(NAUGHTY_DOG_CRATE_SONG),
                LevelId::OXIDE_ENDING => Some(OXIDE_ENDING_SONG),
                LevelId::OXIDE_TRUE_ENDING => Some(OXIDE_TRUE_ENDING_SONG),
                LevelId::CREDITS_CRASH => Some(CREDITS_SONG),
                _ => None,
            },
        };
        self.stop_songs();
        self.end_music();
        self.music.pack = pack
            .map(usize::from)
            .filter(|&p| p < self.music.packs.len());
    }

    /// A level that loads, as [`Howl::set_level_reverb`] and [`Howl::set_level_music`] have it,
    /// except that a map can bring its own: it has the song pack `pack` where it gives one, the
    /// reverb mode `reverb`, and the ADPCM `samples` in place of the game's samples of those
    /// numbers; what it does not give is the game's for `level`. What an earlier map brought is
    /// put back first.
    pub fn set_level_audio(
        &mut self,
        level: u32,
        pack: Option<Cseq>,
        reverb: Option<u8>,
        samples: Vec<(u16, Box<[u8]>)>,
    ) {
        for (sample, adpcm) in std::mem::take(&mut self.replaced).into_iter().rev() {
            self.spu.replace_sample(sample, adpcm);
        }
        for (sample, adpcm) in samples {
            if let Some(old) = self.spu.replace_sample(sample, Some(adpcm)) {
                self.replaced.push((sample, old));
            }
        }
        self.set_level_reverb(level, None);
        if let Some(reverb) = reverb {
            self.set_reverb(reverb as u16);
        }
        self.set_level_music(level, None);
        self.music.packs.truncate(self.music.retail);
        if let Some(pack) = pack {
            self.music.packs.push(pack);
            self.music.pack = Some(self.music.packs.len() - 1);
        }
    }

    /// `Music_Adjust`: the song `id` of the pack plays, from its start if another played, and
    /// `tempo` beats a minute faster than its own.
    pub fn play_music(&mut self, id: u16, tempo: i32) {
        if let Some(playing) = self.music.playing {
            if playing != id {
                self.stop_song(playing);
            } else if self.music.tempo != tempo {
                self.change_tempo(id, tempo);
                self.music.tempo = tempo;
            }
        }
        if self.music.playing != Some(id) {
            self.start_song(id, tempo, true);
            self.music.playing = Some(id);
            self.music.tempo = tempo;
        }
    }

    /// `Music_LowerVolume`: what plays fades down for "FINAL LAP!".
    pub fn lower_music(&mut self) {
        if let Some(id) = self.music.playing {
            let to = if is_mask_song(id) {
                LOWER_MASK_SONG
            } else {
                LOWER_SONG
            };
            self.change_volume(id, to, FADE_STEP);
        }
    }

    /// `Music_RaiseVolume`: and up again after it.
    pub fn raise_music(&mut self) {
        if let Some(id) = self.music.playing {
            let to = if is_mask_song(id) {
                MASK_SONG_VOLUME
            } else {
                SONG_VOLUME
            };
            self.change_volume(id, to, FADE_STEP);
        }
    }

    /// `Music_Restart`: what plays fades out and stops (and so, asked to play again, plays from
    /// its start).
    pub fn fade_out_music(&mut self) {
        if let Some(id) = self.music.playing {
            self.fade_out(id, FADE_STEP);
        }
    }

    /// `Music_Stop`: what plays stops at once.
    pub fn stop_music(&mut self) {
        if let Some(id) = self.music.playing {
            self.stop_song(id);
            self.end_music();
        }
    }

    /// `Music_End`: nothing plays.
    fn end_music(&mut self) {
        self.music.playing = None;
    }

    /// `CseqMusic_Start`: song `id` of the pack starts on a free place of the pool, at
    /// `delta_bpm` beats a minute faster than its own, and when it ends from its start again if
    /// it `loops`. False if there is no such song or no free place.
    pub(crate) fn start_song(&mut self, id: u16, delta_bpm: i32, loops: bool) -> bool {
        if !self.music.has(id) {
            return false;
        }
        let Some(song) = (0..SONGS).find(|&i| self.music.songs[i].flags & song_flags::PLAYING == 0)
        else {
            return false;
        };
        self.start_in_pool(song, id, delta_bpm, loops);
        true
    }

    /// `SongPool_Start`.
    fn start_in_pool(&mut self, at: usize, id: u16, delta_bpm: i32, loops: bool) {
        let music = &mut self.music;
        let Some(cseq) = music.pack.and_then(|p| music.packs.get(p)) else {
            return;
        };
        let header = &cseq.songs[id as usize];
        let bpm = (header.bpm as u16 as i32).wrapping_add(delta_bpm) as i16;
        let volume = if is_mask_song(id) {
            MASK_SONG_VOLUME
        } else {
            SONG_VOLUME
        };
        let mut song = Song {
            flags: song_flags::PLAYING,
            id,
            tpqn: header.tpqn,
            bpm,
            tempo: tempo(header.tpqn, bpm),
            volume: Fade::at(volume),
            ..Song::default()
        };
        for sequence in &header.sequences {
            let Some(free) = music
                .sequences
                .iter()
                .position(|s| s.flags & seq_flags::PLAYING == 0)
            else {
                continue;
            };
            let mut flags = seq_flags::PLAYING;
            if sequence.drums {
                flags |= seq_flags::DRUMS;
            }
            if loops {
                flags |= seq_flags::LOOPS;
            }
            let (next, length) = next_note(&cseq.data, sequence.notes);
            music.sequences[free] = Sequence {
                flags,
                instrument: 0,
                reverb: 0,
                volume: Fade::at(u8::MAX),
                distort: NO_DISTORTION,
                pan: CENTRE,
                song: at,
                length,
                elapsed: 0,
                first: sequence.notes,
                at: next,
            };
            song.sequences.push(free);
        }
        music.songs[at] = song;
    }

    /// `CseqMusic_Pause`, `CseqMusic_Resume`: the songs that play hold where they are, or go on.
    pub(crate) fn pause_songs(&mut self, paused: bool) {
        if self.music.cseq().is_none() {
            return;
        }
        for song in &mut self.music.songs {
            if song.flags & song_flags::PLAYING != 0 {
                song.flags &= !song_flags::PAUSED;
                if paused {
                    song.flags |= song_flags::PAUSED;
                }
            }
        }
    }

    /// `CseqMusic_ChangeVolume`.
    fn change_volume(&mut self, id: u16, volume: u8, step: u8) {
        if !self.music.has(id) {
            return;
        }
        for song in self.music.songs_of(id) {
            self.music.songs[song].volume.set(volume, step);
        }
    }

    /// `CseqMusic_Restart`: song `id` fades out by `step` a vertical blank, and then stops.
    fn fade_out(&mut self, id: u16, step: u8) {
        if !self.music.has(id) {
            return;
        }
        for song in self.music.songs_of(id) {
            let song = &mut self.music.songs[song];
            song.flags |= song_flags::STOPPING;
            song.volume.set(0, step);
        }
    }

    /// `CseqMusic_ChangeTempo`, `SongPool_ChangeTempo`: song `id` at `delta_bpm` beats a minute
    /// faster than its own.
    fn change_tempo(&mut self, id: u16, delta_bpm: i32) {
        let Some(cseq) = self.music.cseq().filter(|_| self.music.has(id)) else {
            return;
        };
        let own = cseq.songs[id as usize].bpm;
        for song in self.music.songs_of(id) {
            let song = &mut self.music.songs[song];
            song.bpm = (own as u16 as i32).wrapping_add(delta_bpm) as i16;
            song.tempo = tempo(song.tpqn, song.bpm);
        }
    }

    /// `CseqMusic_Stop`.
    pub(crate) fn stop_song(&mut self, id: u16) {
        if !self.music.has(id) {
            return;
        }
        for song in self.music.songs_of(id) {
            self.stop_in_pool(song);
        }
    }

    /// `CseqMusic_StopAll`: every song stops (what `HOWL_Music.c` plays is kept).
    pub fn stop_songs(&mut self) {
        if self.music.cseq().is_none() {
            return;
        }
        for song in 0..SONGS {
            self.stop_in_pool(song);
        }
    }

    /// `SongPool_StopAllCseq`: the song in place `at` of the pool stops, and its notes with it.
    fn stop_in_pool(&mut self, at: usize) {
        if self.music.songs[at].flags & song_flags::PLAYING == 0 {
            return;
        }
        for sequence in self.music.songs[at].sequences.clone() {
            self.stop_sequence(sequence);
        }
        self.music.songs[at].flags &= !song_flags::PLAYING;
    }

    /// `SongPool_StopCseq`.
    fn stop_sequence(&mut self, sequence: usize) {
        for channel in self.notes_of(sequence) {
            self.channels.destroy(channel);
        }
        self.music.sequences[sequence].flags &= !seq_flags::PLAYING;
    }

    /// The channels that play the notes of `sequence`.
    fn notes_of(&self, sequence: usize) -> Vec<usize> {
        let channels = &self.channels;
        let of = |&c: &usize| {
            let stats = channels.stats(c);
            stats.kind == Kind::Music && stats.sound == sequence as u32
        };
        channels.taken().iter().copied().filter(of).collect()
    }

    /// `Channel_ParseSongToChannels`: a vertical blank of the songs that play and are not
    /// paused. Each goes on by its tempo, and its sequences play the notes that came due; the
    /// volumes that fade take a step, and the notes that play take their new volume.
    pub(crate) fn run_songs(&mut self) {
        if self.music.cseq().is_none() {
            return;
        }
        let mut faded = false;
        for at in 0..SONGS {
            let song = &mut self.music.songs[at];
            if song.flags & song_flags::PLAYING == 0 || song.flags & song_flags::PAUSED != 0 {
                continue;
            }
            let total = song.fraction.wrapping_add(song.tempo);
            let ticks = total >> 16;
            song.fraction = total & 0xffff;
            if let Some(past) = song.volume.run() {
                faded = true;
                if past && song.flags & song_flags::STOPPING != 0 {
                    self.stop_in_pool(at);
                    self.end_music();
                    self.music.songs[at].flags &= !song_flags::STOPPING;
                }
            }
            for sequence in self.music.songs[at].sequences.clone() {
                faded |= self.music.sequences[sequence].volume.run().is_some();
                self.run_sequence(sequence, ticks);
            }
        }
        if faded {
            self.set_music_volumes();
        }
    }

    /// The notes of `sequence` that `ticks` more bring due. An opcode that is not one holds the
    /// sequence where it is, as in retail (where one after no delay hangs the game).
    fn run_sequence(&mut self, sequence: usize, ticks: u32) {
        let Some(pack) = self.music.pack else {
            return;
        };
        let seq = &mut self.music.sequences[sequence];
        if seq.flags & seq_flags::PLAYING == 0 {
            return;
        }
        seq.elapsed = seq.elapsed.wrapping_add(ticks);
        loop {
            let seq = self.music.sequences[sequence];
            if seq.length > seq.elapsed || seq.flags & seq_flags::PLAYING == 0 {
                return;
            }
            self.music.sequences[sequence].elapsed -= seq.length;
            let data = &self.music.packs[pack].data;
            let Some(&opcode) = data.get(seq.at) else {
                return;
            };
            if opcode as usize >= op::SIZES.len() {
                match seq.length {
                    0 => return,
                    _ => continue,
                }
            }
            let arg = |n: usize| data.get(seq.at + n).copied().unwrap_or(0);
            let args = [arg(1), arg(2)];
            self.opcode(sequence, opcode, args);
            let pack_data = &self.music.packs[pack].data;
            let seq = &mut self.music.sequences[sequence];
            if seq.flags & seq_flags::PLAYING == 0 {
                continue;
            }
            let note = if seq.flags & seq_flags::RESTART != 0 {
                seq.flags &= !seq_flags::RESTART;
                seq.first
            } else {
                seq.at + op::SIZES[opcode as usize]
            };
            (seq.at, seq.length) = next_note(pack_data, note);
        }
    }

    /// `opcodeFunc`: what an opcode of `sequence` does, with the two bytes after it.
    fn opcode(&mut self, sequence: usize, opcode: u8, [first, second]: [u8; 2]) {
        let seq = &mut self.music.sequences[sequence];
        match opcode {
            // `cseq_opcode01_noteoff`: the note of this pitch (or drum) ends.
            op::NOTE_OFF => {
                for channel in self.notes_of(sequence) {
                    if self.channels.stats(channel).note == first {
                        self.channels.destroy(channel);
                    }
                }
            }
            // `cseq_opcode03`: the end of the song, which starts again if it loops.
            op::END => {
                if seq.flags & seq_flags::LOOPS != 0 {
                    seq.flags |= seq_flags::RESTART;
                } else {
                    let song = seq.song;
                    self.stop_in_pool(song);
                }
            }
            // `cseq_opcode05_noteon`: a note of this pitch (or drum) at this volume.
            op::NOTE_ON => self.note_on(sequence, first, second),
            // `cseq_opcode06`, `cseq_opcode07`: the sequence's volume, its pan.
            op::VOLUME | op::PAN => {
                if opcode == op::VOLUME {
                    seq.volume.now = first;
                } else {
                    seq.pan = first;
                }
                self.set_sequence_volume(sequence);
            }
            // `cseq_opcode08`: the notes that play echo or not.
            op::REVERB => {
                for channel in self.notes_of(sequence) {
                    self.channels.attr_mut(channel, update::REVERB).reverb = first as u16;
                }
            }
            // `cseq_opcode09`: the instrument of the notes to come.
            op::INSTRUMENT => seq.instrument = first,
            // `cseq_opcode0a`: the distortion of the notes, those that play too.
            op::DISTORT => {
                seq.distort = first;
                for channel in self.notes_of(sequence) {
                    let note = self.channels.stats(channel).note;
                    if let Some(pitch) = self.note_pitch(sequence, note) {
                        self.channels.attr_mut(channel, update::PITCH).pitch = pitch;
                    }
                }
            }
            _ => {}
        }
    }

    /// `cseq_opcode05_noteon`: note `note` of `sequence` (its pitch, or which drum) starts at
    /// volume `volume` on a channel of its own. Not while the music, the song or the sequence
    /// is silent.
    fn note_on(&mut self, sequence: usize, note: u8, volume: u8) {
        let seq = self.music.sequences[sequence];
        if self.volume_music == 0 || seq.volume.now == 0 {
            return;
        }
        if self.music.songs[seq.song].volume.now == 0 {
            return;
        }
        let Some(attr) = self.music_attr(sequence, note, volume) else {
            return;
        };
        let Some(channel) = self.channels.alloc(update::ALL_ATTRS, &attr) else {
            return;
        };
        let stats = self.channels.stats_mut(channel);
        stats.flags |= flags::LOOPS;
        stats.kind = Kind::Music;
        stats.echo = seq.reverb;
        stats.vol = volume;
        stats.distort = seq.distort;
        stats.lr = seq.pan;
        stats.note = note;
        stats.time_left = 0;
        stats.sound = sequence as u32;
    }

    /// `howl_InitChannelAttr_Music`: what the voice of note `note` of `sequence` at `volume`
    /// plays. `None` for a drum or an instrument the pack does not have (in retail, whatever
    /// follows the pack's).
    pub(crate) fn music_attr(&self, sequence: usize, note: u8, volume: u8) -> Option<Attr> {
        let cseq = self.music.cseq()?;
        let seq = &self.music.sequences[sequence];
        let (sample, ad, sr) = if seq.flags & seq_flags::DRUMS != 0 {
            let drum = cseq.drums.get(note as usize)?;
            (drum.sample, AD, SR)
        } else {
            let instrument = cseq.instruments.get(seq.instrument as usize)?;
            (instrument.sample, instrument.ad, instrument.sr)
        };
        Some(Attr {
            sample,
            ad,
            sr,
            pitch: self.note_pitch(sequence, note)?,
            reverb: seq.reverb as u16,
            volume: self.note_volume(sequence, note, volume)?,
        })
    }

    /// The pitch of note `note` of `sequence`: a drum's own, distorted as a sound effect is, or
    /// the note of its instrument ([`instrument_pitch`]).
    fn note_pitch(&self, sequence: usize, note: u8) -> Option<u16> {
        let cseq = self.music.cseq()?;
        let seq = &self.music.sequences[sequence];
        if seq.flags & seq_flags::DRUMS != 0 {
            let pitch = cseq.drums.get(note as usize)?.pitch;
            Some(if seq.distort == NO_DISTORTION {
                pitch
            } else {
                ((pitch as u32).wrapping_mul(DISTORT_OTHER_FX[seq.distort as usize]) >> 16) as u16
            })
        } else {
            let instrument = cseq.instruments.get(seq.instrument as usize)?;
            Some(instrument_pitch(instrument.base_pitch, note, seq.distort))
        }
    }

    /// `UpdateChannelVol_Music`: the volume of note `note` of `sequence` at `volume`, at the
    /// music's, the song's, the sequence's and the drum's or instrument's volume, and the
    /// sequence's pan.
    fn note_volume(&self, sequence: usize, note: u8, volume: u8) -> Option<[i16; 2]> {
        let cseq = self.music.cseq()?;
        let seq = &self.music.sequences[sequence];
        let of = if seq.flags & seq_flags::DRUMS != 0 {
            cseq.drums.get(note as usize)?.volume
        } else {
            cseq.instruments.get(seq.instrument as usize)?.volume
        };
        let song = self.music.songs[seq.song].volume.now as u32;
        let mix = (self.volume_music as u32 * song * seq.volume.now as u32) >> 10;
        let gain = mix * of as u32 * volume as u32;
        Some(channel::volume(gain >> 15, seq.pan, self.stereo))
    }

    /// `cseq_opcode_from06and07`: the notes of `sequence` that play take its volume and pan
    /// (without the drum's or instrument's volume, as in retail).
    fn set_sequence_volume(&mut self, sequence: usize) {
        let seq = self.music.sequences[sequence];
        let song = self.music.songs[seq.song].volume.now as i32;
        let gain = (self.volume_music as i32) * song * seq.volume.now as i32;
        for channel in self.notes_of(sequence) {
            let of = self.channels.stats(channel).vol as i32;
            let volume = gain.wrapping_mul(of) >> 18;
            let volume = channel::volume(volume as u32, seq.pan, self.stereo);
            self.channels.attr_mut(channel, update::VOLUME).volume = volume;
        }
    }

    /// `UpdateChannelVol_Music_All`: every note that plays at the volumes there are now.
    pub(crate) fn set_music_volumes(&mut self) {
        for channel in self.channels.taken().to_vec() {
            let stats = *self.channels.stats(channel);
            if stats.kind != Kind::Music {
                continue;
            }
            let Some(volume) = self.note_volume(stats.sound as usize, stats.note, stats.vol) else {
                continue;
            };
            self.channels.attr_mut(channel, update::VOLUME).volume = volume;
        }
    }
}

/// Whether song `id` of a pack is a mask's, which plays louder.
fn is_mask_song(id: u16) -> bool {
    matches!(id, AKU_SONG | UKA_SONG)
}

/// A sound engine with one song pack ([`LEVEL_SONGS`]'s first): an instrument at the pitch of the
/// rate (sample 0, a loud square wave that loops), and three songs of one sequence at a tick a
/// vertical blank, which play middle C at 100 and end it 10 ticks later, then end 5 after that.
#[cfg(test)]
pub(crate) fn test_howl() -> Howl {
    use loader::howl::{Instrument, Sequence, Song as Header, block_flags};

    let mut adpcm = vec![];
    for i in 0..1575 {
        let mut block = [if i % 2 == 0 { 0x77 } else { 0x99 }; 16];
        block[0] = 0;
        block[1] = match i {
            0 => block_flags::LOOP_START,
            1574 => block_flags::END | block_flags::REPEAT,
            _ => 0,
        };
        adpcm.extend(block);
    }
    let notes = [0, op::NOTE_ON, 60, 100, 10, op::NOTE_OFF, 60, 5, op::END];
    let song = Header {
        bpm: 60,
        tpqn: 60,
        sequences: vec![Sequence {
            drums: false,
            notes: 0,
        }],
    };
    let cseq = Cseq {
        instruments: vec![Instrument {
            volume: 255,
            base_pitch: 0x1000,
            sample: 0,
            ad: AD,
            sr: SR,
        }],
        drums: vec![],
        songs: vec![song.clone(), song.clone(), song],
        data: notes.to_vec(),
    };
    let mut howl = Howl::new(
        vec![],
        vec![],
        vec![cseq],
        vec![Some(adpcm.into_boxed_slice())],
    );
    howl.set_level_music(0, None);
    howl
}

#[cfg(test)]
impl Howl {
    /// What `HOWL_Music.c` plays, and at how many beats a minute.
    pub(crate) fn music_playing(&self) -> Option<(u16, i16)> {
        let id = self.music.playing?;
        let song = self
            .music
            .songs
            .iter()
            .find(|s| s.flags & song_flags::PLAYING != 0)?;
        Some((id, song.bpm))
    }

    /// The volume of the song that plays, and where it goes.
    pub(crate) fn music_volume(&self) -> Option<(u8, u8)> {
        let song = self
            .music
            .songs
            .iter()
            .find(|s| s.flags & song_flags::PLAYING != 0)?;
        Some((song.volume.now, song.volume.to))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The notes that play, as (sequence, note, pitch, volume of the left).
    fn notes(howl: &Howl) -> Vec<(u32, u8, u16, i16)> {
        let channels = &howl.channels;
        let of = |&c: &usize| {
            let stats = channels.stats(c);
            let attr = channels.new_attr(c);
            (stats.kind == Kind::Music).then_some((
                stats.sound,
                stats.note,
                attr.pitch,
                attr.volume[0],
            ))
        };
        channels.taken().iter().filter_map(of).collect()
    }

    /// The left of a note at 100 of the instrument at 255, in a song at `song`: the music's 175
    /// × the song's × the sequence's 255 / 1024, × 255 × 100 / 32768, 179/256 of it.
    fn left(song: u32) -> i16 {
        let mix = (175 * song * 255) >> 10;
        let volume = (mix * 255 * 100) >> 15;
        ((volume * 179) >> 8) as i16
    }

    #[test]
    fn a_note_plays_from_its_note_on_to_its_note_off_and_the_song_loops() {
        let mut howl = test_howl();
        howl.play_music(LEVEL_SONG, 0);
        let mut heard = vec![];
        for _ in 0..20 {
            howl.vsync();
            heard.push(notes(&howl).len());
        }
        // On at the first blank, off at the tenth; the end at the 15th starts it again.
        let on: Vec<usize> = (0..20).filter(|&i| heard[i] == 1).map(|i| i + 1).collect();
        assert_eq!(on, [1, 2, 3, 4, 5, 6, 7, 8, 9, 15, 16, 17, 18, 19, 20]);
        let (sequence, note, pitch, _) = notes(&howl)[0];
        assert_eq!((sequence, note, pitch), (0, 60, 0x1000));
    }

    #[test]
    fn a_song_that_does_not_loop_stops_at_its_end() {
        let mut howl = test_howl();
        assert!(howl.start_song(LEVEL_SONG, 0, false));
        for _ in 0..15 {
            howl.vsync();
        }
        assert_eq!(howl.music.songs[0].flags & song_flags::PLAYING, 0);
        for _ in 0..20 {
            howl.vsync();
            assert!(notes(&howl).is_empty());
        }
    }

    #[test]
    fn the_notes_are_at_the_musics_the_songs_and_the_instruments_volume() {
        let mut howl = test_howl();
        howl.play_music(LEVEL_SONG, 0);
        howl.vsync();
        assert_eq!(notes(&howl)[0].3, left(190));
        // A mask's song is louder.
        howl.play_music(AKU_SONG, 0);
        howl.vsync();
        assert_eq!(notes(&howl)[0].3, left(255));
    }

    #[test]
    fn the_music_fades_down_and_up_and_out() {
        let mut howl = test_howl();
        howl.play_music(LEVEL_SONG, 0);
        howl.lower_music();
        let volumes: Vec<u8> = (0..15)
            .map(|_| {
                howl.vsync();
                howl.music.songs[0].volume.now
            })
            .collect();
        // 8 a blank, and the last step stops at 90.
        assert_eq!(&volumes[..2], [182, 174]);
        assert_eq!(&volumes[11..], [94, 90, 90, 90]);
        // The note that starts again at the 15th blank is quieter.
        assert_eq!(notes(&howl)[0].3, left(90));
        howl.raise_music();
        for _ in 0..13 {
            howl.vsync();
        }
        assert_eq!(howl.music.songs[0].volume.now, SONG_VOLUME);

        howl.fade_out_music();
        for _ in 0..23 {
            howl.vsync();
        }
        assert_eq!(howl.music.playing, Some(LEVEL_SONG));
        howl.vsync();
        // Down from 190 by 8 the 24th step goes past 0: the song stops, and nothing plays.
        assert_eq!(howl.music.playing, None);
        assert!(notes(&howl).is_empty());
        assert_eq!(howl.music.songs[0].flags, 0);
    }

    #[test]
    fn another_song_plays_from_its_start_and_another_tempo_goes_on_where_it_is() {
        let mut howl = test_howl();
        howl.play_music(LEVEL_SONG, 0);
        for _ in 0..12 {
            howl.vsync();
        }
        // 20 beats a minute faster: 80 of 60 ticks a second.
        howl.play_music(LEVEL_SONG, 20);
        assert_eq!(howl.music.songs[0].bpm, 80);
        let sequence = howl.music.songs[0].sequences[0];
        assert_eq!(howl.music.sequences[sequence].elapsed, 2);
        // Aku Aku's song takes the level's place in the pool, from its start.
        howl.play_music(AKU_SONG, 20);
        howl.vsync();
        assert_eq!(howl.music.playing, Some(AKU_SONG));
        let song = &howl.music.songs[0];
        assert_eq!(
            (song.id, song.flags, song.bpm),
            (AKU_SONG, song_flags::PLAYING, 80)
        );
        assert_eq!(howl.music.songs[1].flags, 0);
        assert_eq!(notes(&howl).len(), 1);
    }

    #[test]
    fn a_pause_holds_the_song_and_its_notes_start_again_after() {
        let mut howl = test_howl();
        howl.play_music(LEVEL_SONG, 0);
        for _ in 0..5 {
            howl.vsync();
        }
        howl.pause();
        for _ in 0..30 {
            howl.vsync();
        }
        assert!(notes(&howl).is_empty());
        howl.unpause();
        assert_eq!(notes(&howl).len(), 1);
        // The note ends where it would have: 5 more blanks.
        for _ in 0..4 {
            howl.vsync();
        }
        assert_eq!(notes(&howl).len(), 1);
        howl.vsync();
        assert!(notes(&howl).is_empty());
    }

    #[test]
    fn a_level_without_a_song_pack_plays_none() {
        let mut howl = test_howl();
        howl.play_music(LEVEL_SONG, 0);
        howl.vsync();
        // A map's, which stops what played.
        howl.set_level_music(0x8000_1234, None);
        howl.vsync();
        assert!(notes(&howl).is_empty());
        howl.play_music(LEVEL_SONG, 0);
        howl.vsync();
        assert!(notes(&howl).is_empty());
    }

    #[test]
    fn the_volumes_are_percents_of_the_games_and_silence_at_zero() {
        let mut howl = test_howl();
        howl.set_volumes(100, 100, 100);
        assert_eq!((howl.volume_fx, howl.volume_voice, howl.volume_music), (215, 255, 175));
        howl.set_volumes(50, 0, 20);
        assert_eq!((howl.volume_fx, howl.volume_voice, howl.volume_music), (107, 51, 0));
        // (More than the game's is the game's.)
        howl.set_volumes(200, 200, 200);
        assert_eq!(howl.volume_music, 175);
    }

    #[test]
    fn an_instruments_notes_are_a_semitone_apart_and_a_distortion_moves_them() {
        let base = 0x0800;
        assert_eq!(instrument_pitch(base, 60, NO_DISTORTION), base);
        assert_eq!(instrument_pitch(base, 72, NO_DISTORTION), 2 * base);
        assert_eq!(instrument_pitch(base, 48, NO_DISTORTION), base / 2);
        // The top bits a semitone down; the rest bend it up towards the next.
        assert_eq!(instrument_pitch(base, 61, 0x40), base);
        let bent = instrument_pitch(base, 60, NO_DISTORTION | 0x3f);
        assert!(bent > base && bent < instrument_pitch(base, 61, NO_DISTORTION));
        // Past the table, silence.
        assert_eq!(instrument_pitch(base, 120, NO_DISTORTION), 0);
    }

    #[test]
    fn a_delay_is_seven_bits_a_byte() {
        assert_eq!(next_note(&[0x05, 9], 0), (1, 5));
        assert_eq!(next_note(&[0x81, 0x80, 0x03, 9], 0), (3, (1 << 14) + 3));
        // 148 beats a minute of 120 ticks: 296 ticks a second, 4.93 a blank.
        assert_eq!(tempo(120, 148), (296 << 16) / 60);
    }
}
