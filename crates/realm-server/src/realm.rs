//! What the realm lists, and what it answers to a request.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tracing::{debug, info, warn};

use crate::tg::Alerts;

use realm_proto::{
    Entry, Listing, MAX_LOBBIES, Reject, Request, Response, State, TRANSFER_SECONDS,
};

/// A connection tells a lobby from another by this.
pub type Connection = usize;

struct Held {
    connection: Connection,
    listing: Listing,
}

/// A lobby that was handed over and waits for its new host. It is not listed meanwhile.
struct Handed {
    listing: Listing,
    until: Instant,
}

#[derive(Default)]
struct Lobbies {
    /// The lobbies, by the endpoint id of the host.
    listed: HashMap<[u8; 32], Held>,
    /// The lobbies handed over, by the endpoint id of the host they wait for.
    handed: HashMap<[u8; 32], Handed>,
}

#[derive(Default)]
pub struct Realm {
    lobbies: Mutex<Lobbies>,
    /// Told of the lobbies that open, race and close, if there is anywhere to tell.
    alerts: Option<Alerts>,
}

impl Realm {
    pub fn new(alerts: Option<Alerts>) -> Realm {
        Realm {
            lobbies: Mutex::default(),
            alerts,
        }
    }

    fn alert(&self, text: String) {
        if let Some(alerts) = &self.alerts {
            alerts.send(text);
        }
    }

    pub fn handle(&self, id: [u8; 32], connection: Connection, request: Request) -> Response {
        self.handle_at(Instant::now(), id, connection, request)
    }

    fn handle_at(
        &self,
        now: Instant,
        id: [u8; 32],
        connection: Connection,
        request: Request,
    ) -> Response {
        let mut guard = self.lobbies.lock().unwrap();
        let Lobbies { listed: lobbies, handed } = &mut *guard;
        handed.retain(|_, h| h.until > now);
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
                let was = lobbies.insert(
                    id,
                    Held {
                        connection,
                        listing: listing.clone(),
                    },
                );
                let Listing {
                    name,
                    players,
                    max_players,
                    state,
                    track,
                    version,
                } = &listing;
                match was {
                    None => {
                        info!(
                            %host, %name, %track, %version, ?state, lobbies = lobbies.len(),
                            "lobby listed ({players}/{max_players})"
                        );
                        self.alert(format!(
                            "Lobby opened: {name}\ntrack {track}, {players}/{max_players}, v{version}\nhost {host}, {} open",
                            lobbies.len()
                        ));
                    }
                    Some(was) if was.listing == listing => debug!(%host, %name, "announced again"),
                    Some(was) => {
                        let before = &was.listing;
                        if players > &before.players {
                            info!(%host, %name, "player joined ({players}/{max_players})");
                        } else if players < &before.players {
                            info!(%host, %name, "player left ({players}/{max_players})");
                        }
                        match (before.state, state) {
                            (State::Lobby, State::Racing) => {
                                info!(%host, %name, %track, "race started");
                                self.alert(format!(
                                    "Race started: {name}\ntrack {track}, {players}/{max_players}"
                                ));
                            }
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
                if let Some(held) = remove(lobbies, id, connection) {
                    info!(host = %short(&id), name = %held.listing.name, lobbies = lobbies.len(), "lobby withdrawn");
                    self.alert(format!("Lobby closed: {}\n{} open", held.listing.name, lobbies.len()));
                }
                Response::Ok
            }
            Request::Transfer(to) => {
                if to == id || !lobbies.get(&id).is_some_and(|h| h.connection == connection) {
                    return Response::Rejected(Reject::NotListed);
                }
                let held = lobbies.remove(&id).unwrap();
                info!(
                    host = %short(&id), to = %short(&to), name = %held.listing.name,
                    "lobby handed over"
                );
                let until = now + Duration::from_secs(TRANSFER_SECONDS);
                handed.insert(to, Handed { listing: held.listing, until });
                Response::Ok
            }
            Request::Claim => {
                if lobbies.len() >= MAX_LOBBIES && !lobbies.contains_key(&id) {
                    return Response::Rejected(Reject::Full);
                }
                let Some(handed) = handed.remove(&id) else {
                    return Response::Rejected(Reject::NothingToClaim);
                };
                info!(host = %short(&id), name = %handed.listing.name, "lobby claimed");
                lobbies.insert(id, Held { connection, listing: handed.listing });
                Response::Ok
            }
            Request::List => {
                debug!(lobbies = lobbies.len(), "list requested");
                Response::Lobbies(
                    lobbies
                        .iter()
                        .map(|(id, held)| Entry {
                            id: *id,
                            listing: held.listing.clone(),
                        })
                        .collect(),
                )
            }
        }
    }

    /// The connection is gone, and takes its lobby with it.
    pub fn closed(&self, id: [u8; 32], connection: Connection) {
        let lobbies = &mut self.lobbies.lock().unwrap().listed;
        if let Some(held) = remove(lobbies, id, connection) {
            info!(host = %short(&id), name = %held.listing.name, lobbies = lobbies.len(), "lobby gone: its host disconnected");
            self.alert(format!("Lobby gone: {}\n{} open", held.listing.name, lobbies.len()));
        }
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.lobbies.lock().unwrap().listed.len()
    }
}

/// Removes the lobby of `id`, if `connection` is still the one that holds it.
fn remove(
    lobbies: &mut HashMap<[u8; 32], Held>,
    id: [u8; 32],
    connection: Connection,
) -> Option<Held> {
    if lobbies
        .get(&id)
        .is_some_and(|held| held.connection == connection)
    {
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
        Bucket {
            tokens: Bucket::BURST,
            at: now,
        }
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
        assert_eq!(
            realm.handle([1; 32], 1, Request::Announce(listing("a"))),
            Response::Ok
        );
        assert_eq!(
            realm.handle([2; 32], 2, Request::Announce(listing("b"))),
            Response::Ok
        );
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
    fn a_lobby_moves_to_the_host_that_claims_it() {
        let realm = Realm::default();
        let now = Instant::now();
        realm.handle_at(now, [1; 32], 1, Request::Announce(listing("a")));
        let response = realm.handle_at(now, [1; 32], 1, Request::Transfer([2; 32]));
        assert_eq!(response, Response::Ok);
        // Not listed while it waits, and the old host cannot get it back by claiming.
        assert!(names(&realm).is_empty());
        let response = realm.handle_at(now, [1; 32], 1, Request::Claim);
        assert_eq!(response, Response::Rejected(Reject::NothingToClaim));
        // The old host's connection going changes nothing.
        realm.closed([1; 32], 1);
        let response = realm.handle_at(now, [2; 32], 5, Request::Claim);
        assert_eq!(response, Response::Ok);
        let Response::Lobbies(entries) = realm.handle([0; 32], 0, Request::List) else {
            panic!("not a list");
        };
        assert_eq!(entries.len(), 1);
        assert_eq!((entries[0].id, entries[0].listing.name.as_str()), ([2; 32], "a"));
        // It lasts as long as the new connection.
        realm.closed([2; 32], 5);
        assert!(names(&realm).is_empty());
    }

    #[test]
    fn a_handed_over_lobby_is_claimed_in_time_or_dropped() {
        let realm = Realm::default();
        let now = Instant::now();
        realm.handle_at(now, [1; 32], 1, Request::Announce(listing("a")));
        realm.handle_at(now, [1; 32], 1, Request::Transfer([2; 32]));
        let late = now + Duration::from_secs(TRANSFER_SECONDS + 1);
        let response = realm.handle_at(late, [2; 32], 5, Request::Claim);
        assert_eq!(response, Response::Rejected(Reject::NothingToClaim));
        assert_eq!(realm.len(), 0);
    }

    #[test]
    fn only_the_right_host_claims_and_only_the_owner_hands_over() {
        let realm = Realm::default();
        let now = Instant::now();
        realm.handle_at(now, [1; 32], 1, Request::Announce(listing("a")));
        // Nothing listed for the sender, an old connection, or to itself.
        for (id, connection, to) in [([9; 32], 9, [2; 32]), ([1; 32], 7, [2; 32]), ([1; 32], 1, [1; 32])] {
            let response = realm.handle_at(now, id, connection, Request::Transfer(to));
            assert_eq!(response, Response::Rejected(Reject::NotListed));
        }
        assert_eq!(names(&realm), ["a"]);
        realm.handle_at(now, [1; 32], 1, Request::Transfer([2; 32]));
        let response = realm.handle_at(now, [3; 32], 3, Request::Claim);
        assert_eq!(response, Response::Rejected(Reject::NothingToClaim));
        let response = realm.handle_at(now, [2; 32], 2, Request::Claim);
        assert_eq!(response, Response::Ok);
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
            assert_eq!(
                realm.handle(id, i, Request::Announce(listing("a"))),
                Response::Ok
            );
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
