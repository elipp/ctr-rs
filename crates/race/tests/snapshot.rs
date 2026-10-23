//! A clone of a race is a snapshot: whatever it is put back to plays on exactly as the original
//! did, which is what rolling back needs. Skipped without the disc image.

#[path = "../../loader/tests/common/mod.rs"]
mod common;

use instance::Objects;
use loader::bigindex::{BigIndex, LevFile, Lod};
use loader::dram::DramFile;
use loader::lev::Level;
use loader::level::LevelId;
use race::{KartSpec, Pad, Race};
use vehicle::buttons;

fn level(id: LevelId) -> Option<(Level, Objects)> {
    let mut b = common::open_bigfile()?;
    let idx = BigIndex::level(id, Lod::OnePlayer, LevFile::Lev);
    let dram = DramFile::parse(&b.big.read(&mut b.disc, idx.get()).unwrap()).unwrap();
    let level = Level::parse(&dram).unwrap();
    let objects = Objects::new(&level, &dram);
    Some((level, objects))
}

fn specs(players: usize) -> Vec<KartSpec> {
    (0..players)
        .map(|slot| KartSpec {
            character: slot * 3 % 16,
            anim_num_frames: [0; 8],
        })
        .collect()
}

/// Everyone's pads for a tick: gas, and a steer that changes with the tick and the slot, with
/// a hop now and then.
fn pads(tick: u32, players: usize) -> Vec<Pad> {
    (0..players)
        .map(|slot| {
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
        })
        .collect()
}

/// What a race is, as text: `Collision` has no `Eq`.
fn fingerprint(race: &Race) -> String {
    let mut text = format!(
        "{} {:?} {:?}\n",
        race.tick,
        race.feedback,
        race.objects.ticks()
    );
    for kart in &race.karts {
        text += &format!("{:?} {:?} {}\n", kart.driver, kart.coll, kart.held);
    }
    text += &format!("{:?}", race.last);
    text
}

#[test]
fn a_snapshot_plays_on_as_the_original_did() {
    for id in [LevelId::DINGO_CANYON, LevelId::MYSTERY_CAVES] {
        let Some((level, objects)) = level(id) else {
            return;
        };
        let players = 3;
        let mut race = Race::new(&level, objects, &specs(players), true);
        for tick in 0..150 {
            race.tick(&level, &pads(tick, players));
        }
        let snapshot = race.clone();
        for tick in 150..400 {
            race.tick(&level, &pads(tick, players));
        }

        let mut replay = snapshot.clone();
        assert_eq!(replay.tick, 150);
        for tick in 150..400 {
            replay.tick(&level, &pads(tick, players));
        }
        assert_eq!(fingerprint(&replay), fingerprint(&race), "{id:?}");
        assert_eq!(replay.checksum(), race.checksum(), "{id:?}");
        // The snapshot itself did not move with the races that grew from it.
        assert_eq!(snapshot.tick, 150);
    }
}

#[test]
fn a_race_given_the_same_pads_plays_the_same() {
    let Some((level, objects)) = level(LevelId::DINGO_CANYON) else {
        return;
    };
    let players = 4;
    let new = || Race::new(&level, objects.clone(), &specs(players), true);
    let (mut a, mut b) = (new(), new());
    for tick in 0..300 {
        a.tick(&level, &pads(tick, players));
        b.tick(&level, &pads(tick, players));
    }
    assert_eq!(fingerprint(&a), fingerprint(&b));
    assert_eq!(a.checksum(), b.checksum());
}

#[test]
fn different_pads_make_a_different_race() {
    let Some((level, objects)) = level(LevelId::DINGO_CANYON) else {
        return;
    };
    let players = 2;
    let mut a = Race::new(&level, objects.clone(), &specs(players), true);
    let mut b = a.clone();
    for tick in 0..60 {
        a.tick(&level, &pads(tick, players));
        let mut other = pads(tick, players);
        other[1].lx = 0;
        b.tick(&level, &other);
    }
    assert_ne!(fingerprint(&a), fingerprint(&b));
    assert_ne!(a.checksum(), b.checksum());
}
