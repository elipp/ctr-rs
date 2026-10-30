//! Whether a patch changes the game's audio: its song packs (`SOUNDS/KART.HWL`) and the tables of
//! the executable that say which pack and which reverb a level has (`data.levBank_Song`,
//! `data.reverbMode`). `ctr-map audio` prints what differs between the disc and the patched disc,
//! so that we can see whether a map's music is there to be kept.

use std::fmt::Write as _;
use std::fs::File;
use std::io::{self, BufReader};
use std::path::Path;

use howl::tables::{LEVEL_REVERBS, LEVEL_SONGS};
use loader::disc::{open_howl_with, read_file_with};

/// The USA game's executable.
const EXE: &str = "SCUS_944.26";
const HWL: &str = "SOUNDS/KART.HWL";
/// The size of a PS-X EXE's header; the code follows it.
const EXE_HEADER: usize = 0x800;
/// Changes of the executable this close are shown as one.
const MERGE_GAP: usize = 32;
/// The changes of the executable to list.
const MAX_RANGES: usize = 40;

/// What differs between the disc and the one the patch makes, for people to read.
pub fn audit(original: &Path, patch: &Path) -> io::Result<String> {
    let bytes = std::fs::read(patch)?;
    let old = || Ok(BufReader::new(File::open(original)?));
    let new = || xdelta::Patched::new(BufReader::new(File::open(original)?), bytes.clone());
    let mut out = String::new();

    howl_report(&mut out, old, new)?;
    out.push('\n');
    exe_report(&mut out, old, new)?;
    Ok(out)
}

fn howl_report<A: io::Read + io::Seek, B: io::Read + io::Seek>(
    out: &mut String,
    old: impl Fn() -> io::Result<A> + Copy,
    new: impl Fn() -> io::Result<B> + Copy,
) -> io::Result<()> {
    // A patch that adds data to the file moves everything after it, so the two files are
    // compared by what is in them (each sample, each pack), not sector by sector.
    let ((mut a, x), (mut b, y)) = (open_howl_with(old)?, open_howl_with(new)?);
    writeln!(out, "{HWL}: {} samples -> {}", x.sample_sizes.len(), y.sample_sizes.len()).unwrap();

    let mut samples = Vec::new();
    for n in 0..x.sample_sizes.len().max(y.sample_sizes.len()) {
        let n = n as u16;
        if x.adpcm(&mut a, n)? != y.adpcm(&mut b, n)? {
            samples.push(n);
        }
    }
    writeln!(out, "  samples that are new or differ: {} {:?}", samples.len(), &samples[..samples.len().min(40)]).unwrap();
    writeln!(
        out,
        "  sound effects changed: {} of {} (engines: {} of {})",
        count_differing(&x.other_fx, &y.other_fx),
        y.other_fx.len(),
        count_differing(&x.engine_fx, &y.engine_fx),
        y.engine_fx.len()
    )
    .unwrap();
    writeln!(out, "  banks: {} -> {}, song packs: {} -> {}", x.bank_offsets.len(), y.bank_offsets.len(), x.song_offsets.len(), y.song_offsets.len()).unwrap();

    let mut same = 0;
    for pack in 0..x.song_offsets.len().max(y.song_offsets.len()) {
        let (p, q) = (x.cseq(&mut a, pack as u16)?, y.cseq(&mut b, pack as u16)?);
        if same_pack(&p, &q) {
            same += 1;
            continue;
        }
        let levels: Vec<usize> = LEVEL_SONGS
            .iter()
            .enumerate()
            .filter(|&(_, &s)| s as usize == pack)
            .map(|(l, _)| l)
            .collect();
        let size = |c: &Option<loader::howl::Cseq>| c.as_ref().map_or(0, |c| c.data.len());
        write!(out, "  song pack {pack} (retail levels {levels:?}) CHANGED: {} -> {} bytes", size(&p), size(&q)).unwrap();
        let (differ, total) = sequences_differing(&p, &q);
        write!(out, "; {differ} of {total} sequences differ").unwrap();
        if let Some(q) = &q {
            let mut used: Vec<u16> = q.samples().collect();
            used.sort();
            used.dedup();
            let new: Vec<u16> = used.iter().copied().filter(|n| samples.contains(n)).collect();
            write!(out, "; plays {} samples, {} of them new or changed {new:?}", used.len(), new.len()).unwrap();
        }
        writeln!(out).unwrap();
    }
    writeln!(out, "  song packs the same as retail: {same}").unwrap();
    Ok(())
}

fn count_differing<T: PartialEq>(a: &[T], b: &[T]) -> usize {
    (0..a.len().max(b.len())).filter(|&i| a.get(i) != b.get(i)).count()
}

fn exe_report<A: io::Read + io::Seek, B: io::Read + io::Seek>(
    out: &mut String,
    old: impl Fn() -> io::Result<A> + Copy,
    new: impl Fn() -> io::Result<B> + Copy,
) -> io::Result<()> {
    let (Some(a), Some(b)) = (read_file_with(old, EXE)?, read_file_with(new, EXE)?) else {
        writeln!(out, "{EXE}: not on the disc").unwrap();
        return Ok(());
    };
    let load = a
        .get(0x18..0x1c)
        .map_or(0x8001_0000, |w| u32::from_le_bytes(w.try_into().unwrap()));
    let ram = |offset: usize| load.wrapping_add(offset.wrapping_sub(EXE_HEADER) as u32);

    if a == b {
        writeln!(out, "{EXE}: unchanged ({} bytes)", a.len()).unwrap();
    } else {
        writeln!(out, "{EXE}: CHANGED ({} -> {} bytes)", a.len(), b.len()).unwrap();
        let mut ranges: Vec<(usize, usize)> = Vec::new();
        for i in (0..a.len().min(b.len())).filter(|&i| a[i] != b[i]) {
            match ranges.last_mut() {
                Some((_, end)) if i < *end + MERGE_GAP => *end = i + 1,
                _ => ranges.push((i, i + 1)),
            }
        }
        writeln!(out, "  {} changed ranges (file offset, address, bytes):", ranges.len()).unwrap();
        for &(start, end) in ranges.iter().take(MAX_RANGES) {
            writeln!(out, "    {start:#07x}  {:#010x}  {}", ram(start), end - start).unwrap();
        }
        if ranges.len() > MAX_RANGES {
            writeln!(out, "    ... {} more", ranges.len() - MAX_RANGES).unwrap();
        }
    }

    for (name, values) in TABLES {
        match find_table(&a, values) {
            Some((at, width)) => {
                write!(out, "{name}: {width}-byte entries at {at:#x} ({:#010x}), ", ram(at)).unwrap();
                match read_table(&b, at, width, values.len()) {
                    Some(v) if v.iter().zip(values).all(|(&x, &y)| x == y as u32) => {
                        writeln!(out, "unchanged").unwrap()
                    }
                    Some(v) => writeln!(out, "CHANGED, now {v:?}").unwrap(),
                    None => writeln!(out, "gone from the patched executable").unwrap(),
                }
            }
            None => {
                writeln!(out, "{name}: not found in the retail executable as u8, u16 or u32").unwrap()
            }
        }
    }
    Ok(())
}

/// The tables of the executable that say which song pack and which reverb a level has.
const TABLES: [(&str, &[u8]); 2] = [("levBank_Song", &LEVEL_SONGS), ("reverbMode", &LEVEL_REVERBS)];

/// Where a table is in the retail executable, found by its contents (the addresses are not in
/// the symbols we have), and how wide its entries are (1, 2 or 4 bytes).
fn find_table(exe: &[u8], values: &[u8]) -> Option<(usize, usize)> {
    [1usize, 2, 4].into_iter().find_map(|width| {
        let pattern: Vec<u8> = values
            .iter()
            .flat_map(|&v| (v as u32).to_le_bytes().into_iter().take(width))
            .collect();
        let at = exe.windows(pattern.len()).position(|w| w == pattern)?;
        Some((at, width))
    })
}

/// The `count` entries of a table at `at` in an executable.
fn read_table(exe: &[u8], at: usize, width: usize, count: usize) -> Option<Vec<u32>> {
    let slice = exe.get(at..at + width * count)?;
    Some(
        slice
            .chunks(width)
            .map(|c| c.iter().rev().fold(0, |n, &b| n << 8 | b as u32))
            .collect(),
    )
}

/// The audio that the patch changes on the track `level`: the song pack the patched executable
/// gives it, if that is not the game's, and the samples that pack plays that are not the game's;
/// its reverb if that is not. Every map made over a track keeps which one, so that what it does
/// not carry is the game's. `song_pack` names the pack to take when the patch does not say which
/// is the track's (`ctr-map audio` shows which it changes), and `reverb` the reverb mode.
pub fn carve(
    original: &Path,
    patch: &[u8],
    level: u32,
    song_pack: Option<u8>,
    reverb: Option<u8>,
) -> io::Result<Option<crate::Audio>> {
    let old = || Ok(BufReader::new(File::open(original)?));
    let new = || xdelta::Patched::new(BufReader::new(File::open(original)?), patch.to_vec());
    let mut audio = crate::Audio {
        level,
        song_pack: None,
        reverb: None,
        samples: Vec::new(),
    };

    // The tables, as the patched executable has them.
    let exes = (read_file_with(old, EXE)?, read_file_with(new, EXE)?);
    let table = |values: &[u8]| -> Option<u8> {
        let (Some(a), Some(b)) = (&exes.0, &exes.1) else {
            return None;
        };
        let (at, width) = find_table(a, values)?;
        let entries = read_table(b, at, width, values.len())?;
        entries.get(level as usize).and_then(|&v| u8::try_from(v).ok())
    };
    let pack = song_pack
        .or_else(|| table(&LEVEL_SONGS))
        .or(LEVEL_SONGS.get(level as usize).copied());
    let patched = table(&LEVEL_REVERBS);
    audio.reverb = reverb.or(patched.filter(|&r| LEVEL_REVERBS.get(level as usize).is_some_and(|&o| o != r)));
    if audio.reverb.is_some_and(|r| r as usize >= howl::tables::REVERBS.len()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("reverb {} is not one of the game's (0 to {})", audio.reverb.unwrap(), howl::tables::REVERBS.len() - 1),
        ));
    }

    if let Some(pack) = pack {
        let ((mut a, x), (mut b, y)) = (open_howl_with(old)?, open_howl_with(new)?);
        let (p, q) = (x.cseq(&mut a, pack as u16)?, y.cseq(&mut b, pack as u16)?);
        if let Some(q) = q.clone().filter(|_| song_pack.is_some() || !same_pack(&p, &q)) {
            let mut used: Vec<u16> = q.samples().collect();
            used.sort();
            used.dedup();
            for n in used {
                let adpcm = y.adpcm(&mut b, n)?;
                if adpcm != x.adpcm(&mut a, n)? {
                    audio.samples.extend(adpcm.map(|adpcm| (n, adpcm)));
                }
            }
            audio.song_pack = Some(q.data);
        }
    }
    Ok(Some(audio))
}

/// A note of a sequence: the delay before it, its opcode, and what the opcode takes.
type Event = (u32, u8, [u8; 2]);

/// How many bytes each opcode is, itself included (`howl::music`'s `op::SIZES`), and the one that
/// ends a sequence.
const OPCODE_SIZES: [usize; 11] = [1, 2, 2, 1, 2, 3, 2, 2, 2, 2, 2];
const OPCODE_END: u8 = 3;
/// The most notes of a sequence to read: a pack is at most 22 KB, so a sequence that goes on
/// longer than that is not ended.
const MAX_EVENTS: usize = 1 << 15;

/// The notes of the sequence that starts at `at` in a song pack, as the sound engine reads them
/// (`howl_GetNextNote`), up to its end.
fn events(data: &[u8], mut at: usize) -> Vec<Event> {
    let mut events = Vec::new();
    while events.len() < MAX_EVENTS {
        // The delay: 7 bits a byte, the top bit set on all but the last.
        let mut delay = 0u32;
        loop {
            let Some(&b) = data.get(at) else {
                return events;
            };
            at += 1;
            delay = (delay << 7).wrapping_add((b & 0x7f) as u32);
            if b & 0x80 == 0 {
                break;
            }
        }
        let Some(&opcode) = data.get(at) else {
            return events;
        };
        let Some(&size) = OPCODE_SIZES.get(opcode as usize) else {
            events.push((delay, opcode, [0; 2]));
            return events;
        };
        // What the opcode takes; the bytes after it are another note's.
        let arg = |n: usize| if n < size { data.get(at + n).copied().unwrap_or(0) } else { 0 };
        events.push((delay, opcode, [arg(1), arg(2)]));
        if opcode == OPCODE_END {
            return events;
        }
        at += size;
    }
    events
}

/// What a song pack sounds like, apart from where its bytes are: its instruments and drums, and
/// each song's tempo and each sequence's notes.
#[derive(PartialEq)]
struct Music {
    instruments: Vec<loader::howl::Instrument>,
    drums: Vec<loader::howl::Drum>,
    songs: Vec<(i16, i16, Vec<(bool, Vec<Event>)>)>,
}

fn music(c: &loader::howl::Cseq) -> Music {
    Music {
        instruments: c.instruments.clone(),
        drums: c.drums.clone(),
        songs: (c.songs.iter())
            .map(|s| {
                let sequences = (s.sequences.iter())
                    .map(|q| (q.drums, events(&c.data, q.notes)))
                    .collect();
                (s.bpm, s.tpqn, sequences)
            })
            .collect(),
    }
}

/// Whether two song packs (or the lack of them) sound the same: a patch that rebuilds the
/// soundtrack moves the bytes of every pack, and what is after a pack's own size is whatever it
/// left there.
fn same_pack(a: &Option<loader::howl::Cseq>, b: &Option<loader::howl::Cseq>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => music(a) == music(b),
        (None, None) => true,
        _ => false,
    }
}

/// How many sequences of the packs differ, of how many the second has.
fn sequences_differing(a: &Option<loader::howl::Cseq>, b: &Option<loader::howl::Cseq>) -> (usize, usize) {
    let all = |c: &Option<loader::howl::Cseq>| -> Vec<(i16, bool, Vec<Event>)> {
        let Some(c) = c else {
            return Vec::new();
        };
        let songs = c.songs.iter();
        songs
            .flat_map(|s| s.sequences.iter().map(|q| (s.bpm, q.drums, events(&c.data, q.notes))))
            .collect()
    };
    let (x, y) = (all(a), all(b));
    ((0..y.len().max(x.len())).filter(|&i| x.get(i) != y.get(i)).count(), y.len())
}

/// A song pack that a patch changes (`changed_packs`).
pub struct PackChange {
    pub pack: u8,
    /// The retail levels that use it.
    pub levels: Vec<usize>,
    /// The bpm of each of its songs, in the game and in the patch.
    pub was: Vec<i16>,
    pub now: Vec<i16>,
    /// How many of its sequences differ, of how many it has.
    pub sequences: (usize, usize),
    /// How many of the samples it plays are new or different.
    pub samples: usize,
}

/// The song packs of the patch that do not sound like the game's.
pub fn changed_packs(original: &Path, patch: &[u8]) -> io::Result<Vec<PackChange>> {
    let old = || Ok(BufReader::new(File::open(original)?));
    let new = || xdelta::Patched::new(BufReader::new(File::open(original)?), patch.to_vec());
    let ((mut a, x), (mut b, y)) = (open_howl_with(old)?, open_howl_with(new)?);
    let tempos = |c: &Option<loader::howl::Cseq>| -> Vec<i16> {
        c.iter().flat_map(|c| &c.songs).map(|s| s.bpm).collect()
    };
    let mut changes = Vec::new();
    for pack in 0..x.song_offsets.len().min(y.song_offsets.len()).min(256) {
        let (p, q) = (x.cseq(&mut a, pack as u16)?, y.cseq(&mut b, pack as u16)?);
        if same_pack(&p, &q) {
            continue;
        }
        let mut samples = 0;
        for n in q.iter().flat_map(|q| q.samples()) {
            samples += (x.adpcm(&mut a, n)? != y.adpcm(&mut b, n)?) as usize;
        }
        changes.push(PackChange {
            pack: pack as u8,
            levels: (LEVEL_SONGS.iter().enumerate())
                .filter(|&(_, &s)| s as usize == pack)
                .map(|(l, _)| l)
                .collect(),
            was: tempos(&p),
            now: tempos(&q),
            sequences: sequences_differing(&p, &q),
            samples,
        });
    }
    Ok(changes)
}

/// The levels whose reverb mode the patched executable changes, with the mode the game has and
/// the one the patch gives (`data.reverbMode`).
pub fn changed_reverbs(original: &Path, patch: &[u8]) -> io::Result<Vec<(usize, u8, u8)>> {
    let old = || Ok(BufReader::new(File::open(original)?));
    let new = || xdelta::Patched::new(BufReader::new(File::open(original)?), patch.to_vec());
    let (Some(a), Some(b)) = (read_file_with(old, EXE)?, read_file_with(new, EXE)?) else {
        return Ok(Vec::new());
    };
    let Some((at, width)) = find_table(&a, &LEVEL_REVERBS) else {
        return Ok(Vec::new());
    };
    let now = read_table(&b, at, width, LEVEL_REVERBS.len()).unwrap_or_default();
    Ok(LEVEL_REVERBS
        .iter()
        .zip(now)
        .enumerate()
        .filter(|&(_, (&was, now))| was as u32 != now)
        .map(|(level, (&was, now))| (level, was, now.min(255) as u8))
        .collect())
}
