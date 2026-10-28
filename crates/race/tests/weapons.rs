//! The bombs and missiles in a race on a real track: fired by a kart, in flight, set off. Skipped
//! without the disc image.

#[path = "../../loader/tests/common/mod.rs"]
mod common;

use instance::Objects;
use loader::bigindex::{BigIndex, LevFile, Lod};
use loader::dram::DramFile;
use loader::lev::{BspKind, Level, SVec3};
use loader::level::LevelId;
use race::{KartSpec, Pad, Race};
use vehicle::item::held;
use vehicle::{buttons, kart_state};

fn race(players: usize) -> Option<(Level, Race)> {
    race_on(LevelId::DINGO_CANYON, players, |_| {})
}

/// A race on `id`, with the level changed by `patch` before the objects are placed.
fn race_on(id: LevelId, players: usize, patch: impl FnOnce(&mut Level)) -> Option<(Level, Race)> {
    let mut b = common::open_bigfile()?;
    let idx = BigIndex::level(id, Lod::OnePlayer, LevFile::Lev);
    let dram = DramFile::parse(&b.big.read(&mut b.disc, idx.get()).unwrap()).unwrap();
    let mut level = Level::parse(&dram).unwrap();
    patch(&mut level);
    let objects = Objects::new(&level, &dram);
    let specs: Vec<KartSpec> = (0..players)
        .map(|slot| KartSpec {
            character: slot,
            anim_num_frames: [0; 8],
        })
        .collect();
    Some((level.clone(), Race::new(&level, objects, &specs, false)))
}

/// Puts a crate of `model_id` (7 is the wumpa crate, 8 the weapon crate) at `pos`, as a copy of
/// one the level has, hitbox and all, and gives the instance it is.
fn add_crate(level: &mut Level, model_id: i32, pos: [i16; 3]) -> usize {
    let model = level
        .instances
        .iter()
        .position(|i| i.model_id == model_id)
        .expect("the level has a crate of the kind");
    let hitbox = level
        .bsp
        .iter()
        .flat_map(|node| match &node.kind {
            BspKind::Leaf { hitboxes, .. } => hitboxes.as_slice(),
            _ => &[],
        })
        .find(|h| h.instance == Some(model))
        .expect("the weapon crate has a hitbox")
        .clone();

    let instance = level.instances.len();
    let mut def = level.instances[model].clone();
    def.pos = SVec3 {
        x: pos[0],
        y: pos[1],
        z: pos[2],
    };
    level.instances.push(def);

    // The same hitbox, moved to the new crate.
    let shift = [
        pos[0] - hitbox.center.x,
        pos[1] - hitbox.center.y,
        pos[2] - hitbox.center.z,
    ];
    let moved = |v: SVec3| SVec3 {
        x: v.x + shift[0],
        y: v.y + shift[1],
        z: v.z + shift[2],
    };
    let mut new = hitbox;
    new.offset = 0xffff_0000;
    new.instance = Some(instance);
    new.center = moved(new.center);
    new.bbox.min = moved(new.bbox.min);
    new.bbox.max = moved(new.bbox.max);
    // In each leaf it reaches.
    for node in &mut level.bsp {
        let (b, h) = (node.bbox, new.bbox);
        let overlaps = b.min.x <= h.max.x
            && h.min.x <= b.max.x
            && b.min.y <= h.max.y
            && h.min.y <= b.max.y
            && b.min.z <= h.max.z
            && h.min.z <= b.max.z;
        if let BspKind::Leaf { hitboxes, .. } = &mut node.kind
            && overlaps
        {
            hitboxes.push(new.clone());
        }
    }
    instance
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
            attacker: Some(0),
            reason: weapon::Reason::Bomb,
            damage: vehicle::pick::damage::BLAST,
            rain: false,
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

#[test]
fn a_potion_thrown_forward_lands_ahead_and_spins_out_the_kart_that_touches_it() {
    use weapon::mine::{MineKind, State};

    let Some((level, mut race)) = race(2) else {
        return;
    };
    race.karts[0].driver.held_item_id = held::POTION;
    // Kart 0 holds Up, which throws a potion forward, and presses the fire button on tick 0.
    let pads = |tick: u32| -> Vec<Pad> {
        (0..2)
            .map(|slot| Pad {
                held: buttons::CROSS
                    | if slot == 0 {
                        buttons::UP | if tick == 0 { buttons::CIRCLE } else { 0 }
                    } else {
                        0
                    },
                ..Pad::default()
            })
            .collect()
    };
    race.tick(&level, &pads(0));
    race.tick(&level, &pads(1));
    let beaker = &race.weapons.mines()[0];
    assert_eq!(beaker.kind, MineKind::GreenBeaker);
    assert_eq!(beaker.state, State::InAir);
    let thrown_from = beaker.pos;

    let mut tick = 2;
    while race
        .weapons
        .mines()
        .first()
        .is_some_and(|m| m.state == State::InAir)
    {
        assert!(tick < 120, "the potion lands");
        race.tick(&level, &pads(tick));
        tick += 1;
    }
    let landed = race.weapons.mines()[0].clone();
    assert_eq!(landed.state, State::Ground);
    assert_ne!(landed.pos, thrown_from);

    // Kart 1 is put on it: it spins out, its screen flashes, and the beaker shatters.
    let [x, y, z] = landed.pos;
    race.karts[1].driver.pos_curr = math::Vec3 {
        x: x << 8,
        y: y << 8,
        z: z << 8,
    };
    race.tick(&level, &pads(tick));
    assert_eq!(
        race.last_weapons.hits,
        vec![weapon::Hit {
            victim: 1,
            attacker: Some(0),
            // It landed a moment ago.
            reason: weapon::Reason::Potion,
            damage: vehicle::pick::damage::SPIN,
            rain: false,
        }]
    );
    assert_eq!(race.last_weapons.flashes, [1]);
    assert_eq!(race.last_weapons.detonations.len(), 1);
    assert!(race.weapons.mines().is_empty());
}

#[test]
fn a_red_beaker_that_spins_out_a_kart_makes_it_rain_on_it() {
    use weapon::mine::{MineKind, State};

    let Some((level, mut race)) = race(2) else {
        return;
    };
    vehicle::item::debug_roll(&mut race.karts[0].driver, held::POTION);
    race.karts[0].driver.num_wumpas = 10;
    let pads = |tick: u32| -> Vec<Pad> {
        (0..2)
            .map(|slot| Pad {
                held: buttons::CROSS
                    | if slot == 0 && tick == 0 {
                        buttons::CIRCLE
                    } else {
                        0
                    },
                ..Pad::default()
            })
            .collect()
    };
    // Dropped (no direction held), it lies where the kart was.
    race.tick(&level, &pads(0));
    let mut tick = 1;
    while race
        .weapons
        .mines()
        .first()
        .is_none_or(|m| m.state != State::Ground)
    {
        assert!(tick < 120, "the potion lands");
        race.tick(&level, &pads(tick));
        tick += 1;
    }
    let landed = race.weapons.mines()[0].clone();
    assert_eq!(landed.kind, MineKind::RedBeaker);

    let [x, y, z] = landed.pos;
    race.karts[1].driver.pos_curr = math::Vec3 {
        x: x << 8,
        y: y << 8,
        z: z << 8,
    };
    race.tick(&level, &pads(tick));
    assert!(race.last_weapons.hits.iter().all(|h| h.rain));
    let cloud = race.karts[1].driver.cloud.expect("the cloud forms");
    // Nothing in hand: the cloud slows the kart.
    assert_eq!(cloud.effect, vehicle::rain_cloud::SLOW);
    assert!(race.karts[0].driver.cloud.is_none());

    // It lasts 240 ticks and then fades away.
    for t in 0..260 {
        race.tick(&level, &pads(tick + 1 + t));
    }
    assert!(race.karts[1].driver.cloud.is_none());
    assert!(race.karts[1].driver.fading_clouds.is_empty());
}

#[test]
fn a_bomb_breaks_the_weapon_crate_it_flies_through_and_the_kart_is_paid() {
    let mut crate_at = 0;
    let Some((level, mut race)) = race_on(LevelId::SLIDE_COLISEUM, 1, |level| {
        crate_at = add_crate(level, 8, [957, 1, -10664]);
    }) else {
        return;
    };
    let index = race
        .objects
        .crates
        .iter()
        .position(|c| c.instance == crate_at)
        .expect("the crate is placed");
    // `/debug set weapon 1`: a bomb, fired on tick 0, and left to fly.
    race.karts[0].driver.held_item_id = held::BOMB_1X;
    let start = race.karts[0].driver.pos_curr;
    let mut flew_to = None;
    for tick in 0..300 {
        race.tick(&level, &pads(tick, 1, &[0]));
        if let Some(p) = race.weapons.projectiles().first() {
            flew_to = Some(p.pos);
        }
        if race.objects.crates[index].sim.breaks != 0 {
            break;
        }
    }
    assert_eq!(
        race.objects.crates[index].sim.breaks, 1,
        "the bomb broke the crate (the kart started at {start:?}, the bomb was last at {flew_to:?})"
    );
    // It was the flight that broke it, in the tick the bomb passed it.
    assert!(
        race.last_weapons
            .crate_hits
            .iter()
            .any(|h| h.instance == crate_at)
    );
}

/// `/debug set weapon 4` and a throw forward, then `/debug set weapon 1` and a shot: the bomb
/// flies through a wumpa crate on its way to the green bottle that lies beyond it, and goes off
/// on the bottle.
#[test]
fn a_bomb_breaks_a_wumpa_crate_on_its_way_and_sets_off_the_bottle_it_reaches() {
    use weapon::mine::{MineKind, State};

    let mut crate_at = 0;
    let Some((level, mut race)) = race_on(LevelId::SLIDE_COLISEUM, 1, |level| {
        crate_at = add_crate(level, 7, [957, 1, -10664]);
    }) else {
        return;
    };
    let index = race
        .objects
        .crates
        .iter()
        .position(|c| c.instance == crate_at)
        .expect("the crate is placed");
    let wumpas_before = race.karts[0].driver.num_wumpas;

    // The kart stands still. `up` throws a potion forward; `fire` presses the fire button.
    let pad = |up: bool, fire: bool| -> Vec<Pad> {
        vec![Pad {
            held: if up { buttons::UP } else { 0 } | if fire { buttons::CIRCLE } else { 0 },
            ..Pad::default()
        }]
    };

    // The bottle: thrown on tick 0, and left to land.
    vehicle::item::debug_roll(&mut race.karts[0].driver, held::POTION);
    let mut tick = 0;
    race.tick(&level, &pad(true, true));
    while race.weapons.mines().is_empty() || race.weapons.mines()[0].state != State::Ground {
        tick += 1;
        assert!(tick < 200, "the bottle lands");
        race.tick(&level, &pad(true, false));
    }
    let bottle = race.weapons.mines()[0].clone();
    assert_eq!(bottle.kind, MineKind::GreenBeaker);
    assert!(
        bottle.pos[0] < 957 - 400,
        "the bottle lies beyond the crate, at {:?}",
        bottle.pos
    );
    assert_eq!(race.objects.crates[index].sim.breaks, 0);

    // The bomb: rolled, and shot straight ahead.
    vehicle::item::debug_roll(&mut race.karts[0].driver, held::BOMB_1X);
    let mut broke = None;
    let mut went_off = None;
    for step in 0..600 {
        tick += 1;
        race.tick(&level, &pad(false, step == 0));
        if broke.is_none() && race.objects.crates[index].sim.breaks != 0 {
            broke = Some(tick);
        }
        if !race.last_weapons.explosions.is_empty() {
            went_off = Some(tick);
            break;
        }
    }
    let (broke, went_off) = (
        broke.expect("the bomb broke the crate"),
        went_off.expect("the bomb went off"),
    );
    // It broke the crate on the way (not with its blast), and went off on the bottle.
    assert!(broke < went_off, "broke on {broke}, went off on {went_off}");
    assert_eq!(race.objects.crates[index].sim.breaks, 1);
    // The bomb itself blew up, at the bottle, and does not fly on.
    let [blast] = race.last_weapons.explosions.as_slice() else {
        panic!("one blast: {:?}", race.last_weapons.explosions);
    };
    assert_eq!(blast.kind, weapon::Kind::Bomb);
    let (dx, dz) = (blast.pos[0] - bottle.pos[0], blast.pos[2] - bottle.pos[2]);
    assert!(
        dx * dx + dz * dz < 96 * 96,
        "blast at {:?}, bottle at {:?}",
        blast.pos,
        bottle.pos
    );
    assert!(race.weapons.projectiles().is_empty());
    let detonations = &race.last_weapons.detonations;
    assert!(
        detonations
            .iter()
            .any(|d| d.kind == MineKind::GreenBeaker && !d.blowup),
        "the bottle went off with the bomb: {detonations:?}"
    );

    // The wumpa fly to the counter, 5 to 8 of them.
    for _ in 0..80 {
        race.tick(&level, &pad(false, false));
    }
    // (The bottle was swept away by the next tick.)
    assert!(race.weapons.mines().is_empty());
    let gained = race.karts[0].driver.num_wumpas - wumpas_before;
    assert!((5..=8).contains(&gained), "{gained} wumpa fruit");
}

/// The first grid spot of Slide Coliseum, which faces -x, with the ground height there.
const START: [i32; 3] = [1679, 1, -10665];

/// Slide Coliseum with `players` karts and a row of five Nitro crates on the first kart's line, 600
/// units ahead and 6 apart (the missile's burst reaches 128, and it goes off up to 96 before the
/// first), and a sixth 1000 ahead. The first kart holds a missile (`/debug set weapon 2`). Returns
/// the x of each of the five, and of the sixth.
fn nitro_row(players: usize) -> Option<(Level, Race, Vec<i32>, i32)> {
    use weapon::mine::MineKind;

    let (level, mut race) = race_on(LevelId::SLIDE_COLISEUM, players, |_| {})?;
    let [x, y, z] = START;
    let row: Vec<i32> = (0..5).map(|i| x - 600 - 6 * i).collect();
    let far = x - 1000;
    for &x in row.iter().chain([&far]) {
        race.weapons.place_mine(MineKind::Nitro, [x, y, z], 0);
    }
    vehicle::item::debug_roll(&mut race.karts[0].driver, held::MISSILE_1X);
    Some((level, race, row, far))
}

/// Nobody drives; kart 0 presses the fire button when `fire`.
fn fire_pads(players: usize, fire: bool) -> Vec<Pad> {
    (0..players)
        .map(|slot| Pad {
            held: if slot == 0 && fire {
                buttons::CIRCLE
            } else {
                0
            },
            ..Pad::default()
        })
        .collect()
}

/// Five Nitro crates side by side on the missile's way, and a sixth far past them: the missile
/// goes off at the first, and its burst sets off the ones inside it. Nitro crates do not chain
/// (a crate that is hit gets a plain explosion with no blast), so the far one lies on.
#[test]
fn a_missile_sets_off_the_nitro_crates_within_its_burst_and_no_others() {
    use weapon::mine::MineKind;

    let Some((level, mut race, row, far)) = nitro_row(1) else {
        return;
    };
    let mut went_off = Vec::new();
    let mut burst = None;
    for tick in 0..600 {
        race.tick(&level, &fire_pads(1, tick == 0));
        went_off.extend(
            race.last_weapons
                .detonations
                .iter()
                .map(|d| (d.kind, d.pos[0], d.blowup)),
        );
        if let [blast] = race.last_weapons.explosions.as_slice() {
            burst = Some(blast.pos);
            break;
        }
    }
    let burst = burst.expect("the missile went off");
    assert!(
        burst[0] > row[0] - 10 && burst[0] <= row[0] + 96,
        "the missile went off at the first crate: {burst:?}, crates at {row:?}"
    );

    // All five went off, each with a plain explosion (no blast of its own), and the far one did
    // not.
    let mut gone: Vec<i32> = went_off.iter().map(|&(_, x, _)| x).collect();
    gone.sort_unstable();
    let mut expected = row.clone();
    expected.sort_unstable();
    assert_eq!(gone, expected, "went off: {went_off:?}");
    assert!(
        went_off
            .iter()
            .all(|&(kind, _, blowup)| kind == MineKind::Nitro && !blowup)
    );

    // (The next tick sweeps the dead away.)
    race.tick(&level, &fire_pads(1, false));
    let left: Vec<i32> = race.weapons.mines().iter().map(|m| m.pos[0]).collect();
    assert_eq!(left, vec![far]);
    assert!(race.weapons.projectiles().is_empty());
    assert!(race.last_weapons.hits.is_empty());
}

/// A second kart stands beside the row, inside the missile's burst but out of reach of every
/// crate itself (120 units), so it is the missile's burst that blasts it and not a crate it touched.
#[test]
fn a_kart_beside_the_nitro_crates_is_blasted_by_the_missile_that_sets_them_off() {
    use weapon::{Hit, Reason};

    // Once to see when and where the missile goes off.
    let Some((level, mut race, ..)) = nitro_row(2) else {
        return;
    };
    let (burst_tick, burst) = (0..600)
        .find_map(|tick| {
            race.tick(&level, &fire_pads(2, tick == 0));
            race.last_weapons
                .explosions
                .first()
                .map(|blast| (tick, blast.pos))
        })
        .expect("the missile went off");

    // Again, with kart 1 put beside the burst on the tick before: 120 units to the side, which is
    // inside the burst's reach (128) and just out of every crate's (120, `mine::HIT_RADIUS`).
    let (level, mut race, ..) = nitro_row(2).unwrap();
    for tick in 0..burst_tick {
        race.tick(&level, &fire_pads(2, tick == 0));
    }
    assert_eq!(race.karts[1].driver.kart_state, kart_state::NORMAL);
    race.karts[1].driver.pos_curr = math::Vec3 {
        x: burst[0] << 8,
        y: burst[1] << 8,
        z: (burst[2] + 120) << 8,
    };
    race.tick(&level, &fire_pads(2, burst_tick == 0));

    // Only the missile hurt it: the kart that fired was out of the burst's reach, and the crates
    // did not touch the kart.
    assert_eq!(
        race.last_weapons.hits,
        vec![Hit {
            victim: 1,
            attacker: Some(0),
            reason: Reason::Missile,
            damage: vehicle::pick::damage::BLAST,
            rain: false,
        }],
        "the burst was at {burst:?}"
    );
    assert!(race.karts[1].driver.pending_tumble);
    race.tick(&level, &fire_pads(2, false));
    assert_eq!(race.karts[1].driver.kart_state, kart_state::BLASTED);
}

/// A missile homes in on a kart that is not on its line of fire. Kart 1 stands 800 units ahead and
/// 150 to the side (inside the aiming cone, and past the missile's 96-unit reach if it flew
/// straight), and the missile turns to it and hits it directly.
#[test]
fn a_missile_homes_in_on_a_kart_off_its_line_and_hits_it() {
    use weapon::{Body, Hit, Reason};

    let Some((level, mut race)) = race_on(LevelId::SLIDE_COLISEUM, 2, |_| {}) else {
        return;
    };
    let [x, _, z] = START;
    // (The grid spreads towards -z, so there is road there.)
    let (target_x, target_z) = (x - 800, z - 150);
    let d = &mut race.karts[1].driver;
    d.pos_curr.x = target_x << 8;
    d.pos_curr.z = target_z << 8;
    vehicle::item::debug_roll(&mut race.karts[0].driver, held::MISSILE_1X);

    let mut flew = Vec::new();
    let mut burst = None;
    for tick in 0..600 {
        race.tick(&level, &fire_pads(2, tick == 0));
        if let Some(p) = race.weapons.projectiles().first() {
            flew.push(p.pos);
        }
        if let Some(blast) = race.last_weapons.explosions.first() {
            burst = Some((tick, blast.pos));
            break;
        }
    }
    let (_, burst) = burst.expect("the missile went off");

    // It left its line to get there: it did not fly straight and blast the kart from the side.
    let strayed = flew.iter().map(|p| (p[2] - z).abs()).max().unwrap_or(0);
    assert!(strayed > 100, "the missile flew {flew:?}");

    // It went off on the kart (within a direct hit's reach on the ground plane), and the kart is
    // the only one hurt.
    let at = Body::of(&race.karts[1].driver).at;
    let (dx, dz) = (burst[0] - at[0], burst[2] - at[2]);
    assert!(
        dx * dx + dz * dz < 96 * 96,
        "went off at {burst:?}, the kart is at {at:?}"
    );
    assert_eq!(
        race.last_weapons.hits,
        vec![Hit {
            victim: 1,
            attacker: Some(0),
            reason: Reason::Missile,
            damage: vehicle::pick::damage::BLAST,
            rain: false,
        }]
    );
    assert!(race.karts[1].driver.pending_tumble);
    assert!(race.weapons.projectiles().is_empty());
}

/// A kart that holds a shield takes the missile that hits it: the shield absorbs the hit and pops,
/// the kart is not blasted, and it is invincible for a moment after.
#[test]
fn a_shield_absorbs_the_missile_that_hits_its_kart_and_pops() {
    use weapon::{Hit, Reason};

    let Some((level, mut race)) = race_on(LevelId::SLIDE_COLISEUM, 2, |_| {}) else {
        return;
    };
    let [x, _, z] = START;
    let d = &mut race.karts[1].driver;
    d.pos_curr.x = (x - 800) << 8;
    d.pos_curr.z = z << 8;
    vehicle::shield::shoot_shield(d, &mut Vec::new());
    vehicle::item::debug_roll(&mut race.karts[0].driver, held::MISSILE_1X);

    let mut went_off = false;
    for tick in 0..600 {
        race.tick(&level, &fire_pads(2, tick == 0));
        if !race.last_weapons.explosions.is_empty() {
            went_off = true;
            break;
        }
    }
    assert!(went_off, "the missile went off");

    // The hit was the missile's, and the shield took it.
    assert_eq!(
        race.last_weapons.hits,
        vec![Hit {
            victim: 1,
            attacker: Some(0),
            reason: Reason::Missile,
            damage: vehicle::pick::damage::BLAST,
            rain: false,
        }]
    );
    let d = &race.karts[1].driver;
    assert!(!d.pending_tumble && d.kart_state != kart_state::BLASTED);
    assert!(d.invincible_timer > 0);
    assert!(d.shield.is_some_and(|s| !s.holding));

    // The shield pops away, and the kart never tumbles.
    for _ in 0..30 {
        race.tick(&level, &fire_pads(2, false));
    }
    let d = &race.karts[1].driver;
    assert!(d.shield.is_none());
    assert_ne!(d.kart_state, kart_state::BLASTED);
}

/// A shield is shot with the fire button: it flies straight ahead (it chases no one, and does not
/// roll), goes off on the kart in its way as an ordinary bomb, and that kart is blasted.
#[test]
fn a_shot_shield_flies_straight_and_blasts_the_kart_in_its_way() {
    use weapon::{Hit, Kind, Reason};

    let Some((level, mut race)) = race_on(LevelId::SLIDE_COLISEUM, 2, |_| {}) else {
        return;
    };
    let [x, _, z] = START;
    let d = &mut race.karts[1].driver;
    d.pos_curr.x = (x - 800) << 8;
    d.pos_curr.z = z << 8;
    vehicle::shield::shoot_shield(&mut race.karts[0].driver, &mut Vec::new());

    let mut flew = Vec::new();
    let mut went_off = false;
    for tick in 0..600 {
        // The first press shoots the bubble.
        race.tick(&level, &fire_pads(2, tick == 0));
        if let Some(p) = race.weapons.projectiles().first() {
            assert_eq!(p.kind, Kind::Shield);
            assert!(race.karts[0].driver.shield.is_none());
            flew.push(p.pos);
        }
        if !race.last_weapons.explosions.is_empty() {
            went_off = true;
            break;
        }
    }
    assert!(went_off, "the shield bomb went off, it flew {flew:?}");
    assert!(
        flew.len() > 3 && flew.iter().all(|p| (p[2] - z).abs() < 60),
        "{flew:?}"
    );
    assert_eq!(
        race.last_weapons.hits,
        vec![Hit {
            victim: 1,
            attacker: Some(0),
            reason: Reason::Bomb,
            damage: vehicle::pick::damage::BLAST,
            rain: false,
        }]
    );
    assert_eq!(race.last_weapons.explosions[0].kind, Kind::Shield);
    assert!(race.karts[1].driver.pending_tumble);
}

/// Polar Pass with one kart, a tick on, and the contact of its first seal.
fn seal_race() -> Option<(Level, Race, instance::Contact)> {
    let (level, mut race) = race_on(LevelId::POLAR_PASS, 1, |_| {})?;
    race.tick(&level, &fire_pads(1, false));
    let seal = *race
        .last
        .contacts
        .iter()
        .find(|c| level.instances[c.instance].model_name == "seal")
        .expect("the level has a seal");
    Some((level, race, seal))
}

/// A Nitro crate in a seal's way goes off when the seal reaches it, with a plain explosion and
/// nobody hurt (`RB_CheckHazardCollisions`' mine bucket).
#[test]
fn a_seal_sets_off_the_mine_in_its_way() {
    use weapon::mine::MineKind;

    let Some((level, mut race, seal)) = seal_race() else {
        return;
    };
    race.weapons.place_mine(MineKind::Nitro, seal.pos, 0);
    race.tick(&level, &fire_pads(1, false));
    let went_off: Vec<_> = race
        .last_weapons
        .detonations
        .iter()
        .map(|d| (d.kind, d.pos, d.blowup))
        .collect();
    assert_eq!(went_off, [(MineKind::Nitro, seal.pos, false)]);
    assert!(race.last_weapons.hits.is_empty());
}

/// A kart in the seal's reach is what it hits: the mine there lies on.
#[test]
fn a_seal_with_a_kart_in_reach_leaves_the_mine_be() {
    use weapon::mine::MineKind;

    let Some((level, mut race, seal)) = seal_race() else {
        return;
    };
    race.weapons.place_mine(MineKind::GreenBeaker, seal.pos, 0);
    // Above the seal, in its reach (128) and out of the beaker's own (80).
    let [x, y, z] = seal.pos;
    race.karts[0].driver.pos_curr = math::Vec3 {
        x: x << 8,
        y: (y + 90) << 8,
        z: z << 8,
    };
    // (Where the hazards find it.)
    race.karts[0].driver.inst_pos = [x, y + 90, z];
    race.tick(&level, &fire_pads(1, false));
    assert_eq!(race.karts[0].driver.kart_state, kart_state::SPINNING);
    assert!(race.last_weapons.detonations.is_empty());
    assert!(race.last_weapons.hits.is_empty());
    assert!(race.weapons.mines().iter().all(|m| !m.is_dead()));
}
