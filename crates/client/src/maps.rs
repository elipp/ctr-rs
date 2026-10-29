//! The maps of `maps/*.ctrmap` (see `paths.rs`; made by `ctr-map import`), which the viewer lists
//! after the game's tracks. A map is a track under its own number (`Map::id`, with the top bit set); the
//! rest of the viewer takes that number for a `LevelId`.

use std::path::PathBuf;
use std::sync::{Mutex, Once};

use bevy::log::warn;
use map::Map;

/// A map that is loaded.
struct Entry {
    id: u32,
    /// `Map::hash`, which tells two maps of the same id apart.
    hash: u64,
    /// The file it came from, which is what a player who lacks the map is sent.
    path: PathBuf,
    map: &'static Map,
}

/// The maps there are: read once, then added to by the ones that come in over the network (they
/// live as long as the program, so that the rest of the viewer can hold on to them).
static MAPS: Mutex<Vec<Entry>> = Mutex::new(Vec::new());
static LOADED: Once = Once::new();

fn directory() -> PathBuf {
    crate::paths::maps()
}

/// Where the maps that other players sent are kept: unlike `maps`, which is ours,
/// everything in it may go (`/cache purge`).
fn cache_directory() -> PathBuf {
    crate::paths::cached_maps()
}

/// Reads the maps there are, once. A file that is not a map is told of and left out.
pub fn load() {
    LOADED.call_once(|| {
        let mut maps = Vec::new();
        for path in [directory(), cache_directory()]
            .into_iter()
            .filter_map(|dir| std::fs::read_dir(dir).ok())
            .flatten()
            .filter_map(|e| Some(e.ok()?.path()))
        {
            if path.extension().is_none_or(|e| e != "ctrmap") {
                continue;
            }
            match Map::read(&path).and_then(|m| Ok((m.id()?, m.hash()?, m))) {
                Ok((id, hash, map)) => maps.push(Entry {
                    id,
                    hash,
                    path,
                    map: Box::leak(Box::new(map)),
                }),
                Err(e) => warn!("{}: {e}", path.display()),
            }
        }
        maps.sort_by(|a, b| a.map.name.cmp(&b.map.name));
        MAPS.lock().unwrap().extend(maps);
    });
}

/// The map that has the track number `id`.
pub fn get(id: u32) -> Option<&'static Map> {
    let maps = MAPS.lock().unwrap();
    maps.iter().find(|e| e.id == id).map(|e| e.map)
}

/// Whether we have the map `id` with these very contents.
pub fn has(id: u32, hash: u64) -> bool {
    let maps = MAPS.lock().unwrap();
    maps.iter().any(|e| e.id == id && e.hash == hash)
}

/// The hash of the map `id`, and its file's bytes: what the others are sent when they lack it.
pub fn file(id: u32) -> Option<(u64, Vec<u8>)> {
    let (hash, path) = {
        let maps = MAPS.lock().unwrap();
        let e = maps.iter().find(|e| e.id == id)?;
        (e.hash, e.path.clone())
    };
    Some((hash, std::fs::read(path).ok()?))
}

/// Takes in a map that was sent to us as the bytes of its file: it has to be the map `id` with
/// this `hash`. It is saved to the cache (`cache/maps/`) and listed from then on.
pub fn receive(id: u32, hash: u64, bytes: &[u8]) -> Result<(), String> {
    let map = Map::from_bytes(bytes).map_err(|e| e.to_string())?;
    let got = (
        map.id().map_err(|e| e.to_string())?,
        map.hash().map_err(|e| e.to_string())?,
    );
    if got != (id, hash) {
        return Err(format!(
            "the map sent is {:#x} / {:#x}, not the {id:#x} / {hash:#x} asked for",
            got.0, got.1
        ));
    }
    let mut maps = MAPS.lock().unwrap();
    if maps.iter().any(|e| e.id == id) {
        // Another map has this id (about one in two billion): not ours to replace.
        return Err(format!("a different map already has the id {id:#x}"));
    }
    let dir = cache_directory();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    // The name of the patch may be one that we have a map of already.
    let stem: String = map
        .name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let mut path = dir.join(format!("{stem}.ctrmap"));
    if path.exists() {
        path = dir.join(format!("{stem}-{id:08x}.ctrmap"));
    }
    std::fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    maps.push(Entry {
        id,
        hash,
        path,
        map: Box::leak(Box::new(map)),
    });
    Ok(())
}

/// Deletes the cache and forgets the maps that were in it; returns how many. (A map that is on
/// screen stays up until the level changes.)
pub fn purge() -> Result<usize, String> {
    let dir = cache_directory();
    let mut maps = MAPS.lock().unwrap();
    let before = maps.len();
    maps.retain(|e| !e.path.starts_with(&dir));
    match std::fs::remove_dir_all(&dir) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("{}: {e}", dir.display())),
    }
    Ok(before - maps.len())
}

/// The track numbers of the maps, by name.
pub fn ids() -> Vec<u32> {
    MAPS.lock().unwrap().iter().map(|e| e.id).collect()
}

/// What the level `id` is called: a map's name, or the game's track number.
pub fn name(id: u32) -> String {
    get(id).map_or_else(|| format!("level {id}"), |m| m.name.clone())
}

/// The tracks `[` and `]` go through: the game's racing tracks, then the maps.
pub fn cycle() -> Vec<u32> {
    use loader::level::LevelId;
    (LevelId::DINGO_CANYON.0..=LevelId::TURBO_TRACK.0)
        .chain(ids())
        .collect()
}
