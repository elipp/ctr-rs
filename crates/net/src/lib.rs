//! Peer-to-peer connections between ctr-rust clients, negotiated with `iroh`.
//!
//! One client [`host`]s a lobby and prints its [`Ticket`]; the others [`join`] with it. A
//! session runs on a thread of its own, so the game never waits on the network: it polls
//! [`Session::events`] once per frame.

pub mod lockstep;
pub mod wire;

pub use wire::{Frame, Pad, Tick};

use std::fmt;
use std::str::FromStr;
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver};
use std::thread::JoinHandle;

use iroh::endpoint::{Connection, presets};
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayUrl};
use bytes::Bytes;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::oneshot;

use crate::lockstep::FrameReceiver;
use crate::wire::{InputSender, Message};

/// What the clients speak, exchanged in the connection handshake.
pub const ALPN: &[u8] = b"ctr-rust/lobby/1";

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
        Ticket { id: addr.id, relay: addr.relay_urls().next().cloned() }
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
            id: id.parse().map_err(|e| format!("bad endpoint id '{id}': {e}"))?,
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
    /// We joined the lobby of this host.
    Joined(EndpointId),
    /// A peer is gone.
    PeerLeft(EndpointId),
    /// The race starts, and this is tick 0. `slot` is ours among the players, and `characters`
    /// is the character each slot drives (so its length is the number of players). The race is
    /// on the level `track` (a `LevelId`), which the game has to be showing.
    Started { slot: usize, characters: Vec<u8>, track: u32 },
    /// Everyone's pad for a tick, which the game plays. They come in order from tick 0.
    Frame { tick: Tick, pads: Frame },
    /// The session could not go on.
    Failed(String),
}

enum Command {
    Start { track: u32 },
    Character(u8),
    Input(Pad),
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

    /// Changes the character we drive, for the race that has not started yet. (Pass the one
    /// given to [`host`] or [`join`] again and nothing changes.)
    pub fn set_character(&self, character: u8) {
        let _ = self.commands.send(Command::Character(character));
    }

    /// Our pad for the tick [`lockstep::INPUT_DELAY`] after the one just played. One per tick played, from
    /// the first [`Event::Frame`] on.
    pub fn input(&self, pad: Pad) {
        let _ = self.commands.send(Command::Input(pad));
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
    spawn(move |endpoint, io| Box::pin(run_host(endpoint, io, character)))
}

/// Joins the lobby of `ticket`, driving `character`.
pub fn join(ticket: Ticket, character: u8) -> Session {
    spawn(move |endpoint, io| Box::pin(run_join(endpoint, io, ticket, character)))
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
                let io = Io { events: events_tx.clone(), commands: commands_rx, stop: stop_rx };
                let result = run(endpoint.clone(), io).await;
                endpoint.close().await;
                result
            });
            if let Err(e) = result {
                let _ = events_tx.send(Event::Failed(e.to_string()));
            }
        })
        .expect("the network thread");
    Session { events: Mutex::new(events), commands, stop: Some(stop), thread: Some(thread) }
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
fn listen(connection: Connection, heard: UnboundedSender<Heard>) {
    tokio::spawn(async move {
        loop {
            let message = tokio::select! {
                datagram = connection.read_datagram() => datagram.ok().map(|d| Message::from_bytes(&d)),
                stream = connection.accept_uni() => match stream {
                    Ok(mut stream) => match stream.read_to_end(1024).await {
                        Ok(bytes) => Some(Message::from_bytes(&bytes)),
                        Err(_) => continue,
                    },
                    Err(_) => None,
                },
            };
            match message {
                Some(Ok(message)) => {
                    if heard.send(Heard::Message(message)).is_err() {
                        return;
                    }
                }
                // Not ours to understand: a stranger's, or a newer build's.
                Some(Err(_)) => {}
                None => {
                    let _ = heard.send(Heard::Closed);
                    return;
                }
            }
        }
    });
}

async fn run_host(endpoint: Endpoint, mut io: Io, mut character: u8) -> Result<(), Error> {
    tokio::select! {
        _ = &mut io.stop => return Ok(()),
        _ = endpoint.online() => {}
    }
    io.event(Event::Open(Ticket::of(&endpoint.addr())));

    // Slot 0 is us; the peers are 1.. in the order they came.
    let mut peers: Vec<Connection> = Vec::new();
    // What each peer said it drives; none until its Hello comes.
    let mut characters: Vec<Option<(u32, u8)>> = Vec::new();
    // The track of a start that waits for the Hellos.
    let mut start_wanted: Option<u32> = None;
    let (heard_tx, mut heard) = unbounded_channel::<(usize, Heard)>();
    let mut race: Option<lockstep::Host> = None;
    let mut timer = send_interval();
    loop {
        tokio::select! {
            _ = &mut io.stop => return Ok(()),
            // Nobody joins a race that has begun.
            incoming = endpoint.accept(), if race.is_none() => {
                let Some(incoming) = incoming else { return Ok(()) };
                let Ok(accepting) = incoming.accept() else { continue };
                let Ok(connection) = accepting.await else { continue };
                let slot = peers.len() + 1;
                io.event(Event::PeerJoined(connection.remote_id()));
                let (tx, mut rx) = unbounded_channel();
                listen(connection.clone(), tx);
                let heard_tx = heard_tx.clone();
                tokio::spawn(async move {
                    while let Some(h) = rx.recv().await {
                        if heard_tx.send((slot, h)).is_err() {
                            return;
                        }
                    }
                });
                peers.push(connection);
                characters.push(None);
            }
            Some(command) = io.commands.recv() => match command {
                // Goes ahead below, once every peer has said what it drives.
                Command::Start { track } => start_wanted = Some(track),
                Command::Character(c) => character = c,
                Command::Input(pad) => if let Some(race) = &mut race {
                    race.push_local(pad);
                },
            },
            Some((slot, heard)) = heard.recv() => match heard {
                Heard::Message(Message::Inputs(inputs)) => if let Some(race) = &mut race {
                    let _ = race.accept(slot, &inputs);
                },
                Heard::Message(Message::Hello { character, seq }) => {
                    let known = &mut characters[slot - 1];
                    if known.is_none_or(|(have, _)| seq > have) {
                        *known = Some((seq, character));
                    }
                }
                Heard::Message(_) => {}
                Heard::Closed => io.event(Event::PeerLeft(peers[slot - 1].remote_id())),
            },
            _ = timer.tick() => if let Some(race) = &race {
                for (i, peer) in peers.iter().enumerate() {
                    send_datagram(peer, &Message::Frames(race.packet(i + 1)));
                }
            },
        }
        if let Some(track) = start_wanted.filter(|_| race.is_none() && characters.iter().all(Option::is_some)) {
            start_wanted = None;
            let all: Vec<u8> = std::iter::once(character).chain(characters.iter().flatten().map(|&(_, c)| c)).collect();
            race = Some(lockstep::Host::new(all.len()));
            for (i, peer) in peers.iter().enumerate() {
                let start = Message::Start { slot: i as u8 + 1, characters: all.clone(), track };
                let _ = send_reliably(peer, &start).await;
            }
            io.event(Event::Started { slot: 0, characters: all, track });
        }
        if let Some(race) = &mut race {
            for (tick, pads) in race.advance() {
                io.event(Event::Frame { tick, pads });
            }
        }
    }
}

async fn run_join(endpoint: Endpoint, mut io: Io, ticket: Ticket, character: u8) -> Result<(), Error> {
    let connection: Connection = tokio::select! {
        _ = &mut io.stop => return Ok(()),
        connection = endpoint.connect(ticket.addr(), ALPN) => connection?,
    };
    io.event(Event::Joined(ticket.id));
    send_reliably(&connection, &Message::Hello { character, seq: 0 }).await?;
    let mut hellos = 1;

    let (heard_tx, mut heard) = unbounded_channel();
    listen(connection.clone(), heard_tx);
    // Once the race has started: our pads, and the frames that came.
    let mut race: Option<(InputSender, FrameReceiver, Tick)> = None;
    let mut timer = send_interval();
    loop {
        tokio::select! {
            _ = &mut io.stop => {
                connection.close(0u32.into(), b"bye");
                return Ok(());
            }
            Some(command) = io.commands.recv() => match command {
                Command::Input(pad) => if let Some((sender, ..)) = &mut race {
                    sender.push(pad);
                },
                // The host takes the latest Hello it has, so one that is sent again overrides.
                Command::Character(character) if race.is_none() => {
                    send_reliably(&connection, &Message::Hello { character, seq: hellos }).await?;
                    hellos += 1;
                }
                Command::Character(_) | Command::Start { .. } => {}
            },
            Some(heard) = heard.recv() => match heard {
                Heard::Message(Message::Start { slot, characters, track }) if race.is_none() => {
                    race = Some((lockstep::local_sender(), FrameReceiver::default(), 0));
                    io.event(Event::Started { slot: slot as usize, characters, track });
                }
                Heard::Message(Message::Frames(frames)) => if let Some((_, receiver, host_has)) = &mut race {
                    *host_has = (*host_has).max(frames.ack);
                    // A gap needs a snapshot, which the race does not have (yet).
                    let new = receiver.accept(&frames).map_err(|g| format!("lost frames {g:?}"))?;
                    for (tick, pads) in new {
                        io.event(Event::Frame { tick, pads });
                    }
                },
                Heard::Message(_) => {}
                Heard::Closed => {
                    io.event(Event::PeerLeft(ticket.id));
                    return Ok(());
                }
            },
            _ = timer.tick() => if let Some((sender, receiver, host_has)) = &mut race {
                let packet = sender.packet(*host_has, receiver.next());
                send_datagram(&connection, &Message::Inputs(packet));
            },
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
