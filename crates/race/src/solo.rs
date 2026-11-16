//! A race alone against the clock (a relic race or a time trial), as a recorded run: the one place
//! that sets it up, so that the client and a replay (local personal bests, later a leaderboard's
//! verifier: RELIC-RACE-LEADERBOARDS.md) can never set it up differently.
//!
//! The file is [`MAGIC`], [`FORMAT`] (little-endian) and the [`Run`] in `bitcode`, compressed with
//! zstd. Bitcode is not self-describing: any change to `Run` or `vehicle::demo` must bump
//! [`FORMAT`], or old files decode as garbage instead of being refused.

use instance::lev_init::LevMode;
use instance::Objects;
use loader::lev::Level;
use vehicle::actions;
use vehicle::demo::Demo;

use crate::{KartSpec, Mode, Pad, Race, SIM_VERSION};

const MAGIC: &[u8; 4] = b"CTRR";
const FORMAT: u32 = 3;

/// Small files, made once per personal best.
const ZSTD_LEVEL: i32 = 19;

/// Far beyond any run: a bad file can't make us allocate.
const MAX_DECODED: usize = 16 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, bitcode::Encode, bitcode::Decode)]
pub enum Kind {
    Relic,
    TimeTrial,
}

impl Kind {
    pub fn mode(self, laps: u8) -> Mode {
        match self {
            Kind::Relic => Mode::Relic { laps },
            Kind::TimeTrial => Mode::TimeTrial { laps },
        }
    }

    pub fn of(mode: Mode) -> Option<(Kind, u8)> {
        match mode {
            Mode::Relic { laps } => Some((Kind::Relic, laps)),
            Mode::TimeTrial { laps } => Some((Kind::TimeTrial, laps)),
            Mode::FreeDrive | Mode::Race { .. } => None,
        }
    }

    /// The `lev_init` filter its level is put through. Both kinds are on the relic LEV
    /// (`levelLOD` 8): only this tells them apart.
    pub fn lev_mode(self) -> LevMode {
        match self {
            Kind::Relic => LevMode::Relic,
            Kind::TimeTrial => LevMode::TimeTrial,
        }
    }
}

/// The race at tick 0, the ceremony to come. `objects` are the level's at tick 0, after
/// `lev_init::lev_init_all(kind.lev_mode())`.
///
/// Kart collisions and the roulette are fixed here rather than taken from the rules: alone, with
/// no weapon crates, they change nothing, and a run must not depend on settings it doesn't record.
pub fn start(level: &Level, objects: Objects, spec: KartSpec, kind: Kind, laps: u8, seed: u16) -> Race {
    let mut race = Race::new(level, objects, &[spec], false);
    race.mode = kind.mode(laps);
    race.objects.random_number = i32::from(seed);
    race.start_ceremony(level);
    race
}

/// A recorded solo run: everything its replay depends on besides the level and the kart's model.
#[derive(Clone, Debug, PartialEq, Eq, bitcode::Encode, bitcode::Decode)]
pub struct Run {
    /// The sim it was played on. Informational: a replay always uses ours, which may give
    /// another time (or no finish) after a bump.
    pub sim_version: u32,
    pub kind: Kind,
    /// `Map::hash()` of the level, relic LEV included.
    pub map_hash: u64,
    pub laps: u8,
    pub seed: u16,
    /// From tick 0, the ceremony included (revving on the grid matters).
    pub demo: Demo,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// `Race::final_ms`: a relic race's perfect bonus taken off.
    pub final_ms: i32,
    pub perfect: bool,
    /// The tick the kart crossed the line at.
    pub finish_tick: u32,
}

impl Run {
    /// The run of `race` (started by [`start`]) up to its finish: `demo` has its pads from tick 0
    /// on, and may go on past the finish (those pads changed nothing). None if it isn't a solo
    /// race, hasn't finished, or `demo` doesn't reach the finish.
    pub fn of_finished(race: &Race, map_hash: u64, seed: u16, mut demo: Demo) -> Option<Run> {
        let (kind, laps) = Kind::of(race.mode)?;
        let finish_tick = *race.finish_ticks.first()?;
        if demo.length <= finish_tick {
            return None;
        }
        demo.truncate(finish_tick + 1);
        Some(Run {
            sim_version: SIM_VERSION,
            kind,
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
        let mut race = start(level, objects, spec, self.kind, self.laps, self.seed);
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
        let mut bytes = MAGIC.to_vec();
        bytes.extend(FORMAT.to_le_bytes());
        let packed = zstd::bulk::compress(&bitcode::encode(self), ZSTD_LEVEL);
        bytes.extend(packed.expect("zstd compresses to memory"));
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Run, String> {
        let rest = bytes.strip_prefix(MAGIC).ok_or("not a solo run")?;
        let (format, rest) = rest.split_first_chunk::<4>().ok_or("cut short")?;
        let format = u32::from_le_bytes(*format);
        if format != FORMAT {
            return Err(format!("solo run format {format}, this build reads {FORMAT}"));
        }
        let unpacked = zstd::bulk::decompress(rest, MAX_DECODED).map_err(|e| format!("not zstd: {e}"))?;
        bitcode::decode(&unpacked).map_err(|e| format!("bad solo run: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            kind: Kind::TimeTrial,
            map_hash: 0x0123_4567_89ab_cdef,
            laps: 3,
            seed: 4242,
            demo,
        };
        assert_eq!(Run::from_bytes(&run.to_bytes()), Ok(run));
        assert!(Run::from_bytes(b"nope").is_err());
    }
}
