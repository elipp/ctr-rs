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

mod conn;
mod names;
mod peers;
mod session;
mod ticket;
pub mod wire;

pub use iroh::EndpointId;
pub use names::{NICK_COOLDOWN, NICK_MAX, clean_nick, display_names};
pub use ticket::Ticket;
pub use wire::{Pad, Tick};

use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use iroh::endpoint::{Connection, presets};
use iroh::Endpoint;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::oneshot;

use crate::peers::Role;
use crate::session::run;

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
