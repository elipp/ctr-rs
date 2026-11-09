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
    race_with(id, level, objects, false)
}

/// [`race`], with the karts bumping into each other or not.
fn race_with(id: LevelId, level: &Level, objects: Objects, kart_collisions: bool) -> Race {
    let mut race = Race::new(level, objects, &[spec(0)], kart_collisions);
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
        // (The karts bump: a bot pushed off its path finds it again.)
        let mut race = race_with(id, &level, objects, true);
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

/// A bot hit by each kind of damage loses its speed, plays the hit out (a blast throws it up, a
/// plant hides it and the mask brings it back), and races on to the finish.
#[test]
fn needs_disc_a_bot_that_is_hit_recovers_and_finishes() {
    use vehicle::bot::{damage_state, flags};
    use vehicle::pick::damage;
    let id = LevelId::DINGO_CANYON;
    let Some((level, objects)) = level(id) else {
        return;
    };
    for hit in [
        damage::SPIN,
        damage::BLAST,
        damage::SQUISH,
        damage::BURN,
        damage::MASK_GRAB,
    ] {
        let mut race = race(id, &level, objects.clone());
        while race.event_ms < 20_000 {
            race.tick(&level, &[Pad::default()]);
        }
        let slot = 4;
        let speed = |race: &Race| {
            race.karts[slot]
                .driver
                .bot
                .as_ref()
                .unwrap()
                .phys
                .speed_linear
        };
        let before = speed(&race);
        assert!(before > 0x1000, "the bot is under way");
        let y = race.karts[slot].driver.pos_curr.y;
        let mut effects = Vec::new();
        assert!(vehicle::bot::hurt(
            &mut race.karts[slot].driver,
            hit,
            Some(true),
            &mut effects
        ));
        race.tick(&level, &[Pad::default()]);
        let d = &race.karts[slot].driver;
        let b = d.bot.as_ref().unwrap();
        assert!(b.flags & flags::DAMAGE_ACTIVE != 0, "hit {hit}");
        assert!(
            speed(&race) < before,
            "hit {hit}: {before} -> {}",
            speed(&race)
        );
        match hit {
            damage::BLAST => {
                assert_eq!(b.damage_state, damage_state::BLAST);
                assert!(vehicle::bot::blast_tumble(d).is_some());
                assert!(
                    effects
                        .iter()
                        .any(|e| matches!(e, vehicle::Effect::Voiceline { .. }))
                );
                for _ in 0..5 {
                    race.tick(&level, &[Pad::default()]);
                }
                assert!(race.karts[slot].driver.pos_curr.y > y + 0x4000, "thrown up");
            }
            damage::MASK_GRAB => assert!(vehicle::bot::hidden(d)),
            damage::BURN => assert!(d.burn_timer > 0),
            damage::SQUISH => assert!(d.squish_timer > 0),
            _ => assert_eq!(b.damage_state, damage_state::SPIN),
        }
        // Over in a few seconds, and back to racing.
        for _ in 0..30 * 6 {
            race.tick(&level, &[Pad::default()]);
        }
        let d = &race.karts[slot].driver;
        assert!(
            d.bot.as_ref().unwrap().flags & flags::DAMAGE_ACTIVE == 0,
            "hit {hit} wore off"
        );
        assert!(!vehicle::bot::hidden(d));
        while race.tick < MAX_TICKS
            && race.karts[slot].driver.actions_flag_set & vehicle::actions::RACE_FINISHED == 0
        {
            race.tick(&level, &[Pad::default()]);
        }
        assert!(
            race.karts[slot].driver.actions_flag_set & vehicle::actions::RACE_FINISHED != 0,
            "hit {hit}: the bot finished"
        );
    }
}

/// A Nitro crate in a bot's way blows it up, and a red beaker leaves a cloud over it.
#[test]
fn needs_disc_a_bot_runs_into_crates_and_beakers() {
    use weapon::mine::MineKind;
    let id = LevelId::DINGO_CANYON;
    let Some((level, objects)) = level(id) else {
        return;
    };
    for kind in [MineKind::Nitro, MineKind::RedBeaker] {
        let mut race = race(id, &level, objects.clone());
        while race.event_ms < 20_000 {
            race.tick(&level, &[Pad::default()]);
        }
        let slot = 4;
        let at = race.karts[slot].driver.inst_pos;
        race.weapons.place_mine(kind, at, 0);
        for _ in 0..3 {
            race.tick(&level, &[Pad::default()]);
        }
        let d = &race.karts[slot].driver;
        let b = d.bot.as_ref().unwrap();
        assert!(
            b.flags & vehicle::bot::flags::DAMAGE_ACTIVE != 0,
            "{kind:?} hit the bot"
        );
        match kind {
            MineKind::Nitro => assert_eq!(b.damage_state, vehicle::bot::damage_state::BLAST),
            _ => assert!(d.cloud.is_some(), "a cloud over the bot"),
        }
    }
}

/// Two bots that touch both lose their path for a moment; a bot the player runs into is pushed,
/// and the player bounces.
#[test]
fn needs_disc_karts_bump_into_bots() {
    use vehicle::bot::flags::FREE_PHYSICS;
    let id = LevelId::DINGO_CANYON;
    let Some((level, objects)) = level(id) else {
        return;
    };
    let free = |race: &Race, slot: usize| {
        race.karts[slot].driver.bot.as_ref().unwrap().flags & FREE_PHYSICS != 0
    };

    let mut race = race_with(id, &level, objects.clone(), true);
    while race.event_ms < 20_000 {
        race.tick(&level, &[Pad::default()]);
    }
    assert!(!free(&race, 4) && !free(&race, 3));
    // Bot 4 ticks before bot 3: it finds bot 3 where it was last drawn.
    let p = race.karts[4].driver.pos_curr;
    race.karts[3].driver.inst_pos = [(p.x >> 8) + 40, p.y >> 8, p.z >> 8];
    race.tick(&level, &[Pad::default()]);
    assert!(free(&race, 4) && free(&race, 3), "both pushed off the path");

    let mut race = race_with(id, &level, objects, true);
    while race.event_ms < 20_000 {
        race.tick(&level, &[Pad::default()]);
    }
    let p = race.karts[0].driver.pos_curr;
    race.karts[5].driver.inst_pos = [(p.x >> 8) + 40, p.y >> 8, p.z >> 8];
    race.tick(&level, &[Pad::default()]);
    // (Its push sideways is small, and wears off within its own tick.)
    let accel = race.karts[5].driver.bot.as_ref().unwrap().phys.accel;
    assert!(accel.x != 0 || accel.z != 0, "the bot was pushed");
    let v = race.karts[0].driver.velocity;
    assert!(v.x != 0 || v.z != 0, "the player bounced off");
}

/// The bots next to a player in the race fire at it now and then, once the race is under way,
/// and say so; some of it hits.
#[test]
fn needs_disc_bots_fire_at_a_player_near_them() {
    let id = LevelId::CRASH_COVE;
    let Some((level, objects)) = level(id) else {
        return;
    };
    let mut race = race_with(id, &level, objects, true);
    // The player holds the gas and drives into the walls, so the bots go by it.
    let gas = Pad {
        held: vehicle::buttons::CROSS,
        ..Pad::default()
    };
    let (mut shots, mut hits) = (0, 0);
    while race.tick < 30 * 200 {
        let ticks = race.tick(&level, &[gas]);
        for tick in &ticks[1..] {
            for effect in &tick.effects {
                if let vehicle::Effect::Voiceline {
                    id: 10 | 11 | 0xf, ..
                } = effect
                {
                    assert!(race.event_ms >= 0x4b00, "nothing in the first seconds");
                    shots += 1;
                }
            }
        }
        hits += race
            .last_weapons
            .hits
            .iter()
            .filter(|h| h.attacker.is_some_and(|a| a > 0))
            .count();
    }
    assert!(shots > 0, "the bots fired");
    assert!(hits > 0, "and hit");
}

/// Where their paths say so, the bots run through the crates and the fruit: they break the one
/// and take the other (for nothing), while the player stays on the grid.
#[test]
fn needs_disc_bots_break_crates_and_take_fruit() {
    let id = LevelId::DINGO_CANYON;
    let Some((level, objects)) = level(id) else {
        return;
    };
    let mut race = race(id, &level, objects);
    while race.tick < 30 * 60 {
        race.tick(&level, &[Pad::default()]);
    }
    let breaks: u32 = race.objects.crates.iter().map(|c| c.sim.breaks).sum();
    let fruit = race
        .objects
        .fruit
        .iter()
        .filter(|f| f.sim.collected)
        .count();
    assert!(breaks > 0, "the bots broke crates");
    assert!(fruit > 0, "and took fruit");
    // A bot gets nothing out of them.
    for kart in &race.karts[1..] {
        assert_eq!(kart.driver.num_wumpas, 0);
        assert_eq!(kart.driver.held_item_id, vehicle::item::held::NONE);
    }
}

/// A bot the plant let go of is brought back by its mask: the mask comes down with the kart,
/// lets go of it on the path, and fades away.
#[test]
fn needs_disc_the_mask_brings_a_grabbed_bot_back() {
    let id = LevelId::DINGO_CANYON;
    let Some((level, objects)) = level(id) else {
        return;
    };
    let mut race = race(id, &level, objects);
    while race.event_ms < 20_000 {
        race.tick(&level, &[Pad::default()]);
    }
    let slot = 4;
    let mut effects = Vec::new();
    assert!(vehicle::bot::hurt(
        &mut race.karts[slot].driver,
        vehicle::pick::damage::MASK_GRAB,
        Some(true),
        &mut effects
    ));
    let revving = |race: &Race| race.karts[slot].driver.bot.as_ref().unwrap().revving;
    while !revving(&race) {
        assert!(race.tick < MAX_TICKS);
        race.tick(&level, &[Pad::default()]);
    }
    let mut carried = 0;
    while revving(&race) {
        let d = &race.karts[slot].driver;
        let mask = d.mask.expect("the mask carries the bot");
        assert!(mask.world_space && !mask.fading);
        assert_eq!(
            mask.pos[1],
            (d.pos_curr.y >> 8) as i16,
            "it comes down with the kart"
        );
        assert!(mask.shown.is_some());
        carried += 1;
        race.tick(&level, &[Pad::default()]);
    }
    assert!(carried > 30, "carried for {carried} ticks");
    let mask = race.karts[slot].driver.mask.expect("it lets go");
    assert!(!mask.world_space);
    for _ in 0..30 {
        race.tick(&level, &[Pad::default()]);
    }
    assert_eq!(race.karts[slot].driver.mask, None, "and is gone");
}
