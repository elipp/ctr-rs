//! The loop of a session: one task that takes the commands of the game, the messages of every
//! connection and the clock, and answers with events.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use iroh::endpoint::Connection;
use iroh::{Endpoint, EndpointId};
use tokio::sync::mpsc::unbounded_channel;

use crate::conn::{
    Heard, RACE_IN_PROGRESS, dial, distinct, listen, send_datagram, send_reliably, stamp,
};
use crate::peers::{Departure, Guest, Link, Plan, Role, stalled, start_over};
use crate::wire::{Message, PING_SECONDS};
use crate::{
    ALPN, Command, Error, Event, Io, KICK_AFTER, KICKED, MAX_BLOB, PeerInfo, Player, Ticket, Tick,
    clean_nick,
};

/// How often the packets go out, once per game tick. They are sent whether or not there is
/// news: when a packet is lost and the game then waits for it, the resend is what gets it going.
pub(crate) fn send_interval() -> tokio::time::Interval {
    let mut interval = tokio::time::interval(std::time::Duration::from_millis(1000 / 30));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    interval
}

/// A message on a stream of its own, which iroh delivers whole and in order.
/// The host tells everyone, itself included, who is in the lobby: itself, then the guests that
/// have said what they drive, in the order they joined.
pub(crate) async fn tell_lobby(me: EndpointId, character: u8, nick: &str, guests: &[Guest], io: &Io) {
    let players: Vec<Player> = std::iter::once(Player {
        id: me,
        character,
        nick: nick.to_string(),
    })
    .chain(guests.iter().filter_map(|g| {
        g.hello.map(|(_, character)| Player {
            id: g.id,
            character,
            nick: g.nick.1.clone(),
        })
    }))
    .collect();
    let message = Message::Lobby {
        players: players
            .iter()
            .map(|p| (*p.id.as_bytes(), p.character, p.nick.clone()))
            .collect(),
    };
    for guest in guests {
        let _ = send_reliably(&guest.connection, &message).await;
    }
    io.event(Event::Lobby {
        players,
        you: Some(0),
    });
}

/// The session of a lobby host or of a player who joined one. They differ in the lobby (the
/// host takes the joins and starts the race) and in the barrier before tick 0 (the host collects
/// the [`Message::Ready`]s and sends the [`Message::Go`]); the race is the same for both.
pub(crate) async fn run(
    endpoint: Endpoint,
    mut io: Io,
    role: Role,
    mut character: u8,
    mut nick: String,
    build: u32,
) -> Result<(), Error> {
    let me = endpoint.id();
    let (heard_tx, mut heard) = unbounded_channel::<(EndpointId, Heard)>();
    let (dialed_tx, mut dialed) = unbounded_channel::<Connection>();

    // The lobby. The host has the guests; a guest has the host.
    let mut guests: Vec<Guest> = Vec::new();
    let mut host_connection: Option<Connection> = None;
    let mut hellos = 0;
    // The track of a start that waits for the Hellos.
    let mut start_wanted: Option<(u32, u64)> = None;
    // The custom tracks we can send, and the ones we asked the host for.
    let mut provided: HashMap<(u32, u64), Arc<Vec<u8>>> = HashMap::new();
    let mut fetching: HashSet<(u32, u64)> = HashSet::new();

    match &role {
        Role::Host => {
            tokio::select! {
                _ = &mut io.stop => return Ok(()),
                _ = endpoint.online() => {}
            }
            io.event(Event::Open(Ticket::of(&endpoint.addr())));
        }
        Role::Join(ticket) => {
            let connection: Connection = tokio::select! {
                _ = &mut io.stop => return Ok(()),
                connection = endpoint.connect(ticket.addr(), ALPN) => connection?,
            };
            io.event(Event::Joined(ticket.id));
            let hello = Message::Hello { character, seq: 0, build, nick: nick.clone() };
            send_reliably(&connection, &hello).await?;
            hellos = 1;
            listen(connection.clone(), ticket.id, heard_tx.clone());
            host_connection = Some(connection);
        }
    }
    let host_id = match &role {
        Role::Host => me,
        Role::Join(ticket) => ticket.id,
    };

    // The race.
    let mut plan: Option<Plan> = None;
    let mut links: HashMap<EndpointId, Link> = HashMap::new();
    // Connections from players that are not in the roster (yet): the start may not have come.
    let mut unclaimed: Vec<Connection> = Vec::new();
    let mut game_ready = false;
    let mut ready_sent = false;
    // The host: who is ready.
    let mut ready_from: HashSet<EndpointId> = HashSet::new();
    let mut running = false;
    // The host: a race has been started and the game has not said it is over.
    let mut racing = false;
    // The host: the players of the race that left it. They go from the lobby when the next race
    // starts.
    let mut gone: HashSet<EndpointId> = HashSet::new();
    // The players that left the race on the way (everyone learns of it from the host), and for
    // the host, the ones whose pads are still being settled by slot.
    let mut departed: HashSet<EndpointId> = HashSet::new();
    let mut departures: HashMap<usize, Departure> = HashMap::new();
    // Which race this is, counted from 0, and our slot and the characters in it.
    let mut epoch: u32 = 0;
    let mut race_info: Option<(usize, Vec<u8>, Vec<EndpointId>)> = None;
    let mut timer = send_interval();
    // The last round trip to each player, from the pings.
    let mut pings: HashMap<EndpointId, Duration> = HashMap::new();
    let mut ping_timer = tokio::time::interval(Duration::from_secs(PING_SECONDS));
    ping_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = &mut io.stop => {
                let lobby = guests.iter().map(|g| &g.connection).chain(host_connection.as_ref());
                for connection in lobby.chain(links.values().map(|l| &l.connection)) {
                    connection.close(0u32.into(), b"bye");
                }
                return Ok(());
            }
            incoming = endpoint.accept() => {
                let Some(incoming) = incoming else { return Ok(()) };
                let Ok(accepting) = incoming.accept() else { continue };
                let Ok(connection) = accepting.await else { continue };
                let id = connection.remote_id();
                match &role {
                    // A player who joins in a race waits for the next one.
                    Role::Host => {
                        // A player who comes back to the lobby (it was removed from the race, or
                        // its connection is dead and we have not seen it yet) is a new guest:
                        // the race it was in is lost to it, but the lobby is not.
                        if let Some(old) = guests.iter().find(|g| g.id == id) {
                            old.connection.close(0u32.into(), b"rejoined");
                        }
                        guests.retain(|g| g.id != id);
                        gone.remove(&id);
                        let guest = Guest { id, connection: connection.clone(), hello: None, nick: (0, String::new()) };
                        io.event(Event::PeerJoined(id));
                        if racing {
                            let wait = Message::Announce { text: RACE_IN_PROGRESS.into() };
                            let _ = send_reliably(&connection, &wait).await;
                        }
                        listen(connection, id, heard_tx.clone());
                        guests.push(guest);
                    }
                    // The others of the race connect to us as the roster says.
                    Role::Join(_) => {
                        listen(connection.clone(), id, heard_tx.clone());
                        unclaimed.push(connection);
                    }
                }
            }
            Some(connection) = dialed.recv() => {
                listen(connection.clone(), connection.remote_id(), heard_tx.clone());
                unclaimed.push(connection);
            }
            Some(command) = io.commands.recv() => match command {
                // Goes ahead below, once every guest has said what it drives.
                Command::Start { track, hash } => if matches!(role, Role::Host) {
                    start_wanted = Some((track, hash));
                },
                Command::Provide { track, hash, bytes } => {
                    provided.insert((track, hash), bytes);
                }
                Command::Fetch { track, hash } => if let Some(connection) = &host_connection {
                    fetching.insert((track, hash));
                    send_reliably(connection, &Message::Want { track, hash }).await?;
                },
                // The same players again: new links, and everyone ready before tick 0 once more.
                Command::Restart { .. } if matches!(role, Role::Host) && !departed.is_empty() => {
                    let text = "A player left the race: start a new one instead of restarting".to_string();
                    io.event(Event::Announce(text));
                }
                Command::Restart { track, hash } => if matches!(role, Role::Host)
                    && plan.is_some()
                    && let Some((slot, characters, players)) = race_info.clone()
                {
                    epoch += 1;
                    for guest in &guests {
                        let _ = send_reliably(&guest.connection, &Message::Restart { epoch, track, hash }).await;
                    }
                    start_over(&mut links, epoch);
                    (running, game_ready, ready_sent) = (false, false, false);
                    ready_from.clear();
                    io.event(Event::Started { slot, characters, players, track, hash });
                },
                Command::Stop => if matches!(role, Role::Host) {
                    for guest in &guests {
                        let _ = send_reliably(&guest.connection, &Message::Stop { epoch }).await;
                    }
                },
                Command::Announce(text) => if matches!(role, Role::Host) {
                    for guest in &guests {
                        let _ = send_reliably(&guest.connection, &Message::Announce { text: text.clone() }).await;
                    }
                    io.event(Event::Announce(text));
                },
                Command::Character(c) => if plan.is_none() {
                    character = c;
                    if matches!(role, Role::Host) {
                        tell_lobby(me, character, &nick, &guests, &io).await;
                    }
                    if let Some(connection) = &host_connection {
                        // The host takes the latest Hello it has, so one that is sent again overrides.
                        let hello = Message::Hello { character, seq: hellos, build, nick: nick.clone() };
                        send_reliably(connection, &hello).await?;
                        hellos += 1;
                    }
                },
                // Unlike the character, a nickname can change in a race: the host tells everyone.
                Command::Nickname(new) => if new != nick {
                    nick = new;
                    if matches!(role, Role::Host) {
                        tell_lobby(me, character, &nick, &guests, &io).await;
                    }
                    if let Some(connection) = &host_connection {
                        let message = Message::Nick { seq: hellos, nick: nick.clone() };
                        send_reliably(connection, &message).await?;
                        hellos += 1;
                    }
                },
                Command::Have { slot, first, pads } => match &host_connection {
                    Some(connection) => {
                        let have = Message::Have { epoch, slot: slot as u8, first, pads };
                        send_reliably(connection, &have).await?;
                    }
                    None => if let Some(d) = departures.get_mut(&slot) {
                        d.heard(0, first, pads);
                    },
                },
                Command::Ready => game_ready = true,
                Command::RaceOver => racing = false,
                Command::NetInfo => {
                    let peers = distinct(
                        guests
                            .iter()
                            .map(|g| &g.connection)
                            .chain(&host_connection)
                            .chain(links.values().map(|l| &l.connection))
                            .chain(&unclaimed),
                    )
                    .into_iter()
                    .map(|c| PeerInfo::of(c, pings.get(&c.remote_id()).copied()))
                    .collect();
                    io.event(Event::NetInfo(peers));
                }
                Command::Input(pad) => if running {
                    for link in links.values_mut() {
                        link.sender.push(pad);
                        link.send();
                    }
                },
                Command::Checksum { tick, hash } => if running {
                    let message = Message::Checksum { epoch, tick, hash };
                    for link in links.values() {
                        // On a stream of its own, which the loop does not wait for.
                        let connection = link.connection.clone();
                        let message = message.clone();
                        tokio::spawn(async move {
                            let _ = send_reliably(&connection, &message).await;
                        });
                    }
                },
            },
            Some((from, heard)) = heard.recv() => match heard {
                Heard::Message(message) => match message {
                    Message::Hello { build: theirs_build, .. } if theirs_build != build => {
                        if let Some(guest) = guests.iter().find(|g| g.id == from) {
                            let reject = Message::Reject { host: build, yours: theirs_build };
                            let _ = send_reliably(&guest.connection, &reject).await;
                            guests.retain(|g| g.id != from);
                            let text = format!(
                                "{} was turned away: it plays sim version {theirs_build}, ours is {build}",
                                from.fmt_short()
                            );
                            io.event(Event::Announce(text));
                        }
                    }
                    Message::Reject { host: theirs, yours } if from == host_id => {
                        return Err(format!(
                            "the host plays another version of the sim ({theirs}, yours is {yours}): you both need the same build"
                        ).into());
                    }
                    Message::Hello { character: theirs, seq, nick: theirs_nick, .. } => {
                        if let Some(guest) = guests.iter_mut().find(|g| g.id == from)
                            && guest.hello.is_none_or(|(have, _)| seq > have)
                        {
                            guest.hello = Some((seq, theirs));
                            if seq >= guest.nick.0 {
                                guest.nick = (seq, clean_nick(&theirs_nick));
                            }
                            tell_lobby(me, character, &nick, &guests, &io).await;
                        }
                    }
                    // (It may beat the Hello: the guest is in the lobby once that comes.)
                    Message::Nick { seq, nick: theirs } => if matches!(role, Role::Host)
                        && let Some(guest) = guests.iter_mut().find(|g| g.id == from)
                        && seq > guest.nick.0
                    {
                        guest.nick = (seq, clean_nick(&theirs));
                        tell_lobby(me, character, &nick, &guests, &io).await;
                    },
                    Message::Lobby { players } if from == host_id && !matches!(role, Role::Host) => {
                        let players: Vec<Player> = players
                            .iter()
                            .map(|(bytes, character, nick)| {
                                EndpointId::from_bytes(bytes).map(|id| Player {
                                    id,
                                    character: *character,
                                    nick: clean_nick(nick),
                                })
                            })
                            .collect::<Result<_, _>>()?;
                        let you = players.iter().position(|p| p.id == me);
                        io.event(Event::Lobby { players, you });
                    }
                    Message::Start { epoch: new, slot, characters, track, hash, roster }
                        if from == host_id && !matches!(role, Role::Host) =>
                    {
                        let roster: Vec<EndpointId> = roster
                            .iter()
                            .map(EndpointId::from_bytes)
                            .collect::<Result<_, _>>()?;
                        let slot = slot as usize;
                        epoch = new;
                        departed.clear();
                        // The host is connected already, and so are the players that were in
                        // the race before this one.
                        links.retain(|id, _| roster.contains(id));
                        links.insert(
                            host_id,
                            Link::new(host_connection.clone().expect("a guest has the host"), epoch),
                        );
                        start_over(&mut links, epoch);
                        (running, game_ready, ready_sent) = (false, false, false);
                        // Players connect to the ones before them; the ones after connect to us.
                        for &id in roster.iter().take(slot).skip(1) {
                            if !links.contains_key(&id) {
                                dial(endpoint.clone(), id, dialed_tx.clone());
                            }
                        }
                        race_info = Some((slot, characters.clone(), roster.clone()));
                        plan = Some(Plan { roster: roster.clone() });
                        io.event(Event::Started { slot, characters, players: roster, track, hash });
                    }
                    Message::Restart { epoch: new, track, hash }
                        if plan.is_some() && from == host_id && !matches!(role, Role::Host) =>
                    {
                        if let Some((slot, characters, players)) = race_info.clone() {
                            epoch = new;
                            start_over(&mut links, epoch);
                            (running, game_ready, ready_sent) = (false, false, false);
                            io.event(Event::Started { slot, characters, players, track, hash });
                        }
                    }
                    Message::Announce { text } => {
                        if from == host_id && !matches!(role, Role::Host) {
                            io.event(Event::Announce(text));
                        }
                    }
                    Message::Ready => {
                        if matches!(role, Role::Host) && plan.is_some() {
                            ready_from.insert(from);
                        }
                    }
                    Message::Go => if from == host_id && !running {
                        running = true;
                        io.event(Event::Go);
                    },
                    Message::Inputs(inputs) => {
                        if inputs.epoch != epoch {
                            // The tail of the race before.
                        } else if let (Some(plan), Some(link)) = (&plan, links.get_mut(&from)) {
                            link.peer_has = link.peer_has.max(inputs.ack);
                            let before = link.receiver.next();
                            link.receiver.accept(&inputs).map_err(|gap| {
                                format!("lost pads of player {}: {gap:?}", plan.slot_of(from).unwrap_or(0))
                            })?;
                            let after = link.receiver.next();
                            if after > before {
                                link.advanced = Instant::now();
                                let pads = (before..after)
                                    .map(|t| link.receiver.get(t).expect("just accepted"))
                                    .collect();
                                link.receiver.forget(after);
                                if let Some(slot) = plan.slot_of(from) {
                                    io.event(Event::Pads { slot, first: before, pads });
                                }
                            }
                            if let Some(slot) = plan.slot_of(from) {
                                let ours = link.sender.tick() as i32 - link.receiver.next() as i32;
                                io.event(Event::Lead { slot, ahead: (ours - inputs.lead) as f32 / 2.0 });
                            }
                        }
                    }
                    Message::Checksum { epoch: sent, tick, hash } => {
                        if sent == epoch && let Some(slot) = plan.as_ref().and_then(|plan| plan.slot_of(from))
                            && links.contains_key(&from)
                        {
                            io.event(Event::Checksum { slot, tick, hash });
                        }
                    }
                    // A guest asks for a track the host chose.
                    Message::Want { track, hash } => if matches!(role, Role::Host)
                        && let Some(guest) = guests.iter().find(|g| g.id == from)
                    {
                        let reply = match provided.get(&(track, hash)) {
                            Some(bytes) if bytes.len() <= MAX_BLOB => {
                                Message::Blob { track, hash, bytes: bytes.to_vec() }
                            }
                            _ => Message::NoBlob { track, hash },
                        };
                        // On a stream of its own, which the loop does not wait for: the
                        // guest takes its time to read it.
                        let connection = guest.connection.clone();
                        tokio::spawn(async move {
                            let _ = send_reliably(&connection, &reply).await;
                        });
                    },
                    // Only what we asked for, and only from the host.
                    Message::Blob { track, hash, bytes } => {
                        if from == host_id && fetching.remove(&(track, hash)) {
                            io.event(Event::Blob { track, hash, bytes });
                        }
                    }
                    Message::NoBlob { track, hash } => {
                        if from == host_id && fetching.remove(&(track, hash)) {
                            io.event(Event::BlobMissing { track, hash });
                        }
                    }
                    Message::Gone { epoch: sent, slot } => {
                        if sent == epoch && from == host_id && !matches!(role, Role::Host) {
                            io.event(Event::Gone { slot: slot as usize });
                        }
                    }
                    Message::Have { epoch: sent, slot, first, pads } => {
                        if sent == epoch
                            && matches!(role, Role::Host)
                            && let Some(from_slot) = plan.as_ref().and_then(|plan| plan.slot_of(from))
                            && let Some(d) = departures.get_mut(&(slot as usize))
                        {
                            d.heard(from_slot, first, pads);
                        }
                    }
                    Message::Left { epoch: sent, slot, first, pads } => {
                        if sent == epoch
                            && from == host_id
                            && !matches!(role, Role::Host)
                            && let Some(plan) = &plan
                            && let Some(&id) = plan.roster.get(slot as usize)
                        {
                            departed.insert(id);
                            io.event(Event::Left { slot: slot as usize, first, pads });
                        }
                    }
                    Message::Snapshot { .. }
                    | Message::Reject { .. }
                    | Message::Start { .. }
                    | Message::Restart { .. }
                    | Message::Lobby { .. } => {}
                    Message::Stop { epoch: stopped } => {
                        if stopped == epoch && from == host_id && !matches!(role, Role::Host) {
                            io.event(Event::Stopped);
                        }
                    }
                    // (`listen` takes these.)
                    Message::Ping { .. } | Message::Pong { .. } => {}
                },
                Heard::Ping(rtt) => {
                    pings.insert(from, rtt);
                    io.event(Event::Rtt { id: from, rtt });
                }
                Heard::Closed { id: closed, kicked } => {
                    // The old connection of a player that came back: the new one is in `guests`.
                    let stale = guests
                        .iter()
                        .any(|g| g.id == from && g.connection.stable_id() != closed);
                    if !stale {
                        io.event(Event::PeerLeft(from));
                        pings.remove(&from);
                    }
                    links.remove(&from);
                    unclaimed.retain(|c| c.remote_id() != from);
                    // A guest whose host is gone has no lobby, nor a race: the host runs both.
                    if from == host_id && !matches!(role, Role::Host) {
                        if kicked {
                            return Err(format!(
                                "the host removed you: nothing came from you for {} seconds. \
                                 You can join the lobby again, but not this race",
                                KICK_AFTER.as_secs()
                            ).into());
                        }
                        return Err("the host closed the lobby".into());
                    }
                    if matches!(role, Role::Host) {
                        // A player of the race that is on stays in the roster, which the race
                        // is made of, until the next one.
                        let in_race = racing && plan.as_ref().is_some_and(|p| p.slot_of(from).is_some());
                        if in_race {
                            if !stale {
                                gone.insert(from);
                            }
                            if let Some(slot) = plan.as_ref().and_then(|plan| plan.slot_of(from))
                                && departed.insert(from)
                            {
                                // Nobody is waited for twice, and whoever is gone is not waited for.
                                for d in departures.values_mut() {
                                    d.waiting.remove(&slot);
                                }
                                let plan = plan.as_ref().expect("the slot came from it");
                                let waiting = (0..plan.roster.len())
                                    .filter(|&s| s != slot && !departed.contains(&plan.roster[s]))
                                    .collect();
                                departures.insert(slot, Departure { waiting, first: 0, pads: Vec::new() });
                                let gone = Message::Gone { epoch, slot: slot as u8 };
                                for guest in &guests {
                                    if plan.slot_of(guest.id).is_some() && !departed.contains(&guest.id) {
                                        let _ = send_reliably(&guest.connection, &gone).await;
                                    }
                                }
                                io.event(Event::Gone { slot });
                            }
                        } else if !stale {
                            let had = guests.len();
                            guests.retain(|g| g.id != from);
                            if guests.len() != had {
                                tell_lobby(me, character, &nick, &guests, &io).await;
                            }
                        }
                    }
                }
            },
            _ = ping_timer.tick() => {
                let connections = guests
                    .iter()
                    .map(|g| &g.connection)
                    .chain(&host_connection)
                    .chain(links.values().map(|l| &l.connection))
                    .chain(&unclaimed);
                for connection in distinct(connections) {
                    send_datagram(connection, &Message::Ping { stamp: stamp() });
                }
            }
            _ = timer.tick() => if running {
                for link in links.values_mut() {
                    link.send();
                }
                if matches!(role, Role::Host) && racing {
                    let own = links.values().next().map(|link| link.sender.tick());
                    let heard: Vec<(EndpointId, Tick, Instant)> = links
                        .iter()
                        .filter(|(id, _)| !departed.contains(*id))
                        .map(|(&id, link)| (id, link.receiver.next(), link.advanced))
                        .collect();
                    let silent = own.map_or_else(Vec::new, |own| stalled(own, &heard, Instant::now()));
                    for id in silent {
                        // Closing the connection is what a player leaving does: the rest follows
                        // from `Heard::Closed`. (The link goes now, or the next tick would do this
                        // again.)
                        links.remove(&id);
                        let Some(guest) = guests.iter().find(|g| g.id == id) else { continue };
                        guest.connection.close(KICKED.into(), b"no pads");
                        let who = if guest.nick.1.is_empty() {
                            id.fmt_short().to_string()
                        } else {
                            guest.nick.1.clone()
                        };
                        let text = format!(
                            "{who} was removed from the race: nothing came from them for {} seconds",
                            KICK_AFTER.as_secs()
                        );
                        for other in guests.iter().filter(|g| g.id != id) {
                            let _ = send_reliably(&other.connection, &Message::Announce { text: text.clone() }).await;
                        }
                        io.event(Event::Announce(text));
                    }
                }
            },
        }

        // The host: once everyone said what it holds of a player's pads that left, they all get
        // the same.
        if matches!(role, Role::Host)
            && let Some(plan) = &plan
        {
            let settled: Vec<usize> = departures
                .iter()
                .filter(|(_, d)| d.waiting.is_empty())
                .map(|(&slot, _)| slot)
                .collect();
            for slot in settled {
                let Some(d) = departures.remove(&slot) else { continue };
                let left = Message::Left { epoch, slot: slot as u8, first: d.first, pads: d.pads.clone() };
                for guest in &guests {
                    if plan.slot_of(guest.id).is_some() && !departed.contains(&guest.id) {
                        let _ = send_reliably(&guest.connection, &left).await;
                    }
                }
                io.event(Event::Left { slot, first: d.first, pads: d.pads });
            }
        }

        // The host starts the race once every guest has said what it drives.
        if let Some((track, hash)) =
            start_wanted.filter(|_| guests.iter().all(|g| g.hello.is_some()))
        {
            start_wanted = None;
            // A race after another is a new one, with the players who are here now.
            guests.retain(|g| !gone.contains(&g.id));
            gone.clear();
            if plan.is_some() {
                epoch += 1;
            }
            departed.clear();
            departures.clear();
            let characters: Vec<u8> = std::iter::once(character)
                .chain(guests.iter().filter_map(|g| g.hello.map(|(_, c)| c)))
                .collect();
            let roster: Vec<EndpointId> = std::iter::once(me)
                .chain(guests.iter().map(|g| g.id))
                .collect();
            for (i, guest) in guests.iter().enumerate() {
                let start = Message::Start {
                    epoch,
                    slot: i as u8 + 1,
                    characters: characters.clone(),
                    track,
                    hash,
                    roster: roster.iter().map(|id| *id.as_bytes()).collect(),
                };
                let _ = send_reliably(&guest.connection, &start).await;
                links
                    .entry(guest.id)
                    .or_insert_with(|| Link::new(guest.connection.clone(), epoch));
            }
            links.retain(|id, _| roster.contains(id));
            start_over(&mut links, epoch);
            (running, game_ready, ready_sent) = (false, false, false);
            ready_from.clear();
            racing = true;
            race_info = Some((0, characters.clone(), roster.clone()));
            plan = Some(Plan {
                roster: roster.clone(),
            });
            io.event(Event::Started {
                slot: 0,
                characters,
                players: roster,
                track,
                hash,
            });
        }

        // Connections that the roster knows are links now.
        if let Some(plan) = &plan {
            unclaimed.retain(|connection| {
                let id = connection.remote_id();
                if plan.slot_of(id).is_none() {
                    return true;
                }
                links
                    .entry(id)
                    .or_insert_with(|| Link::new(connection.clone(), epoch));
                false
            });
        }

        // Connected to every other player, with the game ready: tell the host.
        if let Some(plan) = &plan
            && game_ready
            && !ready_sent
            && plan
                .roster
                .iter()
                .all(|&id| id == me || links.contains_key(&id) || departed.contains(&id))
        {
            ready_sent = true;
            match &host_connection {
                Some(connection) => send_reliably(connection, &Message::Ready).await?,
                None => {
                    ready_from.insert(me);
                }
            }
        }
        // The host: everyone is ready, so this is tick 0.
        if matches!(role, Role::Host)
            && !running
            && let Some(plan) = &plan
            && plan
                .roster
                .iter()
                .all(|id| ready_from.contains(id) || departed.contains(id))
        {
            running = true;
            for link in links.values_mut() {
                link.advanced = Instant::now();
            }
            for guest in &guests {
                let _ = send_reliably(&guest.connection, &Message::Go).await;
            }
            io.event(Event::Go);
        }
    }
}
