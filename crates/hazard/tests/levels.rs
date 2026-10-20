//! The hazards of the real tracks: what each level places, and that they run.
//! Skipped without the disc image.

#[path = "../../loader/tests/common/mod.rs"]
mod common;

use hazard::Hazards;
use loader::bigindex::{BigIndex, LevFile, Lod};
use loader::dram::DramFile;
use loader::lev::Level;
use loader::level::LevelId;

fn hazards_of(id: u32) -> Option<(Hazards, Level)> {
    let mut b = common::open_bigfile()?;
    let idx = BigIndex::level(LevelId(id), Lod::OnePlayer, LevFile::Lev);
    let dram = DramFile::parse(&b.big.read(&mut b.disc, idx.get()).unwrap()).unwrap();
    let level = Level::parse(&dram).unwrap();
    Some((Hazards::new(&level, &dram), level))
}

#[test]
fn each_level_places_its_hazards() {
    // (level, armadillos, fireballs, minecarts, rollers)
    for (id, counts) in [
        (LevelId::DINGO_CANYON.0, [4, 0, 0, 0]),
        (LevelId::DRAGON_MINES.0, [0, 0, 3, 0]),
        (LevelId::BLIZZARD_BLUFF.0, [0, 0, 0, 1]),
        (LevelId::SEWER_SPEEDWAY.0, [0, 0, 0, 2]),
        (LevelId::MYSTERY_CAVES.0, [0, 6, 0, 0]),
        (LevelId::ROO_TUBES.0, [0, 0, 0, 0]),
    ] {
        let Some((h, level)) = hazards_of(id) else { return };
        let found = [h.armadillos.len(), h.fireballs.len(), h.minecarts.len(), h.rollers.len()];
        assert_eq!(found, counts, "level {id}");
        for a in &h.armadillos {
            assert_eq!(level.instances[a.instance].model_name, "armadillo");
            // A turn and a roll animation.
            assert_eq!(a.frames, [32, 10]);
        }
    }
}

#[test]
fn the_hazards_run_and_reach_for_drivers() {
    let Some((mut h, _)) = hazards_of(LevelId::MYSTERY_CAVES.0) else { return };
    let (mut contacts, mut sparks) = (0, 0);
    for _ in 0..300 {
        let tick = h.tick();
        contacts += tick.contacts.len();
        sparks += tick.sparks.len();
    }
    assert_eq!(h.ticks(), 300);
    // Every fireball flies for most of its 90 ticks: a spark and a contact each tick.
    assert!(sparks > 6 * 100, "{sparks}");
    assert_eq!(contacts, sparks);
}

#[test]
fn the_state_can_be_copied_and_run_on() {
    // What the rollback needs: a copy ticks exactly like the original.
    let Some((mut a, _)) = hazards_of(LevelId::DINGO_CANYON.0) else { return };
    for _ in 0..100 {
        a.tick();
    }
    let mut b = a.clone();
    for _ in 0..200 {
        assert_eq!(a.tick(), b.tick());
    }
    assert_eq!(a, b);
}

#[test]
fn a_roller_is_drawn_between_its_frame_and_the_next() {
    let Some((mut h, level)) = hazards_of(LevelId::BLIZZARD_BLUFF.0) else { return };
    let path = &level.paths[h.rollers[0].sim.path];
    assert_eq!(h.roller_frames(0), (path[0], path[1]));
    h.tick();
    assert_eq!(h.roller_frames(0), (path[1], path[2]));
}
