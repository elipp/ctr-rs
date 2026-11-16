//! The game's own tracks as maps: `__builtin__dingo_canyon.ctrmap` and so on, made from the disc
//! once ([`export`]) so that a track no longer has to be read from it. Such a map is the track it
//! is named for (its `LevelId`, not a number made from its contents), so that what goes by the
//! track's number, like its music and the lobby's track list, is as it was.
//!
//! The name is the only thing that says which track a map stands in for, and a map's name is
//! whatever its maker typed. So [`level_of`] is for the maps of our own folder only: a map that
//! other players sent must not be taken for a track of the game.

use std::io;
use std::path::Path;

use loader::bigfile::BigFile;
use loader::bigindex::{BigIndex, LevFile, Lod};
use loader::disc::open_bigfile;
use loader::level::LevelId;

use crate::{Map, Relic};
use crate::import::from_files;

/// What the name of a map starts with when it is a track of the game.
pub const PREFIX: &str = "__builtin__";

/// The racing tracks (the arcade ones; the battle arenas are not here): the name in a file, what
/// the track is called, and its `LevelId`.
const TRACKS: [(&str, &str, LevelId); 18] = [
    ("dingo_canyon", "Dingo Canyon", LevelId::DINGO_CANYON),
    ("dragon_mines", "Dragon Mines", LevelId::DRAGON_MINES),
    ("blizzard_bluff", "Blizzard Bluff", LevelId::BLIZZARD_BLUFF),
    ("crash_cove", "Crash Cove", LevelId::CRASH_COVE),
    ("tiger_temple", "Tiger Temple", LevelId::TIGER_TEMPLE),
    ("papu_pyramid", "Papu's Pyramid", LevelId::PAPU_PYRAMID),
    ("roo_tubes", "Roo's Tubes", LevelId::ROO_TUBES),
    ("hot_air_skyway", "Hot Air Skyway", LevelId::HOT_AIR_SKYWAY),
    ("sewer_speedway", "Sewer Speedway", LevelId::SEWER_SPEEDWAY),
    ("mystery_caves", "Mystery Caves", LevelId::MYSTERY_CAVES),
    ("cortex_castle", "Cortex Castle", LevelId::CORTEX_CASTLE),
    ("n_gin_labs", "N. Gin Labs", LevelId::N_GIN_LABS),
    ("polar_pass", "Polar Pass", LevelId::POLAR_PASS),
    ("oxide_station", "Oxide Station", LevelId::OXIDE_STATION),
    ("coco_park", "Coco Park", LevelId::COCO_PARK),
    ("tiny_arena", "Tiny Arena", LevelId::TINY_ARENA),
    ("slide_coliseum", "Slide Coliseum", LevelId::SLIDE_COLISEUM),
    ("turbo_track", "Turbo Track", LevelId::TURBO_TRACK),
];

/// Gem Stone Valley, the adventure hub where the podium of a cup stands: made from the disc with
/// the racing tracks, but not one of them.
const PODIUM: (&str, &str, LevelId) = (
    "gem_stone_valley",
    "Gem Stone Valley",
    LevelId::GEM_STONE_VALLEY,
);

/// A level made from the disc: the name in its file, and its `LevelId`.
struct Made {
    slug: &'static str,
    id: LevelId,
}

impl Made {
    fn name(&self) -> String {
        format!("{PREFIX}{}", self.slug)
    }

    /// Whether the level has a `Lod::Relic` LEV ([`Map::relic`]): the racing tracks do.
    fn has_relic(&self) -> bool {
        TRACKS.iter().any(|t| t.2 == self.id)
    }

    /// The map, from the disc `big` is the index of.
    fn make<R: io::Read + io::Seek>(&self, disc: &mut R, big: &BigFile) -> io::Result<Map> {
        let mut read = |lod, file| big.read(disc, BigIndex::level(self.id, lod, file).get());
        let mut map = from_files(
            self.name(),
            &read(Lod::OnePlayer, LevFile::Lev)?,
            &read(Lod::OnePlayer, LevFile::Vram)?,
        )?;
        if self.has_relic() {
            let (lev, vram) = (read(Lod::Relic, LevFile::Lev)?, read(Lod::Relic, LevFile::Vram)?);
            map.relic = Some(Box::new(Relic::from_files(&lev, &vram)?));
        }
        Ok(map)
    }
}

/// Every level made from the disc.
fn all() -> impl Iterator<Item = Made> {
    let levels = TRACKS.iter().chain(std::iter::once(&PODIUM));
    levels.map(|&(slug, _, id)| Made { slug, id })
}

/// The game's track that is called `name` (see [`display_name`]), in any case.
pub fn id_of_display_name(name: &str) -> Option<u32> {
    TRACKS
        .iter()
        .find(|t| t.1.eq_ignore_ascii_case(name))
        .map(|t| t.2.0)
}

/// Whether a map's name claims to be a track of the game.
pub fn is_builtin_name(name: &str) -> bool {
    name.starts_with(PREFIX)
}

/// The level that a map named `name` stands in for; none if the name is not one of the levels'.
pub fn level_of(name: &str) -> Option<LevelId> {
    all().find(|m| m.name() == name).map(|m| m.id)
}

/// The name of the map of the game's racing track `id` (`__builtin__dingo_canyon`).
pub fn name_of(id: u32) -> Option<String> {
    TRACKS
        .iter()
        .find(|t| t.2.0 == id)
        .map(|t| format!("{PREFIX}{}", t.0))
}

/// What the game's track `id` is called, as a person says it (none for a battle arena).
pub fn display_name(id: u32) -> Option<&'static str> {
    TRACKS.iter().find(|t| t.2.0 == id).map(|t| t.1)
}

/// The short name of the game's track `id` that a command can take (`dingo_canyon`).
pub fn slug_of(id: u32) -> Option<&'static str> {
    TRACKS.iter().find(|t| t.2.0 == id).map(|t| t.0)
}

/// The game's track that has the short name `slug` (see [`slug_of`]).
pub fn id_of_slug(slug: &str) -> Option<u32> {
    TRACKS.iter().find(|t| t.0 == slug).map(|t| t.2.0)
}

/// Saves a map in `dir`, if it has no file there yet or one of an older
/// version. It is under another name until it is whole, so that a stop leaves no half map.
pub fn save(map: &Map, dir: &Path) -> io::Result<()> {
    let path = dir.join(format!("{}.ctrmap", map.name));
    if Map::is_current(&path) {
        return Ok(());
    }
    std::fs::create_dir_all(dir)?;
    let part = path.with_extension("ctrmap.part");
    map.write(&part)?;
    std::fs::rename(&part, &path)
}

fn file_name(made: &Made) -> String {
    format!("{}.ctrmap", made.name())
}

fn missing(dir: &Path) -> Vec<Made> {
    all()
        .filter(|m| !Map::is_current(&dir.join(file_name(m))))
        .collect()
}

/// How many levels `dir` has no map of: what [`export`] would make.
pub fn missing_count(dir: &Path) -> usize {
    missing(dir).len()
}

/// Makes the map of each level that `dir` has none of (or only one of an older version),
/// from the disc image `original`, and returns how many. The image is not opened if there is
/// nothing to make. The other `.ctrmap` files of `dir` are deleted: the folder is ours, and an
/// older build's leftovers (the `__relic` maps of version 9) would be refused at every start.
pub fn export(original: &Path, dir: &Path) -> io::Result<usize> {
    let missing = missing(dir);
    if missing.is_empty() {
        return Ok(0);
    }
    std::fs::create_dir_all(dir)?;
    let ours: Vec<String> = all().map(|m| file_name(&m)).collect();
    for entry in std::fs::read_dir(dir)?.filter_map(Result::ok) {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".ctrmap") && !ours.contains(&name) {
            std::fs::remove_file(entry.path())?;
        }
    }

    // The reading is quick, the compression slow: the maps are read in turn, then compressed
    // side by side.
    let (mut disc, big) = open_bigfile(original)?;
    let maps = (missing.iter())
        .map(|made| made.make(&mut disc, &big))
        .collect::<io::Result<Vec<_>>>()?;
    std::thread::scope(|scope| {
        let jobs: Vec<_> = (maps.iter())
            .map(|map| scope.spawn(move || save(map, dir)))
            .collect();
        for job in jobs {
            job.join().expect("a map thread panicked")?;
        }
        Ok(maps.len())
    })
}

/// The map of every level, made from the disc image `original` and only held in memory:
/// for a client with no folder to keep them in (a browser). Not compressed, which is what
/// [`export`] spends its time on.
pub fn make_all(original: &Path) -> io::Result<Vec<Map>> {
    let (mut disc, big) = open_bigfile(original)?;
    all().map(|made| made.make(&mut disc, &big)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_levels_go_both_ways() {
        assert_eq!(
            level_of("__builtin__dingo_canyon"),
            Some(LevelId::DINGO_CANYON)
        );
        assert_eq!(
            level_of("__builtin__turbo_track"),
            Some(LevelId::TURBO_TRACK)
        );
        assert_eq!(level_of("__builtin__nitro_court"), None);
        assert_eq!(
            level_of("__builtin__gem_stone_valley"),
            Some(LevelId::GEM_STONE_VALLEY)
        );
        assert_eq!(display_name(LevelId::GEM_STONE_VALLEY.0), None);
        assert_eq!(level_of("dingo_canyon"), None);
        assert!(is_builtin_name("__builtin__anything"));
        assert_eq!(display_name(5), Some("Papu's Pyramid"));
        assert_eq!(display_name(18), None);
        for (i, t) in TRACKS.iter().enumerate() {
            assert_eq!(t.2.0, i as u32, "{}", t.0);
        }
    }
}
