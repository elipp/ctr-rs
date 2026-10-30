//! The XA audio of the disc (`XA/`): the music's jingles, and the lines the drivers, Aku Aku and
//! Uka Uka speak, streamed from the disc as the CD drive plays them (`CDSYS_SetXAToLang`,
//! `CDSYS_XAPlay`; `NativeAudio_LookupXATrackInfo`, `NativeAudio_PrepareXAStream` and
//! `NativeAudio_DecodeXASector*` of `ctr-native`).
//!
//! ```text
//! XA/ENG.XNF                         the index ([`Xnf`])
//! XA/MUSIC/Sxx.XA                    the files of [`Category::Music`]
//! XA/ENG/EXTRA/Sxx.XA                of [`Category::Extra`]
//! XA/ENG/GAME/Sxx.XA                 of [`Category::Game`]
//! ```
//!
//! A file holds several tracks in the Mode 2 Form 2 sectors of the disc, interleaved: each sector
//! is of one track, its channel in the sector's subheader. A track starts at its file's first
//! sector and spans a number of them; it is the sectors in that span on its channel.
//!
//! ```text
//! sector (2336 bytes past the sync and header of the raw 2352)
//!   0x0   subheader: u8 file (1), u8 channel, u8 submode (0x04 audio), u8 coding, and again
//!   0x8   18 sound groups of 128 bytes:
//!           0x0  4 bytes, 8 parameters (shift, filter), 4 bytes
//!           0x10 28 words, each a 4-bit sample of each of the 8 sound units
//!   0x908 the rest (unused)
//! ```
//!
//! The coding says 37800 or 18900 samples a second, mono or stereo (the units in turn left and
//! right), and 4 bits a sample (the game has no 8-bit XA).

use std::io::{self, Read, Seek, SeekFrom};

use crate::disc::RAW_SECTOR_SIZE;

/// `"XINF"`, and the retail version (`CDSYS_XNF_VERSION_RETAIL`).
const MAGIC: u32 = 0x464e_4958;
const VERSION: u32 = 102;
const HEADER_SIZE: usize = 0x44;
const NUM_XAS: usize = 0x0c;
const NUM_TRACKS: usize = 0x10;
const NUM_SONGS: usize = 0x2c;
const FIRST_SONG: usize = 0x38;

/// A sector's bytes past the sync and the header (the CD drive's Form 2 payload).
pub const SECTOR_SIZE: usize = 2336;
/// Where a raw sector's subheader is.
const SUBHEADER_OFFSET: u64 = 16;
const SUBHEADER_SIZE: usize = 8;
const GROUPS: usize = 18;
const GROUP_SIZE: usize = 128;
const UNITS: usize = 8;
const SAMPLES_PER_UNIT: usize = 28;
/// The samples of a sector of mono, each of its units in turn (stereo: half as many frames).
const SECTOR_SAMPLES: usize = GROUPS * UNITS * SAMPLES_PER_UNIT;

/// The submode's bit of a sector of audio.
const SUBMODE_AUDIO: u8 = 0x04;
/// The subheader's file number of every track.
const FILE: u8 = 1;

/// The sample rates of the coding.
pub const RATE_FULL: u32 = 37_800;
pub const RATE_HALF: u32 = 18_900;

/// The prediction filters in 64ths (`s_posTable`, `s_negTable`): of the last sample and of the
/// one before it.
const FILTERS: [(i32, i32); 5] = [(0, 0), (60, 0), (115, -52), (98, -55), (122, -60)];

/// `CDSYS_XA_TYPE_*`: the kinds of XA, each its own files and tracks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Category {
    /// The jingles: the race's intro, "FINAL LAP!", the race's end.
    Music,
    /// Aku Aku's, Uka Uka's and N. Tropy's lines, and the wrong way's.
    Extra,
    /// The drivers' voice lines.
    Game,
}

impl Category {
    pub const ALL: [Category; 3] = [Category::Music, Category::Extra, Category::Game];

    /// The directory of its files (`NativeAudio_BuildXAPath`).
    pub fn dir(self) -> &'static str {
        match self {
            Category::Music => "XA/MUSIC",
            Category::Extra => "XA/ENG/EXTRA",
            Category::Game => "XA/ENG/GAME",
        }
    }
}

/// A track (`struct XaSize`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Track {
    /// Its channel in the subheader (`XaIndex`).
    pub channel: u8,
    /// Its file, `S<file>.XA` (`XaPrefix`).
    pub file: u8,
    /// The sectors it spans from the start of its file, its own and the other tracks'
    /// (`XaBytes`).
    pub sectors: u16,
}

/// `XA/ENG.XNF`: the tracks of each category, and how many files each has.
#[derive(Clone, Debug, Default)]
pub struct Xnf {
    pub files: [u32; 3],
    pub tracks: [Vec<Track>; 3],
}

impl Xnf {
    /// `CDSYS_SetXAToLang`: the index's header, the files' positions on the disc (none in the
    /// file: the game looks them up), and the tracks of every category.
    ///
    /// ```text
    /// 0x0   u32 magic "XINF", u32 version 102, u32 categories (3)
    /// 0xc   u32 files, u32 tracks: in all
    /// 0x14  u32 files[3], u32 firstFile[3]: of each category
    /// 0x2c  u32 tracks[3], u32 firstTrack[3]
    /// 0x44  u32 cdPos[files], then XaSize tracks[tracks]: u8 channel, u8 file, i16 sectors
    /// ```
    pub fn parse(bytes: &[u8]) -> io::Result<Xnf> {
        let invalid = |msg: &str| io::Error::new(io::ErrorKind::InvalidData, msg.to_string());
        let word = |at: usize| -> io::Result<u32> {
            let bytes = bytes
                .get(at..at + 4)
                .ok_or_else(|| invalid("XNF too short"))?;
            Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
        };
        if word(0)? != MAGIC || word(4)? != VERSION || word(8)? != 3 {
            return Err(invalid("not a retail XNF"));
        }
        let table = HEADER_SIZE + word(NUM_XAS)? as usize * 4;
        let all = word(NUM_TRACKS)? as usize;
        let mut xnf = Xnf::default();
        for (i, tracks) in xnf.tracks.iter_mut().enumerate() {
            xnf.files[i] = word(0x14 + i * 4)?;
            let first = word(FIRST_SONG + i * 4)? as usize;
            let count = word(NUM_SONGS + i * 4)? as usize;
            if first + count > all {
                return Err(invalid("XNF tracks out of range"));
            }
            for track in first..first + count {
                let at = table + track * 4;
                let entry = bytes
                    .get(at..at + 4)
                    .ok_or_else(|| invalid("XNF too short"))?;
                tracks.push(Track {
                    channel: entry[0],
                    file: entry[1],
                    sectors: i16::from_le_bytes([entry[2], entry[3]]).max(0) as u16,
                });
            }
        }
        Ok(xnf)
    }

    /// Track `id` of `category`, if there is one that spans a sector.
    pub fn track(&self, category: Category, id: u16) -> Option<Track> {
        self.tracks[category as usize]
            .get(id as usize)
            .copied()
            .filter(|track| track.sectors > 0)
    }
}

/// The XA files of a disc image of raw sectors, which [`XaFiles::read`] reads tracks from.
pub struct XaFiles<R> {
    raw: R,
    /// The first sector of each file, by category.
    starts: [Vec<u32>; 3],
}

impl<R: Read + Seek> XaFiles<R> {
    /// The files of a disc image of raw sectors `raw`, each at its first sector (`starts`, by
    /// category).
    pub fn new(raw: R, starts: [Vec<u32>; 3]) -> XaFiles<R> {
        XaFiles { raw, starts }
    }

    /// `NativeAudio_PrepareXAStream`: the sectors of `track` of `category`, the sectors in its
    /// span that are audio on its channel, of [`SECTOR_SIZE`] each. None if its file is not on
    /// the disc.
    pub fn read(&mut self, category: Category, track: Track) -> io::Result<Option<Vec<u8>>> {
        let Some(&start) = self.starts[category as usize].get(track.file as usize) else {
            return Ok(None);
        };
        read_track(&mut self.raw, start, track).map(Some)
    }
}

/// The sectors of `track` in the file that begins at sector `start` of `raw`, the image of raw
/// sectors (or of one file's, with a `start` of 0).
pub fn read_track<R: Read + Seek>(raw: &mut R, start: u32, track: Track) -> io::Result<Vec<u8>> {
    let span = track.sectors as usize;
    let mut buf = vec![0; span * RAW_SECTOR_SIZE as usize];
    raw.seek(SeekFrom::Start(start as u64 * RAW_SECTOR_SIZE))?;
    raw.read_exact(&mut buf)?;
    let mut sectors = Vec::new();
    for sector in buf.as_chunks::<{ RAW_SECTOR_SIZE as usize }>().0 {
        let sector = &sector[SUBHEADER_OFFSET as usize..][..SECTOR_SIZE];
        if is_audio(sector, track.channel) {
            sectors.extend_from_slice(sector);
        }
    }
    Ok(sectors)
}

/// `NativeAudio_IsXAAudioSector`: a sector of audio of 4-bit samples on `channel`.
fn is_audio(sector: &[u8], channel: u8) -> bool {
    let [file, ch, submode, coding] = sector[..4] else {
        return false;
    };
    file == FILE && ch == channel && submode & SUBMODE_AUDIO != 0 && (coding >> 4) & 3 == 0
}

/// XA decoded: left and right (the same for mono) at `rate`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pcm {
    /// [`RATE_FULL`] or [`RATE_HALF`].
    pub rate: u32,
    pub frames: Vec<[i16; 2]>,
}

/// The two last samples of each side, which the filters go on from.
#[derive(Clone, Copy, Debug, Default)]
struct History {
    old: [i32; 2],
    older: [i32; 2],
}

/// Decodes the sectors [`XaFiles::read`] reads, at the rate and in the channels of the last of
/// them, as `NativeAudio_PrepareXAStream` takes them.
pub fn decode(sectors: &[u8]) -> Pcm {
    let sectors = sectors.as_chunks::<SECTOR_SIZE>().0;
    let Some(last) = sectors.last() else {
        return Pcm::default();
    };
    let coding = last[3];
    let rate = if (coding >> 2) & 3 == 0 {
        RATE_FULL
    } else {
        RATE_HALF
    };
    let stereo = coding & 3 != 0;
    let mut history = History::default();
    let mut frames = Vec::with_capacity(sectors.len() * SECTOR_SAMPLES);
    for sector in sectors {
        for group in &sector[SUBHEADER_SIZE..].as_chunks::<GROUP_SIZE>().0[..GROUPS] {
            if stereo {
                decode_stereo(group, &mut history, &mut frames);
            } else {
                decode_mono(group, &mut history, &mut frames);
            }
        }
    }
    Pcm { rate, frames }
}

/// `NativeAudio_DecodeXASectorMono`, a sound group: its 8 units in turn.
fn decode_mono(group: &[u8; GROUP_SIZE], history: &mut History, out: &mut Vec<[i16; 2]>) {
    for unit in 0..UNITS {
        // Units 0-3 have parameters 0-3, units 4-7 parameters 8-11.
        let param = group[4 + ((unit & 3) | (unit & 4) << 1)];
        let mut samples = [0; SAMPLES_PER_UNIT];
        decode_unit(group, param, unit >> 1, unit & 1, 0, history, &mut samples);
        out.extend(samples.map(|s| [s; 2]));
    }
}

/// `NativeAudio_DecodeXASectorStereo`, a sound group: 4 units of the left (the low nibbles), each
/// with one of the right (the high).
fn decode_stereo(group: &[u8; GROUP_SIZE], history: &mut History, out: &mut Vec<[i16; 2]>) {
    for block in 0..UNITS / 2 {
        let mut sides = [[0; SAMPLES_PER_UNIT]; 2];
        for (side, samples) in sides.iter_mut().enumerate() {
            let param = group[4 + block * 2 + side];
            decode_unit(group, param, block, side, side, history, samples);
        }
        out.extend((0..SAMPLES_PER_UNIT).map(|i| [sides[0][i], sides[1][i]]));
    }
}

/// `NativeAudio_DecodeXA28Nibbles`: the 28 samples of a unit, the nibble `nibble` (0 low, 1 high)
/// of byte `byte` of each word, with the shift and filter of `param`, the filter going on from
/// `side`'s history.
fn decode_unit(
    group: &[u8; GROUP_SIZE],
    param: u8,
    byte: usize,
    nibble: usize,
    side: usize,
    history: &mut History,
    out: &mut [i16; SAMPLES_PER_UNIT],
) {
    let shift = param & 0xf;
    let (gain_old, gain_older) = FILTERS[((param >> 4) as usize).min(4)];
    for (i, sample) in out.iter_mut().enumerate() {
        let word = group[16 + i * 4 + byte];
        let nib = if nibble == 0 { word << 4 } else { word & 0xf0 };
        let mut s = (nib as i8 as i32 * 0x100) >> shift;
        s += (history.old[side] * gain_old) >> 6;
        s += (history.older[side] * gain_older) >> 6;
        let s = s.clamp(i16::MIN as i32, i16::MAX as i32);
        history.older[side] = history.old[side];
        history.old[side] = s;
        *sample = s as i16;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sector of `channel`, with `coding`, each group's parameters `param` and words `word`.
    fn sector(channel: u8, coding: u8, param: u8, word: u8) -> Vec<u8> {
        let mut sector = vec![0; SECTOR_SIZE];
        sector[..4].copy_from_slice(&[FILE, channel, SUBMODE_AUDIO, coding]);
        let groups = &mut sector[SUBHEADER_SIZE..][..GROUPS * GROUP_SIZE];
        for group in groups.as_chunks_mut::<GROUP_SIZE>().0 {
            group[4..16].fill(param);
            group[16..].fill(word);
        }
        sector
    }

    #[test]
    fn a_mono_sector_is_its_units_in_turn_at_the_full_rate() {
        // Shift 12 and no filter: each nibble is itself, the low one of each word first.
        let pcm = decode(&sector(0, 0, 12, 0x31));
        assert_eq!(pcm.rate, RATE_FULL);
        assert_eq!(pcm.frames.len(), SECTOR_SAMPLES);
        assert_eq!(pcm.frames[0], [1, 1]);
        assert_eq!(pcm.frames[SAMPLES_PER_UNIT], [3, 3]);
    }

    #[test]
    fn a_stereo_sector_has_the_low_nibbles_left_and_the_high_right() {
        let pcm = decode(&sector(0, 0x01 | 0x04, 12, 0x31));
        assert_eq!(pcm.rate, RATE_HALF);
        assert_eq!(pcm.frames.len(), SECTOR_SAMPLES / 2);
        assert!(pcm.frames.iter().all(|&f| f == [1, 3]));
    }

    #[test]
    fn the_filter_goes_on_from_the_samples_before() {
        // Filter 1 (60/64 of the last), shift 0: a nibble of 1 is 0x1000.
        let pcm = decode(&sector(0, 0, 0x10, 0x11));
        let [first, second] = [pcm.frames[0][0], pcm.frames[1][0]];
        assert_eq!(first, 0x1000);
        assert_eq!(second as i32, 0x1000 + ((0x1000 * 60) >> 6));
        // It saturates rather than wrapping.
        assert!(pcm.frames.iter().all(|f| f[0] > 0));
    }

    #[test]
    fn a_track_is_the_audio_of_its_channel_in_its_span() {
        let mut raw = Vec::new();
        for channel in [0, 1, 0, 1, 0] {
            let mut sector = vec![0; RAW_SECTOR_SIZE as usize];
            sector[SUBHEADER_OFFSET as usize..][..SECTOR_SIZE].copy_from_slice(&sector_of(channel));
            raw.extend(sector);
        }
        let mut files = XaFiles::new(io::Cursor::new(raw), [vec![], vec![], vec![0]]);
        let track = Track {
            channel: 1,
            file: 0,
            sectors: 3,
        };
        let sectors = files.read(Category::Game, track).unwrap().unwrap();
        assert_eq!(sectors.len(), SECTOR_SIZE);
        assert_eq!(sectors[1], 1);
        let missing = Track { file: 1, ..track };
        assert_eq!(files.read(Category::Game, missing).unwrap(), None);
    }

    fn sector_of(channel: u8) -> Vec<u8> {
        sector(channel, 0, 12, 0)
    }
}
