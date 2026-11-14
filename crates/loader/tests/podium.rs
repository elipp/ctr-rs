mod common;

use loader::bigindex::{BigIndex, LevFile, Lod};
use loader::dram::DramFile;
use loader::lev::{CameraPath, Level};
use loader::level::LevelId;
use loader::model::Model;
use loader::vram::parse_vram_file;

/// Gem Stone Valley, where the podium stands: its camera path, and the recorded path whose first
/// point the podium is put on.
#[test]
fn needs_disc_gem_stone_valley_has_the_podium_camera_path() {
    let Some(mut b) = common::open_bigfile() else {
        return;
    };
    let index = BigIndex::level(LevelId::GEM_STONE_VALLEY, Lod::OnePlayer, LevFile::Lev);
    let file = DramFile::parse(&b.big.read(&mut b.disc, index.get()).unwrap()).unwrap();
    let path = CameraPath::parse(&file).unwrap().expect("a camera path");
    let frames = path.num_points();
    assert_eq!(frames, 451);
    assert!(path.at(0).is_some() && path.at(frames as i32 - 1).is_some());
    assert!(path.at(-1).is_none() && path.at(frames as i32).is_none());
    // Angles come out as a push buffer takes them: a twelfth bit at most.
    for frame in 0..frames as i32 {
        let (point, _) = path.at(frame).unwrap();
        assert!((0..0x1000).contains(&point.rot.x) && (0..0x1000).contains(&point.rot.y));
    }
    let level = Level::parse(&file).unwrap();
    assert!(level.paths.get(1).is_some_and(|p| !p.is_empty()));
}

/// The podium's model and its texture pages, of the pack Gem Stone Valley loads them into.
#[test]
fn needs_disc_the_podium_stand_parses() {
    let Some(mut b) = common::open_bigfile() else {
        return;
    };
    let pages = b
        .big
        .read(&mut b.disc, BigIndex::PODIUM_VRMS.nth(1).get())
        .unwrap();
    assert!(!parse_vram_file(&pages).unwrap().is_empty());
    let raw = b
        .big
        .read(&mut b.disc, BigIndex::PODIUM.nth(1).get())
        .unwrap();
    let model = Model::parse(&DramFile::parse(&raw).unwrap()).unwrap();
    assert_eq!(model.name, "podium");
    assert!(!model.headers.is_empty());
}
