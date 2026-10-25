//! The maps of `assets/maps/*.ctrmap` (made by `ctr-map import`), which the viewer lists after the
//! game's tracks. A map is a track under its own number (`Map::id`, with the top bit set); the
//! rest of the viewer takes that number for a `LevelId`.

use std::path::PathBuf;
use std::sync::OnceLock;

use map::Map;

static MAPS: OnceLock<Vec<(u32, Map)>> = OnceLock::new();

fn directory() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/maps")
}

/// Reads the maps there are, once. A file that is not a map is told of and left out.
pub fn load() {
    MAPS.get_or_init(|| {
        let Ok(entries) = std::fs::read_dir(directory()) else {
            return Vec::new();
        };
        let mut maps = Vec::new();
        for path in entries.filter_map(|e| Some(e.ok()?.path())) {
            if path.extension().is_none_or(|e| e != "ctrmap") {
                continue;
            }
            match Map::read(&path).and_then(|m| Ok((m.id()?, m))) {
                Ok(found) => maps.push(found),
                Err(e) => eprintln!("{}: {e}", path.display()),
            }
        }
        maps.sort_by(|a, b| a.1.name.cmp(&b.1.name));
        maps
    });
}

/// The map that has the track number `id`.
pub fn get(id: u32) -> Option<&'static Map> {
    MAPS.get()?.iter().find(|(n, _)| *n == id).map(|(_, m)| m)
}

/// The track numbers of the maps, by name.
pub fn ids() -> Vec<u32> {
    MAPS.get()
        .map_or_else(Vec::new, |m| m.iter().map(|(n, _)| *n).collect())
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
