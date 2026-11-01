//! The loop of a session: one task that takes the commands of the game, the messages of every
//! connection and the clock, and answers with events.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use iroh::endpoint::Connection;
use iroh::{Endpoint, EndpointAddr, EndpointId};
use n0_future::time::{Instant, Interval, MissedTickBehavior, interval, sleep, timeout};
use tokio::sync::mpsc::unbounded_channel;

use crate::conn::{
    Heard, RACE_IN_PROGRESS, dial, distinct, listen, send_datagram, send_reliably, stamp,
};
use crate::peers::{Departure, Guest, HostState, Heirs, Link, Plan, Role, stalled, start_over};
use crate::realm::Publisher;
use crate::transfers::{Transfer, Verdict};
use crate::wire::{Message, PING_SECONDS};
use crate::{
    ALPN, Command, Error, Event, Io, KICK_AFTER, KICKED, MAX_BLOB, PeerInfo, Player, Public, Tick,
    Ticket, RateLimit, clean_chat, clean_nick,
};

/// How long a guest tries to reach the player that would host the lobby in the place of a host
/// that left, before it goes on to the next one.
const HEIR_DIAL_TIMEOUT: Duration = Duration::from_secs(5);

/// How often the packets go out, once per game tick. They are sent whether or not there is
/// news: when a packet is lost and the game then waits for it, the resend is what gets it going.
pub(crate) fn send_interval() -> Interval {
    let mut interval = interval(Duration::from_millis(1000 / 30));
    interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
    interval
}

/// A message on a stream of its own, which iroh delivers whole and in order.
/// The host tells everyone, itself included, who is in the lobby: itself, then the guests that
/// have said what they drive, in the order they joined. Whoever left (`gone`) during the race
/// that is on, or just ended, is not in it, though the race still holds their place in `guests`.
pub(crate) async fn tell_lobby(
    me: EndpointId,
    character: u8,
    nick: &str,
    host: &HostState,
    io: &Io,
) {
    let guests: Vec<&Guest> = host.guests.iter().filter(|g| !host.gone.contains(&g.id)).collect();
    let players: Vec<Player> = std::iter::once(Player {
        id: me,
        character,
        nick: nick.to_string(),
    })
    .chain(guests.iter().filter(|g| !g.spectator).filter_map(|g| {
        g.hello.map(|(_, character)| Player {
            id: g.id,
            character,
            nick: g.nick.1.clone(),
        })
    }))
    .collect();
    let spectators: Vec<Player> = guests
        .iter()
        .filter(|g| g.spectator && g.hello.is_some())
        .map(|g| Player {
            id: g.id,
            character: 0,
            nick: g.nick.1.clone(),
        })
        .collect();
    let message = Message::Lobby {
        players: players
            .iter()
            .map(|p| (*p.id.as_bytes(), p.character, p.nick.clone()))
            .collect(),
        watching: spectators
            .iter()
            .map(|p| (*p.id.as_bytes(), p.nick.clone()))
            .collect(),
        public: host
            .public
            .as_ref()
            .map(|p| (*p.realm.as_bytes(), p.name.clone(), p.track.clone())),
    };
    for guest in &guests {
        let _ = send_reliably(&guest.connection, &message).await;
    }
    io.event(Event::Lobby {
        players,
        spectators,
        you: Some(0),
    });
}

/// Tells the realm how the lobby stands now (it sends only what changed).
fn refresh_listing(host: &HostState, build: u32) {
    if let Some(publisher) = &host.publisher {
        // Us, and the guests that drive (a spectator takes no place, nor does a guest that has not
        // said what it drives).
        let driving = host
            .guests
            .iter()
            .filter(|g| !g.spectator && g.hello.is_some() && !host.gone.contains(&g.id))
            .count();
        publisher.refresh((1 + driving).min(u8::MAX as usize) as u8, host.racing, build);
    }
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
    mut spectator: bool,
) -> Result<(), Error> {
    let me = endpoint.id();
    let (heard_tx, mut heard) = unbounded_channel::<(EndpointId, Heard)>();
    let (dialed_tx, mut dialed) = unbounded_channel::<Connection>();

    // The lobby. The host has the guests (in `host`); a guest has the host.
    let mut host: Option<HostState> = None;
    let mut host_connection: Option<Connection> = None;
    let mut hellos = 0;
    // The custom tracks we can send, and the ones we asked the host for.
    let mut provided: HashMap<(u32, u64), Arc<Vec<u8>>> = HashMap::new();
    let mut fetching: HashSet<(u32, u64)> = HashSet::new();
    // The host: the sends of maps that ended.
    let (sent_tx, mut sent) = unbounded_channel::<Transfer>();

    match &role {
        Role::Host => {
            tokio::select! {
                _ = &mut io.stop => return Ok(()),
                _ = endpoint.online() => {}
            }
            io.event(Event::Open(Ticket::of(&endpoint.addr())));
            // The lobby has us in it from the start.
            let state = HostState::default();
            tell_lobby(me, character, &nick, &state, &io).await;
            host = Some(state);
        }
        Role::Join(ticket) => {
            let connection: Connection = tokio::select! {
                _ = &mut io.stop => return Ok(()),
                connection = endpoint.connect(ticket.addr(), ALPN) => connection?,
            };
            io.event(Event::Joined(ticket.id));
            let hello = Message::Hello {
                character,
                seq: 0,
                build,
                nick: nick.clone(),
                spectator,
            };
            send_reliably(&connection, &hello).await?;
            hellos = 1;
            listen(connection.clone(), ticket.id, heard_tx.clone());
            host_connection = Some(connection);
        }
    }
    let mut host_id = match &role {
        Role::Host => me,
        Role::Join(ticket) => ticket.id,
    };

    // A guest: what it would carry the lobby on with if the host left, and whether a race is on
    // (which it cannot yet: see `Heard::Closed`).
    let mut heirs = Heirs::default();
    let mut race_on = false;
    let (reached_tx, mut reached) = unbounded_channel::<(EndpointId, Option<Connection>)>();

    // The race.
    let mut plan: Option<Plan> = None;
    let mut links: HashMap<EndpointId, Link> = HashMap::new();
    // Connections from players that are not in the roster (yet): the start may not have come.
    let mut unclaimed: Vec<Connection> = Vec::new();
    let mut game_ready = false;
    let mut ready_sent = false;
    let mut running = false;
    // The players that left the race on the way (everyone learns of it from the host).
    let mut departed: HashSet<EndpointId> = HashSet::new();
    // Which race this is, counted from 0, and our slot and the characters in it.
    let mut epoch: u32 = 0;
    let mut race_info: Option<(usize, Vec<u8>, Vec<EndpointId>)> = None;
    let mut timer = send_interval();
    // The last round trip to each player, from the pings.
    let mut pings: HashMap<EndpointId, Duration> = HashMap::new();
    let mut ping_timer = interval(Duration::from_secs(PING_SECONDS));
    ping_timer.set_missed_tick_behavior(MissedTickBehavior::Delay);
    // The host: how often the listing of the lobby in the realm is checked for news (the guests
    // come and go in many places).
    let mut list_timer = interval(Duration::from_secs(1));
    list_timer.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = &mut io.stop => {
                let lobby = host.iter().flat_map(|h| &h.guests).map(|g| &g.connection).chain(host_connection.as_ref());
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
                match &mut host {
                    // A player who joins in a race waits for the next one.
                    Some(host) => {
                        // A player who comes back to the lobby (it was removed from the race, or
                        // its connection is dead and we have not seen it yet) is a new guest:
                        // the race it was in is lost to it, but the lobby is not.
                        if let Some(old) = host.guests.iter().find(|g| g.id == id) {
                            old.connection.close(0u32.into(), b"rejoined");
                        }
                        host.guests.retain(|g| g.id != id);
                        host.gone.remove(&id);
                        let guest = Guest { id, connection: connection.clone(), hello: None, nick: (0, String::new()), spectator: false };
                        io.event(Event::PeerJoined(id));
                        if host.racing {
                            let wait = Message::Announce { text: RACE_IN_PROGRESS.into() };
                            let _ = send_reliably(&connection, &wait).await;
                        }
                        listen(connection, id, heard_tx.clone());
                        host.guests.push(guest);
                    }
                    // The others of the race connect to us as the roster says.
                    None => {
                        listen(connection.clone(), id, heard_tx.clone());
                        unclaimed.push(connection);
                    }
                }
            }
            Some(transfer) = sent.recv() => if let Some(host) = &mut host {
                host.transfers.finish(&transfer);
            },
            _ = list_timer.tick(), if host.as_ref().is_some_and(|h| h.publisher.is_some()) => {
                if let Some(host) = &host {
                    refresh_listing(host, build);
                }
            }
            Some((candidate, connection)) = reached.recv() => {
                // The heir we dialed is the host now (as far as we know), or could not be reached.
                if heirs.dialing == Some(candidate) {
                    heirs.dialing = None;
                    match connection {
                        Some(connection) => {
                            listen(connection.clone(), candidate, heard_tx.clone());
                            host_id = candidate;
                            host_connection = Some(connection.clone());
                            io.event(Event::Joined(candidate));
                            // As if we joined it: the host takes the latest Hello it has.
                            let hello = Message::Hello { character, seq: hellos, build, nick: nick.clone(), spectator };
                            let _ = send_reliably(&connection, &hello).await;
                            hellos += 1;
                        }
                        None => {
                            heirs.dead.insert(candidate);
                            heirs.searching = true;
                        }
                    }
                }
            }
            Some(connection) = dialed.recv() => {
                listen(connection.clone(), connection.remote_id(), heard_tx.clone());
                unclaimed.push(connection);
            }
            Some(command) = io.commands.recv() => match command {
                // Goes ahead below, once every guest has said what it drives.
                Command::Start { track, hash, laps } => if let Some(host) = &mut host {
                    host.start_wanted = Some((track, hash, laps));
                },
                Command::Provide { track, hash, bytes } => {
                    provided.insert((track, hash), bytes);
                }
                // Once: the host does not answer a second ask for a map that is on its way.
                Command::Fetch { track, hash } => if let Some(connection) = &host_connection
                    && fetching.insert((track, hash))
                {
                    send_reliably(connection, &Message::Want { track, hash }).await?;
                },
                // The same players again: new links, and everyone ready before tick 0 once more.
                Command::Restart { .. } if host.is_some() && !departed.is_empty() => {
                    let text = "A player left the race: start a new one instead of restarting".to_string();
                    io.event(Event::Announce(text));
                }
                Command::Restart { track, hash, laps } => if plan.is_some()
                    && let Some(host) = &mut host
                    && let Some((slot, characters, players)) = race_info.clone()
                {
                    epoch += 1;
                    for guest in &host.guests {
                        let _ = send_reliably(&guest.connection, &Message::Restart { epoch, track, hash, laps }).await;
                    }
                    start_over(&mut links, epoch);
                    (running, game_ready, ready_sent) = (false, false, false);
                    host.ready_from.clear();
                    io.event(Event::Started { slot, characters, players, track, hash, laps });
                },
                Command::Stop => if let Some(host) = &host {
                    for guest in &host.guests {
                        let _ = send_reliably(&guest.connection, &Message::Stop { epoch }).await;
                    }
                },
                Command::Announce(text) => if let Some(host) = &host {
                    for guest in &host.guests {
                        let _ = send_reliably(&guest.connection, &Message::Announce { text: text.clone() }).await;
                    }
                    io.event(Event::Announce(text));
                },
                Command::Say(text) => if !text.is_empty() {
                    if let Some(host) = &host {
                        let chat = Message::Chat { from: *me.as_bytes(), text: text.clone() };
                        for guest in &host.guests {
                            let _ = send_reliably(&guest.connection, &chat).await;
                        }
                        io.event(Event::Chat { from: me, text });
                    } else if let Some(connection) = &host_connection {
                        let _ = send_reliably(connection, &Message::Say { text }).await;
                    }
                },
                Command::Character(c) => if !host.as_ref().is_some_and(|h| h.racing) {
                    character = c;
                    if let Some(host) = &host {
                        tell_lobby(me, character, &nick, host, &io).await;
                    }
                    if let Some(connection) = &host_connection {
                        // The host takes the latest Hello it has, so one that is sent again overrides.
                        let hello = Message::Hello { character, seq: hellos, build, nick: nick.clone(), spectator };
                        send_reliably(connection, &hello).await?;
                        hellos += 1;
                    }
                },
                Command::Spectate(watch) => if watch != spectator && let Some(connection) = &host_connection {
                    spectator = watch;
                    let hello = Message::Hello { character, seq: hellos, build, nick: nick.clone(), spectator };
                    send_reliably(connection, &hello).await?;
                    hellos += 1;
                },
                // Unlike the character, a nickname can change in a race: the host tells everyone.
                Command::Nickname(new) => if new != nick {
                    nick = new;
                    if let Some(host) = &host {
                        tell_lobby(me, character, &nick, host, &io).await;
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
                    None => if let Some(host) = &mut host
                        && let Some(d) = host.departures.get_mut(&slot)
                    {
                        d.heard(0, first, pads);
                    },
                },
                Command::Ready => game_ready = true,
                Command::RaceOver => {
                    race_on = false;
                    if let Some(host) = &mut host {
                        host.racing = false;
                        // Whoever left during the race is out of the lobby now that it is over.
                        if !host.gone.is_empty() {
                            tell_lobby(me, character, &nick, host, &io).await;
                        }
                    }
                }
                Command::Public(public) => if let Some(host) = &mut host {
                    host.public = public.clone();
                    match public {
                        Some(public) => {
                            if host.publisher.as_ref().is_none_or(|p| p.realm != public.realm) {
                                host.publisher = Some(Publisher::start(&endpoint, public.realm, io.events.clone()));
                            }
                            if let Some(publisher) = &mut host.publisher {
                                publisher.set(Some(public));
                            }
                        }
                        None => if let Some(publisher) = &mut host.publisher {
                            publisher.set(None);
                        },
                    }
                    refresh_listing(host, build);
                    // The guests keep how it is listed, for one of them to carry on with.
                    tell_lobby(me, character, &nick, host, &io).await;
                },
                Command::NetInfo => {
                    let peers = distinct(
                        host.iter()
                            .flat_map(|h| &h.guests)
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
                        n0_future::task::spawn(async move {
                            let _ = send_reliably(&connection, &message).await;
                        });
                    }
                },
            },
            Some((from, heard)) = heard.recv() => match heard {
                Heard::Message(message) => match message {
                    // A player that took us for the host: it was right, or will be (we may not
                    // have found out that the host left yet). Kept for then.
                    message @ (Message::Hello { .. } | Message::Nick { .. }) if host.is_none() => {
                        if from != host_id && heirs.heard.len() < 64 {
                            heirs.heard.push((from, message));
                        }
                    }
                    Message::Hello { build: theirs_build, .. } if theirs_build != build => {
                        if let Some(host) = &mut host
                            && let Some(guest) = host.guests.iter().find(|g| g.id == from)
                        {
                            let reject = Message::Reject { host: build, yours: theirs_build };
                            let _ = send_reliably(&guest.connection, &reject).await;
                            host.guests.retain(|g| g.id != from);
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
                    Message::Hello { character: theirs, seq, nick: theirs_nick, spectator: watches, .. } => {
                        if let Some(host) = &mut host
                            && let Some(guest) = host.guests.iter_mut().find(|g| g.id == from)
                            && guest.hello.is_none_or(|(have, _)| seq > have)
                        {
                            guest.hello = Some((seq, theirs));
                            guest.spectator = watches;
                            if seq >= guest.nick.0 {
                                guest.nick = (seq, clean_nick(&theirs_nick));
                            }
                            tell_lobby(me, character, &nick, host, &io).await;
                        }
                    }
                    // (It may beat the Hello: the guest is in the lobby once that comes.)
                    Message::Nick { seq, nick: theirs } => if let Some(host) = &mut host
                        && let Some(guest) = host.guests.iter_mut().find(|g| g.id == from)
                        && seq > guest.nick.0
                    {
                        guest.nick = (seq, clean_nick(&theirs));
                        tell_lobby(me, character, &nick, host, &io).await;
                    },
                    Message::Lobby { players, watching, public } if from == host_id && host.is_none() => {
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
                        let spectators: Vec<Player> = watching
                            .iter()
                            .map(|(bytes, nick)| {
                                EndpointId::from_bytes(bytes).map(|id| Player {
                                    id,
                                    character: 0,
                                    nick: clean_nick(nick),
                                })
                            })
                            .collect::<Result<_, _>>()?;
                        let you = players.iter().position(|p| p.id == me);
                        // The lobby is in the hands of a host that is there: what we kept of the
                        // ones before it, and what players told us in its place, is over.
                        heirs.order = players.iter().map(|p| p.id).collect();
                        heirs.public = public.and_then(|(realm, name, track)| {
                            Some(Public { realm: EndpointId::from_bytes(&realm).ok()?, name, track })
                        });
                        heirs.dead.clear();
                        heirs.heard.clear();
                        io.event(Event::Lobby { players, spectators, you });
                    }
                    Message::Start { epoch: new, slot, characters, track, hash, laps, roster }
                        if from == host_id && host.is_none() =>
                    {
                        let roster: Vec<EndpointId> = roster
                            .iter()
                            .map(EndpointId::from_bytes)
                            .collect::<Result<_, _>>()?;
                        let slot = slot as usize;
                        epoch = new;
                        race_on = true;
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
                        plan = Some(Plan { roster: roster.clone(), players: characters.len() });
                        io.event(Event::Started { slot, characters, players: roster, track, hash, laps });
                    }
                    Message::Restart { epoch: new, track, hash, laps }
                        if plan.is_some() && from == host_id && host.is_none() =>
                    {
                        if let Some((slot, characters, players)) = race_info.clone() {
                            epoch = new;
                            start_over(&mut links, epoch);
                            (running, game_ready, ready_sent) = (false, false, false);
                            io.event(Event::Started { slot, characters, players, track, hash, laps });
                        }
                    }
                    Message::Announce { text } => {
                        if from == host_id && host.is_none() {
                            io.event(Event::Announce(text));
                        }
                    }
                    // Only a guest of the lobby speaks, and the host says who it was.
                    // (One who talks too fast is not heard.)
                    Message::Say { text } => if let Some(host) = &mut host
                        && host.guests.iter().any(|g| g.id == from)
                        && host.talkers
                            .entry(from)
                            .or_insert_with(|| RateLimit::chat_for_host(Instant::now()))
                            .allow(Instant::now())
                    {
                        let text = clean_chat(&text);
                        if !text.is_empty() {
                            let chat = Message::Chat { from: *from.as_bytes(), text: text.clone() };
                            for guest in &host.guests {
                                let _ = send_reliably(&guest.connection, &chat).await;
                            }
                            io.event(Event::Chat { from, text });
                        }
                    },
                    Message::Chat { from: speaker, text } => {
                        if from == host_id
                            && host.is_none()
                            && let Ok(speaker) = EndpointId::from_bytes(&speaker)
                        {
                            let text = clean_chat(&text);
                            if !text.is_empty() {
                                io.event(Event::Chat { from: speaker, text });
                            }
                        }
                    }
                    Message::Ready => {
                        if let Some(host) = &mut host && plan.is_some() {
                            host.ready_from.insert(from);
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
                            // (A spectator makes no pads, so its clock says nothing.)
                            if let Some(slot) = plan.slot_of(from).filter(|_| !plan.is_spectator(from)) {
                                let ours = link.sender.tick() as i32 - link.receiver.next() as i32;
                                io.event(Event::Lead { slot, ahead: (ours - inputs.lead) as f32 / 2.0 });
                            }
                        }
                    }
                    Message::Checksum { epoch: sent, tick, hash } => {
                        let slot = plan.as_ref().and_then(|plan| plan.slot_of(from));
                        let linked = links.contains_key(&from);
                        match slot {
                            Some(slot) if sent == epoch && linked => {
                                io.event(Event::Checksum { slot, tick, hash });
                            }
                            _ => io.event(Event::ChecksumDropped {
                                from,
                                tick,
                                sent,
                                epoch,
                                planned: slot.is_some(),
                                linked,
                            }),
                        }
                    }
                    // A guest asks for a track the host chose.
                    Message::Want { track, hash } => if let Some(host) = &mut host
                        && let Some(guest) = host.guests.iter().find(|g| g.id == from)
                    {
                        let transfer = (from, track, hash);
                        let reply = match provided.get(&(track, hash)) {
                            Some(bytes) if bytes.len() <= MAX_BLOB => {
                                match host.transfers.begin(transfer) {
                                    Verdict::Send => Some(Message::Blob { track, hash, bytes: bytes.to_vec() }),
                                    Verdict::Duplicate => None,
                                    Verdict::Busy => Some(Message::NoBlob { track, hash }),
                                }
                            }
                            _ => Some(Message::NoBlob { track, hash }),
                        };
                        if let Some(reply) = reply {
                            // On a stream of its own, which the loop does not wait for: the
                            // guest takes its time to read it.
                            let connection = guest.connection.clone();
                            let sent_tx = sent_tx.clone();
                            n0_future::task::spawn(async move {
                                let _ = send_reliably(&connection, &reply).await;
                                let _ = sent_tx.send(transfer);
                            });
                        }
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
                        if sent == epoch && from == host_id && host.is_none() {
                            io.event(Event::Gone { slot: slot as usize });
                        }
                    }
                    Message::Have { epoch: sent, slot, first, pads } => {
                        if sent == epoch
                            && let Some(host) = &mut host
                            && let Some(from_slot) = plan.as_ref().and_then(|plan| plan.slot_of(from))
                            && let Some(d) = host.departures.get_mut(&(slot as usize))
                        {
                            d.heard(from_slot, first, pads);
                        }
                    }
                    Message::Left { epoch: sent, slot, first, pads } => {
                        if sent == epoch
                            && from == host_id
                            && host.is_none()
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
                        if stopped == epoch && from == host_id && host.is_none() {
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
                    let stale = host
                        .iter()
                        .flat_map(|h| &h.guests)
                        .any(|g| g.id == from && g.connection.stable_id() != closed);
                    if !stale {
                        io.event(Event::PeerLeft(from));
                        pings.remove(&from);
                    }
                    links.remove(&from);
                    unclaimed.retain(|c| c.remote_id() != from);
                    // A guest whose host is gone has no lobby, nor a race: the host runs both.
                    if from == host_id
                        && host.is_none()
                        && host_connection.as_ref().is_some_and(|c| c.stable_id() == closed)
                    {
                        if kicked {
                            return Err(format!(
                                "the host removed you: nothing came from you for {} seconds. \
                                 You can join the lobby again, but not this race",
                                KICK_AFTER.as_secs()
                            ).into());
                        }
                        // A race has the host to settle the pads of whoever leaves it: it cannot
                        // go on without one (yet), so it is stopped, as if the host had.
                        // The lobby after it goes on with the next player in the order it was
                        // told in.
                        if race_on {
                            race_on = false;
                            running = false;
                            departed.clear();
                            io.event(Event::Stopped);
                            io.event(Event::Announce("The host left the race".into()));
                        } else {
                            io.event(Event::Announce("The host left the lobby".into()));
                        }
                        host_connection = None;
                        fetching.clear();
                        heirs.dead.insert(from);
                        heirs.searching = true;
                    }
                    if let Some(host) = &mut host {
                        // A player of the race that is on stays in the roster, which the race
                        // is made of, until the next one.
                        let in_race = host.racing && plan.as_ref().is_some_and(|p| p.slot_of(from).is_some());
                        if in_race {
                            if !stale {
                                host.gone.insert(from);
                            }
                            if let Some(slot) = plan.as_ref().and_then(|plan| plan.slot_of(from))
                                && departed.insert(from)
                            {
                                // Nobody is waited for twice, and whoever is gone is not waited for.
                                for d in host.departures.values_mut() {
                                    d.waiting.remove(&slot);
                                }
                                let plan = plan.as_ref().expect("the slot came from it");
                                let waiting = (0..plan.roster.len())
                                    .filter(|&s| s != slot && !departed.contains(&plan.roster[s]))
                                    .collect();
                                host.departures.insert(slot, Departure { waiting, first: 0, pads: Vec::new() });
                                let gone = Message::Gone { epoch, slot: slot as u8 };
                                for guest in &host.guests {
                                    if plan.slot_of(guest.id).is_some() && !departed.contains(&guest.id) {
                                        let _ = send_reliably(&guest.connection, &gone).await;
                                    }
                                }
                                io.event(Event::Gone { slot });
                            }
                        } else if !stale {
                            let had = host.guests.len();
                            host.guests.retain(|g| g.id != from);
                            if host.guests.len() != had {
                                tell_lobby(me, character, &nick, host, &io).await;
                            }
                        }
                    }
                }
            },
            _ = ping_timer.tick() => {
                let connections = host
                    .iter()
                    .flat_map(|h| &h.guests)
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
                if let Some(host) = &mut host
                    && host.racing
                {
                    let own = links.values().next().map(|link| link.sender.tick());
                    let heard: Vec<(EndpointId, Tick, Instant)> = links
                        .iter()
                        .filter(|(id, _)| !departed.contains(*id))
                        .filter(|(id, _)| plan.as_ref().is_none_or(|plan| !plan.is_spectator(**id)))
                        .map(|(&id, link)| (id, link.receiver.next(), link.advanced))
                        .collect();
                    let silent = own.map_or_else(Vec::new, |own| stalled(own, &heard, Instant::now()));
                    for id in silent {
                        // Closing the connection is what a player leaving does: the rest follows
                        // from `Heard::Closed`. (The link goes now, or the next tick would do this
                        // again.)
                        links.remove(&id);
                        let Some(guest) = host.guests.iter().find(|g| g.id == id) else { continue };
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
                        for other in host.guests.iter().filter(|g| g.id != id) {
                            let _ = send_reliably(&other.connection, &Message::Announce { text: text.clone() }).await;
                        }
                        io.event(Event::Announce(text));
                    }
                }
            },
        }

        // A guest whose host is gone: the next player of the order hosts, and the others go to it.
        // (TODO: two hosts, should a guest lose its way to an heir that is up and the rest not;
        // the one later in the order should step down for the earlier when it hears of it.)
        if heirs.searching && host.is_none() {
            heirs.searching = false;
            match heirs.next_host() {
                None => return Err("the host closed the lobby".into()),
                Some(id) if id == me => {
                    let mut state = HostState {
                        public: heirs.public.clone(),
                        ..HostState::default()
                    };
                    // The players that dialed us before this are the guests, once they say who
                    // they are (what they said is below).
                    for connection in unclaimed.drain(..) {
                        let id = connection.remote_id();
                        if id == me || state.guests.iter().any(|g| g.id == id) {
                            continue;
                        }
                        io.event(Event::PeerJoined(id));
                        state.guests.push(Guest { id, connection, hello: None, nick: (0, String::new()), spectator: false });
                    }
                    if let Some(public) = &state.public {
                        let mut publisher = Publisher::start(&endpoint, public.realm, io.events.clone());
                        publisher.set(Some(public.clone()));
                        state.publisher = Some(publisher);
                    }
                    // Nothing of the race before is carried over: the next start is a new one.
                    host_connection = None;
                    host_id = me;
                    links.clear();
                    (plan, race_info) = (None, None);
                    departed.clear();
                    epoch += 1;
                    (running, game_ready, ready_sent) = (false, false, false);
                    heirs.dialing = None;
                    io.event(Event::Promoted {
                        ticket: Ticket::of(&endpoint.addr()),
                        public: state.public.clone(),
                    });
                    tell_lobby(me, character, &nick, &state, &io).await;
                    refresh_listing(&state, build);
                    host = Some(state);
                    for (from, message) in std::mem::take(&mut heirs.heard) {
                        let _ = heard_tx.send((from, Heard::Message(message)));
                    }
                }
                Some(id) => {
                    heirs.dialing = Some(id);
                    let (endpoint, reached) = (endpoint.clone(), reached_tx.clone());
                    n0_future::task::spawn(async move {
                        // A failed try is not the heir being gone: its address may not be found
                        // yet (as in `dial`), so it is tried again until the time is up.
                        let deadline = Instant::now() + HEIR_DIAL_TIMEOUT;
                        let mut connection = None;
                        while Instant::now() < deadline {
                            let connect = endpoint.connect(EndpointAddr::new(id), ALPN);
                            match timeout(deadline.saturating_duration_since(Instant::now()), connect).await {
                                Ok(Ok(c)) => {
                                    connection = Some(c);
                                    break;
                                }
                                Ok(Err(_)) => sleep(Duration::from_millis(500)).await,
                                Err(_) => break,
                            }
                        }
                        let _ = reached.send((id, connection));
                    });
                }
            }
        }

        // The host: once everyone said what it holds of a player's pads that left, they all get
        // the same.
        if let Some(host) = &mut host
            && let Some(plan) = &plan
        {
            let settled: Vec<usize> = host
                .departures
                .iter()
                .filter(|(_, d)| d.waiting.is_empty())
                .map(|(&slot, _)| slot)
                .collect();
            for slot in settled {
                let Some(d) = host.departures.remove(&slot) else {
                    continue;
                };
                let left = Message::Left {
                    epoch,
                    slot: slot as u8,
                    first: d.first,
                    pads: d.pads.clone(),
                };
                for guest in &host.guests {
                    if plan.slot_of(guest.id).is_some() && !departed.contains(&guest.id) {
                        let _ = send_reliably(&guest.connection, &left).await;
                    }
                }
                io.event(Event::Left {
                    slot,
                    first: d.first,
                    pads: d.pads,
                });
            }
        }

        // The host starts the race once every guest has said what it drives.
        if let Some(host) = &mut host
            && let Some((track, hash, laps)) = host
                .start_wanted
                .filter(|_| host.guests.iter().all(|g| g.hello.is_some()))
        {
            host.start_wanted = None;
            // A race after another is a new one, with the players who are here now.
            host.guests.retain(|g| !host.gone.contains(&g.id));
            host.gone.clear();
            if plan.is_some() {
                epoch += 1;
            }
            departed.clear();
            host.departures.clear();
            // The players take the slots after ours, in the order they joined, and the spectators
            // the ones after them.
            let mut ordered: Vec<&Guest> = host.guests.iter().filter(|g| !g.spectator).collect();
            let players = 1 + ordered.len();
            ordered.extend(host.guests.iter().filter(|g| g.spectator));
            let characters: Vec<u8> = std::iter::once(character)
                .chain(
                    ordered
                        .iter()
                        .take(players - 1)
                        .filter_map(|g| g.hello.map(|(_, c)| c)),
                )
                .collect();
            let roster: Vec<EndpointId> = std::iter::once(me)
                .chain(ordered.iter().map(|g| g.id))
                .collect();
            for (i, guest) in ordered.iter().enumerate() {
                let start = Message::Start {
                    epoch,
                    slot: i as u8 + 1,
                    characters: characters.clone(),
                    track,
                    hash,
                    laps,
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
            host.ready_from.clear();
            host.racing = true;
            race_info = Some((0, characters.clone(), roster.clone()));
            plan = Some(Plan {
                roster: roster.clone(),
                players,
            });
            io.event(Event::Started {
                slot: 0,
                characters,
                players: roster,
                track,
                hash,
                laps,
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
                    if let Some(host) = &mut host {
                        host.ready_from.insert(me);
                    }
                }
            }
        }
        // The host: everyone is ready, so this is tick 0.
        if let Some(host) = &host
            && !running
            && let Some(plan) = &plan
            && plan
                .roster
                .iter()
                .all(|id| host.ready_from.contains(id) || departed.contains(id))
        {
            running = true;
            for link in links.values_mut() {
                link.advanced = Instant::now();
            }
            for guest in &host.guests {
                let _ = send_reliably(&guest.connection, &Message::Go).await;
            }
            io.event(Event::Go);
        }
    }
}
