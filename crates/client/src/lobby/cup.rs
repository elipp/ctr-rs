//! Cups: a list of tracks the lobby drives one after another, and the points that add up over them
//! (`/cup create`, `/cup start`). A single `/race start` is the same thing with one track and
//! no points.
//!
//! The host runs the series: it starts each race, scores it when it is over and starts the next
//! after [`BETWEEN_RACES_SECONDS`]. The guests only see the races and what the host announces, so
//! nothing of a series travels in the protocol but the laps of each race.

use bevy::prelude::*;
use net::EndpointId;
use std::collections::HashMap;

/// The longest name a cup can have.
pub const MAX_NAME: usize = 24;

/// The most tracks in a cup.
pub const MAX_TRACKS: usize = 32;

/// The most laps a race of a cup can have.
pub const MAX_LAPS: u8 = 20;

/// How many places score in a cup, and the most that a race waits for (`UI_CupStandings`: the
/// first four of the race get points).
pub const SCORED_PLACES: usize = 4;

/// The pause between the end of a race of a cup and the start of the next.
pub const BETWEEN_RACES_SECONDS: u32 = 10;

/// A cup as `/cup create` made it.
#[derive(Clone, Debug, PartialEq)]
pub struct CupDef {
    pub name: String,
    pub tracks: Vec<u32>,
    pub laps: u8,
}

/// The cups made in this session.
#[derive(Resource, Default)]
pub struct Cups(Vec<CupDef>);

impl Cups {
    /// Adds the cup, or replaces the one with its name. Returns whether it replaced one.
    pub fn set(&mut self, cup: CupDef) -> bool {
        match self.0.iter_mut().find(|c| c.name == cup.name) {
            Some(old) => {
                *old = cup;
                true
            }
            None => {
                self.0.push(cup);
                false
            }
        }
    }

    pub fn get(&self, name: &str) -> Option<&CupDef> {
        self.0.iter().find(|c| c.name == name)
    }
}

/// How many players the race waits for: those who would score, among the ones still in it.
pub fn places_waited_for(present: usize) -> usize {
    present.min(SCORED_PLACES)
}

/// The points for coming `place`th (from 0) among `players`: the multiplayer rule of
/// `UI_CupStandings_InputAndDraw`, with no bots to add to the field. The first of four gets 3,
/// and so on down; past the fourth place there are none.
pub fn points(players: usize, place: usize) -> u32 {
    if place < places_waited_for(players) {
        (players - 1 - place) as u32
    } else {
        0
    }
}

/// A player of a race in the order they finished, and what they are called.
#[derive(Clone)]
pub struct Finisher {
    pub id: EndpointId,
    pub name: String,
}

/// The races the host is running: a cup, or the one race of `/race start`.
pub struct Series {
    /// The cup's name; none for a single race.
    pub name: Option<String>,
    pub tracks: Vec<u32>,
    pub laps: u8,
    /// The race being run, in `tracks`.
    pub race: usize,
    /// Whether the races give points (a cup's do).
    pub scored: bool,
    /// The points of every player so far, in the order they were first scored.
    points: Vec<(EndpointId, u32)>,
    names: HashMap<EndpointId, String>,
}

impl Series {
    /// One race, no points.
    pub fn single(track: u32, laps: u8) -> Series {
        Series {
            name: None,
            tracks: vec![track],
            laps,
            race: 0,
            scored: false,
            points: Vec::new(),
            names: HashMap::new(),
        }
    }

    pub fn cup(def: &CupDef) -> Series {
        Series {
            name: Some(def.name.clone()),
            tracks: def.tracks.clone(),
            laps: def.laps,
            race: 0,
            scored: true,
            points: Vec::new(),
            names: HashMap::new(),
        }
    }

    /// The track of the race being run.
    pub fn track(&self) -> u32 {
        self.tracks[self.race]
    }

    /// Whether a race follows the one being run.
    pub fn has_next(&self) -> bool {
        self.race + 1 < self.tracks.len()
    }

    /// Adds the points of a race in which `players` drove and `finishers` crossed the line, in
    /// that order. Returns what each of the first got.
    pub fn score(&mut self, players: usize, finishers: &[Finisher]) -> Vec<(String, u32)> {
        let mut gained = Vec::new();
        for (place, finisher) in finishers.iter().enumerate() {
            self.names.insert(finisher.id, finisher.name.clone());
            let won = points(players, place);
            match self.points.iter_mut().find(|(id, _)| *id == finisher.id) {
                Some((_, total)) => *total += won,
                None => self.points.push((finisher.id, won)),
            }
            if place < places_waited_for(players) {
                gained.push((finisher.name.clone(), won));
            }
        }
        gained
    }

    /// Everyone with points, the most first (the ones tied keep the order they were scored in).
    pub fn standings(&self) -> Vec<(String, u32)> {
        let mut standings: Vec<(String, u32)> = self
            .points
            .iter()
            .map(|(id, total)| (self.names.get(id).cloned().unwrap_or_default(), *total))
            .collect();
        standings.sort_by_key(|(_, total)| std::cmp::Reverse(*total));
        standings
    }
}

/// `1. Name +3` for each of `gained`.
pub fn results_text(gained: &[(String, u32)]) -> String {
    gained
        .iter()
        .enumerate()
        .map(|(place, (name, won))| format!("{}. {name} +{won}", place + 1))
        .collect::<Vec<_>>()
        .join("\n")
}

/// `Name 9` for each of `standings`, a line each.
pub fn standings_text(standings: &[(String, u32)]) -> String {
    standings
        .iter()
        .map(|(name, total)| format!("{name}: {total}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finishers(n: u8) -> Vec<Finisher> {
        (0..n)
            .map(|i| Finisher {
                id: net::SecretKey::from_bytes(&[i + 1; 32]).public(),
                name: format!("p{i}"),
            })
            .collect()
    }

    #[test]
    fn four_players_get_three_two_one_nothing() {
        assert_eq!(
            (0..4).map(|p| points(4, p)).collect::<Vec<_>>(),
            [3, 2, 1, 0]
        );
    }

    #[test]
    fn only_four_places_score_and_a_lone_driver_gets_nothing() {
        assert_eq!(
            (0..8).map(|p| points(8, p)).collect::<Vec<_>>(),
            [7, 6, 5, 4, 0, 0, 0, 0]
        );
        assert_eq!(points(1, 0), 0);
        assert_eq!(points(2, 0), 1);
    }

    #[test]
    fn a_race_waits_for_the_places_that_score() {
        assert_eq!(places_waited_for(1), 1);
        assert_eq!(places_waited_for(3), 3);
        assert_eq!(places_waited_for(8), 4);
    }

    #[test]
    fn points_add_up_over_the_races() {
        let mut series = Series::cup(&CupDef {
            name: "c".into(),
            tracks: vec![1, 2],
            laps: 3,
        });
        let mut order = finishers(3);
        series.score(3, &order);
        order.swap(0, 2);
        let gained = series.score(3, &order);
        assert_eq!(
            gained,
            [
                ("p2".to_string(), 2),
                ("p1".to_string(), 1),
                ("p0".to_string(), 0)
            ]
        );
        // 2+0, 1+1 and 0+2: all tied, in the order they were first scored.
        assert_eq!(
            series.standings(),
            [
                ("p0".to_string(), 2),
                ("p1".to_string(), 2),
                ("p2".to_string(), 2)
            ]
        );
        // A third race, won by p1, breaks the tie.
        series.score(3, &[order[1].clone(), order[0].clone(), order[2].clone()]);
        assert_eq!(series.standings()[0], ("p1".to_string(), 4));
    }

    #[test]
    fn a_cup_with_the_same_name_is_replaced() {
        let mut cups = Cups::default();
        let cup = |laps| CupDef {
            name: "x".into(),
            tracks: vec![1],
            laps,
        };
        assert!(!cups.set(cup(3)));
        assert!(cups.set(cup(5)));
        assert_eq!(cups.get("x").unwrap().laps, 5);
    }
}
