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
    /// The track whose slot in `BIGFILE.BIG` the map was read from.
    pub from_track: u32,
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

/// The map that `patch` makes, found on `original` (the path of the USA `.bin`). A patch puts its
/// track in the slot of one of the game's (or of several, with the same file); `level` names the
/// one to read it from, and without it the patch has to change one level file or the same one
/// in all.
pub fn import(original: &Path, patch: &Path, level: Option<u32>) -> io::Result<Import> {
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
    let replaces = match (level, tracks.as_slice()) {
        (Some(id), _) => id,
        (None, [id]) => *id,
        (None, []) => return Err(invalid("the patch changes no track's level file")),
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
        let index = BigIndex::level(LevelId(replaces), Lod::OnePlayer, file);
        new_big.read(&mut new_disc, index.get())
    };
    let dram = DramFile::parse(&read(LevFile::Lev)?)?;
    let level = Level::parse(&dram)?;
    let vram = parse_vram_file(&read(LevFile::Vram)?)?;
    Ok(Import {
        map: Map {
            name,
            level,
            dram,
            vram,
        },
        from_track: replaces,
        changed,
        tracks,
    })
}
