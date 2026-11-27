//! Quick play (MATCHMAKING.md): a session that first queues at the score server's matchmaker
//! (`score::matching`), and once its match goes, hosts the match's lobby (as its coordinator) or
//! joins it. It queues from the endpoint it then races on, since that id is what the match names
//! it by; like a realm listing, the server sees only that random key and the relay it comes
//! through. The player key only signs the queue entry.

use std::collections::{HashMap, HashSet};
use std::collections::hash_map::RandomState;
use std::hash::BuildHasher;
use std::time::Duration;

use iroh::endpoint::{Connection, RecvStream, SendStream};
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayUrl, SecretKey, Watcher};
use n0_future::time::{Instant, timeout};
use score::matching::{MATCH_ALPN, Match, MatchPlayer, Net, QueueEntry, Refused, ToClient, ToServer, frame, frame_length};
use score::{Action, Auth};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::peers::Role;
use crate::{Command, Error, Event, Io, Rules, Ticket};

/// How long the first net report may take: past it the queue goes without (the worst NAT).
const REPORT_WAIT: Duration = Duration::from_secs(3);

/// How long after the match goes its coordinator waits for everyone to come in, before it
/// starts with whoever did (two at least).
const JOIN_DEADLINE: Duration = Duration::from_secs(30);

/// How long after a match's race its next one starts: everyone still in the lobby races it.
pub(crate) const NEXT_WAIT: Duration = Duration::from_secs(80);

/// How soon the next race starts once everyone in the lobby is ready (unless it is sooner
/// anyway). Not at once: players let in from the queue may still be connecting.
pub(crate) const READY_WAIT: Duration = Duration::from_secs(10);

/// The seconds counted down before a match's race, as a manual host's (`client` `countdown.rs`).
// TODO(matchmaking): back to 5 once the countdown and the gathering board have been looked at:
// 10 only to see them (2026-11-27).
pub(crate) const COUNTDOWN_SECONDS: u64 = 10;

/// What a match found says (see [`Event::Found`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    /// The players' nicks, the coordinator first.
    pub nicks: Vec<String>,
    pub track: u32,
    pub rules: Rules,
    /// The lobby it is of, to rejoin it by ([`quick_play`]'s `rejoin`).
    pub lobby: u64,
}

impl Found {
    fn of(found: &Match) -> Found {
        Found {
            nicks: found.players.iter().map(|p| p.nick.clone()).collect(),
            track: found.track,
            rules: rules_of(found),
            lobby: found.lobby,
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
    /// The match's players, its coordinator first, then the heirs in order; and the players let
    /// in since (`ToClient::Admit`, or a host's `Message::Lobby` that names them).
    pub(crate) players: Vec<EndpointId>,
    /// The matchmaker, which whoever hosts registers the lobby with (`register`), and the
    /// lobby's id there.
    pub(crate) server: EndpointId,
    pub(crate) lobby: u64,
    /// The next race's (the Match's for the first): whoever hosts picks them between races
    /// (`pick`), the others keep the last `Message::Next`/`Start` they heard.
    pub(crate) track: u32,
    pub(crate) rules: Rules,
    /// Between races, everyone (`Message::Next`, so an heir goes on with them): when the next
    /// race starts, and its track unless the votes say otherwise.
    pub(crate) next: Option<Instant>,
    pub(crate) rolled: u32,
    /// The host: what breaks a tie between votes that `rolled` is not in. Fixed for a whole
    /// wait, so the track shown doesn't flicker as votes come.
    tie: u64,
    /// When whoever hosts starts with the players that came.
    pub(crate) deadline: Instant,
    /// The host counts down to the start: when it is.
    pub(crate) countdown: Option<Instant>,
    /// A race of the match has been started: the next ones go by [`Matchmade::next`].
    pub(crate) started: bool,
    /// The host: who is ready (`Message::Again`, ours `Command::Again`) and what each voted for
    /// (`Message::Vote`). Cleared at each start; guests send theirs again to each new host.
    pub(crate) ready: HashSet<EndpointId>,
    votes: HashMap<EndpointId, u32>,
    /// The host: when the next race was before [`Matchmade::hurry`] brought it on, for an
    /// unready to put it back.
    unhurried: Option<Instant>,
}

impl Matchmade {
    fn of(found: &Match, server: EndpointId) -> Result<Matchmade, Error> {
        let players = found
            .players
            .iter()
            .map(|p| EndpointId::from_bytes(&p.endpoint))
            .collect::<Result<_, _>>()?;
        Ok(Matchmade {
            players,
            server,
            lobby: found.lobby,
            track: found.track,
            rules: rules_of(found),
            next: None,
            rolled: found.track,
            tie: 0,
            deadline: Instant::now() + JOIN_DEADLINE,
            countdown: None,
            // (Sent into a lobby that is on: its races have begun, and the host counts us in
            // for the next, `ToClient::Admit`.)
            started: found.lobby != found.id,
            ready: HashSet::new(),
            votes: HashMap::new(),
            unhurried: None,
        })
    }

    /// When the host has to look at the start again: the countdown's end, the first race's
    /// deadline, or when the next race's countdown begins.
    pub(crate) fn wake(&self) -> Option<Instant> {
        match self.countdown {
            Some(at) => Some(at),
            None if !self.started => Some(self.deadline),
            None => self.next.map(|next| next - Duration::from_secs(COUNTDOWN_SECONDS)),
        }
    }

    /// The host, once a race is over: the wait for the next begins, on a new random disc track
    /// (another than the last one's) with a new item seed. Whether it did (a wait that an heir
    /// took over goes on).
    pub(crate) fn wait_next(&mut self, now: Instant) -> bool {
        if self.next.is_some() {
            return false;
        }
        // The time is mixed in for the web, whose `RandomState` is not random.
        let random = RandomState::new().hash_one(self.deadline.elapsed().as_nanos());
        let other = (random % u64::from(race::RACING_TRACKS - 1)) as u32;
        self.rolled = if other >= self.track { other + 1 } else { other };
        self.track = self.rolled;
        self.rules.seed = (random >> 32) as u16;
        self.tie = random.rotate_left(16);
        self.next = Some(now + NEXT_WAIT);
        true
    }

    /// The host: everyone `present` is ready, so the next race is brought to [`READY_WAIT`]
    /// from `now`. Whether it was.
    pub(crate) fn hurry(&mut self, present: &[EndpointId], now: Instant) -> bool {
        let all = present.iter().all(|id| self.ready.contains(id));
        match &mut self.next {
            Some(next) if all && *next > now + READY_WAIT => {
                self.unhurried = Some(*next);
                *next = now + READY_WAIT;
                true
            }
            _ => false,
        }
    }

    /// `id` is ready for the next race. Whether it is news.
    pub(crate) fn ready(&mut self, id: EndpointId) -> bool {
        self.ready.insert(id)
    }

    /// `id` is not ready after all: not everyone is, so a hurried next race is back to its wait.
    /// Whether it is news.
    pub(crate) fn unready(&mut self, id: EndpointId) -> bool {
        if !self.ready.remove(&id) {
            return false;
        }
        if let Some(at) = self.unhurried.take() {
            self.next = Some(at);
        }
        true
    }

    /// `id` votes for a disc track to race next. Whether it is news.
    pub(crate) fn vote(&mut self, id: EndpointId, track: u32) -> bool {
        track < race::RACING_TRACKS && self.votes.insert(id, track) != Some(track)
    }

    /// The votes of the players `present`, the most voted track first, and the track they make
    /// the next race's: the most voted, a tie broken by `rolled` (if it is in it) or by `tie`;
    /// `rolled` when nobody voted.
    pub(crate) fn pick(&mut self, present: &[EndpointId]) -> Vec<(u32, u8)> {
        let mut tally: Vec<(u32, u8)> = Vec::new();
        for (_, &track) in self.votes.iter().filter(|(id, _)| present.contains(id)) {
            match tally.iter_mut().find(|(t, _)| *t == track) {
                Some((_, n)) => *n = n.saturating_add(1),
                None => tally.push((track, 1)),
            }
        }
        tally.sort_by_key(|&(track, n)| (std::cmp::Reverse(n), track));
        let most = tally.first().map_or(0, |&(_, n)| n);
        let tied: Vec<u32> = tally.iter().filter(|&&(_, n)| n == most).map(|&(t, _)| t).collect();
        self.track = match tied.len() {
            0 => self.rolled,
            _ if tied.contains(&self.rolled) => self.rolled,
            len => tied[(self.tie % len as u64) as usize],
        };
        tally
    }

    /// A race of the match starts: the wait, the ready and the votes are of the one before.
    pub(crate) fn starts(&mut self) {
        self.started = true;
        self.countdown = None;
        self.next = None;
        self.ready.clear();
        self.votes.clear();
        self.unhurried = None;
    }

    /// `id` is let in, behind the players there are.
    pub(crate) fn admit(&mut self, id: EndpointId) {
        if self.rank(id).is_none() {
            self.players.push(id);
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
///
/// `rejoin` is a [`Found::lobby`] we were in and want back: the matchmaker sends us there if it
/// is still on, else it queues us as anyone.
pub fn quick_play(server: EndpointId, key: SecretKey, character: u8, extras: &[u8], nick: &str, build: u32, rejoin: Option<u64>) -> crate::Session {
    let nick = crate::clean_nick(nick);
    let extras = crate::clean_extras(extras);
    crate::spawn(move |endpoint, mut io| {
        Box::pin(async move {
            let (mut character, mut extras, mut nick) = (character, extras, nick);
            let queued = queue(&endpoint, &mut io, server, &key, build, rejoin, &mut character, &mut extras, &mut nick).await?;
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
            let matchmade = Matchmade::of(&found, server)?;
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
    rejoin: Option<u64>,
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
        rejoin,
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
                    Some(Ok(ToClient::Admit { .. })) => {}
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

/// A lobby host's registration at the matchmaker (`ToServer::Host`), as long as this is kept:
/// what the session tells it, and what it says. A registration that fails ends quietly: the lobby
/// only goes without backfill.
pub(crate) struct Registration {
    pub(crate) tell: UnboundedSender<ToServer>,
    pub(crate) heard: UnboundedReceiver<ToClient>,
}

pub(crate) fn register(endpoint: &Endpoint, server: EndpointId, lobby: u64) -> Registration {
    let (tell, mut told) = unbounded_channel::<ToServer>();
    let (heard_tx, heard) = unbounded_channel::<ToClient>();
    let endpoint = endpoint.clone();
    n0_future::task::spawn(async move {
        let Ok(connection) = endpoint.connect(EndpointAddr::new(server), MATCH_ALPN).await else {
            return;
        };
        let Ok((mut send, mut recv)) = connection.open_bi().await else {
            return;
        };
        if write(&mut send, &ToServer::Host { lobby }).await.is_err() {
            return;
        }
        // (A read of its own, as in `queue`.)
        let (frames_tx, mut frames) = unbounded_channel();
        n0_future::task::spawn(async move {
            while let Ok(message) = read_frame(&mut recv).await {
                if frames_tx.send(message).is_err() {
                    break;
                }
            }
        });
        loop {
            tokio::select! {
                message = told.recv() => match message {
                    Some(message) => if write(&mut send, &message).await.is_err() {
                        break;
                    },
                    None => break,
                },
                message = frames.recv() => match message {
                    Some(message) => if heard_tx.send(message).is_err() {
                        break;
                    },
                    None => break,
                },
            }
        }
        connection.close(0u32.into(), b"bye");
    });
    Registration { tell, heard }
}

/// What a match's host tells the matchmaker of its lobby (`ToServer::Open`): `players` with
/// itself first, its home relay with it. (The others' are left out: only the host is dialed by
/// ticket, the heirs by id.)
pub(crate) fn listed(players: &[(EndpointId, String)], relay: Option<String>) -> Vec<MatchPlayer> {
    let mut relay = relay;
    players
        .iter()
        .map(|(id, nick)| MatchPlayer { endpoint: *id.as_bytes(), relay: relay.take(), nick: nick.clone() })
        .collect()
}
