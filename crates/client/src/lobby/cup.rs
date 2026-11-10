//! Cups: a list of tracks the lobby drives one after another, and the points that add up over them
//! (`/cup create`, `/cup start`, `/cup delete`). The cups are kept in the config. A single
//! `/race start` is the same thing with one track and no points.
//!
//! The host runs the series: it starts each race, scores it when it is over and starts the next
//! after [`BETWEEN_RACES_SECONDS`]. Of a series, the guests only get the races and, after each
//! race of a cup, where it stands ([`net::Standings`]), which every client shows its own way
//! ([`standings_text`]).

use net::Score;
use serde::{Deserialize, Serialize};
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

/// Whether a cup can be called that: `/cup start` reads its name as one word.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= MAX_NAME
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
}

/// A cup as `/cup create` made it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CupDef {
    pub name: String,
    /// What a command calls each track (`maps::stable_name`), found when the cup starts: a custom
    /// track may have gone by then, and the config is read before the tracks are.
    pub tracks: Vec<String>,
    pub laps: u8,
}

impl CupDef {
    /// Whether `/cup create` could have made it (the config can be edited by hand).
    fn is_valid(&self) -> bool {
        valid_name(&self.name)
            && (1..=MAX_TRACKS).contains(&self.tracks.len())
            && (1..=MAX_LAPS).contains(&self.laps)
    }

    /// The tracks as they are now, or what is wrong with one.
    pub fn resolve(&self) -> Result<Vec<u32>, String> {
        self.tracks
            .iter()
            .map(|track| crate::maps::resolve(track))
            .collect()
    }
}

/// The cups made.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(transparent)]
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

    /// The cup of that name, or else of that name in another case: the command line completes a
    /// name in the case it was begun in.
    pub fn get(&self, name: &str) -> Option<&CupDef> {
        let cup = self.0.iter().find(|c| c.name == name);
        cup.or_else(|| self.0.iter().find(|c| c.name.eq_ignore_ascii_case(name)))
    }

    /// In the order they were first made.
    pub fn all(&self) -> &[CupDef] {
        &self.0
    }

    /// Takes out the cup [`get`](Self::get) finds by that name.
    pub fn remove(&mut self, name: &str) -> Option<CupDef> {
        let name = self.get(name)?.name.clone();
        let at = self.0.iter().position(|c| c.name == name)?;
        Some(self.0.remove(at))
    }

    /// Takes out those `/cup create` could not have made, and returns them.
    pub fn take_invalid(&mut self) -> Vec<CupDef> {
        let (valid, invalid) = std::mem::take(&mut self.0)
            .into_iter()
            .partition(CupDef::is_valid);
        self.0 = valid;
        invalid
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

/// A player of a race in the order they finished, and what they are called. The id is the
/// player's endpoint id, or a bot's (`Online::score_id`).
#[derive(Clone)]
pub struct Finisher {
    pub id: [u8; 32],
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
    points: Vec<([u8; 32], u32)>,
    names: HashMap<[u8; 32], String>,
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

    /// The cup, on the tracks it resolved to.
    pub fn cup(def: &CupDef, tracks: Vec<u32>) -> Series {
        Series {
            name: Some(def.name.clone()),
            tracks,
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
    pub fn score(&mut self, players: usize, finishers: &[Finisher]) -> Vec<Score> {
        let mut gained = Vec::new();
        for (place, finisher) in finishers.iter().enumerate() {
            self.names.insert(finisher.id, finisher.name.clone());
            let won = points(players, place);
            match self.points.iter_mut().find(|(id, _)| *id == finisher.id) {
                Some((_, total)) => *total += won,
                None => self.points.push((finisher.id, won)),
            }
            if place < places_waited_for(players) {
                gained.push(Score {
                    id: finisher.id,
                    name: finisher.name.clone(),
                    points: won,
                });
            }
        }
        gained
    }

    /// Everyone with points, the most first (the ones tied keep the order they were scored in).
    pub fn totals(&self) -> Vec<Score> {
        let mut totals: Vec<Score> = self
            .points
            .iter()
            .map(|(id, total)| Score {
                id: *id,
                name: self.names.get(id).cloned().unwrap_or_default(),
                points: *total,
            })
            .collect();
        totals.sort_by_key(|score| std::cmp::Reverse(score.points));
        totals
    }
}

/// The standings as a toast shows them: the race's results, then the totals and what comes next.
pub fn standings_text(standings: &net::Standings) -> String {
    let gained: Vec<String> = (standings.gained.iter().enumerate())
        .map(|(place, s)| format!("{}. {} +{}", place + 1, s.name, s.points))
        .collect();
    let gained = gained.join("\n");
    let totals: Vec<String> = (standings.totals.iter())
        .map(|s| format!("{}: {}", s.name, s.points))
        .collect();
    let totals = totals.join("\n");
    let mut text = format!(
        "{}: race {} of {}\n\n{gained}",
        standings.cup,
        standings.race as usize + 1,
        standings.races
    );
    match &standings.next {
        Some((_, track)) => text += &format!(
            "\n\nStandings\n{totals}\n\nNext: {track} in {BETWEEN_RACES_SECONDS} seconds"
        ),
        None => text += &format!("\n\nFinal standings\n{totals}"),
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finishers(n: u8) -> Vec<Finisher> {
        (0..n)
            .map(|i| Finisher {
                id: [i + 1; 32],
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
        let mut series = Series::cup(
            &CupDef {
                name: "c".into(),
                tracks: vec!["1".into(), "2".into()],
                laps: 3,
            },
            vec![1, 2],
        );
        let named = |scores: Vec<Score>| -> Vec<(String, u32)> {
            scores.into_iter().map(|s| (s.name, s.points)).collect()
        };
        let mut order = finishers(3);
        series.score(3, &order);
        order.swap(0, 2);
        let gained = named(series.score(3, &order));
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
            named(series.totals()),
            [
                ("p0".to_string(), 2),
                ("p1".to_string(), 2),
                ("p2".to_string(), 2)
            ]
        );
        // A third race, won by p1, breaks the tie.
        series.score(3, &[order[1].clone(), order[0].clone(), order[2].clone()]);
        assert_eq!(named(series.totals())[0], ("p1".to_string(), 4));
    }

    #[test]
    fn a_cup_with_the_same_name_is_replaced() {
        let mut cups = Cups::default();
        let cup = |laps| CupDef {
            name: "x".into(),
            tracks: vec!["dingo_canyon".into()],
            laps,
        };
        assert!(!cups.set(cup(3)));
        assert!(cups.set(cup(5)));
        assert_eq!(cups.get("x").unwrap().laps, 5);
        assert_eq!(cups.get("X").unwrap().laps, 5);
        assert!(cups.get("y").is_none());
        assert!(cups.remove("y").is_none());
        assert_eq!(cups.remove("X").unwrap().name, "x");
        assert!(cups.all().is_empty());
    }

    #[test]
    fn a_cups_tracks_are_found_when_it_starts() {
        let cup = |tracks: &[&str]| CupDef {
            name: "c".into(),
            tracks: tracks.iter().map(|t| t.to_string()).collect(),
            laps: 3,
        };
        assert_eq!(cup(&["dingo_canyon", "papu_pyramid"]).resolve(), Ok(vec![0, 5]));
        let missing = cup(&["dingo_canyon", "gone_track"]).resolve().unwrap_err();
        assert!(missing.contains("gone_track"), "{missing}");
    }

    /// The config can be edited by hand.
    #[test]
    fn what_could_not_have_been_made_is_taken_out() {
        let text = r#"[
            {"name": "good", "tracks": ["dingo_canyon"], "laps": 3},
            {"name": "two words", "tracks": ["dingo_canyon"], "laps": 3},
            {"name": "", "tracks": ["dingo_canyon"], "laps": 3},
            {"name": "no_tracks", "tracks": [], "laps": 3},
            {"name": "no_laps", "tracks": ["dingo_canyon"], "laps": 0},
            {"name": "many_laps", "tracks": ["dingo_canyon"], "laps": 21}
        ]"#;
        let mut cups: Cups = serde_json::from_str(text).unwrap();
        assert_eq!(cups.take_invalid().len(), 5);
        assert_eq!(cups.all().len(), 1);
        assert_eq!(cups.all()[0].name, "good");
    }
}
