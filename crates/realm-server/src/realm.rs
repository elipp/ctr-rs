//! What the realm lists, and what it answers to a request.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use tracing::{debug, info, warn};

use realm_proto::{Entry, Listing, MAX_LOBBIES, Reject, Request, Response, State};

/// A connection tells a lobby from another by this.
pub type Connection = usize;

struct Held {
    connection: Connection,
    listing: Listing,
}

/// The lobbies, by the endpoint id of the host.
#[derive(Default)]
pub struct Realm {
    lobbies: Mutex<HashMap<[u8; 32], Held>>,
}

impl Realm {
    pub fn handle(&self, id: [u8; 32], connection: Connection, request: Request) -> Response {
        let mut lobbies = self.lobbies.lock().unwrap();
        match request {
            Request::Announce(listing) => {
                let host = short(&id);
                if listing.validate().is_err() {
                    warn!(%host, ?listing, "turned down an invalid listing");
                    return Response::Rejected(Reject::Invalid);
                }
                if lobbies.len() >= MAX_LOBBIES && !lobbies.contains_key(&id) {
                    warn!(%host, name = %listing.name, "turned down a listing: the realm is full");
                    return Response::Rejected(Reject::Full);
                }
                // A host that comes back on a new connection takes over its listing from the
                // one that is yet to be found dead.
                let was = lobbies.insert(id, Held { connection, listing: listing.clone() });
                let Listing { name, players, max_players, state, track, version } = &listing;
                match was {
                    None => info!(
                        %host, %name, %track, %version, ?state, lobbies = lobbies.len(),
                        "lobby listed ({players}/{max_players})"
                    ),
                    Some(was) if was.listing == listing => debug!(%host, %name, "announced again"),
                    Some(was) => {
                        let before = &was.listing;
                        if players > &before.players {
                            info!(%host, %name, "player joined ({players}/{max_players})");
                        } else if players < &before.players {
                            info!(%host, %name, "player left ({players}/{max_players})");
                        }
                        match (before.state, state) {
                            (State::Lobby, State::Racing) => info!(%host, %name, %track, "race started"),
                            (State::Racing, State::Lobby) => info!(%host, %name, "race over"),
                            _ => {}
                        }
                        if before.name != *name || before.track != *track {
                            info!(%host, %name, %track, "lobby changed");
                        }
                    }
                }
                Response::Ok
            }
            Request::Withdraw => {
                if let Some(held) = remove(&mut lobbies, id, connection) {
                    info!(host = %short(&id), name = %held.listing.name, lobbies = lobbies.len(), "lobby withdrawn");
                }
                Response::Ok
            }
            Request::List => {
                debug!(lobbies = lobbies.len(), "list requested");
                Response::Lobbies(
                    lobbies
                        .iter()
                        .map(|(id, held)| Entry { id: *id, listing: held.listing.clone() })
                        .collect(),
                )
            }
        }
    }

    /// The connection is gone, and takes its lobby with it.
    pub fn closed(&self, id: [u8; 32], connection: Connection) {
        let mut lobbies = self.lobbies.lock().unwrap();
        if let Some(held) = remove(&mut lobbies, id, connection) {
            info!(host = %short(&id), name = %held.listing.name, lobbies = lobbies.len(), "lobby gone: its host disconnected");
        }
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.lobbies.lock().unwrap().len()
    }
}

/// Removes the lobby of `id`, if `connection` is still the one that holds it.
fn remove(lobbies: &mut HashMap<[u8; 32], Held>, id: [u8; 32], connection: Connection) -> Option<Held> {
    if lobbies.get(&id).is_some_and(|held| held.connection == connection) {
        return lobbies.remove(&id);
    }
    None
}

/// The first bytes of an endpoint id in hex, which is how iroh shortens it.
pub fn short(id: &[u8; 32]) -> String {
    id[..5].iter().map(|b| format!("{b:02x}")).collect()
}

/// How fast a connection may ask: a bucket of tokens that fills at a steady rate, and one is
/// taken for each request.
pub struct Bucket {
    tokens: f32,
    at: Instant,
}

impl Bucket {
    const BURST: f32 = 10.0;
    const PER_SECOND: f32 = 2.0;

    pub fn new(now: Instant) -> Bucket {
        Bucket { tokens: Bucket::BURST, at: now }
    }

    pub fn take(&mut self, now: Instant) -> bool {
        let passed = now.saturating_duration_since(self.at).as_secs_f32();
        self.at = now;
        self.tokens = (self.tokens + passed * Bucket::PER_SECOND).min(Bucket::BURST);
        if self.tokens < 1.0 {
            return false;
        }
        self.tokens -= 1.0;
        true
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use realm_proto::State;

    use super::*;

    fn listing(name: &str) -> Listing {
        Listing {
            name: name.into(),
            players: 1,
            max_players: 8,
            state: State::Lobby,
            track: "Dingo Canyon".into(),
            version: "v".into(),
        }
    }

    fn names(realm: &Realm) -> Vec<String> {
        let Response::Lobbies(mut entries) = realm.handle([0; 32], 0, Request::List) else {
            panic!("not a list");
        };
        entries.sort_by(|a, b| a.id.cmp(&b.id));
        entries.into_iter().map(|e| e.listing.name).collect()
    }

    #[test]
    fn a_lobby_lasts_as_long_as_its_connection() {
        let realm = Realm::default();
        assert_eq!(realm.handle([1; 32], 1, Request::Announce(listing("a"))), Response::Ok);
        assert_eq!(realm.handle([2; 32], 2, Request::Announce(listing("b"))), Response::Ok);
        assert_eq!(names(&realm), ["a", "b"]);
        realm.closed([1; 32], 1);
        assert_eq!(names(&realm), ["b"]);
    }

    #[test]
    fn an_old_connection_does_not_take_the_lobby_of_a_new_one() {
        let realm = Realm::default();
        realm.handle([1; 32], 1, Request::Announce(listing("old")));
        realm.handle([1; 32], 2, Request::Announce(listing("new")));
        realm.closed([1; 32], 1);
        assert_eq!(names(&realm), ["new"]);
        realm.handle([1; 32], 1, Request::Withdraw);
        assert_eq!(names(&realm), ["new"]);
        realm.handle([1; 32], 2, Request::Withdraw);
        assert!(names(&realm).is_empty());
    }

    #[test]
    fn a_bad_listing_is_rejected() {
        let realm = Realm::default();
        let response = realm.handle([1; 32], 1, Request::Announce(listing("")));
        assert_eq!(response, Response::Rejected(Reject::Invalid));
        assert_eq!(realm.len(), 0);
    }

    #[test]
    fn the_realm_fills_up() {
        let realm = Realm::default();
        for i in 0..MAX_LOBBIES {
            let mut id = [0; 32];
            id[..8].copy_from_slice(&(i as u64).to_le_bytes());
            assert_eq!(realm.handle(id, i, Request::Announce(listing("a"))), Response::Ok);
        }
        let response = realm.handle([0xff; 32], 0, Request::Announce(listing("b")));
        assert_eq!(response, Response::Rejected(Reject::Full));
        // A lobby that is listed can still change.
        let response = realm.handle([0; 32], 0, Request::Announce(listing("c")));
        assert_eq!(response, Response::Ok);
    }

    #[test]
    fn requests_are_limited() {
        let start = Instant::now();
        let mut bucket = Bucket::new(start);
        assert!((0..10).all(|_| bucket.take(start)));
        assert!(!bucket.take(start));
        assert!(bucket.take(start + Duration::from_secs(1)));
    }
}
