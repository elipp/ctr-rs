//! The maps of the game's own tracks, made from the disc, read without it.

use std::path::PathBuf;

use loader::level::LevelId;
use map::Map;
use map::builtin::{level_of, name_of};

/// `CTR_DISC_DIR` overrides the default of `<repo>/assets/disc`.
#[test]
fn every_racing_track_has_its_map() {
    let dir = std::env::var_os("CTR_DISC_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/disc"))
        .join("maps");
    if !dir.is_dir() {
        eprintln!("skipping: {} not found (set CTR_DISC_DIR)", dir.display());
        return;
    }
    assert_eq!(map::builtin::missing_count(&dir), 0);
    for id in LevelId::DINGO_CANYON.0..=LevelId::TURBO_TRACK.0 {
        let name = name_of(id).expect("a racing track has a name");
        let path = dir.join(format!("{name}.ctrmap"));
        let map = Map::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert_eq!(map.name, name);
        assert_eq!(level_of(&map.name), Some(LevelId(id)));
        assert!(!map.level.quad_blocks.is_empty(), "{name} has no collision");
    }
}
