//! The realm: a directory of the public lobbies.
//!
//! A host that makes its lobby public connects to the realm server and announces it. The
//! listing lasts as long as the connection does, so a host that leaves or crashes drops off
//! without a word. A client that browses connects, asks for the list, and joins one of the
//! lobbies with the id it is listed under (the id the host's connection proved, which is also
//! the id the lobby is joined with). The realm carries no game traffic.
//!
//! Every request is a bidirectional stream of its own: the request is written and the stream
//! finished, and the response comes back on the same stream. The ends of the stream frame the
//! messages, so there are no length prefixes.

use bitcode::{Decode, Encode};

/// What the realm server and its clients speak, exchanged in the connection handshake.
pub const ALPN: &[u8] = b"ctr-rust/realm/1";

/// The most chars of a lobby's name.
pub const MAX_NAME: usize = 32;
/// The most chars of a track's name.
pub const MAX_TRACK: usize = 32;
/// The most chars of a version.
pub const MAX_VERSION: usize = 32;

/// The most bytes of a request. A listing is a few dozen at most.
pub const MAX_REQUEST: usize = 1024;
/// The most bytes of a response: the whole list.
pub const MAX_RESPONSE: usize = 256 << 10;

/// The most lobbies the realm lists, and so the most a response holds.
pub const MAX_LOBBIES: usize = 1000;

/// Whether a lobby's name may have this char: the ones the discs' fonts have, which show a name as
/// the menus do, and a space. (Letters are capitals there, whatever their case.) Not `#`, `(` and
/// `)`, which the middle font has since been given: older builds could not draw them. The
/// client's tests keep this equal to `render::font::in_original_fonts`.
pub fn is_name_char(c: char) -> bool {
    c == ' '
        || c.is_ascii_alphanumeric()
        || matches!(
            c,
            '\'' | '!' | ':' | ',' | '=' | '-' | '%' | '.' | '+' | '?' | '_' | '/'
        )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Encode, Decode)]
pub enum State {
    /// Gathering, and a player can join.
    Lobby,
    /// A race is on. A player who joins waits for the next one.
    Racing,
}

/// What a host says about its lobby.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub struct Listing {
    pub name: String,
    pub players: u8,
    pub max_players: u8,
    pub state: State,
    /// The track that is played, as the players see it.
    pub track: String,
    /// The lobby protocol the host speaks (`net::ALPN`), or anything else that is equal for the
    /// builds that can play together. A client greys out the lobbies of another.
    pub version: String,
}

impl Listing {
    /// Whether the realm takes this. The server checks it, and a client can too, to tell the
    /// player what is wrong before it asks.
    pub fn validate(&self) -> Result<(), Reject> {
        let text =
            |s: &str, max: usize| s.chars().count() <= max && !s.chars().any(char::is_control);
        let ok = !self.name.trim().is_empty()
            && self.name.chars().count() <= MAX_NAME
            && self.name.chars().all(is_name_char)
            && text(&self.track, MAX_TRACK)
            && !self.version.is_empty()
            && text(&self.version, MAX_VERSION)
            && self.max_players > 0
            && self.players <= self.max_players;
        if ok { Ok(()) } else { Err(Reject::Invalid) }
    }
}

/// A listed lobby.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub struct Entry {
    /// The lobby's endpoint id (an `iroh::EndpointId`, as its bytes: this crate does not need
    /// iroh).
    pub id: [u8; 32],
    pub listing: Listing,
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub enum Request {
    /// List the lobby of the connection, or replace what is listed for it. A host sends this
    /// again whenever the players, the state or the track change.
    Announce(Listing),
    /// Take the lobby of the connection off the list, and keep the connection.
    Withdraw,
    List,
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub enum Response {
    /// The announce or the withdraw is done.
    Ok,
    Rejected(Reject),
    Lobbies(Vec<Entry>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Encode, Decode)]
pub enum Reject {
    /// The listing is not one [`Listing::validate`] takes.
    Invalid,
    /// The connection asks too often.
    TooFast,
    /// The realm lists as many as it does.
    Full,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn listing() -> Listing {
        Listing {
            name: "Fast lane".into(),
            players: 3,
            max_players: 8,
            state: State::Lobby,
            track: "Dingo Canyon".into(),
            version: "ctr-rust/lobby/9".into(),
        }
    }

    #[test]
    fn messages_round_trip() {
        for request in [
            Request::Announce(listing()),
            Request::Withdraw,
            Request::List,
        ] {
            assert_eq!(Request::from_bytes(&request.to_bytes()).unwrap(), request);
        }
        let lobbies = Response::Lobbies(vec![Entry {
            id: [7; 32],
            listing: listing(),
        }]);
        for response in [Response::Ok, Response::Rejected(Reject::Full), lobbies] {
            assert_eq!(
                Response::from_bytes(&response.to_bytes()).unwrap(),
                response
            );
        }
        assert!(Request::from_bytes(b"\xff\xff\xff").is_err());
    }

    #[test]
    fn listings_are_validated() {
        assert_eq!(listing().validate(), Ok(()));
        let bad = [
            Listing {
                name: "  ".into(),
                ..listing()
            },
            Listing {
                name: "x".repeat(MAX_NAME + 1),
                ..listing()
            },
            Listing {
                name: "a\nb".into(),
                ..listing()
            },
            Listing {
                name: "Fäst".into(),
                ..listing()
            },
            Listing {
                track: "x".repeat(MAX_TRACK + 1),
                ..listing()
            },
            Listing {
                version: String::new(),
                ..listing()
            },
            Listing {
                max_players: 0,
                players: 0,
                ..listing()
            },
            Listing {
                players: 9,
                ..listing()
            },
        ];
        for listing in bad {
            assert_eq!(listing.validate(), Err(Reject::Invalid), "{listing:?}");
        }
    }

    #[test]
    fn a_full_list_fits_the_response() {
        let entry = Entry {
            id: [0xff; 32],
            listing: Listing {
                name: "ü".repeat(MAX_NAME),
                track: "ü".repeat(MAX_TRACK),
                version: "ü".repeat(MAX_VERSION),
                ..listing()
            },
        };
        let bytes = Response::Lobbies(vec![entry; MAX_LOBBIES]).to_bytes();
        assert!(bytes.len() <= MAX_RESPONSE, "{}", bytes.len());
    }
}
