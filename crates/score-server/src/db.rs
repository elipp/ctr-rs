//! The SQLite store: players, every verified run, and rejections (telemetry, no run bytes).
//!
//! Integers SQLite can't hold unsigned (`map_hash`) are stored as their `i64` bit pattern.

use std::path::Path;

use race::solo::Kind;
use rusqlite::{Connection, OptionalExtension, params};
use score::{Board, BoardId, Entry, PublicKey};

const SCHEMA: &str = "
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS players (
  key            BLOB PRIMARY KEY,
  nick           TEXT NOT NULL,
  first_seen     INTEGER NOT NULL,
  last_seen      INTEGER NOT NULL,
  connections    INTEGER NOT NULL,
  client_version TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS runs (
  id           INTEGER PRIMARY KEY,
  player       BLOB NOT NULL REFERENCES players(key) ON DELETE CASCADE,
  kind         INTEGER NOT NULL,
  map_hash     INTEGER NOT NULL,
  sim_version  INTEGER NOT NULL,
  laps         INTEGER NOT NULL,
  character    INTEGER NOT NULL,
  final_ms     INTEGER NOT NULL,
  perfect      INTEGER NOT NULL,
  submitted_at INTEGER NOT NULL,
  pads_hash    BLOB NOT NULL,
  run          BLOB NOT NULL,
  signed_ts    INTEGER NOT NULL,
  signature    BLOB NOT NULL
);
CREATE INDEX IF NOT EXISTS runs_board
  ON runs (kind, map_hash, sim_version, laps, final_ms, submitted_at);
CREATE UNIQUE INDEX IF NOT EXISTS runs_pads
  ON runs (kind, map_hash, sim_version, laps, pads_hash);

CREATE TABLE IF NOT EXISTS rejections (
  player         BLOB NOT NULL REFERENCES players(key) ON DELETE CASCADE,
  at             INTEGER NOT NULL,
  reason         INTEGER NOT NULL,
  client_version TEXT NOT NULL
);
";

/// Each player's best run on the board `?1..?4` (the earliest of equal times), ranked. Ties
/// after the time go by `id`, which is submit order.
const BESTS: &str = "
WITH bests AS (
  SELECT * FROM (
    SELECT r.*, ROW_NUMBER() OVER (PARTITION BY player ORDER BY final_ms, id) AS nth
    FROM runs r WHERE kind = ?1 AND map_hash = ?2 AND sim_version = ?3 AND laps = ?4
  ) WHERE nth = 1
), ranked AS (
  SELECT ROW_NUMBER() OVER (ORDER BY b.final_ms, b.id) AS rank,
    b.id, b.player, p.nick, b.character, b.final_ms, b.perfect, b.submitted_at
  FROM bests b JOIN players p ON p.key = b.player
)";

/// Fixed once given: stored in `runs.kind`.
fn kind_code(kind: Kind) -> i64 {
    match kind {
        Kind::Relic => 0,
        Kind::TimeTrial => 1,
    }
}

fn kind_of_code(code: i64) -> Option<Kind> {
    [Kind::Relic, Kind::TimeTrial].into_iter().find(|&k| kind_code(k) == code)
}

pub struct NewRun<'a> {
    pub player: &'a PublicKey,
    pub board: BoardId,
    pub character: u8,
    pub final_ms: i32,
    pub perfect: bool,
    pub at: i64,
    pub pads_hash: &'a [u8; 32],
    pub run: &'a [u8],
    pub signed_ts: u64,
    pub signature: &'a [u8; 64],
}

pub struct Db(Connection);

impl Db {
    pub fn open(path: &Path) -> rusqlite::Result<Db> {
        let conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch(SCHEMA)?;
        Ok(Db(conn))
    }

    /// A signed request from `key`: the player is (re)labelled `nick`, and seen now.
    pub fn seen(&self, key: &PublicKey, nick: &str, version: &str, now: i64) -> rusqlite::Result<()> {
        self.0
            .execute(
                "INSERT INTO players (key, nick, first_seen, last_seen, connections, client_version)
                 VALUES (?1, ?2, ?3, ?3, 1, ?4)
                 ON CONFLICT (key) DO UPDATE SET nick = ?2, last_seen = ?3,
                   connections = connections + 1, client_version = ?4",
                params![key.as_bytes(), nick, now, version],
            )
            .map(drop)
    }

    pub fn rejected(&self, key: &PublicKey, reason: u8, version: &str, now: i64) -> rusqlite::Result<()> {
        self.0
            .execute(
                "INSERT INTO rejections (player, at, reason, client_version) VALUES (?1, ?2, ?3, ?4)",
                params![key.as_bytes(), now, reason, version],
            )
            .map(drop)
    }

    /// The run on `board` with these pads, if any.
    pub fn same_pads(&self, board: BoardId, pads_hash: &[u8; 32]) -> rusqlite::Result<Option<i64>> {
        let (k, m, s, l) = board_args(board);
        self.0
            .query_row(
                "SELECT id FROM runs WHERE kind = ?1 AND map_hash = ?2 AND sim_version = ?3
                   AND laps = ?4 AND pads_hash = ?5",
                params![k, m, s, l, pads_hash],
                |row| row.get(0),
            )
            .optional()
    }

    /// The new run's id, or Err(Ok(id)) of the run with the same pads (sent at the same moment,
    /// past [`Db::same_pads`]).
    pub fn insert(&self, run: &NewRun) -> rusqlite::Result<Result<i64, i64>> {
        let (k, m, s, l) = board_args(run.board);
        let inserted = self.0.execute(
            "INSERT INTO runs (player, kind, map_hash, sim_version, laps, character, final_ms,
               perfect, submitted_at, pads_hash, run, signed_ts, signature)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                run.player.as_bytes(),
                k,
                m,
                s,
                l,
                run.character,
                run.final_ms,
                run.perfect,
                run.at,
                run.pads_hash,
                run.run,
                run.signed_ts as i64,
                run.signature
            ],
        );
        match inserted {
            Ok(_) => Ok(Ok(self.0.last_insert_rowid())),
            Err(rusqlite::Error::SqliteFailure(e, _))
                if e.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE =>
            {
                let of = self.same_pads(run.board, run.pads_hash)?;
                Ok(Err(of.unwrap_or_default()))
            }
            Err(e) => Err(e),
        }
    }

    /// The first `n` of `board`.
    pub fn top(&self, board: BoardId, n: u32) -> rusqlite::Result<Vec<Entry>> {
        let mut stmt = self
            .0
            .prepare_cached(&format!("{BESTS} SELECT * FROM ranked ORDER BY rank LIMIT ?5"))?;
        let (k, m, s, l) = board_args(board);
        let rows = stmt.query_map(params![k, m, s, l, n], entry)?;
        rows.collect()
    }

    /// `key`'s best on `board`, ranked.
    pub fn player_best(&self, board: BoardId, key: &PublicKey) -> rusqlite::Result<Option<Entry>> {
        let mut stmt =
            self.0.prepare_cached(&format!("{BESTS} SELECT * FROM ranked WHERE player = ?5"))?;
        let (k, m, s, l) = board_args(board);
        stmt.query_row(params![k, m, s, l, key.as_bytes()], entry).optional()
    }

    /// Every board of `sim_version` with runs, with its #1.
    pub fn boards(&self, sim_version: u32) -> rusqlite::Result<Vec<Board>> {
        let mut stmt = self.0.prepare_cached(
            "SELECT DISTINCT kind, map_hash, laps FROM runs WHERE sim_version = ?1
             ORDER BY map_hash, kind, laps",
        )?;
        let keys = stmt
            .query_map([sim_version], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, u8>(2)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut boards = Vec::new();
        for (kind, map_hash, laps) in keys {
            let Some(kind) = kind_of_code(kind) else { continue };
            let board = BoardId { kind, map_hash: map_hash as u64, sim_version, laps };
            let (k, m, s, l) = board_args(board);
            let players: u32 = self.0.query_row(
                "SELECT COUNT(DISTINCT player) FROM runs WHERE kind = ?1 AND map_hash = ?2
                   AND sim_version = ?3 AND laps = ?4",
                params![k, m, s, l],
                |row| row.get(0),
            )?;
            if let Some(top) = self.top(board, 1)?.into_iter().next() {
                boards.push(Board { board, players, top });
            }
        }
        Ok(boards)
    }

    pub fn run_bytes(&self, id: i64) -> rusqlite::Result<Option<Vec<u8>>> {
        self.0
            .query_row("SELECT run FROM runs WHERE id = ?1", [id], |row| row.get(0))
            .optional()
    }
}

/// `?1..?4` of a board query.
fn board_args(b: BoardId) -> (i64, i64, u32, u8) {
    (kind_code(b.kind), b.map_hash as i64, b.sim_version, b.laps)
}

/// A row of `ranked`.
fn entry(row: &rusqlite::Row) -> rusqlite::Result<Entry> {
    Ok(Entry {
        rank: row.get("rank")?,
        run_id: row.get("id")?,
        key: row.get("player")?,
        nick: row.get("nick")?,
        character: row.get("character")?,
        final_ms: row.get("final_ms")?,
        perfect: row.get("perfect")?,
        at: row.get("submitted_at")?,
    })
}
