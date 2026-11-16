//! Makes [`Map`]s from a community patch: the patch is an xdelta3 file against the USA disc image
//! that replaces tracks in `BIGFILE.BIG`. The patched disc is read through [`xdelta::Patched`]
//! (nothing is written), the entries of its index that differ from the original's are found
//! ([`Scan`]), and each track that changed is parsed as the viewer does.
//!
//! A pack (Coco Team Racing's 160 MB one) changes many slots at once, maybe code too: each slot
//! with its own level file is a map, and what a map can't carry is listed ([`Import::unfollowed`]),
//! not followed: the port plays retail rules.

use std::io::{self, Read, Seek};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use loader::bigfile::BigFile;
use loader::bigindex::{BigIndex, LevFile, Lod};
use loader::disc::{Disc, FileDisc, RawSectorReader, open_bigfile, open_bigfile_with};
use loader::dram::DramFile;
use loader::lev::Level;
use loader::level::LevelId;
use loader::vram::parse_vram_file;

use crate::{Map, Relic};

/// The tracks that have a group of files of their own in the index: the arcade tracks and the
/// battle arenas (`LevelId` 0 to 24).
const TRACKS: u32 = 25;
/// The ones of them that have a relic LEV (the arcade tracks).
const RELIC_TRACKS: u32 = 18;

/// Where a map is read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// A track's slot (`LevelId`), and its relic slot if the patch changed that too.
    Slot(u32),
    /// Entries that a tool put the level and its textures in (CTR Editor's "TrackROM" uses two
    /// that the game uses for code), over the track `over` (whose music is the map's).
    Entries { lev: usize, vram: usize, over: u32 },
}

impl Source {
    fn track(self) -> u32 {
        match self {
            Source::Slot(id) | Source::Entries { over: id, .. } => id,
        }
    }

    fn describe(self) -> String {
        match self {
            Source::Slot(id) => format!("the slot of track {id}"),
            Source::Entries { lev, vram, .. } => format!("entries {lev} (level) and {vram} (VRAM)"),
        }
    }
}

/// A map an import made.
pub struct Imported {
    pub map: Map,
    pub source: String,
}

/// What an import found.
pub struct Import {
    pub maps: Vec<Imported>,
    /// The entries of `BIGFILE.BIG` that the patch changes or adds.
    pub changed: Vec<usize>,
    /// The tracks whose one-player level file is among them.
    pub tracks: Vec<u32>,
    /// What else the patch changes, which no map carries: code (the exe, overlays), menus,
    /// models, other files of the disc.
    pub unfollowed: Vec<String>,
}

fn invalid(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

type PatchedDisc = RawSectorReader<xdelta::Patched<FileDisc>>;

/// A patch read against the disc: both discs open, and what the patch changes in `BIGFILE.BIG`.
pub struct Scan {
    original: PathBuf,
    name: String,
    patch: Arc<[u8]>,
    disc: Disc,
    big: BigFile,
    new_disc: PatchedDisc,
    new_big: BigFile,
    pub changed: Vec<usize>,
    pub tracks: Vec<u32>,
}

fn one_player(id: u32) -> usize {
    BigIndex::level(LevelId(id), Lod::OnePlayer, LevFile::Lev).get()
}

impl Scan {
    /// Reads the patch `patch` for the maps called `name`, on `original` (the USA `.bin`).
    pub fn new(original: &Path, name: String, patch: Arc<[u8]>) -> io::Result<Scan> {
        let (mut disc, big) = open_bigfile(original)?;
        let (mut new_disc, new_big) = open_bigfile_with(patched(original, &patch))?;
        let changed = changed_entries((&mut disc, &big), (&mut new_disc, &new_big))?;
        let tracks = (0..TRACKS)
            .filter(|&id| changed.contains(&one_player(id)))
            .collect();
        Ok(Scan {
            original: original.to_owned(),
            name,
            patch,
            disc,
            big,
            new_disc,
            new_big,
            changed,
            tracks,
        })
    }

    /// Where the maps are: `entries` named by hand, the slot `level`, or else every changed slot
    /// with a level file of its own (a pack may put the same file in several: one map), or the
    /// entries that a tool used instead of a slot.
    pub fn sources(
        &mut self,
        level: Option<u32>,
        entries: Option<(usize, usize)>,
    ) -> io::Result<Vec<Source>> {
        let entries = match entries {
            Some(e) => Some(e),
            None if level.is_none() && self.tracks.is_empty() => {
                find_entries(&self.changed, &self.new_big, &mut self.new_disc)?
            }
            None => None,
        };
        if let Some((lev, vram)) = entries {
            let over = level.unwrap_or(0);
            return Ok(vec![Source::Entries { lev, vram, over }]);
        }
        if let Some(id) = level {
            return Ok(vec![Source::Slot(id)]);
        }
        if self.tracks.is_empty() {
            let size = |big: &BigFile, i: usize| big.entries.get(i).map(|e| e.size);
            let list: Vec<String> = (self.changed.iter().take(24))
                .map(|&i| format!("#{i}: {:?} -> {:?}", size(&self.big, i), size(&self.new_big, i)))
                .collect();
            return Err(invalid(format!(
                "the patch changes no track's level file; it changes {} entries of BIGFILE.BIG (index: old -> new size): {}",
                self.changed.len(),
                list.join(", ")
            )));
        }
        let mut seen: Vec<Vec<u8>> = Vec::new();
        let mut sources = Vec::new();
        for &id in &self.tracks {
            let lev = self.new_big.read(&mut self.new_disc, one_player(id))?;
            if !seen.contains(&lev) {
                seen.push(lev);
                sources.push(Source::Slot(id));
            }
        }
        Ok(sources)
    }

    /// The maps of `sources`. `song_pack` and `reverb` are a one-map import's (see
    /// `audio::carve`): with several, each map has its slot's own pack and the patch's reverb
    /// for it.
    pub fn import(
        mut self,
        sources: &[Source],
        song_pack: Option<u8>,
        reverb: Option<u8>,
    ) -> io::Result<Import> {
        let one = sources.len() == 1;
        let names = if one { None } else { self.track_names() };
        let mut maps = Vec::new();
        for &source in sources {
            let id = source.track();
            let name = match &names {
                None => self.name.clone(),
                Some((retail, patched)) => {
                    let get = |n: &Vec<Vec<u8>>| {
                        let s = n.get(loader::lng::track_name(LevelId(id))?)?;
                        Some(String::from_utf8_lossy(s).trim().to_string())
                    };
                    let (was, now) = (get(retail), get(patched));
                    match now.filter(|n| !n.is_empty() && Some(n) != was.as_ref()) {
                        Some(theirs) => theirs,
                        None => {
                            let ours = crate::builtin::display_name(id).map(String::from);
                            let ours = ours.or(was).unwrap_or_else(|| format!("track {id}"));
                            format!("{} - {ours}", self.name)
                        }
                    }
                }
            };
            let (lev, vram) = match source {
                Source::Slot(id) => (
                    BigIndex::level(LevelId(id), Lod::OnePlayer, LevFile::Lev).get(),
                    BigIndex::level(LevelId(id), Lod::OnePlayer, LevFile::Vram).get(),
                ),
                Source::Entries { lev, vram, .. } => (lev, vram),
            };
            let mut read = |i| self.new_big.read(&mut self.new_disc, i);
            let mut map = from_files(name, &read(lev)?, &read(vram)?)?;
            // An unchanged relic slot is retail's, over retail's geometry.
            let relic = |file| BigIndex::level(LevelId(id), Lod::Relic, file).get();
            if matches!(source, Source::Slot(_))
                && id < RELIC_TRACKS
                && self.changed.contains(&relic(LevFile::Lev))
            {
                let (lev, vram) = (read(relic(LevFile::Lev))?, read(relic(LevFile::Vram))?);
                map.relic = Some(Box::new(Relic::from_files(&lev, &vram)?));
            }
            let (pack, mode) = if one { (song_pack, reverb) } else { (None, None) };
            map.audio = crate::audio::carve(&self.original, &self.patch, id, pack, mode, !one)?;
            maps.push(Imported {
                map,
                source: source.describe(),
            });
        }
        let unfollowed = self.unfollowed(sources)?;
        Ok(Import {
            maps,
            changed: self.changed,
            tracks: self.tracks,
            unfollowed,
        })
    }

    /// The English strings of the disc and of the patched one; none if either won't parse (the
    /// maps are then named for the retail tracks).
    fn track_names(&mut self) -> Option<(Vec<Vec<u8>>, Vec<Vec<u8>>)> {
        let entry = BigIndex::LANGUAGE_FILE.nth(loader::lng::ENGLISH).get();
        let retail = loader::lng::parse(&self.big.read(&mut self.disc, entry).ok()?).ok()?;
        let patched = self.new_big.read(&mut self.new_disc, entry).ok()?;
        Some((retail, loader::lng::parse(&patched).ok()?))
    }

    /// What the maps of `sources` leave out: the changed entries outside the track groups (but
    /// those a source was read from), and the disc's other files that differ.
    fn unfollowed(&mut self, sources: &[Source]) -> io::Result<Vec<String>> {
        let used: Vec<usize> = (sources.iter())
            .flat_map(|s| match *s {
                Source::Entries { lev, vram, .. } => vec![lev, vram],
                Source::Slot(_) => Vec::new(),
            })
            .collect();
        let mut out: Vec<String> = (self.changed.iter())
            .filter(|&&i| !BigIndex(i as u32).is_track_file() && !used.contains(&i))
            .map(|&i| match i < self.big.entries.len() {
                true => BigIndex(i as u32).describe(),
                false => format!("#{i} (added)"),
            })
            .collect();

        let open = || loader::disc::open_file(&self.original);
        let new = patched(&self.original, &self.patch);
        let (old_files, new_files) = (loader::disc::files_with(open)?, loader::disc::files_with(&new)?);
        for file in &new_files {
            if file.path == "BIGFILE.BIG" {
                continue;
            }
            let note = match file.path.as_str() {
                crate::audio::EXE => " (the executable; its song and reverb tables are carved)",
                crate::audio::HWL => " (the music; carved per map)",
                _ => "",
            };
            match old_files.iter().find(|f| f.path == file.path) {
                None => out.push(format!("{} (added){note}", file.path)),
                Some(old) => {
                    if !loader::disc::same_file((&mut self.disc, old), (&mut self.new_disc, file))? {
                        out.push(format!("{}{note}", file.path));
                    }
                }
            }
        }
        for file in &old_files {
            if !new_files.iter().any(|f| f.path == file.path) {
                out.push(format!("{} (removed)", file.path));
            }
        }
        Ok(out)
    }
}

/// What opens the disc that `patch` makes of `original`.
fn patched(
    original: &Path,
    patch: &Arc<[u8]>,
) -> impl Fn() -> io::Result<xdelta::Patched<FileDisc>> {
    let (original, patch) = (original.to_owned(), patch.clone());
    move || xdelta::Patched::new(loader::disc::open_file(&original)?, patch.clone())
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

/// The changed entries that are a level and its textures: the first that parses as a level with
/// quad blocks, and the one after it (or else any other) that parses as VRAM images. Not a
/// retail level's files: those are that level edited (Coco Team Racing's Citadel City hub), and
/// lev+1 there is its PTR file, not its VRAM.
fn find_entries<R: Read + Seek>(
    changed: &[usize],
    big: &BigFile,
    disc: &mut R,
) -> io::Result<Option<(usize, usize)>> {
    // A level is at least tens of kilobytes; smaller entries are not worth parsing.
    let big_ones = |i: &&usize| big.entries.get(**i).is_some_and(|e| e.size > 16 * 1024);
    let not_levels = |i: &&usize| !BigIndex(**i as u32).is_level_file();
    for &lev in changed.iter().filter(big_ones).filter(not_levels) {
        let is_level = DramFile::parse(&big.read(disc, lev)?)
            .and_then(|d| Level::parse(&d))
            .is_ok_and(|l| !l.quad_blocks.is_empty());
        if !is_level {
            continue;
        }
        let mut candidates: Vec<usize> = (changed.iter().filter(not_levels))
            .copied()
            .filter(|&i| i != lev)
            .collect();
        candidates.sort_by_key(|&i| i != lev + 1);
        for vram in candidates {
            if parse_vram_file(&big.read(disc, vram)?).is_ok_and(|v| !v.is_empty()) {
                return Ok(Some((lev, vram)));
            }
        }
    }
    Ok(None)
}

impl Relic {
    /// A relic level file and its texture pages, as the viewer parses them.
    pub(crate) fn from_files(lev: &[u8], vram: &[u8]) -> io::Result<Relic> {
        let dram = DramFile::parse(lev)?;
        Ok(Relic {
            level: Level::parse(&dram)?,
            dram,
            vram: parse_vram_file(vram)?,
        })
    }
}

/// The map of a level file and a texture page file (`LevFile::Lev`, `LevFile::Vram`), as the
/// viewer parses them.
pub(crate) fn from_files(name: String, lev: &[u8], vram: &[u8]) -> io::Result<Map> {
    let Relic { level, dram, vram } = Relic::from_files(lev, vram)?;
    Ok(Map {
        name,
        level,
        dram,
        vram,
        audio: None,
        relic: None,
    })
}

/// The maps of `patch` with nobody to ask (the client's drag and drop): every changed track, as
/// the CLI finds them, every song pack the patch changes a soundtrack of a one-track patch, and
/// its reverb the patch's when it changes exactly one, the track's own otherwise.
pub fn import_unattended(original: &Path, name: String, patch: Arc<[u8]>) -> io::Result<Import> {
    let mut scan = Scan::new(original, name, patch)?;
    let sources = scan.sources(None, None)?;
    let reverb = match crate::audio::changed_reverbs(original, &scan.patch)?.as_slice() {
        [(_, _, now)] if sources.len() == 1 => Some(*now),
        _ => None,
    };
    scan.import(&sources, None, reverb)
}
