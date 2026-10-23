//! Peer-to-peer connections between ctr-rust clients, negotiated with `iroh`.
//!
//! One client [`host`]s a lobby and prints its [`Ticket`]; the others [`join`] with it. A
//! session runs on a thread of its own, so the game never waits on the network: it polls
//! [`Session::events`] once per frame.
//!
//! The host only coordinates: it takes the joins, starts the race, and says when tick 0 is. The
//! race itself is a mesh. When it starts, every player connects to every other, and each sends
//! its pads straight to each of the others ([`Event::Pads`] brings the ones that came in). There
//! is no relay and no delay: a player plays its own pad at once, and the others' as they arrive
//! (the game predicts them until then, see `race::rollback`).

pub mod wire;

pub use wire::{Pad, Tick};

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::str::FromStr;
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver};
use std::thread::JoinHandle;

use bytes::Bytes;
use iroh::endpoint::{Connection, presets};
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayUrl};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::oneshot;

use crate::wire::{InputReceiver, InputSender, Message};

/// What the clients speak, exchanged in the connection handshake.
pub const ALPN: &[u8] = b"ctr-rust/lobby/2";

type Error = Box<dyn std::error::Error + Send + Sync>;

/// What a client needs to find a lobby: `<endpoint id>`, then `@<relay url>` when it has one.
/// With the id alone, iroh looks the lobby's address up (the `N0` preset's DNS service); the
/// relay makes the first contact quicker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ticket {
    pub id: EndpointId,
    pub relay: Option<RelayUrl>,
}

impl Ticket {
    fn of(addr: &EndpointAddr) -> Ticket {
        Ticket {
            id: addr.id,
            relay: addr.relay_urls().next().cloned(),
        }
    }

    fn addr(&self) -> EndpointAddr {
        let addr = EndpointAddr::new(self.id);
        match &self.relay {
            Some(relay) => addr.with_relay_url(relay.clone()),
            None => addr,
        }
    }
}

impl fmt::Display for Ticket {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.id)?;
        if let Some(relay) = &self.relay {
            write!(f, "@{relay}")?;
        }
        Ok(())
    }
}

impl FromStr for Ticket {
    type Err = String;

    fn from_str(s: &str) -> Result<Ticket, String> {
        let (id, relay) = match s.split_once('@') {
            Some((id, relay)) => (id, Some(relay)),
            None => (s, None),
        };
        Ok(Ticket {
            id: id
                .parse()
                .map_err(|e| format!("bad endpoint id '{id}': {e}"))?,
            relay: relay
                .map(|r| r.parse().map_err(|e| format!("bad relay url '{r}': {e}")))
                .transpose()?,
        })
    }
}

#[derive(Debug)]
pub enum Event {
    /// The lobby is up: this is what the others join with.
    Open(Ticket),
    /// Someone joined the lobby we host.
    PeerJoined(EndpointId),
    /// We host the lobby, and a guest says what it drives (`first` for the first time it says,
    /// which is when it has joined; again when it changes its character). `character` is in
    /// `GAME_CHARACTER_IDS` order.
    Character {
        id: EndpointId,
        character: u8,
        first: bool,
    },
    /// We joined the lobby of this host.
    Joined(EndpointId),
    /// A peer is gone.
    PeerLeft(EndpointId),
    /// A race is about to start. `slot` is ours among the players, and `characters` is the
    /// character each slot drives (so its length is the number of players). The race is on the
    /// level `track` (a `LevelId`), which the game has to be showing. Once it is, and the game
    /// has built the race, [`Session::ready`] says so; [`Event::Go`] then starts it.
    Started {
        slot: usize,
        characters: Vec<u8>,
        track: u32,
    },
    /// This is tick 0: the game plays it, and sends its pad for it with [`Session::input`].
    Go,
    /// Pads of the player `slot`, for the ticks from `first` on. They come in order, each once,
    /// and may come before [`Event::Go`] (the others started first).
    Pads {
        slot: usize,
        first: Tick,
        pads: Vec<Pad>,
    },
    /// How far ahead of player `slot` our clock is, in ticks (negative when behind): half the
    /// difference between how far each of us is past the other's last pad. It comes with every
    /// packet of theirs, so it is noisy; the game smooths it.
    Lead { slot: usize, ahead: f32 },
    /// A message the host broadcast (see [`Session::announce`]) for the players to read.
    Announce(String),
    /// The hash player `slot` made of the sim before `tick` (see [`Session::checksum`]).
    Checksum { slot: usize, tick: Tick, hash: u64 },
    /// The session could not go on.
    Failed(String),
}

enum Command {
    Start { track: u32 },
    Restart { track: u32 },
    Announce(String),
    Character(u8),
    Ready,
    Input(Pad),
    Checksum { tick: Tick, hash: u64 },
}

/// A lobby being hosted, or joined. Dropping it closes the connections.
pub struct Session {
    events: Mutex<Receiver<Event>>,
    commands: UnboundedSender<Command>,
    stop: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl Session {
    /// What happened since the last call.
    pub fn poll(&self) -> Vec<Event> {
        self.events.lock().unwrap().try_iter().collect()
    }

    /// Starts the race on level `track`, if we host the lobby. Everyone then gets
    /// [`Event::Started`].
    pub fn start(&self, track: u32) {
        let _ = self.commands.send(Command::Start { track });
    }

    /// Starts the race again on level `track`, if we host the lobby and a race has been started
    /// before. Everyone gets [`Event::Started`] again, with the same players, and the race goes
    /// the way it did the first time.
    pub fn restart(&self, track: u32) {
        let _ = self.commands.send(Command::Restart { track });
    }

    /// Broadcasts `text` to everyone in the lobby, us included ([`Event::Announce`]), if we host
    /// it.
    pub fn announce(&self, text: impl Into<String>) {
        let _ = self.commands.send(Command::Announce(text.into()));
    }

    /// Changes the character we drive, for the race that has not started yet. (Pass the one
    /// given to [`host`] or [`join`] again and nothing changes.)
    pub fn set_character(&self, character: u8) {
        let _ = self.commands.send(Command::Character(character));
    }

    /// The game is ready to play tick 0 (the level is up and the race built), after
    /// [`Event::Started`]. Once every player is, [`Event::Go`] comes.
    pub fn ready(&self) {
        let _ = self.commands.send(Command::Ready);
    }

    /// Our pad for the tick being played, which goes to the others at once. One per tick
    /// played, from [`Event::Go`] on.
    pub fn input(&self, pad: Pad) {
        let _ = self.commands.send(Command::Input(pad));
    }

    /// Tells the others the hash of our sim before `tick`, for them to compare with theirs.
    /// (The hashes are taken at the same ticks by all, see `race::rollback::Rollback`.)
    pub fn checksum(&self, tick: Tick, hash: u64) {
        let _ = self.commands.send(Command::Checksum { tick, hash });
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// What the network thread has of the game.
struct Io {
    events: mpsc::Sender<Event>,
    commands: UnboundedReceiver<Command>,
    stop: oneshot::Receiver<()>,
}

impl Io {
    fn event(&self, event: Event) {
        let _ = self.events.send(event);
    }
}

/// Opens a lobby, driving `character`. [`Event::Open`] comes once it can be reached.
pub fn host(character: u8) -> Session {
    spawn(move |endpoint, io| Box::pin(run(endpoint, io, Role::Host, character)))
}

/// Joins the lobby of `ticket`, driving `character`.
pub fn join(ticket: Ticket, character: u8) -> Session {
    spawn(move |endpoint, io| Box::pin(run(endpoint, io, Role::Join(ticket), character)))
}

type Task = std::pin::Pin<Box<dyn Future<Output = Result<(), Error>>>>;

fn spawn(run: impl FnOnce(Endpoint, Io) -> Task + Send + 'static) -> Session {
    let (events_tx, events) = mpsc::channel();
    let (commands, commands_rx) = unbounded_channel();
    let (stop, stop_rx) = oneshot::channel();
    let thread = std::thread::Builder::new()
        .name("net".into())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("the network runtime");
            let result = runtime.block_on(async {
                let endpoint = Endpoint::builder(presets::N0)
                    .alpns(vec![ALPN.to_vec()])
                    .bind()
                    .await?;
                let io = Io {
                    events: events_tx.clone(),
                    commands: commands_rx,
                    stop: stop_rx,
                };
                let result = run(endpoint.clone(), io).await;
                endpoint.close().await;
                result
            });
            if let Err(e) = result {
                let _ = events_tx.send(Event::Failed(e.to_string()));
            }
        })
        .expect("the network thread");
    Session {
        events: Mutex::new(events),
        commands,
        stop: Some(stop),
        thread: Some(thread),
    }
}

/// How often the packets go out, once per game tick. They are sent whether or not there is
/// news: when a packet is lost and the game then waits for it, the resend is what gets it going.
fn send_interval() -> tokio::time::Interval {
    let mut interval = tokio::time::interval(std::time::Duration::from_millis(1000 / 30));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    interval
}

/// A message on a stream of its own, which iroh delivers whole and in order.
async fn send_reliably(connection: &Connection, message: &Message) -> Result<(), Error> {
    let mut stream = connection.open_uni().await?;
    stream.write_all(&message.to_bytes()).await?;
    stream.finish()?;
    Ok(())
}

fn send_datagram(connection: &Connection, message: &Message) {
    // A datagram that does not go is as good as one that is lost.
    let _ = connection.send_datagram(Bytes::from(message.to_bytes()));
}

/// What a connection said, tagged with who.
enum Heard {
    Message(Message),
    Closed,
}

/// Reads the datagrams and the reliable messages of a connection until it closes.
fn listen(connection: Connection, from: EndpointId, heard: UnboundedSender<(EndpointId, Heard)>) {
    tokio::spawn(async move {
        loop {
            let message = tokio::select! {
                datagram = connection.read_datagram() => datagram.ok().map(|d| Message::from_bytes(&d)),
                stream = connection.accept_uni() => match stream {
                    Ok(mut stream) => match stream.read_to_end(1024).await {
                        Ok(bytes) => Some(Message::from_bytes(&bytes)),
                        Err(_) => continue,
                    },
                    Err(_) => None ,
                },
            };
            match message {
                Some(Ok(message)) => {
                    if heard.send((from, Heard::Message(message))).is_err() {
                        return;
                    }
                }
                // Not ours to understand: a stranger's, or a newer build's.
                Some(Err(_)) => {}
                None => {
                    let _ = heard.send((from, Heard::Closed));
                    return;
                }
            }
        }
    });
}

#[derive(Clone)]
enum Role {
    Host,
    Join(Ticket),
}

/// A player in the lobby, as the host knows it.
struct Guest {
    id: EndpointId,
    connection: Connection,
    /// What it said it drives, and the number of the Hello that said it; none  until it comes.
    hello: Option<(u32, u8)>,
}

/// The race as agreed at the start.
struct Plan {
    /// The endpoint of each slot. The host is 0.
    roster: Vec<EndpointId>,
}

impl Plan {
    fn slot_of(&self, id: EndpointId) -> Option<usize> {
        self.roster.iter().position(|&r| r == id)
    }
}

/// The connection to another player in the race, and the pads that go over it.
struct Link {
    connection: Connection,
    /// Our pads, until it confirms them.
    sender: InputSender,
    /// Its pads, as far as they have come in.
    receiver: InputReceiver,
    /// The first of our ticks it has not confirmed.
    peer_has: Tick,
    /// The race the link is for.
    epoch: u32,
}

impl Link {
    fn new(connection: Connection, epoch: u32) -> Link {
        Link {
            epoch,
            connection,
            sender: InputSender::default(),
            receiver: InputReceiver::default(),
            peer_has: 0,
        }
    }

    /// Sends the pads it lacks of ours, and what we have of its.
    fn send(&mut self) {
        let mut packet = self.sender.packet(self.peer_has, self.receiver.next());
        packet.epoch = self.epoch;
        send_datagram(&self.connection, &Message::Inputs(packet));
    }
}

/// Starts the links over for the race of `epoch`: nothing sent or received yet, on the
/// connections that are up.
fn start_over(links: &mut HashMap<EndpointId, Link>, epoch: u32) {
    for link in links.values_mut() {
        *link = Link::new(link.connection.clone(), epoch);
    }
}

/// Connects to a player of the roster, trying for a while: it may not be listening yet.
fn dial(endpoint: Endpoint, id: EndpointId, dialed: UnboundedSender<Connection>) {
    tokio::spawn(async move {
        for _ in 0..20 {
            if let Ok(connection) = endpoint.connect(EndpointAddr::new(id), ALPN).await {
                let _ = dialed.send(connection);
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    });
}

/// The session of a lobby host or of a player who joined one. They differ in the lobby (the
/// host takes the joins and starts the race) and in the barrier before tick 0 (the host collects
/// the [`Message::Ready`]s and sends the [`Message::Go`]); the race is the same for both.
async fn run(endpoint: Endpoint, mut io: Io, role: Role, mut character: u8) -> Result<(), Error> {
    let me = endpoint.id();
    let (heard_tx, mut heard) = unbounded_channel::<(EndpointId, Heard)>();
    let (dialed_tx, mut dialed) = unbounded_channel::<Connection>();

    // The lobby. The host has the guests; a guest has the host.
    let mut guests: Vec<Guest> = Vec::new();
    let mut host_connection: Option<Connection> = None;
    let mut hellos = 0;
    // The track of a start that waits for the Hellos.
    let mut start_wanted: Option<u32> = None;

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
            send_reliably(&connection, &Message::Hello { character, seq: 0 }).await?;
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
    // Which race this is, counted from 0, and our slot and the characters in it.
    let mut epoch: u32 = 0;
    let mut race_info: Option<(usize, Vec<u8>)> = None;
    let mut timer = send_interval();

    loop {
        tokio::select! {
            _ = &mut io.stop => {
                for link in links.values() {
                    link.connection.close(0u32.into(), b"bye");
                }
                return Ok(());
            }
            incoming = endpoint.accept() => {
                let Some(incoming) = incoming else { return Ok(()) };
                let Ok(accepting) = incoming.accept() else { continue };
                let Ok(connection) = accepting.await else { continue };
                let id = connection.remote_id();
                match &role {
                    // Nobody joins a race that has begun.
                    Role::Host if plan.is_none() => {
                        let guest = Guest { id, connection: connection.clone(), hello: None };
                        io.event(Event::PeerJoined(id));
                        listen(connection, id, heard_tx.clone());
                        guests.push(guest);
                    }
                    Role::Host => {}
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
                Command::Start { track } => if matches!(role, Role::Host) {
                    start_wanted = Some(track);
                },
                // The same players again: new links, and everyone ready before tick 0 once more.
                Command::Restart { track } => if matches!(role, Role::Host)
                    && plan.is_some()
                    && let Some((slot, characters)) = race_info.clone()
                {
                    epoch += 1;
                    for guest in &guests {
                        let _ = send_reliably(&guest.connection, &Message::Restart { epoch, track }).await;
                    }
                    start_over(&mut links, epoch);
                    (running, game_ready, ready_sent) = (false, false, false);
                    ready_from.clear();
                    io.event(Event::Started { slot, characters, track });
                },
                Command::Announce(text) => if matches!(role, Role::Host) {
                    for guest in &guests {
                        let _ = send_reliably(&guest.connection, &Message::Announce { text: text.clone() }).await;
                    }
                    io.event(Event::Announce(text));
                },
                Command::Character(c) => if plan.is_none() {
                    character = c;
                    if let Some(connection) = &host_connection {
                        // The host takes the latest Hello it has, so one that is sent again overrides.
                        send_reliably(connection, &Message::Hello { character, seq: hellos }).await?;
                        hellos += 1;
                    }
                },
                Command::Ready => game_ready = true,
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
                    Message::Hello { character, seq } => {
                        if let Some(guest) = guests.iter_mut().find(|g| g.id == from)
                            && guest.hello.is_none_or(|(have, _)| seq > have)
                        {
                            let first = guest.hello.is_none();
                            guest.hello = Some((seq, character));
                            io.event(Event::Character { id: from, character, first });
                        }
                    }
                    Message::Start { slot, characters, track, roster }
                        if plan.is_none() && from == host_id && !matches!(role, Role::Host) =>
                    {
                        let roster: Vec<EndpointId> = roster
                            .iter()
                            .map(|bytes| EndpointId::from_bytes(bytes))
                            .collect::<Result<_, _>>()?;
                        let slot = slot as usize;
                        // The host is connected already.
                        links.insert(
                            host_id,
                            Link::new(host_connection.clone().expect("a guest has the host"), epoch),
                        );
                        // Players connect to the ones before them; the ones after connect to us.
                        for &id in roster.iter().take(slot).skip(1) {
                            dial(endpoint.clone(), id, dialed_tx.clone());
                        }
                        plan = Some(Plan { roster });
                        race_info = Some((slot, characters.clone()));
                        io.event(Event::Started { slot, characters, track });
                    }
                    Message::Restart { epoch: new, track }
                        if plan.is_some() && from == host_id && !matches!(role, Role::Host) =>
                    {
                        if let Some((slot, characters)) = race_info.clone() {
                            epoch = new;
                            start_over(&mut links, epoch);
                            (running, game_ready, ready_sent) = (false, false, false);
                            io.event(Event::Started { slot, characters, track });
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
                    Message::Snapshot { .. } | Message::Start { .. } | Message::Restart { .. } => {}
                },
                Heard::Closed => {
                    io.event(Event::PeerLeft(from));
                    links.remove(&from);
                    unclaimed.retain(|c| c.remote_id() != from);
                    if plan.is_none() {
                        guests.retain(|g| g.id != from);
                        // A guest whose host is gone has no lobby.
                        if from == host_id && !matches!(role, Role::Host) {
                            return Ok(());
                        }
                    }
                }
            },
            _ = timer.tick() => if running {
                for link in links.values_mut() {
                    link.send();
                }
            },
        }

        // The host starts the race once every guest has said what it drives.
        if let Some(track) =
            start_wanted.filter(|_| plan.is_none() && guests.iter().all(|g| g.hello.is_some()))
        {
            start_wanted = None;
            let characters: Vec<u8> = std::iter::once(character)
                .chain(guests.iter().filter_map(|g| g.hello.map(|(_, c)| c)))
                .collect();
            let roster: Vec<EndpointId> = std::iter::once(me)
                .chain(guests.iter().map(|g| g.id))
                .collect();
            for (i, guest) in guests.iter().enumerate() {
                let start = Message::Start {
                    slot: i as u8 + 1,
                    characters: characters.clone(),
                    track,
                    roster: roster.iter().map(|id| *id.as_bytes()).collect(),
                };
                let _ = send_reliably(&guest.connection, &start).await;
                links.insert(guest.id, Link::new(guest.connection.clone(), epoch));
            }
            plan = Some(Plan { roster });
            race_info = Some((0, characters.clone()));
            io.event(Event::Started {
                slot: 0,
                characters,
                track,
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
                .all(|&id| id == me || links.contains_key(&id))
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
            && plan.roster.iter().all(|id| ready_from.contains(id))
        {
            running = true;
            for guest in &guests {
                let _ = send_reliably(&guest.connection, &Message::Go).await;
            }
            io.event(Event::Go);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticket_round_trips() {
        let id = iroh::SecretKey::generate().public();
        for relay in [None, Some("https://relay.example.com./".parse().unwrap())] {
            let ticket = Ticket { id, relay };
            assert_eq!(ticket.to_string().parse(), Ok(ticket));
        }
        assert!("nonsense".parse::<Ticket>().is_err());
    }
}
