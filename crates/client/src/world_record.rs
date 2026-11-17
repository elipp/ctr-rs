//! The boards' reads (TIME-TRIAL-RELIC-LEADERBOARDS.md, "Client" 3 and 5): the world record
//! ghost, the #1 of the solo race's board, and `/score top`. Anonymous, in the background; being
//! offline only means the cached #1, or none.
//!
//! The cache is per board, not per run id as the doc first had it: offline, the run id of the #1
//! is what we don't know.

use bevy::platform::time::Instant;
use bevy::prelude::*;
use race::solo::{Ghost, Kind, Run};
use score::{BoardId, Entry, Request, Response};

use crate::command::Action;
use crate::console::console;
use crate::race_on::{CurrentRace, Phase, RaceOn};
use crate::toast::Toast;

/// How long a board's answer holds before a start asks again: restarts come fast, and each ask
/// binds an endpoint and goes through the relay.
const FRESH_SECS: u64 = 60;

/// How many `/score top` lists.
const TOP: u32 = 10;

const RED: Color = Color::srgb(1.0, 0.4, 0.4);

/// A board's #1.
pub struct Record {
    pub entry: Entry,
    pub run: Run,
    /// As the board sent it, for the cache.
    bytes: Vec<u8>,
}

/// The #1 of the board of the last solo race. Kept here, not in `CurrentRace`: every restart is
/// a new `CurrentRace`.
#[derive(Resource, Default)]
pub struct WorldRecord {
    board: Option<BoardId>,
    /// Never ours: it would look the same as the best's ghost.
    record: Option<Record>,
    asking: Option<Asking>,
    asked: Option<Instant>,
    /// `record` is another run than the last one told (or than none), and not yet told.
    untold: bool,
}

enum Asking {
    Top(net::score::Answer),
    Run(Entry, net::score::Answer),
}

/// `/score top`s on their way: the track's name, the board.
#[derive(Resource, Default)]
struct Tops(Vec<(String, BoardId, net::score::Answer)>);

pub struct WorldRecordPlugin;

impl Plugin for WorldRecordPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WorldRecord>()
            .init_resource::<Tops>()
            .add_systems(Update, (answer, tell, top, listed));
    }
}

fn board_of(race: &CurrentRace) -> Option<BoardId> {
    Some(BoardId {
        kind: race.solo?,
        map_hash: race.hash,
        sim_version: race::SIM_VERSION,
        laps: race.rules.laps,
    })
}

fn is_ours(entry: &Entry) -> bool {
    crate::player::key().is_ok_and(|key| *key.public().as_bytes() == entry.key)
}

impl WorldRecord {
    /// At each start of a solo race (`drive_tick`, as it builds the best's ghost): the world
    /// record's ghost at tick 0 if one is known, and the board asked again unless it was lately.
    pub fn start(&mut self, level: &loader::lev::Level, sim: &crate::props::Sim, race: &CurrentRace) -> Option<Ghost> {
        let board = board_of(race)?;
        if self.board != Some(board) {
            let record = cache::load(board).filter(|r| !is_ours(&r.entry));
            *self = WorldRecord {
                board: Some(board),
                untold: record.is_some(),
                record,
                asking: None,
                asked: None,
            };
        }
        let stale = self.asked.is_none_or(|at| at.elapsed().as_secs() >= FRESH_SECS);
        if self.asking.is_none() && stale
            && let Some(server) = net::score::default_score()
        {
            self.asking = Some(Asking::Top(net::score::ask(server, Request::Top { board, n: 1 })));
            self.asked = Some(Instant::now());
        }
        crate::pb::ghost_of(level, sim, race, &self.record.as_ref()?.run)
    }

    /// The board's #1 is `record` now (None: there is none, or it is ours).
    fn set(&mut self, board: BoardId, record: Option<Record>) {
        let record = record.filter(|r| !is_ours(&r.entry));
        match &record {
            Some(record) => cache::save(board, record),
            None => cache::remove(board),
        }
        let was = self.record.as_ref().map(|r| r.entry.run_id);
        self.untold = record.is_some() && record.as_ref().map(|r| r.entry.run_id) != was;
        self.record = record;
    }
}

/// Takes the board's answers; a #1 that comes (or goes) mid-race is put in (or taken out of) the
/// race at once. A ghost is built at tick 0 and `ghost::step` fast-forwards it.
fn answer(
    mut world: ResMut<WorldRecord>,
    mut race_on: ResMut<RaceOn>,
    track: Res<crate::Track>,
    sim: Res<crate::props::Sim>,
) {
    let world = &mut *world;
    let (Some(board), Some(asking)) = (world.board, world.asking.as_ref()) else {
        return;
    };
    let answer = match asking {
        Asking::Top(answer) | Asking::Run(_, answer) => answer,
    };
    let Some(response) = answer.poll() else {
        return;
    };
    let response = response.map_err(|e| e.to_string()).and_then(|r| r);
    let record = match (world.asking.take(), response) {
        (Some(Asking::Top(_)), Ok(Response::Top(top))) => match top.into_iter().next() {
            Some(entry) if world.record.as_ref().is_some_and(|r| r.entry.run_id == entry.run_id) => {
                // The same run: only the nick (and the suspect mark) may have changed.
                if let Some(record) = world.record.as_mut() {
                    record.entry = entry;
                    cache::save(board, record);
                }
                return;
            }
            Some(entry) if !is_ours(&entry) => {
                let Some(server) = net::score::default_score() else { return };
                let id = entry.run_id;
                world.asking = Some(Asking::Run(entry, net::score::ask(server, Request::Run { id })));
                return;
            }
            _ => None,
        },
        (Some(Asking::Run(entry, _)), Ok(Response::Run(Some(bytes)))) => match Run::from_bytes(&bytes) {
            Ok(run)
                if run.kind == board.kind
                    && run.map_hash == board.map_hash
                    && run.laps == board.laps
                    && run.sim_version == board.sim_version =>
            {
                Some(Record { entry, run, bytes })
            }
            Ok(_) => {
                warn!("world record: run {} is of another board", entry.run_id);
                return;
            }
            Err(e) => {
                warn!("world record: run {}: {e}", entry.run_id);
                return;
            }
        },
        // Deleted between the two asks: the next start asks again.
        (Some(Asking::Run(..)), Ok(Response::Run(None))) => {
            world.asked = None;
            return;
        }
        (_, Ok(other)) => {
            warn!("world record: the score server answered {other:?}");
            return;
        }
        (_, Err(e)) => {
            info!("world record: {e}");
            return;
        }
    };
    world.set(board, record);
    let Some(race) = race_on.get_mut().filter(|race| board_of(race) == Some(board)) else {
        return;
    };
    // Before `OnGrid` the sim isn't on the race's grid yet: `drive_tick`'s start builds it.
    if race.phase >= Phase::OnGrid {
        let ghost = world.record.as_ref().and_then(|r| crate::pb::ghost_of(&track.0, &sim, race, &r.run));
        race.ghosts.world = ghost;
    }
}

/// Says who holds the world record, once its ghost is in the race.
fn tell(mut world: ResMut<WorldRecord>, race_on: Res<RaceOn>, mut toasts: MessageWriter<Toast>) {
    if !world.untold {
        return;
    }
    let Some(race) = race_on.get().filter(|race| board_of(race) == world.board) else {
        return;
    };
    let Some(record) = world.record.as_ref().filter(|_| race.ghosts.world.is_some()) else {
        return;
    };
    let entry = &record.entry;
    toasts.write(Toast::new(
        format!("World record {} by {}, {}", crate::laps::clock(entry.final_ms), who(entry), date(entry.at)),
        4000,
    ));
    world.untold = false;
}

/// A board's player as shown: the nick and the key's fingerprint, which tells apart two of the
/// same nick.
fn who(entry: &Entry) -> String {
    let fingerprint = score::PublicKey::from_bytes(&entry.key)
        .map_or_else(|_| "?".to_string(), |key| score::fingerprint(&key));
    match entry.nick.as_str() {
        "" => fingerprint,
        nick => format!("{nick} ({fingerprint})"),
    }
}

/// In UTC: the client has no time zone database (`Cargo.toml`).
fn date(at: i64) -> String {
    jiff::Timestamp::from_second(at).map_or_else(|_| "?".to_string(), |t| t.strftime("%Y-%m-%d").to_string())
}

fn top(
    mut actions: MessageReader<Action>,
    mut tops: ResMut<Tops>,
    level: Res<crate::resources::LevelNumber>,
    mut toasts: MessageWriter<Toast>,
) {
    for action in actions.read() {
        let Action::ScoreTop { kind, track } = action else {
            continue;
        };
        let id = track.unwrap_or(level.0);
        let asked = || -> Result<_, String> {
            let server = net::score::default_score().ok_or("No score server is set (CTR_SCORE)")?;
            let map_hash = crate::maps::hash(id).ok_or("No map of this track")?;
            let board = BoardId {
                kind: *kind,
                map_hash,
                sim_version: race::SIM_VERSION,
                laps: race::DEFAULT_LAPS,
            };
            Ok((board, net::score::ask(server, Request::Top { board, n: TOP })))
        };
        match asked() {
            Ok((board, answer)) => tops.0.push((crate::maps::stable_name(id), board, answer)),
            Err(why) => {
                toasts.write(Toast::new(why, 3000).color(RED));
            }
        }
    }
}

fn listed(mut tops: ResMut<Tops>, mut toasts: MessageWriter<Toast>) {
    let mut i = 0;
    while i < tops.0.len() {
        let Some(response) = tops.0[i].2.poll() else {
            i += 1;
            continue;
        };
        let (name, board, _) = tops.0.remove(i);
        let kind = match board.kind {
            Kind::Relic => "Relic race",
            Kind::TimeTrial => "Time trial",
        };
        match response.map_err(|e| e.to_string()).and_then(|r| r) {
            Ok(Response::Top(top)) if top.is_empty() => console!("{kind}, {name}: nobody on the board yet"),
            Ok(Response::Top(top)) => {
                console!("{kind}, {name}, {} laps:", board.laps);
                for entry in &top {
                    let mut line = format!(
                        "{:>3}. {}  {}  {}",
                        entry.rank,
                        crate::laps::clock(entry.final_ms),
                        who(entry),
                        date(entry.at)
                    );
                    if entry.perfect {
                        line += "  perfect";
                    }
                    if entry.suspect {
                        line += "  suspect";
                    }
                    if is_ours(entry) {
                        line += "  (you)";
                    }
                    console!("{line}");
                }
            }
            Ok(other) => {
                warn!("the score server answered a top with {other:?}");
                toasts.write(Toast::new("The board answered nonsense", 3000).color(RED));
            }
            Err(e) => {
                toasts.write(Toast::new(format!("Could not reach the board: {e}"), 5000).color(RED));
            }
        }
    }
}

/// The boards' #1s in `loader::store`, so the world record's ghost works offline once fetched.
/// A file is the `Entry` and the run's bytes, bitcode.
mod cache {
    use super::Record;
    use score::BoardId;

    fn path(board: BoardId) -> std::path::PathBuf {
        let kind = match board.kind {
            race::solo::Kind::Relic => "relic",
            race::solo::Kind::TimeTrial => "trial",
        };
        crate::paths::world_records().join(format!(
            "{:016x}-{}laps-{kind}-sim{}.wr",
            board.map_hash, board.laps, board.sim_version
        ))
    }

    pub fn load(board: BoardId) -> Option<Record> {
        let bytes = loader::store::read(&path(board)).ok()?;
        let (entry, run_bytes): (score::Entry, Vec<u8>) = bitcode::decode(&bytes).ok()?;
        let run = race::solo::Run::from_bytes(&run_bytes).ok()?;
        Some(Record { entry, run, bytes: run_bytes })
    }

    pub fn save(board: BoardId, record: &Record) {
        let bytes = bitcode::encode(&(record.entry.clone(), record.bytes.clone()));
        if let Err(e) = loader::store::write(&path(board), &bytes) {
            bevy::log::warn!("world record cache: {e}");
        }
    }

    pub fn remove(board: BoardId) {
        let _ = loader::store::remove_file(&path(board));
    }
}
