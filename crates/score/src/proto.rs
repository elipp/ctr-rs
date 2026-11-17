//! The leaderboard protocol, in bitcode over iroh. The server binds without IP transports (as the
//! realm does), so it is reached through the relay only and never learns a player's address; the
//! client connects from a random endpoint key of its own, so the relay never learns the player
//! key. The player key only shows inside a signed request.
//!
//! As with the realm, each request is a bidirectional stream of its own: the request is written
//! and the stream finished, and the response comes back on the same stream.

use bitcode::{Decode, Encode};
pub use race::solo::Kind;

pub const ALPN: &[u8] = b"ctr-rust/score/1";

/// The most bytes of a request: a run is a few KB, a long 7-lap one still fits with room.
pub const MAX_REQUEST: usize = 64 << 10;
/// The most bytes of a response: a run, or [`MAX_TOP`] entries.
pub const MAX_RESPONSE: usize = 256 << 10;
/// The most chars of a client version that are kept.
pub const MAX_VERSION: usize = 32;
/// The most entries of a [`Request::Top`].
pub const MAX_TOP: u32 = 100;

/// A board. The character is per run, not part of the board (retail's records are
/// character-agnostic too).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Encode, Decode)]
pub struct BoardId {
    pub kind: Kind,
    /// `Map::hash()`, relic LEV included.
    pub map_hash: u64,
    /// A bump starts fresh boards; old ones stay readable.
    pub sim_version: u32,
    pub laps: u8,
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub enum Request {
    Submit(Submit),
    /// Every board of `sim_version` with runs, with its #1.
    Boards { sim_version: u32 },
    /// The first `n` (at most [`MAX_TOP`]) of a board.
    Top { board: BoardId, n: u32 },
    /// A player's best on a board.
    Player { board: BoardId, key: [u8; 32] },
    /// A run's bytes, for a ghost.
    Run { id: i64 },
    /// Delete the signer's player row and every run of theirs. Signed over an empty body, for
    /// [`crate::Action::Forget`].
    Forget { auth: crate::Auth },
}

/// A run to rank.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub struct Submit {
    /// Over `run`, for [`crate::Action::Submit`].
    pub auth: crate::Auth,
    /// The player's nick from now on (`clean_nick`ed by the server), and the client's version.
    pub nick: String,
    pub version: String,
    /// The time the client's own sim gave the run: the server's replay must agree.
    pub claimed_ms: i32,
    /// `race::solo::Run::to_bytes`.
    pub run: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub enum Response {
    Submitted(Submitted),
    Rejected(Rejected),
    /// The signature didn't hold. `now` is the server's clock: a client whose is off retries
    /// once with the offset.
    Unauthorized { why: String, now: u64 },
    Boards(Vec<Board>),
    Top(Vec<Entry>),
    Player(Option<Entry>),
    Run(Option<Vec<u8>>),
    /// The signer is gone from the boards, with this many runs.
    Forgotten { runs: u32 },
    /// Too many submits from this key: try later.
    TooMany,
    /// No verifier came free in time: try again.
    Busy,
    /// The server's fault; it logged it.
    Failed,
}

/// A player's best on a board.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub struct Entry {
    /// From 1. Equal times go to the earlier submit.
    pub rank: u32,
    pub run_id: i64,
    pub key: [u8; 32],
    /// The player's latest nick, empty for none.
    pub nick: String,
    pub character: u8,
    pub final_ms: i32,
    pub perfect: bool,
    /// Unix seconds, as SQLite keeps them and `jiff::Timestamp::from_second` takes them.
    pub at: i64,
    /// Much like an earlier run of another player's, and as fast: maybe a copy of its ghost
    /// (TIME-TRIAL-RELIC-LEADERBOARDS.md, "Run theft"). Shown, not hidden: the operator decides.
    pub suspect: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub struct Board {
    pub board: BoardId,
    pub players: u32,
    pub top: Entry,
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub struct Submitted {
    pub run_id: i64,
    /// The submitter's rank on the board now, by their best (which may be an older run).
    pub rank: u32,
    pub pb_ms: i32,
    pub top_ms: i32,
}

/// The server didn't take the run. `NoFinish` and `Mismatch` mean the client's sim and the
/// server's differ, which is a bug.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub enum Rejected {
    BadFormat { why: String },
    WrongSimVersion { ours: u32 },
    UnknownMap,
    WrongSeed,
    NoFinish,
    Mismatch { ours_ms: i32 },
    Duplicate { of_run: i64 },
    Banned,
}

impl Rejected {
    /// For the `rejections` table: fixed once given.
    pub fn code(&self) -> u8 {
        match self {
            Rejected::BadFormat { .. } => 0,
            Rejected::WrongSimVersion { .. } => 1,
            Rejected::UnknownMap => 2,
            Rejected::WrongSeed => 3,
            Rejected::NoFinish => 4,
            Rejected::Mismatch { .. } => 5,
            Rejected::Duplicate { .. } => 6,
            Rejected::Banned => 7,
        }
    }

    /// The name of a [`Rejected::code`], for the operator's `stats`.
    pub fn code_name(code: u8) -> &'static str {
        match code {
            0 => "bad format",
            1 => "wrong sim version",
            2 => "unknown map",
            3 => "wrong seed",
            4 => "no finish",
            5 => "mismatch",
            6 => "duplicate",
            7 => "banned",
            _ => "?",
        }
    }
}

impl std::fmt::Display for Rejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Rejected::BadFormat { why } => write!(f, "not a valid run: {why}"),
            Rejected::WrongSimVersion { ours } => write!(f, "the server runs sim version {ours}"),
            Rejected::UnknownMap => f.write_str("the server doesn't have this track"),
            Rejected::WrongSeed => f.write_str("not the board's seed"),
            Rejected::NoFinish => f.write_str("the server's replay doesn't finish (a bug)"),
            Rejected::Mismatch { ours_ms } => {
                write!(f, "the server's replay times {ours_ms} ms (a bug)")
            }
            Rejected::Duplicate { of_run } => write!(f, "the same inputs as run {of_run}"),
            Rejected::Banned => f.write_str("this key is banned from the boards"),
        }
    }
}

impl Request {
    pub fn to_bytes(&self) -> Vec<u8> {
        bitcode::encode(self)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Request, bitcode::Error> {
        bitcode::decode(bytes)
    }
}

impl Response {
    pub fn to_bytes(&self) -> Vec<u8> {
        bitcode::encode(self)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Response, bitcode::Error> {
        bitcode::decode(bytes)
    }
}
