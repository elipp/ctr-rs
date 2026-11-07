//! Bots on the game's tracks: they leave the grid when the lights go out, follow their paths,
//! and finish the race on their own. Skipped without the disc image.

#[path = "../../loader/tests/common/mod.rs"]
mod common;

use instance::Objects;
use loader::bigindex::{BigIndex, LevFile, Lod};
use loader::dram::DramFile;
use loader::lev::Level;
use loader::level::LevelId;
use race::{KartSpec, Pad, Race};
use vehicle::bot::{Setup, difficulty_level};

fn level(id: LevelId) -> Option<(Level, Objects)> {
    let mut b = common::open_bigfile()?;
    let idx = BigIndex::level(id, Lod::OnePlayer, LevFile::Lev);
    let dram = DramFile::parse(&b.big.read(&mut b.disc, idx.get()).unwrap()).unwrap();
    let level = Level::parse(&dram).unwrap();
    let objects = Objects::new(&level, &dram);
    Some((level, objects))
}

fn spec(character: usize) -> KartSpec {
    KartSpec {
        character,
        anim_num_frames: [0; 8],
    }
}

/// One player who stays on the grid and seven bots, with the start ceremony.
fn race(id: LevelId, level: &Level, objects: Objects) -> Race {
    let mut race = Race::new(level, objects, &[spec(0)], false);
    let setup = Setup {
        difficulty: difficulty_level::MEDIUM,
        track: Some(id.0 as u8),
        cup: false,
        players: 1,
    };
    let bots: Vec<KartSpec> = (1..8).map(spec).collect();
    race.add_bots(level, setup, &bots);
    race.start_ceremony(level);
    race
}

/// Eight minutes of the race at 30 ticks a second (a bot chasing a player that never left the
/// grid takes its time).
const MAX_TICKS: u32 = 30 * 60 * 8;

#[test]
fn needs_disc_bots_finish_every_track() {
    if common::open_bigfile().is_none() {
        return;
    }
    for id in LevelId::DINGO_CANYON.0..=LevelId::TURBO_TRACK.0 {
        let id = LevelId(id);
        let (level, objects) = level(id).unwrap();
        let mut race = race(id, &level, objects);
        assert_eq!(race.karts.len(), 8, "level {}: every bot has a path", id.0);
        let start: Vec<_> = race.karts.iter().map(|k| k.driver.pos_curr).collect();
        let mut finished_at = [None; 8];
        while race.tick < MAX_TICKS && finished_at[1..].iter().any(Option::is_none) {
            let ticks = race.tick(&level, &[Pad::default()]);
            for (slot, tick) in ticks.iter().enumerate() {
                if tick.lap.finished {
                    finished_at[slot] = Some(race.event_ms);
                }
            }
            if race.lights_ms > 0 {
                for (slot, (kart, start)) in race.karts.iter().zip(&start).enumerate() {
                    let at = kart.driver.pos_curr;
                    assert!(
                        at.x == start.x && at.z == start.z && (at.y - start.y).abs() <= 0x200,
                        "level {}: kart {slot} moved before the green light, {start:?} to {at:?}",
                        id.0
                    );
                }
            }
        }
        let times: Vec<String> = finished_at[1..]
            .iter()
            .map(|t| t.map_or("-".into(), |ms| format!("{:.1}", ms as f64 / 1000.0)))
            .collect();
        println!("level {:>2}: the bots finish at {}", id.0, times.join(" "));
        assert!(
            finished_at[1..].iter().all(Option::is_some),
            "level {}: a bot did not finish in eight minutes: {times:?}",
            id.0
        );
        assert!(finished_at[0].is_none(), "the player stayed on the grid");
    }
}

#[test]
fn needs_disc_a_race_with_bots_replays_the_same() {
    let id = LevelId::DINGO_CANYON;
    let Some((level, objects)) = level(id) else {
        return;
    };
    let mut a = race(id, &level, objects);
    for _ in 0..400 {
        a.tick(&level, &[Pad::default()]);
    }
    let mut b = a.clone();
    for _ in 0..600 {
        a.tick(&level, &[Pad::default()]);
        b.tick(&level, &[Pad::default()]);
    }
    assert_eq!(a.checksum(), b.checksum());
}
