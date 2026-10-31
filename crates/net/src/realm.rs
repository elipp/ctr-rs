//! Keeping a lobby in the realm's list (see `realm-proto`).
//!
//! The host's session works out what the list should say about its lobby, and a task of its own
//! holds the connection to the realm and says it. The listing lasts as long as the connection,
//! so a session that ends needs no goodbye; one that loses the connection (the realm restarted,
//! the network went) is listed again when the task gets another.

use std::time::Duration;

use iroh::endpoint::Connection;
use iroh::{Endpoint, EndpointAddr, EndpointId};
use realm_proto::{ALPN, Listing, MAX_RESPONSE, Reject, Request, Response, State};
use std::sync::mpsc::Sender;
use tokio::sync::watch;

use crate::{Error, Event};

/// The realm that lobbies are listed in unless the player picks another.
pub const DEFAULT_REALM: &str = "255860d3bf91bdb74ba38490edad92a818a760496dd1465f1f583c2a49d6c2e9";

/// How many players a lobby is listed as having room for. (The listing says it; the host does
/// not hold the lobby to it.)
pub const LOBBY_CAPACITY: u8 = 8;

/// The first wait before the realm is tried again, and the longest.
const RETRY_MIN: Duration = Duration::from_secs(1);
const RETRY_MAX: Duration = Duration::from_secs(30);

/// How long the realm may take to answer.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(10);

/// The realm to list lobbies in: the id in the environment variable `CTR_REALM` if there is one
/// (to try a realm of one's own), else [`DEFAULT_REALM`].
pub fn default_realm() -> EndpointId {
    std::env::var("CTR_REALM")
        .ok()
        .and_then(|id| id.trim().parse().ok())
        .unwrap_or_else(|| DEFAULT_REALM.parse().expect("the default realm is an endpoint id"))
}

/// What a lobby is called in the list when the host does not say.
pub const DEFAULT_LOBBY_NAME: &str = "Public lobby";

/// A lobby name as the list takes it: no control characters, spaces trimmed and not doubled, at
/// most [`realm_proto::MAX_NAME`] chars. Empty if nothing is left of it.
pub fn clean_lobby_name(name: &str) -> String {
    let words: Vec<&str> = name
        .split_whitespace()
        .map(|w| w.trim_matches(char::is_control))
        .filter(|w| !w.is_empty())
        .collect();
    let name: String = words.join(" ").chars().filter(|c| !c.is_control()).collect();
    name.chars().take(realm_proto::MAX_NAME).collect::<String>().trim_end().to_string()
}

/// How a public lobby is listed: the rest of the listing (the players, whether a race is on)
/// is the session's to know.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Public {
    /// The realm to list the lobby in.
    pub realm: EndpointId,
    /// What the lobby is called.
    pub name: String,
    /// The level it drives on, as the players see it.
    pub track: String,
}

/// The listing of one lobby, kept up to date in one realm.
pub(crate) struct Publisher {
    pub(crate) realm: EndpointId,
    listing: watch::Sender<Option<Listing>>,
    /// The name and the track.
    about: Option<(String, String)>,
}

impl Publisher {
    pub(crate) fn start(
        endpoint: &Endpoint,
        realm: EndpointId,
        events: Sender<Event>,
    ) -> Publisher {
        let (listing, rx) = watch::channel(None);
        tokio::spawn(keep(endpoint.clone(), realm, rx, events));
        Publisher { realm, listing, about: None }
    }

    /// Lists the lobby as `public` says, or takes it off the list (`None`).
    pub(crate) fn set(&mut self, public: Option<Public>) {
        self.about = public.map(|p| (p.name, p.track));
    }

    /// Sends the listing if it changed. `players` is how many play in the lobby (the host
    /// too), and `build` the sim version, which a guest has to share.
    pub(crate) fn refresh(&self, players: u8, racing: bool, build: u32) {
        let listing = self.about.as_ref().map(|(name, track)| Listing {
            name: name.clone(),
            players,
            max_players: LOBBY_CAPACITY.max(players),
            state: if racing { State::Racing } else { State::Lobby },
            track: track.clone(),
            version: format!("{} sim {build}", String::from_utf8_lossy(crate::ALPN)),
        });
        self.listing.send_if_modified(|current| {
            let changed = *current != listing;
            *current = listing;
            changed
        });
    }
}

/// Holds the connection to the realm while there is a listing, and keeps it up to date. Ends
/// when the publisher is dropped.
async fn keep(
    endpoint: Endpoint,
    realm: EndpointId,
    mut listing: watch::Receiver<Option<Listing>>,
    events: Sender<Event>,
) {
    let mut wait = RETRY_MIN;
    let mut told: Option<String> = None;
    loop {
        if listing.wait_for(Option::is_some).await.is_err() {
            return;
        }
        match list(&endpoint, realm, &mut listing, &events).await {
            Ok(()) => {
                wait = RETRY_MIN;
                told = None;
                continue;
            }
            Err(e) => {
                // One report of a failure, not one for each try.
                let e = e.to_string();
                if told.as_ref() != Some(&e) {
                    let _ = events.send(Event::Listed(Err(e.clone())));
                    told = Some(e);
                }
            }
        }
        tokio::time::sleep(wait).await;
        wait = (wait * 2).min(RETRY_MAX);
    }
}

/// One connection's worth: lists the lobby, and follows it, until it is taken off the list
/// (`Ok`, the connection is closed) or the connection fails.
async fn list(
    endpoint: &Endpoint,
    realm: EndpointId,
    listing: &mut watch::Receiver<Option<Listing>>,
    events: &Sender<Event>,
) -> Result<(), Error> {
    let connection = endpoint.connect(EndpointAddr::new(realm), ALPN).await?;
    let mut said: Option<Listing> = None;
    loop {
        let current = listing.borrow_and_update().clone();
        match &current {
            None => {
                connection.close(0u32.into(), b"unlisted");
                return Ok(());
            }
            Some(_) if current == said => {}
            Some(now) => match ask(&connection, &Request::Announce(now.clone())).await? {
                Response::Ok => {
                    if said.is_none() {
                        let _ = events.send(Event::Listed(Ok(())));
                    }
                    said = current;
                }
                // Asking again with the same listing would only be told the same.
                Response::Rejected(Reject::Invalid) => {
                    let _ = events.send(Event::Listed(Err("the realm does not take the listing".into())));
                    said = current;
                }
                Response::Rejected(Reject::TooFast) => return Err("asking the realm too often".into()),
                Response::Rejected(Reject::Full) => return Err("the realm is full".into()),
                Response::Lobbies(_) => return Err("the realm answered nonsense".into()),
            },
        }
        tokio::select! {
            changed = listing.changed() => if changed.is_err() {
                connection.close(0u32.into(), b"bye");
                return Ok(());
            },
            _ = connection.closed() => return Err("the realm closed the connection".into()),
        }
    }
}

async fn ask(connection: &Connection, request: &Request) -> Result<Response, Error> {
    let (mut send, mut recv) = connection.open_bi().await?;
    send.write_all(&request.to_bytes()).await?;
    send.finish()?;
    let bytes = tokio::time::timeout(ANSWER_TIMEOUT, recv.read_to_end(MAX_RESPONSE))
        .await
        .map_err(|_| "the realm did not answer")??;
    Ok(Response::from_bytes(&bytes)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lobby_names_are_cleaned() {
        assert_eq!(clean_lobby_name("  Fast   lane \n"), "Fast lane");
        assert_eq!(clean_lobby_name("\u{7}"), "");
        let long = clean_lobby_name(&"ab ".repeat(40));
        assert!(long.chars().count() <= realm_proto::MAX_NAME);
        assert!(!long.ends_with(' '));
    }
}
