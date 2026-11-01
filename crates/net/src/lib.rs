//! Peer-to-peer connections between ctr-rust clients, negotiated with `iroh`.
//!
//! One client [`host`]s a lobby and prints its [`Ticket`]; the others [`join`] with it. A
//! session runs on a thread of its own (in a browser, a task of the page), so the game never
//! waits on the network: it polls [`Session::events`] once per frame.
//!
//! The host only coordinates: it takes the joins, starts the race, and says when tick 0 is. The
//! race itself is a mesh. When it starts, every player connects to every other, and each sends
//! its pads straight to each of the others ([`Event::Pads`] brings the ones that came in). There
//! is no relay and no delay: a player plays its own pad at once, and the others' as they arrive
//! (the game predicts them until then, see `race::rollback`).

mod conn;
mod limit;
mod names;
mod peers;
mod realm;
mod session;
mod ticket;
mod transfers;
pub mod wire;

pub use iroh::{EndpointId, SecretKey};
pub use limit::{CHAT_BURST, CHAT_PER_SECOND, RateLimit};
/// The clock of [`RateLimit`], as the session's: tokio's natively, the browser's in a browser
/// (where std's panics).
pub use n0_future::time::Instant;
pub use names::{CHAT_MAX, NICK_COOLDOWN, NICK_MAX, clean_chat, clean_nick, display_names};
pub use realm::{
    Browse, DEFAULT_LOBBY_NAME, DEFAULT_REALM, LOBBY_CAPACITY, MAX_LOBBY_NAME, Public, PublicLobby,
    browse, default_realm, lobby_name,
};
pub use realm_proto::is_name_char as is_lobby_name_char;
pub use ticket::Ticket;
pub use wire::{Pad, Tick};

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
#[cfg(not(target_arch = "wasm32"))]
use std::thread::JoinHandle;
use std::time::Duration;

use iroh::Endpoint;
use iroh::endpoint::{Builder, Connection, presets};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::oneshot;

use crate::peers::Role;
use crate::session::run;

static RELAY_ONLY: AtomicBool = AtomicBool::new(false);

/// Whether the sessions and fetches that start from now on go through the relay only (`true`), or
/// find the shortest way to the others, a direct one when there is one (the default). With the
/// relay only, no packet leaves our address for another player's (nor for the realm's): they do
/// not learn it, at the price of the relay's extra round trip. A session that is up keeps the
/// way it started with.
pub fn set_relay_only(relay_only: bool) {
    RELAY_ONLY.store(relay_only, Ordering::Relaxed);
}

/// The endpoint of a session or a fetch, as [`set_relay_only`] says.
fn endpoint_builder() -> Builder {
    let builder = Endpoint::builder(presets::N0);
    // (A browser has no sockets of its own: there the relay is the only way already.)
    #[cfg(not(target_arch = "wasm32"))]
    if RELAY_ONLY.load(Ordering::Relaxed) {
        // No sockets of our own: the relay is the one way left.
        return builder.clear_ip_transports();
    }
    builder
}

/// A new endpoint, with the protocols of a session.
async fn bind_endpoint() -> Result<Endpoint, Error> {
    Ok(endpoint_builder().alpns(vec![ALPN.to_vec(), MESH_ALPN.to_vec(), SWAP_ALPN.to_vec()]).bind().await?)
}

/// What the clients speak, exchanged in the connection handshake.
pub const ALPN: &[u8] = b"ctr-rust/lobby/12";

/// What the connections between the players of a race speak. They carry the pads and nothing
/// else; the lobby's own connection (to the host) carries the rest. Every pair of players has
/// one, the host's pair with each guest included, so the host is no different from the others
/// to the network.
pub const MESH_ALPN: &[u8] = b"ctr-rust/mesh/1";

/// What the host speaks when it dials a guest whose own lobby connection stays on a relay: the
/// connection takes the place of the guest's (the NAT of the guest only lets traffic in after the
/// host has sent out, so it may be the host that has to dial). It is the lobby's, otherwise.
pub const SWAP_ALPN: &[u8] = b"ctr-rust/lobby-swap/1";

/// The most a custom track may weigh, as the bytes it is sent in. A stream that carries more is
/// not read.
pub const MAX_BLOB: usize = 32 << 20;

/// How long the host waits for a new pad of a player in a race before it removes them. The
/// others wait for the player from 2 seconds on (the game's `MAX_LEAD`), so it is up to 6 seconds
/// of that.
pub const KICK_AFTER: Duration = Duration::from_secs(8);

/// The code that the host closes the connection of a player it removes with.
const KICKED: u32 = 1;

type Error = Box<dyn std::error::Error + Send + Sync>;

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
        /// The guests that only watch (their `character` is 0), in the order they joined.
        spectators: Vec<Player>,
        you: Option<usize>,
    },
    /// We joined the lobby of this host. (Also when the host left and the lobby went on with
    /// another one: we joined that one.)
    Joined(EndpointId),
    /// The host left and we are the host now: the lobby goes on with us. Its address is ours,
    /// so the old ticket does not work any more; `ticket` is what the others join with. The
    /// guests that follow us are [`Event::PeerJoined`] and in the next [`Event::Lobby`]. If the
    /// lobby was public, it is listed again under our id (`public`).
    Promoted {
        ticket: Ticket,
        public: Option<Public>,
    },
    /// A peer is gone.
    PeerLeft(EndpointId),
    /// A race is about to start. `slot` is ours among the players, and `characters` is the
    /// character each slot drives (so its length is the number of players), and `players` is the
    /// endpoint of each slot (the host is 0). The race is on the
    /// level `track` (a `LevelId`), which the game has to be showing. A custom track comes with
    /// the `hash` of its contents (0 for the game's own); a game that lacks it asks for it with
    /// [`Session::fetch`] first. Once the level is up, and the game has built the race,
    /// [`Session::ready`] says so; [`Event::Go`] then starts it. The race has `laps` laps.
    Started {
        slot: usize,
        characters: Vec<u8>,
        players: Vec<EndpointId>,
        track: u32,
        hash: u64,
        laps: u8,
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
    /// `from` said `text` to the lobby (see [`Session::say`]): us too, when we did, in the same
    /// order as everyone reads it. It is cleaned already.
    Chat { from: EndpointId, text: String },
    /// The hash player `slot` made of the sim before `tick` (see [`Session::checksum`]).
    Checksum { slot: usize, tick: Tick, hash: u64 },
    /// A hash from `from` that was not taken in, and why: it is of another race (`sent` is not
    /// our `epoch`), or we know of no such player in the race (`planned`) or have no link to it.
    ChecksumDropped {
        from: EndpointId,
        tick: Tick,
        sent: u32,
        epoch: u32,
        planned: bool,
        linked: bool,
    },
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
    Rtt {
        id: EndpointId,
        rtt: Duration,
        /// Whether our data to it goes through a relay (none while there is no path).
        relay: Option<bool>,
    },
    /// A line about a link to another player, for the log: it came up, was dialed again because it
    /// stayed on a relay, got a second connection, switched between them, or changed its way.
    Link(String),
    /// The answer to [`Session::net_info`]: how each peer is reached.
    NetInfo(Vec<PeerInfo>),
    /// The lobby is in the realm's list (see [`Session::set_public`]), or it is not and why: the
    /// realm cannot be reached, or turns the listing down. The session keeps trying, so `Ok`
    /// can follow an `Err`.
    Listed(Result<(), String>),
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
        laps: u8,
    },
    Restart {
        track: u32,
        hash: u64,
        laps: u8,
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
    Say(String),
    Character(u8),
    Spectate(bool),
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
    Public(Option<Public>),
}

/// A lobby being hosted, or joined. Dropping it closes the connections.
pub struct Session {
    events: Mutex<Receiver<Event>>,
    commands: UnboundedSender<Command>,
    stop: Option<oneshot::Sender<()>>,
    #[cfg(not(target_arch = "wasm32"))]
    thread: Option<JoinHandle<()>>,
}

impl Session {
    /// What happened since the last call.
    pub fn poll(&self) -> Vec<Event> {
        self.events.lock().unwrap().try_iter().collect()
    }

    /// The first of what happened since the last call (or [`poll`](Self::poll)), if anything did.
    pub fn next_event(&self) -> Option<Event> {
        self.events.lock().unwrap().try_recv().ok()
    }

    /// Starts the race on level `track`, if we host the lobby. Everyone then gets
    /// [`Event::Started`]. `hash` is that of a custom track's contents (0 for the game's own),
    /// which has to be [provided](Self::provide) for the players who lack it. The race has `laps`
    /// laps.
    pub fn start(&self, track: u32, hash: u64, laps: u8) {
        let _ = self.commands.send(Command::Start { track, hash, laps });
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
    pub fn restart(&self, track: u32, hash: u64, laps: u8) {
        let _ = self.commands.send(Command::Restart { track, hash, laps });
    }

    /// Broadcasts `text` to everyone in the lobby, us included ([`Event::Announce`]), if we host
    /// it.
    pub fn announce(&self, text: impl Into<String>) {
        let _ = self.commands.send(Command::Announce(text.into()));
    }

    /// Says `text` to everyone in the lobby, us included ([`Event::Chat`]): a guest's goes by the
    /// host. Cleaned with [`clean_chat`]; nothing is sent if that leaves nothing.
    pub fn say(&self, text: &str) {
        let _ = self.commands.send(Command::Say(clean_chat(text)));
    }

    /// Changes the character we drive, for the race that has not started yet. (Pass the one
    /// given to [`host`] or [`join`] again and nothing changes.)
    pub fn set_character(&self, character: u8) {
        let _ = self.commands.send(Command::Character(character));
    }

    /// Changes whether we only watch, for the race that has not started yet (a guest's to do;
    /// the host always plays). The lobby's [`Event::Lobby`] follows.
    pub fn set_spectator(&self, spectator: bool) {
        let _ = self.commands.send(Command::Spectate(spectator));
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

    /// Lists the lobby in a realm, for anyone to find and join, or takes it off the list
    /// (`None`), if we host it. [`Event::Listed`] says whether it worked. The players and the
    /// state of the lobby in the list follow the lobby; the name and the track are as given
    /// here, so call it again when the track changes.
    pub fn set_public(&self, public: Option<Public>) {
        let _ = self.commands.send(Command::Public(public));
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
        #[cfg(not(target_arch = "wasm32"))]
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
    spawn(move |endpoint, io| {
        Box::pin(run(endpoint, io, Role::Host, character, nick, build, false))
    })
}

/// Joins the lobby of `ticket`, driving `character` and called `nick`.
///
/// `build` is the version of the sim (`race::SIM_VERSION`): the host turns us away if its own is
/// another.
pub fn join(ticket: Ticket, character: u8, nick: &str, build: u32) -> Session {
    enter(ticket, character, nick, build, false)
}

/// Joins the lobby of `ticket` to watch: we are not one of the players of its races and make no
/// pads, but get everyone's, so the game can play the race along with them. We are in the roster
/// after the last player ([`Event::Started`]'s `slot` is past its `characters`), and in no
/// [`Event::Lobby`]. `build` is as in [`join`].
pub fn spectate(ticket: Ticket, nick: &str, build: u32) -> Session {
    enter(ticket, 0, nick, build, true)
}

fn enter(ticket: Ticket, character: u8, nick: &str, build: u32, spectator: bool) -> Session {
    let nick = clean_nick(nick);
    spawn(move |endpoint, io| {
        Box::pin(run(
            endpoint,
            io,
            Role::Join(ticket),
            character,
            nick,
            build,
            spectator,
        ))
    })
}

type Task = std::pin::Pin<Box<dyn Future<Output = Result<(), Error>>>>;

fn spawn(run: impl FnOnce(Endpoint, Io) -> Task + Send + 'static) -> Session {
    let (events_tx, events) = mpsc::channel();
    let (commands, commands_rx) = unbounded_channel();
    let (stop, stop_rx) = oneshot::channel();
    // Made where it runs: the session is not `Send`.
    let session = move || async move {
        let result: Result<(), Error> = async {
            let endpoint = bind_endpoint().await?;
            let io = Io {
                events: events_tx.clone(),
                commands: commands_rx,
                stop: stop_rx,
            };
            let result = run(endpoint.clone(), io).await;
            endpoint.close().await;
            result
        }
        .await;
        if let Err(e) = result {
            let _ = events_tx.send(Event::Failed(e.to_string()));
        }
    };
    #[cfg(not(target_arch = "wasm32"))]
    let thread = std::thread::Builder::new()
        .name("net".into())
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("the network runtime")
                .block_on(session())
        })
        .expect("the network thread");
    // A browser has no threads: there the session is a task of the page's event loop, which
    // runs between the frames.
    #[cfg(target_arch = "wasm32")]
    n0_future::task::spawn(session());
    Session {
        events: Mutex::new(events),
        commands,
        stop: Some(stop),
        #[cfg(not(target_arch = "wasm32"))]
        thread: Some(thread),
    }
}
