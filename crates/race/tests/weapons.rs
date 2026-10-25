//! The bombs and missiles in a race on a real track: fired by a kart, in flight, set off. Skipped
//! without the disc image.

#[path = "../../loader/tests/common/mod.rs"]
mod common;

use instance::Objects;
use loader::bigindex::{BigIndex, LevFile, Lod};
use loader::dram::DramFile;
use loader::lev::Level;
use loader::level::LevelId;
use race::{KartSpec, Pad, Race};
use vehicle::item::held;
use vehicle::{buttons, kart_state};

fn race(players: usize) -> Option<(Level, Race)> {
    let mut b = common::open_bigfile()?;
    let idx = BigIndex::level(LevelId::DINGO_CANYON, Lod::OnePlayer, LevFile::Lev);
    let dram = DramFile::parse(&b.big.read(&mut b.disc, idx.get()).unwrap()).unwrap();
    let level = Level::parse(&dram).unwrap();
    let objects = Objects::new(&level, &dram);
    let specs: Vec<KartSpec> = (0..players)
        .map(|slot| KartSpec {
            character: slot,
            anim_num_frames: [0; 8],
        })
        .collect();
    Some((level.clone(), Race::new(&level, objects, &specs, false)))
}

/// Everyone drives straight; kart 0 presses the fire button on the ticks in `presses`.
fn pads(tick: u32, players: usize, presses: &[u32]) -> Vec<Pad> {
    (0..players)
        .map(|slot| Pad {
            held: buttons::CROSS
                | if slot == 0 && presses.contains(&tick) {
                    buttons::CIRCLE
                } else {
                    0
                },
            ..Pad::default()
        })
        .collect()
}

#[test]
fn a_bomb_is_fired_and_set_off_by_a_second_press() {
    let Some((level, mut race)) = race(2) else {
        return;
    };
    race.karts[0].driver.held_item_id = held::BOMB_1X;
    let presses = [0, 20];
    let mut exploded = None;
    for tick in 0..60 {
        race.tick(&level, &pads(tick, 2, &presses));
        match tick {
            0 => assert!(
                race.weapons.projectiles().is_empty(),
                "fires on the next tick"
            ),
            1 => {
                assert_eq!(race.weapons.projectiles().len(), 1);
                assert!(race.karts[0].driver.bomb_thrown);
            }
            _ => {}
        }
        if !race.last_weapons.explosions.is_empty() && exploded.is_none() {
            exploded = Some(tick);
        }
    }
    // The bomb flew until the second press, and went off with that tick's flight.
    assert_eq!(exploded, Some(20));
    assert!(race.weapons.projectiles().is_empty());
    assert!(!race.karts[0].driver.bomb_thrown);
    // The cooldown took the used-up weapon away.
    assert_eq!(race.karts[0].driver.held_item_id, held::NONE);
}

#[test]
fn a_missile_flies_on_without_a_target_and_goes_off_at_the_end_of_the_track() {
    let Some((level, mut race)) = race(1) else {
        return;
    };
    race.karts[0].driver.held_item_id = held::MISSILE_1X;
    let mut seen = false;
    let mut gone = None;
    for tick in 0..600 {
        race.tick(&level, &pads(tick, 1, &[0]));
        seen |= !race.weapons.projectiles().is_empty();
        if seen && race.weapons.projectiles().is_empty() {
            gone = Some(tick);
            break;
        }
    }
    assert!(seen && gone.is_some(), "a missile flew and ended");
    assert!(!race.last_weapons.explosions.is_empty());
    // It never hurt the kart that fired it.
    assert!(race.last_weapons.hits.is_empty());
    assert_ne!(race.karts[0].driver.kart_state, kart_state::BLASTED);
}

#[test]
fn a_kart_the_bomb_lands_on_is_blasted_and_tumbles_back_to_driving() {
    let Some((level, mut race)) = race(2) else {
        return;
    };
    race.karts[0].driver.held_item_id = held::BOMB_1X;
    race.tick(&level, &pads(0, 2, &[0]));
    race.tick(&level, &pads(1, 2, &[]));
    // Kart 1 is put under the bomb, which goes off on it in the next tick.
    let [x, y, z] = race.weapons.projectiles()[0].pos;
    race.karts[1].driver.pos_curr = math::Vec3 {
        x: x << 8,
        y: y << 8,
        z: z << 8,
    };
    race.tick(&level, &pads(2, 2, &[]));
    assert_eq!(
        race.last_weapons.hits,
        vec![weapon::Hit {
            victim: 1,
            attacker: 0,
            reason: weapon::Reason::Bomb
        }]
    );
    assert!(race.karts[1].driver.pending_tumble);
    // The tumble starts on the next tick, and ends when its timer has run out.
    race.tick(&level, &pads(3, 2, &[]));
    assert_eq!(race.karts[1].driver.kart_state, kart_state::BLASTED);
    let mut tick = 4;
    while race.karts[1].driver.kart_state == kart_state::BLASTED {
        assert!(tick < 200, "the tumble ends");
        race.tick(&level, &pads(tick, 2, &[]));
        tick += 1;
    }
    assert_eq!(race.karts[1].driver.kart_state, kart_state::NORMAL);
}

#[test]
fn weapons_are_part_of_the_snapshot() {
    let Some((level, mut race)) = race(2) else {
        return;
    };
    race.karts[0].driver.held_item_id = held::MISSILE_1X;
    for tick in 0..3 {
        race.tick(&level, &pads(tick, 2, &[0]));
    }
    assert!(!race.weapons.projectiles().is_empty());
    let snapshot = race.clone();
    let mut a = race;
    let mut b = snapshot;
    for tick in 3..40 {
        a.tick(&level, &pads(tick, 2, &[]));
        b.tick(&level, &pads(tick, 2, &[]));
    }
    assert_eq!(a.checksum(), b.checksum());
}
