//! Cups: a list of tracks the lobby drives one after another, and the points that add up over them
//! (`/cup create`, `/cup start`, `/cup delete`). The cups are kept in the config, but for
//! `/cup random`'s, of the game's racing tracks drawn at random. A single `/race start` is the
//! same thing with one track and no points.
//!
//! The host runs the series: it starts each race, scores it when it is over and starts the next
//! after [`BETWEEN_RACES_SECONDS`]. `/cup pause` pauses the race for everyone, or the countdown
//! to the next race, until `/cup resume`. Of a series, the guests only get the races and, after
//! each race of a cup, where it stands ([`net::Standings`]), which the scoreboard shows
//! (`scoreboard.rs`).
//!
//! Bots race for points too, and with them in the race the points are retail's of a cup with
//! bots (`cupPointsPerPosition`: 9, 6, 3 and 1).

use net::{EndpointId, Score};
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
pub const BETWEEN_RACES_SECONDS: u32 = 5;

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

/// `count` of the game's racing tracks, none twice, in an order `seed` shuffles them to.
pub fn random_tracks(count: usize, mut seed: u64) -> Vec<u32> {
    let mut tracks: Vec<u32> = (0..race::RACING_TRACKS).collect();
    shuffle(&mut tracks, &mut seed);
    tracks.truncate(count);
    tracks
}

/// Fisher-Yates, on [`splitmix`].
pub(crate) fn shuffle<T>(items: &mut [T], seed: &mut u64) {
    for i in (1..items.len()).rev() {
        items.swap(i, (splitmix(seed) % (i as u64 + 1)) as usize);
    }
}

/// splitmix64: the next number of the sequence `seed` is at.
pub(crate) fn splitmix(seed: &mut u64) -> u64 {
    *seed = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *seed;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// A seed for [`random_tracks`], new each call. (The time `now` is mixed in for the web, whose
/// `RandomState` is not random.)
pub fn random_seed(now: f32) -> u64 {
    std::hash::BuildHasher::hash_one(
        &std::collections::hash_map::RandomState::new(),
        now.to_bits(),
    )
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

/// `data.cupPointsPerPosition`: the points of the places of a race with bots in it.
const POINTS_WITH_BOTS: [u32; 4] = [9, 6, 3, 1];

/// The points for coming `place`th (from 0) among `players`: the multiplayer rule of
/// `UI_CupStandings_InputAndDraw` when no bots race. The first of four gets 3, and so on down;
/// past the fourth place there are none.
pub fn points(players: usize, place: usize) -> u32 {
    if place < places_waited_for(players) {
        (players - 1 - place) as u32
    } else {
        0
    }
}

/// Who crossed the line: a player, by its machine and its seat there (0 for the first, 1 for
/// "name@2" of a split screen, ...), or a bot, by its place among the series' bots
/// (`Series::brains` / `retail.characters`, fixed for the cup). Not by character: with
/// `BotPick`'s repeats two bots can drive the same one, and would share their points.
/// Only `Score::character` tells a guest the character; the id's last byte is just a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Who {
    Player(EndpointId, u8),
    Bot(u8),
}

impl Who {
    /// What [`net::Score`] says it is: a player's key, or a made-up one for a bot.
    pub(crate) fn id(&self) -> [u8; 32] {
        match self {
            Who::Player(id, _) => *id.as_bytes(),
            Who::Bot(bot) => {
                let mut id = [BOT_ID; 32];
                id[31] = *bot;
                id
            }
        }
    }

    /// What [`net::Score::seat`] says it is.
    pub(crate) fn seat(&self) -> u8 {
        match self {
            Who::Player(_, seat) => *seat,
            Who::Bot(_) => 0,
        }
    }

    /// Whether `score` is this driver's.
    pub(crate) fn matches(&self, score: &Score) -> bool {
        score.id == self.id() && score.seat == self.seat()
    }

    /// Whether a [`net::Score`]'s id is a bot's.
    pub(crate) fn is_bot_id(id: &[u8; 32]) -> bool {
        id[..31].iter().all(|&b| b == BOT_ID)
    }
}

/// What a bot's made-up id is filled with.
const BOT_ID: u8 = 0xb0;

/// A kart of a race in the order they finished, what it is called and the character it drives.
#[derive(Clone)]
pub struct Finisher {
    pub who: Who,
    pub name: String,
    pub character: u8,
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
    /// The bots of the races it starts: the host's brain bots (of these characters), or retail
    /// ones. One kind or none. `/bots` changes them for the next race.
    pub brains: Vec<u8>,
    pub retail: net::RetailBots,
    /// `/cup pause` between races: the next one waits for `/cup resume`.
    pub paused: bool,
    /// The points of every player and bot so far, in the order they were first scored.
    points: Vec<(Who, u32)>,
    /// How each was last scored: its name and character.
    drivers: HashMap<Who, Finisher>,
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
            brains: Vec::new(),
            retail: net::RetailBots::default(),
            paused: false,
            points: Vec::new(),
            drivers: HashMap::new(),
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
            brains: Vec::new(),
            retail: net::RetailBots::default(),
            paused: false,
            points: Vec::new(),
            drivers: HashMap::new(),
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

    /// Whether its races have bots, of either kind.
    pub fn has_bots(&self) -> bool {
        !self.brains.is_empty() || !self.retail.characters.is_empty()
    }

    /// Adds the points of a race in which `players` drove and `finishers` crossed the line, in
    /// that order (with bots, every kart of the race is one, in the order of the race). Returns
    /// what each of the first got.
    pub fn score(&mut self, players: usize, finishers: &[Finisher]) -> Vec<Score> {
        let bots = self.has_bots();
        let shown = if bots {
            POINTS_WITH_BOTS.len()
        } else {
            places_waited_for(players)
        };
        let mut gained = Vec::new();
        for (place, finisher) in finishers.iter().enumerate() {
            self.drivers.insert(finisher.who, finisher.clone());
            let won = if bots {
                POINTS_WITH_BOTS.get(place).copied().unwrap_or(0)
            } else {
                points(players, place)
            };
            match self.points.iter_mut().find(|(who, _)| *who == finisher.who) {
                Some((_, total)) => *total += won,
                None => self.points.push((finisher.who, won)),
            }
            if place < shown {
                gained.push(Score {
                    id: finisher.who.id(),
                    seat: finisher.who.seat(),
                    name: finisher.name.clone(),
                    character: finisher.character,
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
            .map(|(who, total)| {
                let driver = self.drivers.get(who);
                Score {
                    id: who.id(),
                    seat: who.seat(),
                    name: driver.map(|d| d.name.clone()).unwrap_or_default(),
                    character: driver.map_or(0, |d| d.character),
                    points: *total,
                }
            })
            .collect();
        totals.sort_by_key(|score| std::cmp::Reverse(score.points));
        totals
    }

    /// The standings of a cup the host stopped, if anyone has points yet.
    pub fn final_standings(&self) -> Option<net::Standings> {
        if !self.scored || self.points.is_empty() {
            return None;
        }
        Some(net::Standings {
            cup: self.name.clone().unwrap_or_else(|| "Cup".into()),
            race: self.race as u8,
            races: self.tracks.len() as u8,
            gained: Vec::new(),
            totals: self.totals(),
            next: None,
            stopped: true,
        })
    }
}

/// Who stands on the podium of a cup that ended: the three with the most points, the most first
/// (`UI_CupStandings` ranks the cup's drivers so, and `Podium_InitModels` takes the top three).
/// None for a cup that goes on, one the host stopped, or one where no one scored.
pub fn podium(standings: &net::Standings) -> Option<[Option<u8>; 3]> {
    if standings.stopped || standings.next.is_some() || standings.totals.is_empty() {
        return None;
    }
    let mut top = standings.totals.iter().map(|score| score.character);
    Some([top.next(), top.next(), top.next()])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finishers(n: u8) -> Vec<Finisher> {
        (0..n)
            .map(|i| Finisher {
                who: Who::Player(net::SecretKey::from_bytes(&[i + 1; 32]).public(), 0),
                name: format!("p{i}"),
                character: i,
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
    fn the_top_three_of_a_finished_cup_go_on_the_podium() {
        let def = CupDef {
            name: "c".into(),
            tracks: vec!["1".into()],
            laps: 3,
        };
        let mut series = Series::cup(&def, vec![1]);
        let mut order = finishers(4);
        order.reverse();
        series.score(4, &order);
        let mut standings = series.final_standings().unwrap();
        // Stopped: no podium.
        assert_eq!(podium(&standings), None);
        standings.stopped = false;
        assert_eq!(podium(&standings), Some([Some(3), Some(2), Some(1)]));
        // Another race follows: not yet.
        standings.next = Some((2, "two".into()));
        assert_eq!(podium(&standings), None);
        // Two scored: the third step is empty.
        standings.next = None;
        standings.totals.truncate(2);
        assert_eq!(podium(&standings), Some([Some(3), Some(2), None]));
    }

    #[test]
    fn a_stopped_cup_has_final_standings_once_anyone_has_points() {
        let def = CupDef {
            name: "c".into(),
            tracks: vec!["1".into(), "2".into()],
            laps: 3,
        };
        let mut series = Series::cup(&def, vec![1, 2]);
        assert!(series.final_standings().is_none());
        series.score(3, &finishers(3));
        let standings = series.final_standings().unwrap();
        assert!(standings.stopped && standings.next.is_none() && standings.gained.is_empty());
        assert_eq!(standings.totals[0].name, "p0");
        assert_eq!(standings.totals[0].character, 0);
        assert!(Series::single(1, 3).final_standings().is_none());
    }

    #[test]
    fn a_split_screens_players_keep_their_own_points() {
        let key = net::SecretKey::from_bytes(&[9; 32]).public();
        let seat = |n: u8| Finisher {
            who: Who::Player(key, n),
            name: format!("p@{n}"),
            character: n,
        };
        let mut series = Series::single(0, 3);
        series.score(2, &[seat(0), seat(1)]);
        let totals = series.totals();
        assert_eq!(totals.len(), 2);
        assert_eq!((totals[0].seat, totals[0].points), (0, 1));
        assert_eq!((totals[1].seat, totals[1].points), (1, 0));
        assert!(Who::Player(key, 1).matches(&totals[1]));
        assert!(!Who::Player(key, 0).matches(&totals[1]));
    }

    #[test]
    fn with_bots_the_first_four_get_retails_points() {
        let mut series = Series::single(0, 3);
        series.retail.characters = vec![3, 4, 5];
        let mut order = finishers(1);
        for (bot, character) in [3, 4, 5].into_iter().enumerate() {
            order.insert(
                0,
                Finisher {
                    who: Who::Bot(bot as u8),
                    name: format!("bot{character}"),
                    character,
                },
            );
        }
        let gained: Vec<u32> = series.score(1, &order).iter().map(|s| s.points).collect();
        assert_eq!(gained, [9, 6, 3, 1]);
        // Each bot keeps its points under its own made-up key.
        let totals = series.totals();
        assert_eq!(totals[0].name, "bot5");
        assert_ne!(totals[0].id, totals[1].id);
    }

    #[test]
    fn a_random_cup_has_no_track_twice() {
        for seed in 0..50 {
            let mut tracks = random_tracks(race::RACING_TRACKS as usize, seed);
            tracks.sort();
            assert_eq!(tracks, (0..race::RACING_TRACKS).collect::<Vec<_>>());
        }
        assert_eq!(random_tracks(5, 1).len(), 5);
        assert_ne!(random_tracks(5, 1), random_tracks(5, 2));
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
