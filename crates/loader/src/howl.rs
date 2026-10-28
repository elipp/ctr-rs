//! `SOUNDS/KART.HWL`: the game's sound effects, and the samples and sequences of its music
//! (`howl_LoadHeader`, `howl_ParseHeader`, `Bank_Alloc`, `Bank_AssignSpuAddrs`).
//!
//! ```text
//! HowlHeader                         0x28 bytes: "HOWL", version 0x80, the counts below,
//!                                    and the size of the tables after it
//! SpuAddrEntry[numSpuAddrs]          u16 spuAddr (0 in the file), u16 size in 8-byte units
//! OtherFX[numOtherFX]                u8 flags, u8 volume, u16 pitch, u16 sample, u16 duration
//! EngineFX[numEngineFX]             u8 flags, u8 volume, u16 pitch, u16 (unknown), u16 sample
//! u16 bankOffsets[numBanks]          in sectors from the start of the file
//! u16 songOffsets[numSequences]      in sectors from the start of the file
//! ```
//!
//! A bank is a sector holding `i16 numSamples` and the sample numbers, then from its next sector
//! the samples' PS1 ADPCM one after the other, in that order. The game loads the banks a level
//! needs into the 512 KB of sound RAM (bank 0 for every level, then the level's own, the
//! characters', the music's). A sample number is the same sample in every bank that has it, so
//! here the banks only say where each sample is on the disc: [`Howl::adpcm`] reads it from the
//! first bank that has it.
//!
//! A song pack ([`Cseq`], `howl_LoadSong`) is the music of a level: its instruments and drums (each
//! a sample of the banks), and its songs, the level's, Aku Aku's and Uka Uka's, each of sequences
//! of notes.

use std::io::{self, Read, Seek, SeekFrom};

use crate::disc::USER_DATA_SIZE;

const MAGIC: &[u8; 4] = b"HOWL";
/// `HowlHeader::version` of the retail game.
const VERSION: u32 = 0x80;
const HEADER_SIZE: usize = 0x28;
const SPU_ADDR_SIZE: usize = 4;
const FX_SIZE: usize = 8;

/// The samples' playback rate at a pitch of [`BASE_PITCH`].
pub const BASE_RATE: u32 = 44_100;
/// The pitch that plays a sample at [`BASE_RATE`] (the SPU's 0x1000).
pub const BASE_PITCH: u32 = 0x1000;

/// The bits of [`OtherFx::flags`].
pub mod other_fx_flags {
    /// The sound loops until it is stopped (`OtherFX_Play_LowLevel` gives its voice no timer).
    pub const LOOPS: u8 = 0x2;
    /// A voice clip: it plays at the voice volume, not the effects' (`vol_Voice`).
    pub const VOICE: u8 = 0x4;
}

/// `struct OtherFX`: a sound effect, by its number in `OtherFX_Play`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OtherFx {
    pub flags: u8,
    /// 0 to 255.
    pub volume: u8,
    /// [`BASE_PITCH`] plays the sample at [`BASE_RATE`].
    pub pitch: u16,
    /// The sample it plays (`spuIndex`).
    pub sample: u16,
    /// How long its voice is held, in 300ths of a second: the sound timer counts it down by 5 a
    /// frame (`howl_PlayAudio_Update`).
    pub duration: u16,
}

/// `struct EngineFX`: the loop of an engine, by `engine class * 4 + player`
/// (`VehBirth_EngineAudio_AllPlayers`), and the two of the AIs at 0x10 and 0x11.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EngineFx {
    pub flags: u8,
    /// 0 to 255.
    pub volume: u8,
    /// [`BASE_PITCH`] plays the sample at [`BASE_RATE`].
    pub pitch: u16,
    /// The sample it loops (`spuIndex`).
    pub sample: u16,
}

/// The header of `KART.HWL`, and where each sample is in it.
#[derive(Clone, Debug)]
pub struct Howl {
    /// The file's first sector on the disc.
    pub start_sector: u32,
    /// The size of each sample's ADPCM in bytes, by sample number.
    pub sample_sizes: Vec<u32>,
    pub other_fx: Vec<OtherFx>,
    pub engine_fx: Vec<EngineFx>,
    /// Where each bank starts, in sectors from the start of the file.
    pub bank_offsets: Vec<u16>,
    /// Where each song pack starts, in sectors from the start of the file (`howl_songOffsets`).
    pub song_offsets: Vec<u16>,
    /// The disc byte offset of each sample's ADPCM in the first bank that has it, by sample
    /// number. `None` for the samples no bank has.
    places: Vec<Option<u64>>,
}

impl Howl {
    /// Reads the header of `KART.HWL`, which begins at `start_sector` of the 2048-byte-sector
    /// `disc`, and the sample list of each bank.
    pub fn open<R: Read + Seek>(disc: &mut R, start_sector: u32) -> io::Result<Self> {
        let start = start_sector as u64 * USER_DATA_SIZE;
        let mut fixed = [0u8; HEADER_SIZE];
        disc.seek(SeekFrom::Start(start))?;
        disc.read_exact(&mut fixed)?;
        let word = |at: usize| u32::from_le_bytes(fixed[at..at + 4].try_into().unwrap());
        if &fixed[..4] != MAGIC || word(4) != VERSION {
            return Err(invalid(format!(
                "not a KART.HWL of the retail game: magic {:02x?}, version {:#x}",
                &fixed[..4],
                word(4)
            )));
        }
        let [samples, other_fx, engine_fx, banks, songs] = [0x10, 0x14, 0x18, 0x1c, 0x20].map(word);
        let tables = (samples as usize * SPU_ADDR_SIZE)
            + (other_fx as usize + engine_fx as usize) * FX_SIZE
            + (banks as usize + songs as usize) * 2;
        if tables > word(0x24) as usize {
            return Err(invalid(format!(
                "KART.HWL tables need {tables} bytes, the header gives {}",
                word(0x24)
            )));
        }
        let mut table = vec![0u8; tables];
        disc.read_exact(&mut table)?;
        let half = |at: usize| u16::from_le_bytes(table[at..at + 2].try_into().unwrap());

        let sample_sizes = (0..samples as usize)
            .map(|i| half(i * SPU_ADDR_SIZE + 2) as u32 * 8)
            .collect::<Vec<_>>();
        let other_at = samples as usize * SPU_ADDR_SIZE;
        let other_fx = (0..other_fx as usize)
            .map(|i| {
                let at = other_at + i * FX_SIZE;
                OtherFx {
                    flags: table[at],
                    volume: table[at + 1],
                    pitch: half(at + 2),
                    sample: half(at + 4),
                    duration: half(at + 6),
                }
            })
            .collect::<Vec<_>>();
        let engine_at = other_at + other_fx.len() * FX_SIZE;
        let engine_fx = (0..engine_fx as usize)
            .map(|i| {
                let at = engine_at + i * FX_SIZE;
                EngineFx {
                    flags: table[at],
                    volume: table[at + 1],
                    pitch: half(at + 2),
                    sample: half(at + 6),
                }
            })
            .collect::<Vec<_>>();
        let banks_at = engine_at + engine_fx.len() * FX_SIZE;
        let bank_offsets = (0..banks as usize)
            .map(|i| half(banks_at + i * 2))
            .collect::<Vec<_>>();
        let songs_at = banks_at + bank_offsets.len() * 2;
        let song_offsets = (0..songs as usize)
            .map(|i| half(songs_at + i * 2))
            .collect::<Vec<_>>();

        let mut places = vec![None; sample_sizes.len()];
        let mut list = vec![0u8; USER_DATA_SIZE as usize];
        for &offset in &bank_offsets {
            let bank = start + offset as u64 * USER_DATA_SIZE;
            disc.seek(SeekFrom::Start(bank))?;
            disc.read_exact(&mut list)?;
            let count = i16::from_le_bytes([list[0], list[1]]).max(0) as usize;
            let count = count.min(list.len() / 2 - 1);
            let mut at = bank + USER_DATA_SIZE;
            for i in 0..count {
                let sample = i16::from_le_bytes([list[2 + i * 2], list[3 + i * 2]]) as usize;
                let size = *sample_sizes.get(sample).ok_or_else(|| {
                    invalid(format!(
                        "a KART.HWL bank lists sample {sample} of {}",
                        sample_sizes.len()
                    ))
                })?;
                places[sample].get_or_insert(at);
                at += size as u64;
            }
        }
        Ok(Self {
            start_sector,
            sample_sizes,
            other_fx,
            engine_fx,
            bank_offsets,
            song_offsets,
            places,
        })
    }

    /// `howl_LoadSong`: the song pack `song` (its first sector, then as many more as its size
    /// says), or `None` if there is no such pack.
    pub fn cseq<R: Read + Seek>(&self, disc: &mut R, song: u16) -> io::Result<Option<Cseq>> {
        let Some(&offset) = self.song_offsets.get(song as usize) else {
            return Ok(None);
        };
        let sector = USER_DATA_SIZE as usize;
        let mut data = vec![0u8; sector];
        let at = (self.start_sector as u64 + offset as u64) * USER_DATA_SIZE;
        disc.seek(SeekFrom::Start(at))?;
        disc.read_exact(&mut data)?;
        let size = u32::from_le_bytes(data[..4].try_into().unwrap()) as usize;
        if size > MAX_CSEQ_SIZE {
            return Err(invalid(format!(
                "song pack {song} is {size} bytes, more than the {MAX_CSEQ_SIZE} the game has room for"
            )));
        }
        data.resize(size.div_ceil(sector).max(1) * sector, 0);
        disc.read_exact(&mut data[sector..])?;
        Cseq::parse(data).map(Some)
    }

    /// The ADPCM of sample `sample`, or `None` if no bank has it.
    pub fn adpcm<R: Read + Seek>(&self, disc: &mut R, sample: u16) -> io::Result<Option<Vec<u8>>> {
        let Some(&Some(at)) = self.places.get(sample as usize) else {
            return Ok(None);
        };
        let mut data = vec![0u8; self.sample_sizes[sample as usize] as usize];
        disc.seek(SeekFrom::Start(at))?;
        disc.read_exact(&mut data)?;
        Ok(Some(data))
    }
}

/// The room a song pack is loaded into (`sampleBlock1` and `tenSampleBlocks`).
const MAX_CSEQ_SIZE: usize = 11 * USER_DATA_SIZE as usize;

/// `struct SampleInstrument`: what a sequence plays its notes on, each at its own pitch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Instrument {
    /// 0 to 255.
    pub volume: u8,
    /// The pitch of middle C (`basePitch`).
    pub base_pitch: u16,
    pub sample: u16,
    /// Its envelope, as the SPU's two registers.
    pub ad: u16,
    pub sr: u16,
}

/// `struct SampleDrums`: a drum, which a note of drums picks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Drum {
    /// 0 to 255.
    pub volume: u8,
    pub pitch: u16,
    pub sample: u16,
}

/// A song of a pack (`CseqSongHeader`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Song {
    /// Beats a minute, and ticks a beat (`tpqn`).
    pub bpm: i16,
    pub tpqn: i16,
    pub sequences: Vec<Sequence>,
}

/// A sequence of a song (`SongNoteHeader`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sequence {
    /// Its notes are drums rather than an instrument's.
    pub drums: bool,
    /// Where its notes start in [`Cseq::data`].
    pub notes: usize,
}

/// A song pack (`CseqHeader`, `howl_ParseCseqHeader`):
///
/// ```text
/// i32 size                           in bytes, this header too
/// u8 numInstruments, u8 numDrums
/// u16 numSongs
/// SampleInstrument[numInstruments]   u8 1, u8 volume, u16 0, u16 basePitch, u16 sample, u16 ad,
///                                    u16 sr
/// SampleDrums[numDrums]              u8 1, u8 volume, u16 pitch, u16 sample, u16 0
/// u16 songOffsets[numSongs]          from the songs' data, which starts 4-aligned after them
/// ```
///
/// A song is `u8, u8 numSequences, i16 bpm, i16 tpqn, u16 sequenceOffsets[numSequences]`, and
/// its sequences start 4-aligned after it, each at its offset: `u8 flags` (1 for drums), a `u8`
/// that nothing reads, then its notes: a delay in ticks (7 bits a byte, the top bit set on all but the last), an
/// opcode and what it takes, over and over.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cseq {
    pub instruments: Vec<Instrument>,
    pub drums: Vec<Drum>,
    pub songs: Vec<Song>,
    /// The pack as it was loaded, which the notes are read from.
    pub data: Vec<u8>,
}

impl Cseq {
    pub fn parse(data: Vec<u8>) -> io::Result<Cseq> {
        let short = || invalid("a song pack ends early".into());
        let byte = |at: usize| data.get(at).copied().ok_or_else(short);
        let half = |at: usize| Ok::<_, io::Error>(u16::from_le_bytes([byte(at)?, byte(at + 1)?]));
        let align = |at: usize| at.next_multiple_of(4);

        let (instruments, drums, songs) = (byte(4)? as usize, byte(5)? as usize, half(6)?);
        let mut at = 8;
        let instruments = (0..instruments)
            .map(|i| {
                let at = at + i * 12;
                Ok(Instrument {
                    volume: byte(at + 1)?,
                    base_pitch: half(at + 4)?,
                    sample: half(at + 6)?,
                    ad: half(at + 8)?,
                    sr: half(at + 10)?,
                })
            })
            .collect::<io::Result<Vec<_>>>()?;
        at += instruments.len() * 12;
        let drums = (0..drums)
            .map(|i| {
                let at = at + i * 8;
                Ok(Drum {
                    volume: byte(at + 1)?,
                    pitch: half(at + 2)?,
                    sample: half(at + 4)?,
                })
            })
            .collect::<io::Result<Vec<_>>>()?;
        at += drums.len() * 8;
        let songs_at = align(at + songs as usize * 2);
        let songs = (0..songs as usize)
            .map(|i| {
                let song = songs_at + half(at + i * 2)? as usize;
                let count = byte(song + 1)? as usize;
                let first = align(song + 6 + count * 2);
                let sequences = (0..count)
                    .map(|j| {
                        let header = first + half(song + 6 + j * 2)? as usize;
                        Ok(Sequence {
                            drums: byte(header)? & 1 != 0,
                            notes: header + 2,
                        })
                    })
                    .collect::<io::Result<Vec<_>>>()?;
                Ok(Song {
                    bpm: half(song + 2)? as i16,
                    tpqn: half(song + 4)? as i16,
                    sequences,
                })
            })
            .collect::<io::Result<Vec<_>>>()?;
        Ok(Cseq {
            instruments,
            drums,
            songs,
            data,
        })
    }

    /// The samples its instruments and drums play.
    pub fn samples(&self) -> impl Iterator<Item = u16> + '_ {
        let instruments = self.instruments.iter().map(|i| i.sample);
        instruments.chain(self.drums.iter().map(|d| d.sample))
    }
}

fn invalid(msg: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

/// A decoded sample: 16-bit mono at the rate its pitch gives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pcm {
    pub samples: Vec<i16>,
    /// Where the sample goes on from after its end, for a sample that loops.
    pub loop_start: Option<usize>,
}

/// The bits of an ADPCM block's flag byte.
pub mod block_flags {
    /// The last block: after it the voice jumps to the loop start.
    pub const END: u8 = 0x1;
    /// With [`END`]: the voice goes on playing from the loop start. Without it, the voice stops.
    pub const REPEAT: u8 = 0x2;
    /// The loop starts at this block.
    pub const LOOP_START: u8 = 0x4;
}

/// The SPU's prediction filters, in 64ths: how much of the last sample and of the one before it
/// a block adds to each of its own.
const FILTERS: [(i32, i32); 5] = [(0, 0), (60, 0), (115, -52), (98, -55), (122, -60)];

pub const BLOCK_SIZE: usize = 16;
pub const SAMPLES_PER_BLOCK: usize = 28;

/// Decodes one block of the SPU's ADPCM: a shift and filter byte, a flag byte, then 28 4-bit
/// samples, low nibble first. `history` is the last sample decoded and the one before it, which
/// the filter goes on from. Returns the block's flags ([`block_flags`]).
pub fn decode_block(
    block: &[u8; BLOCK_SIZE],
    history: &mut (i32, i32),
    out: &mut [i16; SAMPLES_PER_BLOCK],
) -> u8 {
    // Shifts above 12 act as 9 on the hardware; filters above 4 do not occur.
    let shift = match block[0] & 0xf {
        s @ 0..=12 => s,
        _ => 9,
    };
    let (gain_last, gain_before) = FILTERS[((block[0] >> 4) as usize).min(4)];
    let (last, before) = history;
    for (i, &byte) in block[2..].iter().enumerate() {
        for (j, nibble) in [byte & 0xf, byte >> 4].into_iter().enumerate() {
            let delta = ((nibble as u16) << 12) as i16 as i32 >> shift;
            let predicted = (*last * gain_last + *before * gain_before + 32) >> 6;
            let sample = (delta + predicted).clamp(i16::MIN as i32, i16::MAX as i32);
            out[i * 2 + j] = sample as i16;
            (*last, *before) = (sample, *last);
        }
    }
    block[1]
}

/// Decodes the SPU's ADPCM, blocks of 16 bytes ([`decode_block`]). The sample ends with the
/// first block that has the end flag.
pub fn decode(adpcm: &[u8]) -> Pcm {
    let mut samples = Vec::with_capacity(adpcm.len() / BLOCK_SIZE * SAMPLES_PER_BLOCK);
    let mut history = (0, 0);
    let mut loop_block = 0;
    let mut loop_start = None;
    for (i, block) in adpcm.as_chunks::<BLOCK_SIZE>().0.iter().enumerate() {
        let mut out = [0; SAMPLES_PER_BLOCK];
        let flags = decode_block(block, &mut history, &mut out);
        samples.extend_from_slice(&out);
        if flags & block_flags::LOOP_START != 0 {
            loop_block = i;
        }
        if flags & block_flags::END != 0 {
            if flags & block_flags::REPEAT != 0 {
                loop_start = Some(loop_block * SAMPLES_PER_BLOCK);
            }
            break;
        }
    }
    Pcm {
        samples,
        loop_start,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn block(shift_filter: u8, flags: u8, nibbles: [u8; 28]) -> [u8; 16] {
        let mut b = [0u8; 16];
        b[0] = shift_filter;
        b[1] = flags;
        for (i, pair) in nibbles.chunks(2).enumerate() {
            b[2 + i] = pair[0] | pair[1] << 4;
        }
        b
    }

    #[test]
    fn decodes_the_nibbles_with_shift_and_filter() {
        let mut nibbles = [0u8; 28];
        nibbles[0] = 0x1; // 1 << 12 >> 12 = 1
        nibbles[1] = 0xf; // -1
        nibbles[2] = 0x7; // 7
        let mut adpcm = block(12, 0, nibbles).to_vec();
        // Shift 8: each nibble is worth 16; filter 1 adds 60/64 of the last sample.
        let mut ramp = [0u8; 28];
        ramp[0] = 0x4;
        adpcm.extend(block(0x18, block_flags::END, ramp));
        let pcm = decode(&adpcm);
        assert_eq!(pcm.samples.len(), 56);
        assert_eq!(&pcm.samples[..4], &[1, -1, 7, 0]);
        assert_eq!(pcm.samples[28], 64); // 4 << 12 >> 8, and the last sample was 0
        assert_eq!(pcm.samples[29], 60); // (64 * 60 + 32) >> 6
        assert_eq!(pcm.loop_start, None);
    }

    #[test]
    fn stops_at_the_end_flag_and_keeps_the_loop() {
        let silent = [0u8; 28];
        let adpcm = [
            block(0, 0, silent),
            block(0, block_flags::LOOP_START, silent),
            block(0, block_flags::END | block_flags::REPEAT, silent),
            block(0, 0, silent),
        ]
        .concat();
        let pcm = decode(&adpcm);
        assert_eq!(pcm.samples.len(), 3 * 28);
        assert_eq!(pcm.loop_start, Some(28));
    }

    #[test]
    fn reads_the_tables_and_finds_samples_in_banks() {
        // KART.HWL at sector 1: the header sector, then bank 0 at +1 (sample list) and +2 (data).
        let sector = USER_DATA_SIZE as usize;
        let mut header = vec![0u8; sector];
        header[..4].copy_from_slice(MAGIC);
        header[4..8].copy_from_slice(&VERSION.to_le_bytes());
        for (at, n) in [
            (0x10, 2u32),
            (0x14, 1),
            (0x18, 1),
            (0x1c, 1),
            (0x20, 0),
            (0x24, 26),
        ] {
            header[at..at + 4].copy_from_slice(&n.to_le_bytes());
        }
        let mut at = HEADER_SIZE;
        for size in [3u16, 2] {
            header[at + 2..at + 4].copy_from_slice(&size.to_le_bytes());
            at += SPU_ADDR_SIZE;
        }
        header[at..at + FX_SIZE].copy_from_slice(&[2, 200, 0, 0x10, 1, 0, 9, 0]);
        at += FX_SIZE;
        header[at..at + FX_SIZE].copy_from_slice(&[0, 153, 0x74, 0x04, 0, 0x10, 1, 0]);
        at += FX_SIZE;
        header[at..at + 2].copy_from_slice(&1u16.to_le_bytes());
        let mut list = vec![0u8; sector];
        list[..6].copy_from_slice(&[2, 0, 1, 0, 0, 0]);
        let mut data = vec![0u8; sector];
        data[..16].fill(0xbb);
        data[16..40].fill(0xaa);
        let disc = [vec![0u8; sector], header, list, data].concat();

        let mut disc = Cursor::new(disc);
        let howl = Howl::open(&mut disc, 1).unwrap();
        assert_eq!(howl.sample_sizes, [24, 16]);
        assert_eq!(
            howl.other_fx,
            [OtherFx {
                flags: other_fx_flags::LOOPS,
                volume: 200,
                pitch: 0x1000,
                sample: 1,
                duration: 9,
            }]
        );
        assert_eq!(
            howl.engine_fx,
            [EngineFx {
                flags: 0,
                volume: 153,
                pitch: 0x474,
                sample: 1,
            }]
        );
        // Sample 1 is first in the bank, sample 0 after it.
        assert_eq!(howl.adpcm(&mut disc, 1).unwrap(), Some(vec![0xbb; 16]));
        assert_eq!(howl.adpcm(&mut disc, 0).unwrap(), Some(vec![0xaa; 24]));
        assert_eq!(howl.adpcm(&mut disc, 2).unwrap(), None);
    }
}
