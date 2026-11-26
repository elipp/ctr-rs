//! A race of several karts (a lobby's, or one alone with bots) as a recording: the one place that
//! sets such a race up ([`start`]), so that the client and a replay (watching a race again, the
//! desync tool, later the ranked re-sim: MATCHMAKING.md) can never set it up differently.
//!
//! The file is [`MAGIC`], [`FORMAT`] (little-endian) and the [`Recording`] in `bitcode`,
//! compressed with zstd. Bitcode is not self-describing: any change to `Recording`, [`Setup`] or
//! `vehicle::demo` must bump [`FORMAT`], or old files decode as garbage instead of being refused.
//!
//! Retail bots take no pads: the header says which there were, and the sim drives them alike on
//! every replay. The host's brain bots are players to the race (pad streams of the host's), so
//! they are recorded as players are.

use instance::Objects;
use loader::lev::Level;
use vehicle::actions;
use vehicle::demo::Demo;

use crate::{Band, KartSpec, Mode, Pad, RACING_TRACKS, Race, SIM_VERSION};

const MAGIC: &[u8; 4] = b"CTRM";
const FORMAT: u32 = 1;

/// One is made at every race's end: quick beats small.
const ZSTD_LEVEL: i32 = 9;

/// Far beyond any race: a bad file can't make us allocate.
const MAX_DECODED: usize = 64 << 20;

/// Everything a race's tick 0 depends on besides the level and the karts' models.
#[derive(Clone, Debug, PartialEq, Eq, bitcode::Encode, bitcode::Decode)]
pub struct Setup {
    /// The `LevelId` raced on. The retail bots take their rubber band from it (a custom track's
    /// is the first's).
    pub track: u32,
    /// `Map::hash()` of the level.
    pub map_hash: u64,
    pub laps: u8,
    /// `vehicle::item::Roulette::masked`.
    pub masked_weapons: u16,
    /// `vehicle::item::Odds::Even`, else retail's.
    pub even_odds: bool,
    /// `sdata->randomNumber` at tick 0.
    pub seed: u16,
    pub kart_collisions: bool,
    /// The players' characters, by slot (`GAME_CHARACTER_IDS` order): the slots that take pads.
    pub characters: Vec<u8>,
    /// How many of the players are the host's brain bots, the last of them: [`Race::band`] acts
    /// on these.
    pub brain_bots: u8,
    /// The retail bots' characters, in the slots after the players'. `Race::add_bots` leaves out
    /// the ones the level has no path for, on every replay alike.
    pub retail: Vec<u8>,
    /// `vehicle::bot::difficulty_level`, of the retail bots and the band alike.
    pub difficulty: i16,
    /// The race is one of a cup (`vehicle::bot::Setup::cup`).
    pub cup: bool,
}

impl Setup {
    /// The karts of the players and of the retail bots, each with the frames `anim` says its
    /// character's model has.
    pub fn specs(&self, anim: impl Fn(usize) -> [u16; 8]) -> (Vec<KartSpec>, Vec<KartSpec>) {
        let spec = |&character: &u8| KartSpec {
            character: character as usize,
            anim_num_frames: anim(character as usize),
            bot: false,
            line: None,
        };
        (
            self.characters.iter().map(spec).collect(),
            self.retail.iter().map(spec).collect(),
        )
    }

    pub fn mode(&self) -> Mode {
        Mode::Race { laps: self.laps }
    }
}

/// The race of `setup` at tick 0, the ceremony to come. `specs` and `retail` are its players'
/// and retail bots' karts ([`Setup::specs`]); `objects` are the level's at tick 0.
pub fn start(level: &Level, objects: Objects, setup: &Setup, specs: &[KartSpec], retail: &[KartSpec]) -> Race {
    debug_assert!(
        specs.iter().map(|s| s.character).eq(setup.characters.iter().map(|&c| c as usize))
            && retail.iter().map(|s| s.character).eq(setup.retail.iter().map(|&c| c as usize)),
        "the karts are not the setup's"
    );
    let players = specs.len();
    let mut race = Race::new(level, objects, specs, setup.kart_collisions);
    if setup.brain_bots > 0 {
        let brains = players.saturating_sub(setup.brain_bots as usize)..players;
        race.band = Some(Band::new(brains, setup.difficulty));
    }
    if !retail.is_empty() {
        let bots = vehicle::bot::Setup {
            difficulty: setup.difficulty,
            track: (setup.track < RACING_TRACKS).then_some(setup.track as u8),
            cup: setup.cup,
            players,
        };
        race.add_bots(level, bots, retail);
    }
    race.mode = setup.mode();
    race.objects.random_number = i32::from(setup.seed);
    race.roulette = vehicle::item::Roulette {
        masked: setup.masked_weapons,
        odds: if setup.even_odds {
            vehicle::item::Odds::Even
        } else {
            vehicle::item::Odds::Retail
        },
    };
    race.start_ceremony(level);
    race
}

/// A recorded race, as one client saw it once its ticks were final.
#[derive(Clone, Debug, PartialEq, Eq, bitcode::Encode, bitcode::Decode)]
pub struct Recording {
    /// The sim it was played on. Informational: a replay always uses ours, which may go
    /// differently after a bump.
    pub sim_version: u32,
    pub setup: Setup,
    /// What each player was called, by slot.
    pub names: Vec<String>,
    /// The slot of the client that recorded it; None for a spectator.
    pub recorder: Option<u8>,
    /// Each player's pads from tick 0 (the ceremony included), by slot, all as long. A player
    /// that left has neutral ones from [`Recording::left`] on, as the race had.
    pub demos: Vec<Demo>,
    /// The first tick each player that left had no pad of.
    pub left: Vec<Option<u32>>,
    /// The recorder's hashes of its sim, as (tick, hash of the state before that tick):
    /// `Rollback::check_every`'s. Empty for a race nobody compared with (alone with bots).
    pub checksums: Vec<(u32, u64)>,
}

/// What a replay of a [`Recording`] came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// The karts that crossed the line, by place: (slot, the tick). Retail bots are among them,
    /// past the players' slots.
    pub finishers: Vec<(usize, u32)>,
    /// The first tick whose hash is not the recorder's, if any: the replay went otherwise.
    pub mismatch: Option<u32>,
}

impl Recording {
    /// What `rollback` (started by [`start`] from `setup`, and `Rollback::record`ed) holds so
    /// far: its final ticks. None if it records nothing.
    pub fn of(
        rollback: &crate::rollback::Rollback<Race>,
        setup: Setup,
        names: Vec<String>,
        recorder: Option<u8>,
    ) -> Option<Recording> {
        let recorded = rollback.recorded()?;
        Some(Recording {
            sim_version: SIM_VERSION,
            setup,
            names,
            recorder,
            demos: recorded.demos.clone(),
            left: (0..rollback.players()).map(|slot| rollback.left_at(slot)).collect(),
            checksums: recorded.checksums.clone(),
        })
    }

    /// The ticks recorded.
    pub fn length(&self) -> u32 {
        self.demos.iter().map(|d| d.length).min().unwrap_or(0)
    }

    /// The pad of `slot` for `tick`; past the recording, a neutral one.
    pub fn pad(&self, slot: usize, tick: u32) -> Pad {
        // (A pad has no taps: `previous_held` is of no matter.)
        (self.demos.get(slot))
            .and_then(|demo| demo.input(tick, 0))
            .map_or_else(Pad::default, |input| Pad::of_input(&input))
    }

    pub fn pads(&self, tick: u32) -> Vec<Pad> {
        (0..self.demos.len()).map(|slot| self.pad(slot, tick)).collect()
    }

    /// Plays the whole recording on `level` from `objects`, with the karts of
    /// [`Setup::specs`].
    pub fn replay(&self, level: &Level, objects: Objects, specs: &[KartSpec], retail: &[KartSpec]) -> Outcome {
        let mut player = Player::new(level, objects, self, specs, retail);
        while player.race.tick < self.length() {
            player.tick(level);
        }
        player.outcome()
    }

    /// The first tick and slot whose pads are not `other`'s, over the ticks both have; a slot
    /// only one of them has differs at tick 0.
    pub fn first_pad_difference(&self, other: &Recording) -> Option<(u32, usize)> {
        let length = self.length().min(other.length());
        let slots = self.demos.len().max(other.demos.len());
        (0..slots)
            .filter_map(|slot| {
                let (Some(a), Some(b)) = (self.demos.get(slot), other.demos.get(slot)) else {
                    return Some((0, slot));
                };
                let tick = (0..length).find(|&t| a.input(t, 0) != b.input(t, 0))?;
                Some((tick, slot))
            })
            .min()
    }

    /// The first tick both hashed whose hashes differ.
    pub fn first_hash_difference(&self, other: &Recording) -> Option<u32> {
        let theirs: std::collections::HashMap<u32, u64> = other.checksums.iter().copied().collect();
        (self.checksums.iter())
            .filter(|(tick, hash)| theirs.get(tick).is_some_and(|h| h != hash))
            .map(|&(tick, _)| tick)
            .min()
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = MAGIC.to_vec();
        bytes.extend(FORMAT.to_le_bytes());
        let packed = zstd::bulk::compress(&bitcode::encode(self), ZSTD_LEVEL);
        bytes.extend(packed.expect("zstd compresses to memory"));
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Recording, String> {
        let rest = bytes.strip_prefix(MAGIC).ok_or("not a race recording")?;
        let (format, rest) = rest.split_first_chunk::<4>().ok_or("cut short")?;
        let format = u32::from_le_bytes(*format);
        if format != FORMAT {
            return Err(format!("race recording format {format}, this build reads {FORMAT}"));
        }
        let unpacked = zstd::bulk::decompress(rest, MAX_DECODED).map_err(|e| format!("not zstd: {e}"))?;
        bitcode::decode(&unpacked).map_err(|e| format!("bad race recording: {e}"))
    }
}

/// A recording played back a tick at a time, checking the recorder's hashes as it goes.
#[derive(Clone)]
pub struct Player {
    race: Race,
    recording: Recording,
    /// Into `recording.checksums`: the next to check.
    next_sum: usize,
    mismatch: Option<u32>,
}

impl Player {
    /// At tick 0 on `level` from `objects`, with the karts of [`Setup::specs`].
    pub fn new(level: &Level, objects: Objects, recording: &Recording, specs: &[KartSpec], retail: &[KartSpec]) -> Player {
        let mut checksums = recording.checksums.clone();
        checksums.sort_unstable();
        Player {
            race: start(level, objects, &recording.setup, specs, retail),
            recording: Recording {
                checksums,
                ..recording.clone()
            },
            next_sum: 0,
            mismatch: None,
        }
    }

    /// Plays the next tick with the recorded pads; past the recording's end, neutral ones.
    pub fn tick(&mut self, level: &Level) {
        let tick = self.race.tick;
        let sums = &self.recording.checksums;
        while let Some(&(at, hash)) = sums.get(self.next_sum)
            && at <= tick
        {
            self.next_sum += 1;
            if at == tick && self.mismatch.is_none() && self.race.checksum() != hash {
                self.mismatch = Some(tick);
            }
        }
        self.race.tick(level, &self.recording.pads(tick));
    }

    pub fn race(&self) -> &Race {
        &self.race
    }

    /// Who has crossed the line so far, and whether the hashes have held.
    pub fn outcome(&self) -> Outcome {
        let mut finishers: Vec<(i16, usize, u32)> = (self.race.karts.iter().enumerate())
            .filter(|(_, kart)| kart.driver.actions_flag_set & actions::RACE_FINISHED != 0)
            .filter_map(|(slot, kart)| {
                let rank = kart.driver.driver_rank;
                Some((rank, slot, *self.race.finish_ticks.get(usize::try_from(rank).ok()?)?))
            })
            .collect();
        finishers.sort_unstable();
        Outcome {
            finishers: finishers.into_iter().map(|(_, slot, tick)| (slot, tick)).collect(),
            mismatch: self.mismatch,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recording(held: &[u32]) -> Recording {
        let mut demo = Demo::new(1);
        for (tick, &held) in held.iter().enumerate() {
            let input = vehicle::Input {
                buttons_held: held,
                ..vehicle::Input::default()
            };
            demo.record(tick as u32, &input);
        }
        Recording {
            sim_version: SIM_VERSION,
            setup: Setup {
                track: 3,
                map_hash: 0x0123_4567_89ab_cdef,
                laps: 3,
                masked_weapons: 0,
                even_odds: false,
                seed: 77,
                kart_collisions: true,
                characters: vec![1, 2],
                brain_bots: 1,
                retail: vec![4],
                difficulty: 0xf0,
                cup: false,
            },
            names: vec!["a".into(), "b".into()],
            recorder: Some(0),
            demos: vec![demo.clone(), demo],
            left: vec![None, Some(2)],
            checksums: vec![(0, 5), (30, 6)],
        }
    }

    #[test]
    fn bytes_round_trip() {
        let rec = recording(&[0, 1, 1]);
        assert_eq!(Recording::from_bytes(&rec.to_bytes()), Ok(rec));
        assert!(Recording::from_bytes(b"nope").is_err());
    }

    #[test]
    fn differences_are_the_first_tick() {
        let a = recording(&[0, 1, 1, 2, 2]);
        let mut b = recording(&[0, 1, 1, 2, 2]);
        assert_eq!(a.first_pad_difference(&b), None);
        assert_eq!(a.first_hash_difference(&b), None);
        b.demos[1] = recording(&[0, 1, 3, 2, 2]).demos[1].clone();
        b.checksums[1].1 = 7;
        assert_eq!(a.first_pad_difference(&b), Some((2, 1)));
        assert_eq!(a.first_hash_difference(&b), Some(30));
        // Past the shorter one's end nothing is compared.
        let short = recording(&[0, 1]);
        assert_eq!(b.first_pad_difference(&short), None);
    }
}
