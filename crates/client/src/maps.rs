//! The maps of `maps/*.ctrmap` (see `paths.rs`; made by `ctr-map import`), which the viewer lists
//! after the game's tracks. A map is a track under its own number (`Map::id`, with the top bit set); the
//! rest of the viewer takes that number for a `LevelId`.
//!
//! The exception is the game's own tracks, made from the disc the first time the client runs
//! (`map::builtin`): such a map has the number of the track it is named for, is not listed as a
//! map, and is only taken from the folder of what is made from the disc (`paths::builtin_maps`),
//! never from our own maps or from what other players sent.

use std::path::PathBuf;
use std::sync::{Mutex, Once};

use map::{Map, builtin};

/// A map that is loaded.
struct Entry {
    id: u32,
    /// `Map::hash`, which tells two maps of the same id apart.
    hash: u64,
    /// What a player who lacks the map is sent.
    file: File,
    map: &'static Map,
}

/// Where the bytes of a map's file are.
enum File {
    Disk(PathBuf),
    /// A map sent to us in a browser, which has no folder to keep it in: the bytes as they came.
    #[cfg(target_arch = "wasm32")]
    Held(Vec<u8>),
    /// One of the game's tracks in a browser: made from the disc at the start, and never sent.
    #[cfg(target_arch = "wasm32")]
    Made,
}

/// The maps there are: read once, then added to by the ones that come in over the network (they
/// live as long as the program, so that the rest of the viewer can hold on to them).
static MAPS: Mutex<Vec<Entry>> = Mutex::new(Vec::new());
static LOADED: Once = Once::new();
fn directory() -> PathBuf {
    crate::paths::maps()
}

/// Where the maps of the game's own tracks are.
fn builtin_directory() -> PathBuf {
    crate::paths::builtin_maps()
}

/// Where the maps that other players sent are kept: unlike `maps`, which is ours,
/// everything in it may go (`/cache purge`).
fn cache_directory() -> PathBuf {
    crate::paths::cached_maps()
}

/// Reads the maps there are, once, before the log is set up (what is wrong with a file is
/// printed). A file that is not a map is told of and left out.
pub fn load() {
    LOADED.call_once(|| {
        let mut maps = Vec::new();
        // Whether the folder is the one the game's own tracks are in.
        for (builtins, dir) in [
            (false, directory()),
            (true, builtin_directory()),
            (false, cache_directory()),
        ] {
            let Ok(entries) = std::fs::read_dir(dir) else {
                continue;
            };
            for path in entries.filter_map(|e| Some(e.ok()?.path())) {
                if path.extension().is_none_or(|e| e != "ctrmap") {
                    continue;
                }
                let read = Map::read(&path).and_then(|m| Ok((m.id()?, m.hash()?, m)));
                let (mut id, hash, map) = match read {
                    Ok(read) => read,
                    Err(e) => {
                        eprintln!("{}: {e}", path.display());
                        continue;
                    }
                };
                if builtin::is_builtin_name(&map.name) != builtins {
                    let why = if builtins {
                        "that folder is for the maps of the game's tracks"
                    } else {
                        "a map cannot be named for one of the game's tracks"
                    };
                    eprintln!("{}: {why}", path.display());
                    continue;
                }
                if builtins {
                    match builtin::level_of(&map.name) {
                        Some(level) => id = level.0,
                        None => {
                            eprintln!("{}: not a map of one of the game's tracks", path.display());
                            continue;
                        }
                    }
                }
                maps.push(Entry {
                    id,
                    hash,
                    file: File::Disk(path),
                    map: Box::leak(Box::new(map)),
                });
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

/// The hash of the contents of the map `id`: a game track's too, whose map is made from the disc.
pub fn hash(id: u32) -> Option<u64> {
    let maps = MAPS.lock().unwrap();
    maps.iter().find(|e| e.id == id).map(|e| e.hash)
}

/// Whether we have the map `id` with these very contents. For one of the game's own tracks, whose
/// map is made from our disc, it is whether ours is the same as the host's: that, or there is
/// nothing to tell (no `hash`, or none of ours). A host never sends those, so the other answer
/// is "ask, and be told no".
pub fn has(id: u32, hash: u64) -> bool {
    if !map::is_map_id(id) {
        return hash == 0 || self::hash(id).is_none_or(|ours| ours == hash);
    }
    let maps = MAPS.lock().unwrap();
    maps.iter().any(|e| e.id == id && e.hash == hash)
}

/// The hash of the map `id`, and its file's bytes: what the others are sent when they lack it.
pub fn file(id: u32) -> Option<(u64, Vec<u8>)> {
    let (hash, path) = {
        let maps = MAPS.lock().unwrap();
        let e = maps.iter().find(|e| e.id == id)?;
        match &e.file {
            File::Disk(path) => (e.hash, path.clone()),
            #[cfg(target_arch = "wasm32")]
            File::Held(bytes) => return Some((e.hash, bytes.clone())),
            #[cfg(target_arch = "wasm32")]
            File::Made => return None,
        }
    };
    Some((hash, std::fs::read(path).ok()?))
}

/// Takes in a map that was sent to us as the bytes of its file: it has to be the map `id` with
/// this `hash`. It is saved to the cache (`cache/maps/`), or held in memory in a browser, and
/// listed from then on.
pub fn receive(id: u32, hash: u64, bytes: &[u8]) -> Result<(), String> {
    let map = Map::from_bytes(bytes).map_err(|e| e.to_string())?;
    if builtin::is_builtin_name(&map.name) {
        return Err("a map cannot be named for one of the game's tracks".into());
    }
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
    let file = keep(&map.name, id, bytes)?;
    maps.push(Entry {
        id,
        hash,
        file,
        map: Box::leak(Box::new(map)),
    });
    Ok(())
}

/// Saves a map that was sent to us to the cache.
#[cfg(not(target_arch = "wasm32"))]
fn keep(name: &str, id: u32, bytes: &[u8]) -> Result<File, String> {
    let dir = cache_directory();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    // The name of the patch may be one that we have a map of already.
    let stem: String = name
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
    Ok(File::Disk(path))
}

/// A browser has no folder: there a map sent to us is held in memory, until the page is left.
#[cfg(target_arch = "wasm32")]
fn keep(_name: &str, _id: u32, bytes: &[u8]) -> Result<File, String> {
    Ok(File::Held(bytes.to_vec()))
}

/// Deletes the cache and forgets the maps that were in it; returns how many. (A map that is on
/// screen stays up until the level changes.)
pub fn purge() -> Result<usize, String> {
    let dir = cache_directory();
    let mut maps = MAPS.lock().unwrap();
    let before = maps.len();
    maps.retain(|e| match &e.file {
        File::Disk(path) => !path.starts_with(&dir),
        #[cfg(target_arch = "wasm32")]
        File::Held(_) => false,
        #[cfg(target_arch = "wasm32")]
        File::Made => true,
    });
    #[cfg(not(target_arch = "wasm32"))]
    match std::fs::remove_dir_all(&dir) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("{}: {e}", dir.display())),
    }
    Ok(before - maps.len())
}

/// Makes the maps of the game's racing tracks that we lack, or have of an older version, from the
/// disc, which takes a while the first time. (`disc::locate` has seen to it that there is a disc
/// when there are any to make.)
#[cfg(not(target_arch = "wasm32"))]
pub fn make_builtins() {
    let dir = builtin_directory();
    if map::builtin::missing_count(&dir) == 0 {
        return;
    }
    let Some(image) = crate::disc::image() else {
        eprintln!("The maps of the game's tracks are missing, and there is no disc image to make them from");
        return;
    };
    eprintln!("Making maps of the game's tracks from the disc (this is done once)...");
    match map::builtin::export(&image, &dir) {
        Ok(made) => eprintln!("Made {made} maps in {}", dir.display()),
        Err(e) => eprintln!("Could not make the maps of the game's tracks: {e}"),
    }
}

/// A browser has no folder to keep them in: there the maps of the game's tracks are made from the
/// disc at every start, and only held in memory.
#[cfg(target_arch = "wasm32")]
pub fn make_builtins() {
    let Some(image) = crate::disc::image() else {
        return;
    };
    let made = match builtin::make_all(&image) {
        Ok(made) => made,
        Err(e) => {
            eprintln!("Could not make the maps of the game's tracks: {e}");
            return;
        }
    };
    let mut maps = MAPS.lock().unwrap();
    for map in made {
        let (Some(level), Ok(hash)) = (builtin::level_of(&map.name), map.hash()) else {
            continue;
        };
        maps.push(Entry {
            id: level.0,
            hash,
            file: File::Made,
            map: Box::leak(Box::new(map)),
        });
    }
}

/// The track numbers of the maps, by name. (Not the game's own tracks, which are maps too.)
pub fn ids() -> Vec<u32> {
    let maps = MAPS.lock().unwrap();
    maps.iter()
        .map(|e| e.id)
        .filter(|&id| map::is_map_id(id))
        .collect()
}

/// What the level `id` is called: the game's track, a map's name, or the track number.
pub fn name(id: u32) -> String {
    if let Some(name) = builtin::display_name(id) {
        return name.into();
    }
    get(id).map_or_else(|| format!("level {id}"), |m| m.name.clone())
}

/// The tracks `[` and `]` go through: the game's racing tracks, then the maps.
pub fn cycle() -> Vec<u32> {
    use loader::level::LevelId;
    (LevelId::DINGO_CANYON.0..=LevelId::TURBO_TRACK.0)
        .chain(ids())
        .collect()
}
