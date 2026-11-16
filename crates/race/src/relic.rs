//! A relic race alone, as a recorded run: the one place that sets it up, so that the client and a
//! replay (local personal bests, later a leaderboard's verifier: RELIC-RACE-LEADERBOARDS.md) can
//! never set it up differently.
//!
//! The file is zstd-compressed text: a header line,
//! `relic 1 sim=<SIM_VERSION> map=<hash, hex> laps=<n> seed=<n>`, then the `vehicle::demo` text.

use instance::Objects;
use loader::lev::Level;
use vehicle::actions;
use vehicle::demo::Demo;

use crate::{KartSpec, Mode, Pad, Race, SIM_VERSION};

const HEADER: &str = "relic 1";

/// Small files, made once per personal best.
const ZSTD_LEVEL: i32 = 19;

/// Far beyond any demo (a 3-lap run is a few KB of text): a bad file can't make us allocate.
const MAX_TEXT: usize = 16 << 20;

/// The race at tick 0, the ceremony to come. `objects` are the level's at tick 0, after
/// `lev_init::lev_init_all(LevMode::Relic)`.
///
/// Kart collisions and the roulette are fixed here rather than taken from the rules: alone, with
/// no weapon crates, they change nothing, and a run must not depend on settings it doesn't record.
pub fn start(level: &Level, objects: Objects, spec: KartSpec, laps: u8, seed: u16) -> Race {
    let mut race = Race::new(level, objects, &[spec], false);
    race.mode = Mode::Relic { laps };
    race.objects.random_number = i32::from(seed);
    race.start_ceremony(level);
    race
}

/// The relics a run can earn, from the slowest time to the fastest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    Sapphire,
    Gold,
    Platinum,
}

/// Retail's relic times of the racing tracks (by `LevelId`), sapphire, gold, platinum, in its
/// clock's units (960 a second): `data.RelicTime`. They are for its 3 laps.
const RELIC_TIMES: [[u32; 3]; 18] = [
    [0x13ec0, 0x102c0, 0xc6c0],
    [0x14a00, 0x10a40, 0xca80],
    [0x15180, 0xff00, 0xa8c0],
    [0x120c0, 0xf3c0, 0xc300],
    [0x12c00, 0xe880, 0xa140],
    [0x16080, 0x102c0, 0x9d80],
    [0x11940, 0xf3c0, 0xce40],
    [0x2b5c0, 0x24180, 0x1c980],
    [0x15cc0, 0xf3c0, 0x8ac0],
    [0x1af40, 0x18600, 0x15900],
    [0x24540, 0x1d100, 0x15900],
    [0x1fa40, 0x16080, 0xc6c0],
    [0x2a300, 0x23dc0, 0x1d4c0],
    [0x2e2c0, 0x29400, 0x24180],
    [0x16440, 0x10e00, 0xb7c0],
    [0x34bc0, 0x2f580, 0x29b80],
    [0x1af40, 0x189c0, 0x17700],
    [0x189c0, 0x15900, 0x12840],
];

/// The relic to go for on `track` (a `LevelId`) with `laps`, and its time in ms: the slowest one
/// `best_ms` (`Outcome::final_ms`) hasn't earned. Retail earns a relic at its time or under
/// (`RR_EndEvent_UnlockAward`). None on a track or lap count retail has no times for, or once
/// the platinum is earned.
pub fn target(track: u32, laps: u8, best_ms: Option<i32>) -> Option<(Tier, i32)> {
    let times = RELIC_TIMES.get(track as usize).filter(|_| laps == 3)?;
    [Tier::Sapphire, Tier::Gold, Tier::Platinum]
        .into_iter()
        .zip(times.map(|units| (units * 1000 / 960) as i32))
        .find(|&(_, ms)| best_ms.is_none_or(|best| best > ms))
}

/// A recorded relic run: everything its replay depends on besides the level and the kart's model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    /// The sim it was played on. Informational: a replay always uses ours, which may give
    /// another time (or no finish) after a bump.
    pub sim_version: u32,
    /// `Map::hash()` of the level, relic LEV included.
    pub map_hash: u64,
    pub laps: u8,
    pub seed: u16,
    /// From tick 0, the ceremony included (revving on the grid matters).
    pub demo: Demo,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// `Race::final_ms`: the perfect bonus taken off.
    pub final_ms: i32,
    pub perfect: bool,
    /// The tick the kart crossed the line at.
    pub finish_tick: u32,
}

impl Run {
    /// The run of `race` (started by [`start`]) up to its finish: `demo` has its pads from tick 0
    /// on, and may go on past the finish (those pads changed nothing). None if it hasn't
    /// finished, or `demo` doesn't reach the finish.
    pub fn of_finished(race: &Race, map_hash: u64, seed: u16, mut demo: Demo) -> Option<Run> {
        let Mode::Relic { laps } = race.mode else {
            return None;
        };
        let finish_tick = *race.finish_ticks.first()?;
        if demo.length <= finish_tick {
            return None;
        }
        demo.truncate(finish_tick + 1);
        Some(Run {
            sim_version: SIM_VERSION,
            map_hash,
            laps,
            seed,
            demo,
        })
    }

    /// Plays the run on `level` from `objects` (as for [`start`]) with the kart `spec`, whose
    /// character must be the demo's. None if the kart hasn't finished when the pads run out.
    pub fn replay(&self, level: &Level, objects: Objects, spec: KartSpec) -> Option<Outcome> {
        debug_assert_eq!(spec.character, self.demo.character);
        let mut race = start(level, objects, spec, self.laps, self.seed);
        let mut held = 0;
        for tick in 0..self.demo.length {
            let input = self.demo.input(tick, held)?;
            held = input.buttons_held;
            race.tick(level, &[Pad::of_input(&input)]);
            if race.karts[0].driver.actions_flag_set & actions::RACE_FINISHED != 0 {
                return Some(Outcome {
                    final_ms: race.final_ms(0)?,
                    perfect: race.perfect(0),
                    finish_tick: tick,
                });
            }
        }
        None
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let text = format!(
            "{HEADER} sim={} map={:016x} laps={} seed={}\n{}",
            self.sim_version,
            self.map_hash,
            self.laps,
            self.seed,
            self.demo.to_text()
        );
        zstd::bulk::compress(text.as_bytes(), ZSTD_LEVEL).expect("zstd compresses to memory")
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Run, String> {
        let text = zstd::bulk::decompress(bytes, MAX_TEXT).map_err(|e| format!("not zstd: {e}"))?;
        let text = String::from_utf8(text).map_err(|_| "not text")?;
        let (head, demo) = text.split_once('\n').ok_or("no header line")?;
        let fields = head
            .strip_prefix(HEADER)
            .ok_or_else(|| format!("not a relic run: '{head}'"))?;
        let mut run = Run {
            sim_version: 0,
            map_hash: 0,
            laps: 0,
            seed: 0,
            demo: Demo::from_text(demo)?,
        };
        let mut seen = 0;
        for field in fields.split_whitespace() {
            let bad = || format!("bad relic run field '{field}'");
            let (key, value) = field.split_once('=').ok_or_else(bad)?;
            match key {
                "sim" => run.sim_version = value.parse().map_err(|_| bad())?,
                "map" => run.map_hash = u64::from_str_radix(value, 16).map_err(|_| bad())?,
                "laps" => run.laps = value.parse().map_err(|_| bad())?,
                "seed" => run.seed = value.parse().map_err(|_| bad())?,
                // (Fields of a later version 1 writer are skipped.)
                _ => continue,
            }
            seen += 1;
        }
        if seen < 4 {
            return Err(format!("relic run header lacks fields: '{head}'"));
        }
        Ok(run)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_target_is_the_next_relic_not_earned() {
        // Dingo Canyon: 1:25, 1:09, 0:53.
        assert_eq!(target(0, 3, None), Some((Tier::Sapphire, 85_000)));
        assert_eq!(target(0, 3, Some(85_000)), Some((Tier::Gold, 69_000)));
        assert_eq!(target(0, 3, Some(53_001)), Some((Tier::Platinum, 53_000)));
        assert_eq!(target(0, 3, Some(53_000)), None);
        // Turbo Track's platinum: 1:19.
        assert_eq!(target(17, 3, Some(80_000)), Some((Tier::Platinum, 79_000)));
        assert_eq!(target(0, 5, None), None);
        assert_eq!(target(18, 3, None), None);
    }

    #[test]
    fn bytes_round_trip() {
        let mut demo = Demo::new(2);
        demo.record(0, &vehicle::Input::default());
        demo.record(
            1,
            &vehicle::Input {
                buttons_held: vehicle::buttons::CROSS,
                ..vehicle::Input::default()
            },
        );
        let run = Run {
            sim_version: SIM_VERSION,
            map_hash: 0x0123_4567_89ab_cdef,
            laps: 3,
            seed: 4242,
            demo,
        };
        assert_eq!(Run::from_bytes(&run.to_bytes()), Ok(run));
        assert!(Run::from_bytes(b"nope").is_err());
    }
}
