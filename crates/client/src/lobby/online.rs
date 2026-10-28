//! The race that has started: [`Online`], and the desyncs it reports.

use bevy::prelude::*;
use net::{EndpointId, Pad};

/// A race that has started.
pub struct Online {
    /// Our player.
    pub slot: usize,
    /// The character each slot drives, which sets its kart's stats.
    pub characters: Vec<u8>,
    /// The short form of each slot's iroh key (`fmt_short`), which keys the pings.
    pub names: Vec<String>,
    /// The endpoint of each slot.
    pub ids: Vec<EndpointId>,
    /// What each slot is called on screen (`net::display_names`): the nickname if there is one.
    /// It follows the nicknames, which can change in a race.
    pub labels: Vec<String>,
    /// The `LevelId` it is on, which the viewer shows before anything is played.
    pub track: u32,
    /// The hash of the contents of the map, if it is a map (0 for the game's tracks).
    pub hash: u64,
    /// The pads of the others that have come in and are not in the sim yet: (slot, first tick,
    /// the pads).
    pub inbox: Vec<(usize, u32, Vec<Pad>)>,
    /// Set by the start and cleared by `drive_tick`, which puts the sim back to tick 0.
    pub fresh: bool,
    /// Set once `remote::build` has put the other karts on the grid.
    pub built: bool,
    /// Tick 0 has come: everyone is ready.
    pub go: bool,
    /// The kart of each slot, as the race takes them: set by `remote::build`, which has their
    /// models.
    pub specs: Vec<race::KartSpec>,
    /// How many times a late pad differed from its prediction, and the ticks played again for it.
    pub corrections: u32,
    pub replayed: u32,
    /// The hashes of the sims, ours and the others'.
    pub checks: race::desync::Checks,
    /// How many ticks our clock is ahead of each player's (smoothed; ours is 0), from `net`.
    pub ahead: Vec<f32>,
    /// The players whose sim differs from ours, with the first tick it was seen at.
    pub desync: Vec<Desync>,
    /// The winner has been announced; the race ends [`AFTERMATH_SECONDS`] later.
    pub won: bool,
    /// The players that left the race (by slot): they have a neutral pad from their last on, and
    /// the race does not wait for them to finish.
    pub gone: Vec<bool>,
    /// The players that left whose pads we still have to report (`Event::Gone`): `drive_tick`
    /// answers once the pads that came in are taken.
    pub asked: Vec<usize>,
    /// The kart the camera follows: ours, or another's once we have finished (R1 and L1 change
    /// it).
    pub watching: usize,
    /// The slots whose pads we stopped taking, having reported what we hold of them.
    pub frozen: Vec<bool>,
    /// Where the pads of the players that left end (`Event::Left`), for `drive_tick` to apply:
    /// (slot, first tick, pads).
    pub lefts: Vec<(usize, u32, Vec<Pad>)>,
}

/// A player whose sim has parted from ours.
pub struct Desync {
    pub slot: usize,
    pub tick: u32,
    /// The hashes that differed, if it was they that told.
    pub hashes: Option<race::desync::Hashes>,
    /// Whether the player has been told of it (by a toast).
    pub told: bool,
}

impl Online {
    /// A line for the HUD: where the race is, and how many ticks were played on a prediction, and how often (and how far back) a
    /// wrong one had to be put right.
    pub fn summary(&self, tick: u32, predicted: u32) -> String {
        let wait = if !self.go { " (waiting)" } else { "" };
        let mut line = format!(
            "NET {tick} you p{}/{}{wait}  predicting {predicted}  rollbacks {} ({} ticks)",
            self.slot,
            self.characters.len(),
            self.corrections,
            self.replayed,
        );
        for d in &self.desync {
            line += &format!("  DESYNC with p{} at {}", d.slot, d.tick);
        }
        line
    }

    /// Notes the players whose hash differs from ours. A sim that has parted stays apart, so
    /// only the first is kept for each player.
    pub fn found(&mut self, mismatches: impl IntoIterator<Item = race::desync::Mismatch>) {
        for m in mismatches {
            if self.desync.iter().all(|d| d.slot != m.slot) {
                self.desync.push(Desync {
                    slot: m.slot,
                    tick: m.tick,
                    hashes: m.hashes,
                    told: false,
                });
            }
        }
    }
}

