//! The matchmaking protocol (MATCHMAKING.md), on an ALPN of its own beside the boards'. Unlike
//! theirs, a client keeps one bidirectional stream open for as long as it queues: the queue
//! membership is that stream's life. Both ways it is [`frame`]s: a little-endian `u32` length,
//! then that many bytes of bitcode.
//!
//! The client queues from the endpoint of the session it will race on (a random key per session,
//! never the player key), so that the match can name it to the others. The server learns only
//! that id, its home relay and what the endpoint's net report says of its NAT, never an address.

use bitcode::{Decode, Encode};

pub const MATCH_ALPN: &[u8] = b"ctr-rust/match/2";

/// The most bytes of a frame either way: a [`Match`] of 8 is well under 2 KB.
pub const MAX_FRAME: usize = 16 << 10;

/// The players of a match, at most: `net::MAX_PLAYERS`.
pub const MAX_MATCH: usize = 8;

/// How long a client has to accept a [`Found`] match, in seconds.
pub const ACCEPT_SECONDS: u32 = 15;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Encode, Decode)]
pub enum Mode {
    Quick,
    Ranked,
}

/// What the endpoint's net report says (`iroh::net_report::Report`), for picking the
/// coordinator. Self-reported: a lie only costs its group latency.
#[derive(Clone, Debug, Default, PartialEq, Eq, Encode, Decode)]
pub struct Net {
    /// The NAT maps another public address for each peer (`mapping_varies_by_dest`); None if
    /// the report hasn't found out.
    pub hard: Option<bool>,
    /// A UDP round trip to a relay worked.
    pub udp: bool,
    /// The milliseconds to each relay the report reached, by URL.
    pub relays: Vec<(String, u16)>,
}

/// What a [`Queue`] is signed over (`Action::Queue`), as bitcode.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub struct QueueEntry {
    pub mode: Mode,
    /// Only clients of the same version are grouped: the lobby protocol and the sim, as a
    /// realm listing's `version`.
    pub version: String,
    /// The session's endpoint id. In the signed bytes, so that a captured queue can't be used
    /// from another endpoint.
    pub endpoint: [u8; 32],
    /// Its home relay, for the others' tickets.
    pub relay: Option<String>,
    pub net: Net,
    /// `clean_nick`ed by the server.
    pub nick: String,
    /// The [`Match::lobby`] we were in and want back: held for it while its host is registered
    /// ([`ToServer::Host`]), an ordinary entry once it isn't.
    pub rejoin: Option<u64>,
}

/// Client to server.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub enum ToServer {
    /// The first frame, and only once.
    Queue {
        auth: crate::Auth,
        /// A [`QueueEntry`] in bitcode: the bytes `auth` signs.
        entry: Vec<u8>,
    },
    Accept { id: u64 },
    Decline { id: u64 },
    /// The first frame, instead of a `Queue`, of whoever hosts the lobby of match `lobby`. Not
    /// signed: the connection's endpoint id, which must be one of the lobby's players, is the
    /// proof. A later registration of the same lobby replaces this one (an heir's, after a
    /// crash the server hasn't seen yet).
    Host { lobby: u64 },
    /// The host: how many more players the lobby takes now (0 in a race or a countdown), and
    /// who is in it, the host first and then the heirs.
    Open { open: u8, players: Vec<MatchPlayer> },
    /// The host: the players of [`ToClient::Admit`] `id` are let in, and may come.
    Admitted { id: u64 },
}

/// Server to client.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub enum ToClient {
    /// Every second or so while queued.
    Searching(Searching),
    /// A group is formed: accept within [`ACCEPT_SECONDS`].
    Found(Match),
    /// Someone of the match declined or didn't answer: back in the queue, the wait kept.
    Requeued,
    /// Everyone accepted: race. The server is done with this client.
    Go { id: u64 },
    /// The signature didn't hold. `now` is the server's clock, as `Response::Unauthorized`'s.
    Unauthorized { why: String, now: u64 },
    /// Not queued, and why; the stream ends.
    Refused(Refused),
    /// To a lobby's host: let these in (they accepted a [`Match`] of the lobby), then answer
    /// [`ToServer::Admitted`].
    Admit { id: u64, players: Vec<MatchPlayer> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Encode, Decode)]
pub struct Searching {
    /// The players queued for the same mode and version, us included.
    pub players: u16,
    pub waited_s: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub enum Refused {
    /// The key declined a match or let one time out lately: no queue until `until` (unix
    /// seconds).
    Cooldown { until: u64 },
    /// The key is queued already, from another session.
    AlreadyQueued,
    NotOpen { mode: Mode },
    Banned,
    BadFormat { why: String },
    /// The client declined, or didn't answer in time.
    Declined,
    /// A [`ToServer::Host`] for a lobby the server doesn't know, or not from one of its players.
    NoSuchLobby,
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refused::Cooldown { until } => write!(f, "declined a match lately: wait until {until}"),
            Refused::AlreadyQueued => f.write_str("already queued from another game"),
            Refused::NotOpen { mode } => write!(f, "{mode:?} isn't open"),
            Refused::Banned => f.write_str("this key is banned"),
            Refused::BadFormat { why } => write!(f, "bad request: {why}"),
            Refused::Declined => f.write_str("the match was declined"),
            Refused::NoSuchLobby => f.write_str("no such lobby"),
        }
    }
}

/// A group to race, as the server formed it.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub struct Match {
    /// Of the ready check.
    pub id: u64,
    /// The lobby it is the players of: `id` for a new group; another's for players sent into
    /// a lobby that is on (backfill, a rejoin), whose players come first.
    pub lobby: u64,
    pub mode: Mode,
    /// The coordinator (the lobby's host) first, then its heirs in handoff order.
    pub players: Vec<MatchPlayer>,
    /// A `LevelId` of a disc track.
    pub track: u32,
    pub laps: u8,
    pub masked_weapons: u16,
    pub even_odds: bool,
    pub seed: u16,
    /// Slots the coordinator fills with bots. Always 0 for now (MATCHMAKING.md, bot fill).
    pub bots: u8,
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub struct MatchPlayer {
    pub endpoint: [u8; 32],
    pub relay: Option<String>,
    pub nick: String,
}

/// `value` as a frame.
pub fn frame<T: Encode>(value: &T) -> Vec<u8> {
    let body = bitcode::encode(value);
    let mut bytes = (body.len() as u32).to_le_bytes().to_vec();
    bytes.extend(body);
    bytes
}

/// The length a frame's 4 bytes say, if it may be read.
pub fn frame_length(head: [u8; 4]) -> Option<usize> {
    let length = u32::from_le_bytes(head) as usize;
    (length <= MAX_FRAME).then_some(length)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_carry_their_length() {
        let message = ToClient::Searching(Searching { players: 3, waited_s: 12 });
        let bytes = frame(&message);
        let length = frame_length(bytes[..4].try_into().unwrap()).unwrap();
        assert_eq!(length, bytes.len() - 4);
        assert_eq!(bitcode::decode::<ToClient>(&bytes[4..]).unwrap(), message);
        assert_eq!(frame_length((MAX_FRAME as u32 + 1).to_le_bytes()), None);
    }
}
