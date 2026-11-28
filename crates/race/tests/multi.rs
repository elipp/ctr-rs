//! A race played through rollback, with late pads and a player that leaves, replays from its
//! recording as it was played. Skipped without the disc image.

#[path = "../../loader/tests/common/mod.rs"]
mod common;

use instance::Objects;
use loader::bigindex::{BigIndex, LevFile, Lod};
use loader::dram::DramFile;
use loader::lev::Level;
use loader::level::LevelId;
use race::multi::{Player, Recording, Setup};
use race::rollback::Rollback;
use race::Pad;
use vehicle::buttons;

fn level(id: LevelId) -> Option<(Level, Objects)> {
    let mut b = common::open_bigfile()?;
    let idx = BigIndex::level(id, Lod::OnePlayer, LevFile::Lev);
    let dram = DramFile::parse(&b.big.read(&mut b.disc, idx.get()).unwrap()).unwrap();
    let level = Level::parse(&dram).unwrap();
    let objects = Objects::new(&level, &dram);
    Some((level, objects))
}

fn pad(tick: u32, slot: usize) -> Pad {
    let phase = tick as usize / 20 + slot;
    Pad {
        held: buttons::CROSS
            | if (tick as usize + slot * 7) % 90 < 12 {
                buttons::R1
            } else {
                0
            },
        lx: [0x30, 0x80, 0xd0, 0x80][phase % 4],
        ..Pad::default()
    }
}

const PLAYERS: usize = 3;
/// Slot 1's pads come this many ticks late, so they are predicted and put right.
const LATE: u32 = 5;
const LEAVER: usize = 2;
const LEAVES_AT: u32 = 200;
const TICKS: u32 = 700;

#[test]
fn needs_disc_a_recording_replays_as_the_race_was_played() {
    let Some((level, objects)) = level(LevelId::DINGO_CANYON) else {
        return;
    };
    let setup = Setup {
        track: LevelId::DINGO_CANYON.0,
        map_hash: 0,
        laps: 3,
        masked_weapons: 0,
        even_odds: false,
        seed: 1234,
        kart_collisions: true,
        characters: vec![0, 3, 6],
        brain_bots: 1,
        retail: vec![9, 12],
        difficulty: vehicle::bot::difficulty_level::MEDIUM,
        cup: false,
    };
    let (specs, retail) = setup.specs(|_| [0; 8]);
    let race = race::multi::start(&level, objects.clone(), &setup, &specs, &retail);
    let mut rollback = Rollback::new(race, PLAYERS, Some(0), 120)
        .record(specs.iter().map(|s| s.character))
        .check_every(30);
    for tick in 0..TICKS {
        rollback.confirm(0, tick, pad(tick, 0)).unwrap();
        if let Some(late) = tick.checked_sub(LATE) {
            rollback.confirm(1, late, pad(late, 1)).unwrap();
        }
        if tick < LEAVES_AT {
            rollback.confirm(LEAVER, tick, pad(tick, LEAVER)).unwrap();
        } else if tick == LEAVES_AT {
            rollback.leave(LEAVER);
        }
        rollback.advance(&level);
    }
    let recording = Recording::of(&rollback, setup, vec!["a".into(), "b".into(), "c".into()], Some(0)).unwrap();
    let recording = Recording::from_bytes(&recording.to_bytes()).unwrap();
    assert_eq!(recording.length(), TICKS - LATE);
    assert_eq!(recording.left, [None, None, Some(LEAVES_AT)]);
    assert!(recording.checksums.len() > 10);

    // The same race played straight, every pad known.
    let mut straight = race::multi::start(&level, objects.clone(), &recording.setup, &specs, &retail);
    for tick in 0..recording.length() {
        let pads: Vec<Pad> = (0..PLAYERS)
            .map(|slot| if slot == LEAVER && tick >= LEAVES_AT { Pad::default() } else { pad(tick, slot) })
            .collect();
        straight.tick(&level, &pads);
    }
    let mut player = Player::new(&level, objects, &recording, &specs, &retail);
    while player.race().tick < recording.length() {
        player.tick(&level);
    }
    assert_eq!(player.outcome().mismatch, None);
    assert_eq!(player.race().karts.len(), PLAYERS + retail.len());
    assert_eq!(player.race().checksum(), straight.checksum());
}
