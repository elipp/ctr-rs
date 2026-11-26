//! Quick play (MATCHMAKING.md): a session that first queues at the score server's matchmaker
//! (`score::matching`), and once its match goes, hosts the match's lobby (as its coordinator) or
//! joins it. It queues from the endpoint it then races on, since that id is what the match names
//! it by; like a realm listing, the server sees only that random key and the relay it comes
//! through. The player key only signs the queue entry.

use std::time::Duration;

use iroh::endpoint::{Connection, RecvStream, SendStream};
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayUrl, SecretKey, Watcher};
use n0_future::time::{Instant, timeout};
use score::matching::{MATCH_ALPN, Match, Net, QueueEntry, Refused, ToClient, ToServer, frame, frame_length};
use score::{Action, Auth};
use tokio::sync::mpsc::unbounded_channel;

use crate::peers::Role;
use crate::{Command, Error, Event, Io, Rules, Ticket};

/// How long the first net report may take: past it the queue goes without (the worst NAT).
const REPORT_WAIT: Duration = Duration::from_secs(3);

/// How long after the match goes its coordinator waits for everyone to come in, before it
/// starts with whoever did (two at least).
const JOIN_DEADLINE: Duration = Duration::from_secs(30);

/// The seconds counted down before a match's race, as a manual host's (`client` `countdown.rs`).
pub(crate) const COUNTDOWN_SECONDS: u64 = 5;

/// What a match found says (see [`Event::Found`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    /// The players' nicks, the coordinator first.
    pub nicks: Vec<String>,
    pub track: u32,
    pub rules: Rules,
}

impl Found {
    fn of(found: &Match) -> Found {
        Found {
            nicks: found.players.iter().map(|p| p.nick.clone()).collect(),
            track: found.track,
            rules: rules_of(found),
        }
    }
}

fn rules_of(found: &Match) -> Rules {
    Rules {
        laps: found.laps,
        masked_weapons: found.masked_weapons,
        even_odds: found.even_odds,
        seed: found.seed,
    }
}

/// The lobby of a match, as every one of its players keeps it (any of them may come to host it).
/// The session holds to it, not the game: whoever hosts has none of a host's say.
pub(crate) struct Matchmade {
    /// The match's players, its coordinator first, then the heirs in order.
    pub(crate) players: Vec<EndpointId>,
    pub(crate) track: u32,
    pub(crate) rules: Rules,
    /// When whoever hosts starts with the players that came.
    pub(crate) deadline: Instant,
    /// The host counts down to the start: when it is, and the seconds it last said were left.
    pub(crate) countdown: Option<(Instant, u64)>,
    /// The match's race has been started (a lobby of a match races once: what comes after it is
    /// step 3's, MATCHMAKING.md).
    pub(crate) started: bool,
}

impl Matchmade {
    fn of(found: &Match) -> Result<Matchmade, Error> {
        let players = found
            .players
            .iter()
            .map(|p| EndpointId::from_bytes(&p.endpoint))
            .collect::<Result<_, _>>()?;
        Ok(Matchmade {
            players,
            track: found.track,
            rules: rules_of(found),
            deadline: Instant::now() + JOIN_DEADLINE,
            countdown: None,
            started: false,
        })
    }

    /// When the host has to look at the start again: the deadline, or the countdown's next second.
    pub(crate) fn wake(&self) -> Option<Instant> {
        match self.countdown {
            _ if self.started => None,
            Some((at, said)) => Some(at - Duration::from_secs(said.saturating_sub(1))),
            None => Some(self.deadline),
        }
    }

    /// Where `id` is in the match's order; none for a player not in it, who is let in nowhere.
    pub(crate) fn rank(&self, id: EndpointId) -> Option<usize> {
        self.players.iter().position(|&p| p == id)
    }
}

/// What the endpoint's net report says, once it has one (or [`REPORT_WAIT`] is up).
async fn net_of(endpoint: &Endpoint) -> Net {
    let mut watcher = endpoint.net_report();
    let Ok(report) = timeout(REPORT_WAIT, watcher.initialized()).await else {
        return Net::default();
    };
    let mut relays: Vec<(String, u16)> = Vec::new();
    for (_, url, latency) in report.relay_latency.iter() {
        let ms = latency.as_millis().min(u16::MAX as u128) as u16;
        let url = url.to_string();
        match relays.iter_mut().find(|(u, _)| *u == url) {
            Some((_, best)) => *best = (*best).min(ms),
            None => relays.push((url, ms)),
        }
    }
    Net {
        hard: report.mapping_varies_by_dest(),
        udp: report.has_udp(),
        relays,
    }
}

async fn read_frame(recv: &mut RecvStream) -> Result<ToClient, Error> {
    let mut head = [0; 4];
    recv.read_exact(&mut head).await?;
    let length = frame_length(head).ok_or("the matchmaker sent a frame too long")?;
    let mut body = vec![0; length];
    recv.read_exact(&mut body).await?;
    Ok(bitcode::decode(&body)?)
}

async fn write(send: &mut SendStream, message: &ToServer) -> Result<(), Error> {
    Ok(send.write_all(&frame(message)).await?)
}

/// What the queueing came to.
enum Queued {
    Go(Match),
    /// The game ended the session.
    Stopped,
}

/// Queues for quick play at `server`, signed by the player `key`, and races the match it finds;
/// the rest is as [`crate::host`]/[`crate::join`]. [`Event::Searching`] comes every second or so
/// while queued, [`Event::Found`] when a group forms, to be answered with
/// [`crate::Session::answer`] within `score::matching::ACCEPT_SECONDS`.
pub fn quick_play(server: EndpointId, key: SecretKey, character: u8, extras: &[u8], nick: &str, build: u32) -> crate::Session {
    let nick = crate::clean_nick(nick);
    let extras = crate::clean_extras(extras);
    crate::spawn(move |endpoint, mut io| {
        Box::pin(async move {
            let (mut character, mut extras, mut nick) = (character, extras, nick);
            let queued = queue(&endpoint, &mut io, server, &key, build, &mut character, &mut extras, &mut nick).await?;
            let Queued::Go(found) = queued else {
                return Ok(());
            };
            let coordinator = found.players.first().ok_or("a match of nobody")?;
            let role = if coordinator.endpoint == *endpoint.id().as_bytes() {
                Role::Host
            } else {
                Role::Join(Ticket {
                    id: EndpointId::from_bytes(&coordinator.endpoint)?,
                    relay: coordinator.relay.as_deref().and_then(|r| r.parse::<RelayUrl>().ok()),
                })
            };
            let matchmade = Matchmade::of(&found)?;
            crate::session::run(endpoint, io, role, character, extras, nick, build, false, Some(matchmade)).await
        })
    })
}

#[allow(clippy::too_many_arguments)]
async fn queue(
    endpoint: &Endpoint,
    io: &mut Io,
    server: EndpointId,
    key: &SecretKey,
    build: u32,
    character: &mut u8,
    extras: &mut Vec<u8>,
    nick: &mut String,
) -> Result<Queued, Error> {
    tokio::select! {
        _ = &mut io.stop => return Ok(Queued::Stopped),
        _ = endpoint.online() => {}
    }
    let net = net_of(endpoint).await;
    let entry = QueueEntry {
        mode: score::matching::Mode::Quick,
        version: crate::realm::version(build),
        endpoint: *endpoint.id().as_bytes(),
        relay: endpoint.addr().relay_urls().next().map(|r| r.to_string()),
        net,
        nick: nick.clone(),
    };
    let entry = bitcode::encode(&entry);
    // A second try only for a clock that was off, with the server's.
    for attempt in 0..2 {
        let connection: Connection = tokio::select! {
            _ = &mut io.stop => return Ok(Queued::Stopped),
            connection = endpoint.connect(EndpointAddr::new(server), MATCH_ALPN) => match connection {
                Ok(connection) => connection,
                Err(e) if crate::realm::is_alpn_mismatch(&e) => return Err(crate::realm::OUTDATED.into()),
                Err(e) => return Err(e.into()),
            },
        };
        let (mut send, mut recv) = connection.open_bi().await?;
        let ts = crate::score::signing_time();
        let auth = Auth::sign(key, Action::Queue, ts, &entry);
        write(&mut send, &ToServer::Queue { auth, entry: entry.clone() }).await?;

        // (A read of its own: `read_exact` loses what it read if a `select!` drops it.)
        let (heard_tx, mut heard) = unbounded_channel();
        n0_future::task::spawn(async move {
            loop {
                let message = read_frame(&mut recv).await;
                let end = message.is_err();
                if heard_tx.send(message).is_err() || end {
                    break;
                }
            }
        });
        let mut found: Option<Match> = None;
        let outcome: Result<Queued, Error> = loop {
            tokio::select! {
                _ = &mut io.stop => break Ok(Queued::Stopped),
                Some(command) = io.commands.recv() => match command {
                    Command::Answer(accept) => if let Some(found) = &found {
                        let id = found.id;
                        let answer = if accept { ToServer::Accept { id } } else { ToServer::Decline { id } };
                        write(&mut send, &answer).await?;
                    },
                    Command::Character(c) => *character = c,
                    Command::Extras(e) => *extras = crate::clean_extras(&e),
                    Command::Nickname(n) => *nick = crate::clean_nick(&n),
                    // (Nothing else means anything before there is a lobby.)
                    _ => {}
                },
                message = heard.recv() => match message {
                    Some(Ok(ToClient::Searching(s))) => io.event(Event::Searching {
                        players: s.players,
                        waited: Duration::from_secs(s.waited_s.into()),
                    }),
                    Some(Ok(ToClient::Found(m))) => {
                        io.event(Event::Found(Found::of(&m)));
                        found = Some(m);
                    }
                    Some(Ok(ToClient::Requeued)) => {
                        found = None;
                        io.event(Event::Requeued);
                    }
                    Some(Ok(ToClient::Go { id })) => match found.take() {
                        Some(m) if m.id == id => break Ok(Queued::Go(m)),
                        _ => break Err("the matchmaker started a match we weren't found for".into()),
                    },
                    Some(Ok(ToClient::Unauthorized { now, .. })) if attempt == 0 && ts.abs_diff(now) > score::MAX_SKEW => {
                        crate::score::learn_clock(now);
                        break Err(Retry.into());
                    }
                    Some(Ok(ToClient::Unauthorized { why, .. })) => break Err(format!("the matchmaker refused the queue: {why}").into()),
                    Some(Ok(ToClient::Refused(refused))) => break Err(refusal(&refused).into()),
                    Some(Err(e)) => break Err(format!("the matchmaker: {e}").into()),
                    None => break Err("the matchmaker went away".into()),
                },
            }
        };
        connection.close(0u32.into(), b"bye");
        match outcome {
            Err(e) if e.is::<Retry>() => continue,
            outcome => return outcome,
        }
    }
    Err("the matchmaker refused our clock".into())
}

#[derive(Debug)]
struct Retry;

impl std::fmt::Display for Retry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("retry")
    }
}

impl std::error::Error for Retry {}

fn refusal(refused: &Refused) -> String {
    match refused {
        Refused::Cooldown { until } => {
            let left = until.saturating_sub(crate::score::unix_now());
            format!("You declined a match lately: you can queue again in {left} s")
        }
        Refused::AlreadyQueued => "You are queued already, in another game".into(),
        Refused::Declined => "The match was declined".into(),
        other => format!("The matchmaker refused: {other}"),
    }
}
