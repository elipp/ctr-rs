//! Makes a [`Map`] from a community patch: the patch is an xdelta3 file against the USA disc
//! image that replaces a track in `BIGFILE.BIG`. The patched disc is read through
//! [`xdelta::Patched`] (nothing is written), the entries of its index that differ from the
//! original's are found, and the track that changed is parsed as the viewer does.

use std::fs::File;
use std::io::{self, BufReader, Read, Seek};
use std::path::Path;

use loader::bigfile::BigFile;
use loader::bigindex::{BigIndex, LevFile, Lod};
use loader::disc::{open_bigfile, open_bigfile_with};
use loader::dram::DramFile;
use loader::lev::Level;
use loader::level::LevelId;
use loader::vram::parse_vram_file;

use crate::Map;

/// The tracks that have a group of files of their own in the index: the arcade tracks and the
/// battle arenas (`LevelId` 0 to 24).
const TRACKS: u32 = 25;

/// What an import found, besides the map.
pub struct Import {
    pub map: Map,
    /// Where in `BIGFILE.BIG` the map was read from: a track's slot, or the entries a tool put it in.
    pub source: String,
    /// The entries of `BIGFILE.BIG` that the patch changes or adds.
    pub changed: Vec<usize>,
    /// The tracks whose one-player level file is among them.
    pub tracks: Vec<u32>,
}

fn invalid(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

/// The entries of `patched` that are not the same as `original`'s.
fn changed_entries<A: Read + Seek, B: Read + Seek>(
    original: (&mut A, &BigFile),
    patched: (&mut B, &BigFile),
) -> io::Result<Vec<usize>> {
    let (a, big_a) = original;
    let (b, big_b) = patched;
    let mut changed = Vec::new();
    for i in 0..big_a.entries.len().max(big_b.entries.len()) {
        let (Some(x), Some(y)) = (big_a.entries.get(i), big_b.entries.get(i)) else {
            changed.push(i);
            continue;
        };
        if x.size != y.size || big_a.read(a, i)? != big_b.read(b, i)? {
            changed.push(i);
        }
    }
    Ok(changed)
}

/// Whether the entries `indices` of the disc are the same bytes.
fn same_file<R: Read + Seek>(
    big: &BigFile,
    disc: &mut R,
    indices: impl Iterator<Item = usize>,
) -> io::Result<bool> {
    let mut first: Option<Vec<u8>> = None;
    for index in indices {
        let bytes = big.read(disc, index)?;
        match &first {
            Some(f) if *f != bytes => return Ok(false),
            Some(_) => {}
            None => first = Some(bytes),
        }
    }
    Ok(true)
}

/// The changed entries that are a level and its textures: the first that parses as a level with
/// quad blocks, and the one after it (or else any other) that parses as VRAM images.
fn find_entries<R: Read + Seek>(
    changed: &[usize],
    big: &BigFile,
    disc: &mut R,
) -> io::Result<Option<(usize, usize)>> {
    // A level is at least tens of kilobytes; smaller entries are not worth parsing.
    let big_ones = |i: &&usize| big.entries.get(**i).is_some_and(|e| e.size > 16 * 1024);
    for &lev in changed.iter().filter(big_ones) {
        let is_level = DramFile::parse(&big.read(disc, lev)?)
            .and_then(|d| Level::parse(&d))
            .is_ok_and(|l| !l.quad_blocks.is_empty());
        if !is_level {
            continue;
        }
        let mut others = changed.iter().copied().filter(|&i| i != lev);
        let mut candidates: Vec<usize> = others.by_ref().collect();
        candidates.sort_by_key(|&i| i != lev + 1);
        for vram in candidates {
            if parse_vram_file(&big.read(disc, vram)?).is_ok_and(|v| !v.is_empty()) {
                return Ok(Some((lev, vram)));
            }
        }
    }
    Ok(None)
}

/// The map of a level file and a texture page file (`LevFile::Lev`, `LevFile::Vram`), as the
/// viewer parses them.
pub(crate) fn from_files(name: String, lev: &[u8], vram: &[u8]) -> io::Result<Map> {
    let dram = DramFile::parse(lev)?;
    let level = Level::parse(&dram)?;
    let vram = parse_vram_file(vram)?;
    Ok(Map {
        name,
        level,
        dram,
        vram,
        audio: None,
    })
}

/// The map that `patch` makes, found on `original` (the path of the USA `.bin`). A patch puts its
/// track in the slot of one of the game's (or of several, with the same file); `level` names the
/// one to read it from, and without it the patch has to change one level file or the same one
/// in all.
pub fn import(
    original: &Path,
    patch: &Path,
    level: Option<u32>,
    entries: Option<(usize, usize)>,
    song_pack: Option<u8>,
) -> io::Result<Import> {
    let name = patch
        .file_stem()
        .map_or_else(|| "map".into(), |s| s.to_string_lossy().into_owned());
    let bytes = std::fs::read(patch)?;

    let (mut disc, big) = open_bigfile(original)?;
    let patched = || {
        let source = BufReader::new(File::open(original)?);
        xdelta::Patched::new(source, bytes.clone())
    };
    let (mut new_disc, new_big) = open_bigfile_with(patched)?;

    let changed = changed_entries((&mut disc, &big), (&mut new_disc, &new_big))?;
    let one_player = |id| BigIndex::level(LevelId(id), Lod::OnePlayer, LevFile::Lev).get();
    let tracks: Vec<u32> = (0..TRACKS)
        .filter(|&id| changed.contains(&one_player(id)))
        .collect();
    // Some tools do not use a track's slot: the level and its textures are put in other entries
    // (CTR Editor's "TrackROM" uses two that the game uses for code).
    let mut entries = entries;
    if entries.is_none() && level.is_none() && tracks.is_empty() {
        entries = find_entries(&changed, &new_big, &mut new_disc)?;
    }
    let replaces = match (level, tracks.as_slice()) {
        // Entries named by hand: no slot to find.
        _ if entries.is_some() => level.unwrap_or(0),
        (Some(id), _) => id,
        (None, [id]) => *id,
        (None, []) => {
            let size = |big: &BigFile, i: usize| big.entries.get(i).map(|e| e.size);
            let list: Vec<String> = changed
                .iter()
                .take(24)
                .map(|&i| format!("#{i}: {:?} -> {:?}", size(&big, i), size(&new_big, i)))
                .collect();
            return Err(invalid(format!(
                "the patch changes no track's level file; it changes {} entries of BIGFILE.BIG (index: old -> new size): {}",
                changed.len(),
                list.join(", ")
            )));
        }
        // The same file in several slots (a patch may put its track in more than one): the first.
        (None, many)
            if same_file(
                &new_big,
                &mut new_disc,
                many.iter().map(|&id| one_player(id)),
            )? =>
        {
            many[0]
        }
        (None, many) => {
            let size = |big: &BigFile, id| big.entries.get(one_player(id) as usize).map(|e| e.size);
            let sizes: Vec<String> = many
                .iter()
                .map(|&id| {
                    format!(
                        "track {id}: {} -> {} bytes",
                        size(&big, id).unwrap_or(0),
                        size(&new_big, id).unwrap_or(0)
                    )
                })
                .collect();
            return Err(invalid(format!(
                "the patch changes the level files of several tracks ({}): pick one with --level",
                sizes.join("; ")
            )));
        }
    };

    let mut read = |file| {
        let index = match entries {
            Some((lev, _)) if file == LevFile::Lev => lev,
            Some((_, vram)) => vram,
            None => BigIndex::level(LevelId(replaces), Lod::OnePlayer, file).get(),
        };
        new_big.read(&mut new_disc, index)
    };
    let mut map = from_files(name, &read(LevFile::Lev)?, &read(LevFile::Vram)?)?;
    map.audio = crate::audio::carve(original, &bytes, replaces, song_pack)?;
    Ok(Import {
        map,
        source: match entries {
            Some((lev, vram)) => format!("entries {lev} (level) and {vram} (VRAM)"),
            None => format!("the slot of track {replaces}"),
        },
        changed,
        tracks,
    })
}
