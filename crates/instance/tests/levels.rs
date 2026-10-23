//! The objects of the real tracks: what each level places, and that they run.
//! Skipped without the disc image.

#[path = "../../loader/tests/common/mod.rs"]
mod common;

use instance::Objects;
use loader::bigindex::{BigIndex, LevFile, Lod};
use loader::dram::DramFile;
use loader::lev::Level;
use loader::level::LevelId;

fn objects_of(id: u32) -> Option<(Objects, Level)> {
    let mut b = common::open_bigfile()?;
    let idx = BigIndex::level(LevelId(id), Lod::OnePlayer, LevFile::Lev);
    let dram = DramFile::parse(&b.big.read(&mut b.disc, idx.get()).unwrap()).unwrap();
    let level = Level::parse(&dram).unwrap();
    Some((Objects::new(&level, &dram), level))
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
        let Some((h, level)) = objects_of(id) else {
            return;
        };
        let found = [
            h.armadillos.len(),
            h.fireballs.len(),
            h.minecarts.len(),
            h.rollers.len(),
        ];
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
    let Some((mut h, _)) = objects_of(LevelId::MYSTERY_CAVES.0) else {
        return;
    };
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
    let Some((mut a, _)) = objects_of(LevelId::DINGO_CANYON.0) else {
        return;
    };
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
    let Some((mut h, level)) = objects_of(LevelId::BLIZZARD_BLUFF.0) else {
        return;
    };
    let path = &level.paths[h.rollers[0].sim.path];
    assert_eq!(h.roller_frames(0), (path[0], path[1]));
    h.tick();
    assert_eq!(h.roller_frames(0), (path[1], path[2]));
}

#[test]
fn a_kart_that_drives_into_a_turtle_is_thrown() {
    use coll::Collision;
    use loader::lev::BspKind;
    use vehicle::scrub::IDENTITY_LIGHT_MATRIX;
    use vehicle::search::moved_player_search;
    use vehicle::{Driver, forced_jump};

    let Some((mut h, level)) = objects_of(LevelId::MYSTERY_CAVES.0) else {
        return;
    };
    assert_eq!(h.turtles.len(), 10);
    assert!(h.turtles.iter().all(|t| t.frames[0] == 11));
    let hitbox_of = |instance: usize| {
        level
            .bsp
            .iter()
            .filter_map(|n| match &n.kind {
                BspKind::Leaf { hitboxes, .. } => Some(hitboxes),
                _ => None,
            })
            .flatten()
            .find(|b| b.instance == Some(instance))
            .map(|b| b.center)
    };
    assert!(
        h.turtles.iter().all(|t| hitbox_of(t.instance).is_some()),
        "every turtle has a hitbox"
    );

    // A kart driving through the first turtle's hitbox, from 200 units before its centre to 200
    // after, at the given `speed_approx`: the jump it is given, if any.
    let center = hitbox_of(h.turtles[0].instance).unwrap();
    let drive_through = |h: &Objects, speed_approx: i16| {
        let mut driver = Driver::new(0, 0);
        driver.pos_curr = math::Vec3 {
            x: (center.x as i32 - 200 - driver.origin_to_center.x) << 8,
            // Above the floor, within the hitbox's reach.
            y: (center.y as i32 + 100 - driver.origin_to_center.y) << 8,
            z: (center.z as i32 - driver.origin_to_center.z) << 8,
        };
        driver.velocity.x = 400 << 8;
        driver.speed_approx = speed_approx;
        moved_player_search(
            &mut driver,
            &mut Collision::default(),
            &level,
            &IDENTITY_LIGHT_MATRIX,
            32,
            &mut h.clone(),
            &mut Vec::new(),
        );
        driver.forced_jump_type
    };

    // Over the whole cycle it is thrown both low (fully down) and high, and never when slow.
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..200 {
        let jump = drive_through(&h, 0x2000);
        assert_eq!(
            jump == forced_jump::LOW,
            h.turtles[0].sim.state == instance::hazard::turtle::State::FullyDown
        );
        seen.insert(jump);
        assert_eq!(drive_through(&h, 0x1400), forced_jump::NONE);
        h.tick();
    }
    assert_eq!(seen, [forced_jump::LOW, forced_jump::HIGH].into());
}

#[test]
fn a_kart_that_drives_through_a_fruit_picks_it_up_once() {
    use coll::Collision;
    use loader::lev::BspKind;
    use vehicle::Driver;
    use vehicle::scrub::IDENTITY_LIGHT_MATRIX;
    use vehicle::search::moved_player_search;

    let Some((mut h, level)) = objects_of(LevelId::DINGO_CANYON.0) else {
        return;
    };
    assert_eq!(h.fruit.len(), 16);
    let center = level
        .bsp
        .iter()
        .filter_map(|n| match &n.kind {
            BspKind::Leaf { hitboxes, .. } => Some(hitboxes),
            _ => None,
        })
        .flatten()
        .find(|b| b.instance == Some(h.fruit[0].instance))
        .expect("a fruit has a hitbox")
        .center;

    let drive_through = |h: &mut Objects, driver: &mut Driver| {
        driver.pos_curr = math::Vec3 {
            x: (center.x as i32 - 150 - driver.origin_to_center.x) << 8,
            y: (center.y as i32 + 40 - driver.origin_to_center.y) << 8,
            z: (center.z as i32 - driver.origin_to_center.z) << 8,
        };
        driver.velocity.x = 300 << 8;
        moved_player_search(
            driver,
            &mut Collision::default(),
            &level,
            &IDENTITY_LIGHT_MATRIX,
            32,
            h,
            &mut Vec::new(),
        );
    };

    let mut driver = Driver::new(0, 0);
    drive_through(&mut h, &mut driver);
    assert!(h.fruit[0].sim.collected);
    assert_eq!(driver.pickup_wumpa_collected, 1);
    assert_eq!(h.fruit.iter().filter(|f| f.sim.collected).count(), 1);

    // A second pass finds nothing there.
    drive_through(&mut h, &mut driver);
    assert_eq!(driver.pickup_wumpa_collected, 1);
}

#[test]
fn a_kart_that_drives_through_a_crate_breaks_it() {
    use coll::Collision;
    use loader::lev::BspKind;
    use vehicle::Driver;
    use vehicle::scrub::IDENTITY_LIGHT_MATRIX;
    use vehicle::search::moved_player_search;

    let Some((mut h, level)) = objects_of(LevelId::DINGO_CANYON.0) else {
        return;
    };
    // 16 weapon crates and 2 fruit crates.
    assert_eq!(h.crates.len(), 18);
    let center = level
        .bsp
        .iter()
        .filter_map(|n| match &n.kind {
            BspKind::Leaf { hitboxes, .. } => Some(hitboxes),
            _ => None,
        })
        .flatten()
        .find(|b| b.instance == Some(h.crates[0].instance))
        .expect("a crate has a hitbox")
        .center;

    let drive_through = |h: &mut Objects, driver: &mut Driver| {
        driver.pos_curr = math::Vec3 {
            x: (center.x as i32 - 150 - driver.origin_to_center.x) << 8,
            y: (center.y as i32 + 20 - driver.origin_to_center.y) << 8,
            z: (center.z as i32 - driver.origin_to_center.z) << 8,
        };
        driver.velocity.x = 300 << 8;
        moved_player_search(
            driver,
            &mut Collision::default(),
            &level,
            &IDENTITY_LIGHT_MATRIX,
            32,
            h,
            &mut Vec::new(),
        );
    };

    let mut driver = Driver::new(0, 0);
    drive_through(&mut h, &mut driver);
    assert_eq!((h.crates[0].sim.scale, h.crates[0].sim.breaks), (0, 1));
    // Hitting it again while it is gone does nothing, and it comes back by itself.
    drive_through(&mut h, &mut driver);
    assert_eq!(h.crates[0].sim.breaks, 1);
    for _ in 0..100 {
        h.tick();
    }
    assert_eq!(h.crates[0].sim.scale, instance::pickup::crates::FULL_SCALE);
}
