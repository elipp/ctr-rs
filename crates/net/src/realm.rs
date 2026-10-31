//! Keeping a lobby in the realm's list (see `realm-proto`).
//!
//! The host's session works out what the list should say about its lobby, and a task of its own
//! holds the connection to the realm and says it. The listing lasts as long as the connection,
//! so a session that ends needs no goodbye; one that loses the connection (the realm restarted,
//! the network went) is listed again when the task gets another.

use std::time::Duration;

use iroh::endpoint::Connection;
use iroh::{Endpoint, EndpointAddr, EndpointId};
use realm_proto::{
    ALPN, Listing, MAX_RESPONSE, Reject, Request, Response, State, is_name_char,
};
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
#[cfg(not(target_arch = "wasm32"))]
use std::thread::JoinHandle;
use tokio::sync::watch;

use crate::{Error, Event};

/// The realm that lobbies are listed in unless the player picks another.
pub const DEFAULT_REALM: &str = "255860d3bf91bdb74ba38490edad92a818a760496dd1465f1f583c2a49d6c2e9";

/// How many players a lobby is listed as having room for. (The listing says it; the host does
/// not hold the lobby to it.)
pub const LOBBY_CAPACITY: u8 = 8;

/// What a listing says of the builds that can play together: the lobby protocol, and the sim.
fn version(build: u32) -> String {
    format!("{} sim {build}", String::from_utf8_lossy(crate::ALPN))
}

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

/// The most chars of a lobby's name.
pub const MAX_LOBBY_NAME: usize = realm_proto::MAX_NAME;

/// A lobby name as the list takes it: spaces trimmed and not doubled, or why it cannot be one.
/// Only the chars the game's fonts have are allowed ([`is_name_char`]), and at most
/// [`MAX_LOBBY_NAME`] of them. (The realm turns down any other.)
pub fn lobby_name(name: &str) -> Result<String, String> {
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if let Some(c) = name.chars().find(|&c| !is_name_char(c)) {
        return Err(format!(
            "A lobby name cannot have '{c}': it can have letters A to Z, digits, spaces and ' ! : , = - % . + ? _ /"
        ));
    }
    if name.chars().count() > MAX_LOBBY_NAME {
        return Err(format!("A lobby name has at most {MAX_LOBBY_NAME} characters"));
    }
    Ok(name)
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
        n0_future::task::spawn(keep(endpoint.clone(), realm, rx, events));
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
            version: version(build),
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
        n0_future::time::sleep(wait).await;
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
    let bytes = n0_future::time::timeout(ANSWER_TIMEOUT, recv.read_to_end(MAX_RESPONSE))
        .await
        .map_err(|_| "the realm did not answer")??;
    Ok(Response::from_bytes(&bytes)?)
}

/// A lobby in the realm's list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublicLobby {
    /// What it is joined with (see [`Ticket`](crate::Ticket)).
    pub id: EndpointId,
    pub name: String,
    pub players: u8,
    pub max_players: u8,
    /// A race is on: whoever joins waits for the next one.
    pub racing: bool,
    pub track: String,
    /// It plays our build (see `build` of [`browse`]): a lobby that does not turns us away.
    pub compatible: bool,
}

/// The list being fetched from a realm, which [`Browse::poll`] hands over when it is here.
pub struct Browse {
    result: Mutex<Receiver<Result<Vec<PublicLobby>, String>>>,
    #[cfg(not(target_arch = "wasm32"))]
    _thread: JoinHandle<()>,
}

impl Browse {
    /// The list or what went wrong, once, when the realm has answered.
    pub fn poll(&self) -> Option<Result<Vec<PublicLobby>, String>> {
        match self.result.lock().unwrap().try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err("the network thread stopped".into())),
        }
    }
}

/// How long the whole fetch may take.
const BROWSE_TIMEOUT: Duration = Duration::from_secs(15);

/// Fetches the lobbies listed in `realm`, on a thread of its own (in a browser, a task of the
/// page): the game polls the [`Browse`].
/// `build` is the version of the sim (`race::SIM_VERSION`), to tell which lobbies we can join.
pub fn browse(realm: EndpointId, build: u32) -> Browse {
    let (tx, result) = mpsc::channel();
    let list = move || async move {
        let list = n0_future::time::timeout(BROWSE_TIMEOUT, fetch(realm, build))
            .await
            .unwrap_or_else(|_| Err("the realm took too long".into()));
        let _ = tx.send(list.map_err(|e| e.to_string()));
    };
    #[cfg(not(target_arch = "wasm32"))]
    let thread = std::thread::Builder::new()
        .name("realm".into())
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("the network runtime")
                .block_on(list())
        })
        .expect("the network thread");
    #[cfg(target_arch = "wasm32")]
    n0_future::task::spawn(list());
    Browse {
        result: Mutex::new(result),
        #[cfg(not(target_arch = "wasm32"))]
        _thread: thread,
    }
}

async fn fetch(realm: EndpointId, build: u32) -> Result<Vec<PublicLobby>, Error> {
    let endpoint = crate::endpoint_builder().bind().await?;
    let list = async {
        let connection = endpoint.connect(EndpointAddr::new(realm), ALPN).await?;
        let response = ask(&connection, &Request::List).await;
        connection.close(0u32.into(), b"bye");
        response
    }
    .await;
    endpoint.close().await;
    let Response::Lobbies(entries) = list? else {
        return Err("the realm answered nonsense".into());
    };
    let ours = version(build);
    Ok(entries
        .into_iter()
        .filter_map(|entry| {
            let Listing { name, players, max_players, state, track, version } = entry.listing;
            Some(PublicLobby {
                id: EndpointId::from_bytes(&entry.id).ok()?,
                name,
                players,
                max_players,
                racing: state == State::Racing,
                track,
                compatible: version == ours,
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lobby_names_are_checked() {
        assert_eq!(lobby_name("  Fast   lane \n").as_deref(), Ok("Fast lane"));
        assert_eq!(lobby_name("").as_deref(), Ok(""));
        assert!(lobby_name("Fäst").unwrap_err().contains("'ä'"));
        assert!(lobby_name("a\u{7}").is_err());
        assert!(lobby_name(&"a".repeat(MAX_LOBBY_NAME)).is_ok());
        assert!(lobby_name(&"a".repeat(MAX_LOBBY_NAME + 1)).is_err());
    }
}
