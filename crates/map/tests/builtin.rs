//! The maps of the game's own tracks, made from the disc, read without it.

use std::path::PathBuf;

use loader::level::LevelId;
use map::Map;
use map::builtin::{level_of, name_of};

/// `CTR_BIN` overrides the default disc image of `<repo>/assets/ctr-u.bin`. The maps are made
/// into a folder of their own under the system's temporary one, so that what the client keeps in
/// `assets/disc/maps` (stale or not) does not decide the result.
#[test]
fn needs_disc_every_racing_track_and_the_podium_hub_has_its_map_and_each_track_its_relic_lev() {
    let image = std::env::var_os("CTR_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/ctr-u.bin"));
    if !image.is_file() {
        eprintln!("skipping: {} not found (set CTR_BIN)", image.display());
        return;
    }
    let dir = std::env::temp_dir().join(format!("ctrmap-builtin-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    let tracks = LevelId::DINGO_CANYON.0..=LevelId::TURBO_TRACK.0;
    let made = map::builtin::export(&image, &dir)
        .unwrap_or_else(|e| panic!("{}: {e}", image.display()));
    // And Gem Stone Valley, for the podium.
    assert_eq!(made, tracks.clone().count() + 1);
    assert_eq!(map::builtin::missing_count(&dir), 0);
    let hub = Map::read(&dir.join("__builtin__gem_stone_valley.ctrmap")).unwrap();
    assert_eq!(level_of(&hub.name), Some(LevelId::GEM_STONE_VALLEY));
    assert!(hub.relic.is_none());
    for id in tracks {
        let name = name_of(id).expect("a racing track has a name");
        let path = dir.join(format!("{name}.ctrmap"));
        let map = Map::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert_eq!(map.name, name);
        assert_eq!(level_of(&map.name), Some(LevelId(id)));
        assert!(!map.level.quad_blocks.is_empty(), "{name} has no collision");

        // `STATIC_TIME_CRATE_01..03` (`instance::lev_init`): the 1P LEV has none.
        let time_crates = |level: &loader::lev::Level| {
            let crates = level.instances.iter();
            crates.filter(|i| matches!(i.model_id, 0x5c | 0x64 | 0x65)).count()
        };
        let relic = map.relic.as_ref().unwrap_or_else(|| panic!("{name} has no relic LEV"));
        assert_eq!(time_crates(&map.level), 0, "{name}'s 1P LEV");
        assert!(time_crates(&relic.level) > 0, "{name}'s relic LEV has no time crates");
    }

    let _ = std::fs::remove_dir_all(&dir);
}
