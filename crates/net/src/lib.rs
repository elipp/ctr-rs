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
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use bytes::Bytes;
use iroh::endpoint::{Connection, ConnectionError, presets};
pub use iroh::EndpointId;
use iroh::{Endpoint, EndpointAddr, RelayUrl};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::oneshot;

use crate::wire::{InputReceiver, InputSender, Message, PING_SECONDS};

/// What the clients speak, exchanged in the connection handshake.
pub const ALPN: &[u8] = b"ctr-rust/lobby/6";

/// The most a custom track may weigh, as the bytes it is sent in. A stream that carries more is
/// not read.
pub const MAX_BLOB: usize = 32 << 20;

/// How long the host waits for a new pad of a player in a race before it removes them. The
/// others wait for the player from 2 seconds on (the game's `MAX_LEAD`), so it is up to 6 seconds
/// of that.
pub const KICK_AFTER: Duration = Duration::from_secs(8);

/// The code that the host closes the connection of a player it removes with.
const KICKED: u32 = 1;

/// The most characters of a nickname.
pub const NICK_MAX: usize = 12;

/// How long a player waits between changes of its nickname (the game's to keep: see
/// [`Session::set_nickname`]).
pub const NICK_COOLDOWN: Duration = Duration::from_secs(60);

/// A nickname as it is kept and sent: trimmed, without control characters, and at most
/// [`NICK_MAX`] characters. Empty means none.
pub fn clean_nick(nick: &str) -> String {
    nick.chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .chars()
        .take(NICK_MAX)
        .collect()
}

/// What each player is called, given the id and the nickname of each (empty for none): the
/// nickname, or the short form of the key without one. Players with the same nickname (whatever
/// the case) all get the first 3 characters of their key added, so that they can be told apart.
pub fn display_names(players: &[(EndpointId, &str)]) -> Vec<String> {
    players
        .iter()
        .map(|&(id, nick)| {
            let short = id.fmt_short().to_string();
            if nick.is_empty() {
                return short;
            }
            let twins = players
                .iter()
                .filter(|(_, other)| other.to_lowercase() == nick.to_lowercase())
                .count();
            if twins > 1 {
                format!("{nick}#{}", &short[..3])
            } else {
                nick.to_string()
            }
        })
        .collect()
}

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
    /// Who is in the lobby and the character each drives (`GAME_CHARACTER_IDS` order), the host
    /// first. It comes to everyone whenever someone joins, leaves or changes character; a guest
    /// is in it once it has said what it drives. `you` is our own place in the list, if we are in
    /// it yet.
    Lobby {
        players: Vec<Player>,
        you: Option<usize>,
    },
    /// We joined the lobby of this host.
    Joined(EndpointId),
    /// A peer is gone.
    PeerLeft(EndpointId),
    /// A race is about to start. `slot` is ours among the players, and `characters` is the
    /// character each slot drives (so its length is the number of players), and `players` is the
    /// endpoint of each slot (the host is 0). The race is on the
    /// level `track` (a `LevelId`), which the game has to be showing. A custom track comes with
    /// the `hash` of its contents (0 for the game's own); a game that lacks it asks for it with
    /// [`Session::fetch`] first. Once the level is up, and the game has built the race,
    /// [`Session::ready`] says so; [`Event::Go`] then starts it.
    Started {
        slot: usize,
        characters: Vec<u8>,
        players: Vec<EndpointId>,
        track: u32,
        hash: u64,
    },
    /// The host sent the custom track asked for with [`Session::fetch`], as the bytes it
    /// [provided](Session::provide) it as. They are not checked: that the contents hash to `hash`
    /// is the game's to tell.
    Blob {
        track: u32,
        hash: u64,
        bytes: Vec<u8>,
    },
    /// The host has no custom track `track` with that `hash` to send.
    BlobMissing { track: u32, hash: u64 },
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
    /// The host stopped the race (see [`Session::stop`]).
    Stopped,
    /// The player in `slot` left the race. The game answers with [`Session::have`] once it has
    /// taken the pads that came in so far, and takes no more pads of that player.
    Gone { slot: usize },
    /// The pads of the player in `slot`, which left, end with these (`pads` are the ticks from
    /// `first`; the game has some of them already, and may lack some before them). Everyone gets
    /// the same: `race::rollback::Rollback::leave` follows once they are confirmed.
    Left {
        slot: usize,
        first: Tick,
        pads: Vec<Pad>,
    },
    /// The round trip to a player, from the latest ping (see [`Message::Ping`]): one for each
    /// ping answered, every [`wire::PING_SECONDS`].
    Rtt { id: EndpointId, rtt: Duration },
    /// The answer to [`Session::net_info`]: how each peer is reached.
    NetInfo(Vec<PeerInfo>),
    /// The session could not go on.
    Failed(String),
}

/// A player in the lobby.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Player {
    pub id: EndpointId,
    /// What it drives (`GAME_CHARACTER_IDS` order).
    pub character: u8,
    /// What it wants to be called, empty for none (see [`display_names`]).
    pub nick: String,
}

/// How we are connected to one peer.
#[derive(Debug)]
pub struct PeerInfo {
    pub id: EndpointId,
    /// The open network paths of the connection, the one in use or not.
    pub paths: Vec<PathInfo>,
    /// The round trip of our last ping to it and back (see [`Message::Ping`]), if one has come
    /// back yet. Unlike the paths' figures, this is the game's own message through the whole
    /// stack.
    pub ping: Option<Duration>,
    /// Packets sent and lost over all paths.
    pub sent_packets: u64,
    pub lost_packets: u64,
}

/// A network path to a peer: straight to it, or through a relay server.
#[derive(Debug)]
pub struct PathInfo {
    pub relay: bool,
    /// The remote address (the relay's URL for a relay path).
    pub addr: String,
    /// Whether our data goes this way.
    pub selected: bool,
    pub rtt: std::time::Duration,
}

impl PeerInfo {
    fn of(connection: &Connection, ping: Option<Duration>) -> PeerInfo {
        let stats = connection.stats();
        PeerInfo {
            id: connection.remote_id(),
            ping,
            paths: connection
                .paths()
                .iter()
                .map(|path| PathInfo {
                    relay: path.is_relay(),
                    addr: path.remote_addr().to_string(),
                    selected: path.is_selected(),
                    rtt: path.rtt(),
                })
                .collect(),
            sent_packets: stats.sent_packets,
            lost_packets: stats.lost_packets,
        }
    }
}

enum Command {
    Start {
        track: u32,
        hash: u64,
    },
    Restart {
        track: u32,
        hash: u64,
    },
    Provide {
        track: u32,
        hash: u64,
        bytes: Arc<Vec<u8>>,
    },
    Fetch {
        track: u32,
        hash: u64,
    },
    Announce(String),
    Character(u8),
    Nickname(String),
    RaceOver,
    Ready,
    Have {
        slot: usize,
        first: Tick,
        pads: Vec<Pad>,
    },
    Input(Pad),
    Checksum {
        tick: Tick,
        hash: u64,
    },
    NetInfo,
    Stop,
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
    /// [`Event::Started`]. `hash` is that of a custom track's contents (0 for the game's own),
    /// which has to be [provided](Self::provide) for the players who lack it.
    pub fn start(&self, track: u32, hash: u64) {
        let _ = self.commands.send(Command::Start { track, hash });
    }

    /// Makes the custom track `track` (whose contents hash to `hash`) available to the players
    /// who ask for it, as `bytes`: what they get in [`Event::Blob`]. Larger than [`MAX_BLOB`] and
    /// it is not sent.
    pub fn provide(&self, track: u32, hash: u64, bytes: Arc<Vec<u8>>) {
        let _ = self.commands.send(Command::Provide { track, hash, bytes });
    }

    /// Asks the host for the custom track `track` that hashes to `hash`: [`Event::Blob`] brings
    /// it, or [`Event::BlobMissing`] says it cannot.
    pub fn fetch(&self, track: u32, hash: u64) {
        let _ = self.commands.send(Command::Fetch { track, hash });
    }

    /// Starts the race again on level `track`, if we host the lobby and a race has been started
    /// before. Everyone gets [`Event::Started`] again, with the same players, and the race goes
    /// the way it did the first time.
    pub fn restart(&self, track: u32, hash: u64) {
        let _ = self.commands.send(Command::Restart { track, hash });
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

    /// Changes what we are called, at any time (in a race too): everyone in the lobby is told,
    /// and gets [`Event::Lobby`]. Cleaned with [`clean_nick`]. Holding to [`NICK_COOLDOWN`]
    /// between changes is the game's to do.
    pub fn set_nickname(&self, nick: &str) {
        let _ = self.commands.send(Command::Nickname(clean_nick(nick)));
    }

    /// The game is ready to play tick 0 (the level is up and the race built), after
    /// [`Event::Started`]. Once every player is, [`Event::Go`] comes.
    pub fn ready(&self) {
        let _ = self.commands.send(Command::Ready);
    }

    /// The answer to [`Event::Gone`]: the latest pads of that player we hold, from tick `first`
    /// (see `race::rollback::Rollback::recent`). We take no more of its pads after this.
    pub fn have(&self, slot: usize, first: Tick, pads: Vec<Pad>) {
        let _ = self.commands.send(Command::Have { slot, first, pads });
    }

    /// Our pad for the tick being played, which goes to the others at once. One per tick
    /// played, from [`Event::Go`] on.
    pub fn input(&self, pad: Pad) {
        let _ = self.commands.send(Command::Input(pad));
    }

    /// Tells the session that the race is over, which the game decides (a win, a stop): players
    /// who join from now on are not told that a race is in progress. (The host's to say.)
    pub fn over(&self) {
        let _ = self.commands.send(Command::RaceOver);
    }

    /// Ends the race in progress for everyone, if we host the lobby: they get [`Event::Stopped`]
    /// (we do not; we know). The lobby stays, and [`Session::restart`] starts the same players again.
    pub fn stop(&self) {
        let _ = self.commands.send(Command::Stop);
    }

    /// Asks how each peer is reached; [`Event::NetInfo`] answers.
    pub fn net_info(&self) {
        let _ = self.commands.send(Command::NetInfo);
    }

    /// Tells the others the hash of our sim before `tick`, for them to compare with theirs.
    /// (The hashes are taken at the same ticks by all, see `race::rollback::Rollback`.)
    pub fn checksum(&self, tick: Tick, hash: u64) {
        let _ = self.commands.send(Command::Checksum { tick, hash });
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // The thread says goodbye to the peers (which can take a while) on its own: the game
        // does not wait for it.
        self.stop.take();
        self.thread.take();
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

/// Opens a lobby, driving `character` and called `nick` (empty for none, see [`clean_nick`]).
/// [`Event::Open`] comes once it can be reached.
///
/// `build` is the version of the sim (`race::SIM_VERSION`): a guest with another is turned away.
pub fn host(character: u8, nick: &str, build: u32) -> Session {
    let nick = clean_nick(nick);
    spawn(move |endpoint, io| Box::pin(run(endpoint, io, Role::Host, character, nick, build)))
}

/// Joins the lobby of `ticket`, driving `character` and called `nick`.
///
/// `build` is the version of the sim (`race::SIM_VERSION`): the host turns us away if its own is
/// another.
pub fn join(ticket: Ticket, character: u8, nick: &str, build: u32) -> Session {
    let nick = clean_nick(nick);
    spawn(move |endpoint, io| {
        Box::pin(run(endpoint, io, Role::Join(ticket), character, nick, build))
    })
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
/// The host tells everyone, itself included, who is in the lobby: itself, then the guests that
/// have said what they drive, in the order they joined.
async fn tell_lobby(me: EndpointId, character: u8, nick: &str, guests: &[Guest], io: &Io) {
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

/// What a player who joins during a race is told.
const RACE_IN_PROGRESS: &str = "Race in progress, you will join the next one";

/// What a connection said, tagged with who.
enum Heard {
    Message(Message),
    /// The round trip of a ping we sent, which came back.
    Ping(Duration),
    /// The connection closed: the number it has (see `Connection::stable_id`), and whether the
    /// host removed us ([`KICKED`]).
    Closed { id: usize, kicked: bool },
}

/// The clock that the stamps of the pings are read from. Only ours reads them.
fn stamp() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_micros() as u64
}

/// The connections without the repeats: a player can be in the lobby and the race both.
fn distinct<'a>(connections: impl Iterator<Item = &'a Connection>) -> Vec<&'a Connection> {
    let mut seen = HashSet::new();
    connections
        .filter(|c| seen.insert(c.stable_id()))
        .collect()
}

/// Reads the datagrams and the reliable messages of a connection until it closes.
fn listen(connection: Connection, from: EndpointId, heard: UnboundedSender<(EndpointId, Heard)>) {
    tokio::spawn(async move {
        loop {
            let message = tokio::select! {
                datagram = connection.read_datagram() => datagram.ok().map(|d| Message::from_bytes(&d)),
                stream = connection.accept_uni() => match stream {
                    Ok(mut stream) => match stream.read_to_end(MAX_BLOB + 1024).await {
                        Ok(bytes) => Some(Message::from_bytes(&bytes)),
                        Err(_) => continue,
                    },
                    Err(_) => None ,
                },
            };
            match message {
                // Answered here, where the connection is, and at once: the loop may be busy.
                Some(Ok(Message::Ping { stamp })) => send_datagram(&connection, &Message::Pong { stamp }),
                Some(Ok(Message::Pong { stamp: sent })) => {
                    let rtt = Duration::from_micros(stamp().saturating_sub(sent));
                    if heard.send((from, Heard::Ping(rtt))).is_err() {
                        return;
                    }
                }
                Some(Ok(message)) => {
                    if heard.send((from, Heard::Message(message))).is_err() {
                        return;
                    }
                }
                // Not ours to understand: a stranger's, or a newer build's.
                Some(Err(_)) => {}
                None => {
                    let kicked = matches!(
                        connection.close_reason(),
                        Some(ConnectionError::ApplicationClosed(close))
                            if close.error_code.into_inner() == u64::from(KICKED)
                    );
                    let id = connection.stable_id();
                    let _ = heard.send((from, Heard::Closed { id, kicked }));
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
    /// What it wants to be called, and the number of the Hello or Nick that said it.
    nick: (u32, String),
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

/// The host's tally of a player that left the race: who has not said yet what they hold of
/// its pads, and the most anyone held.
struct Departure {
    /// The slots to hear from.
    waiting: HashSet<usize>,
    /// The first tick of `pads`, and the pads: the longest run any client has.
    first: Tick,
    pads: Vec<Pad>,
}

impl Departure {
    /// `slot` holds `pads` of the player that left, from `first`.
    fn heard(&mut self, slot: usize, first: Tick, pads: Vec<Pad>) {
        self.waiting.remove(&slot);
        let end = |first: Tick, pads: &[Pad]| first as usize + pads.len();
        if end(first, &pads) > end(self.first, &self.pads) {
            self.first = first;
            self.pads = pads;
        }
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
    /// When its pads last went further (or the link began).
    advanced: Instant,
}

impl Link {
    fn new(connection: Connection, epoch: u32) -> Link {
        Link {
            epoch,
            connection,
            sender: InputSender::default(),
            receiver: InputReceiver::default(),
            peer_has: 0,
            advanced: Instant::now(),
        }
    }

    /// Sends the pads it lacks of ours, and what we have of its.
    fn send(&mut self) {
        let mut packet = self.sender.packet(self.peer_has, self.receiver.next());
        packet.epoch = self.epoch;
        send_datagram(&self.connection, &Message::Inputs(packet));
    }
}

/// The players to remove from a race: the ones whose pads stopped coming for [`KICK_AFTER`], and
/// are behind everyone else. (When one player hangs, the rest wait for it and stop sending as
/// well, so being silent is not enough: the one to blame is the one that is furthest back.) `own`
/// is how far our pads go, and `guests` has for each player how far theirs go and when that
/// last changed.
fn stalled(own: Tick, guests: &[(EndpointId, Tick, Instant)], now: Instant) -> Vec<EndpointId> {
    let back = guests.iter().map(|&(_, next, _)| next).fold(own, Tick::min);
    guests
        .iter()
        .filter(|&&(_, next, since)| next <= back && now.duration_since(since) >= KICK_AFTER)
        .map(|&(id, _, _)| id)
        .collect()
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
async fn run(
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

    #[test]
    fn the_furthest_back_silent_player_is_removed() {
        let [a, b] = std::array::from_fn(|_| iroh::SecretKey::generate().public());
        let now = Instant::now();
        let long_ago = now - KICK_AFTER - Duration::from_secs(1);
        // The hung player is at 100; the one waiting for it went on to 160 and is silent as well.
        assert_eq!(stalled(160, &[(a, 100, long_ago), (b, 160, long_ago)], now), [a]);
        // Silent for a moment only.
        assert!(stalled(160, &[(a, 100, now), (b, 160, long_ago)], now).is_empty());
        // Everyone is where the host is.
        assert_eq!(stalled(100, &[(a, 100, long_ago)], now), [a]);
        // The host is the furthest back: it is not blamed, nor are the others.
        assert!(stalled(90, &[(a, 100, long_ago), (b, 160, long_ago)], now).is_empty());
    }

    #[test]
    fn nicknames_are_trimmed_and_cut() {
        assert_eq!(clean_nick("  Penta \n"), "Penta");
        assert_eq!(clean_nick("Dingodile the Great"), "Dingodile th");
        assert_eq!(clean_nick("a\u{7}b"), "ab");
        assert_eq!(clean_nick("ääääääääääääää").chars().count(), NICK_MAX);
        assert_eq!(clean_nick("   "), "");
    }

    #[test]
    fn equal_nicknames_get_the_key_added() {
        let [a, b, c] = std::array::from_fn(|_| iroh::SecretKey::generate().public());
        let short = |id: EndpointId| id.fmt_short().to_string()[..3].to_string();
        let names = display_names(&[(a, "Penta"), (b, "penta"), (c, "")]);
        assert_eq!(names[0], format!("Penta#{}", short(a)));
        assert_eq!(names[1], format!("penta#{}", short(b)));
        assert_eq!(names[2], c.fmt_short().to_string());
        assert_eq!(display_names(&[(a, "Penta"), (b, "Tiny")]), ["Penta", "Tiny"]);
    }
}
