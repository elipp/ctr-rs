//! The online lobby : `/lobby create` opens one and prints its address, `/lobby join <address>` (`command.rs`)
//! joins one. The host's `/race start` starts the race, which is tick 0 for everyone. The connections themselves
//! are `net`'s; this reports what happens to them and hands the race to `drive.rs`.
//!
//! * `/lobby create`: open a lobby (`/lobby close` closes it)
//! * `/lobby join <address>`: join the lobby at an address
//! * `/lobby spectate <address>`: join the lobby at an address to watch its races, not to drive in them
//! * `/cache purge`: delete the maps other players sent us (the `cache/maps` folder, see `paths.rs`)
//! * `/race start`: start the race, once the others have joined (the host only)
//! * `/race restart`: start the race in progress over (the host only)
//! * `/net info`: how each peer is reached (direct or relay), the ping and the packets lost
//! * `/net smoothing ping|replay|<duration>`: how long the drawing takes to ease a kart that a late
//!   pad put right, by the ping, by the ticks the rollbacks replay, or fixed (`kart::Smoothing`), at
//!   any time
//! * `/net rollbacks on|off`: a toast for each rollback (on at first)
//! * `/race stop`: end the race in progress for everyone, with no winner (the host only)
//!
//! The race is a mesh with rollback: every player plays its own pad at once and sends it to all
//! the others, and the pads of the others reach `drive_tick` through [`Online::inbox`]. A pad
//! that has not come yet is predicted, and put right when it does (`race::rollback`).

use bevy::prelude::*;
use net::{EndpointId, Pad, Session, Ticket};

use crate::maps;

use commands::run;
use countdown::{COUNTDOWN_SECONDS, Countdown, Then, countdown};
pub use hud::spawn_waiting;
use hud::waiting;
pub use online::Online;
use progress::{RollbackToasts, finish, pace, rollback_toasts};
use report::report;

mod commands;
mod countdown;
mod hud;
mod online;
mod progress;
mod report;

/// The session this client is in, if any.
#[derive(Resource, Default)]
pub struct Lobby {
    session: Option<Session>,
    /// We joined someone else's lobby (`/lobby join`) instead of creating one: the host starts the race.
    joined: bool,
    /// We only watch (`/lobby spectate`), and drive in no race.
    spectator: bool,
    /// Who is in the lobby and the character each drives (the short key and the character), as
    /// the last [`Event::Lobby`] said, and whether there has been one: what changes in the next is
    /// what the players are told.
    roster: Vec<Member>,
    roster_seen: bool,
    /// The guests that only watch, as the same said: the name and the short key of each.
    watchers: Vec<(String, String)>,
    /// The nickname of every player the lobby has told of (empty for none).
    nicks: std::collections::HashMap<EndpointId, String>,
    /// The last round trip to each player in the lobby, by short key, from the pings, and an
    /// average of them (which the smoothing follows, not to be moved by one slow packet).
    pub pings: std::collections::HashMap<String, std::time::Duration>,
    pub average_pings: std::collections::HashMap<String, std::time::Duration>,
    /// The map the race is on that we are waiting for the host to send.
    fetching: Option<u32>,
    /// The level the host picked to drive on between races (`Event::Track`) that we have not gone
    /// to yet: we lack the map, or the level switch has not come to it.
    pending_track: Option<(u32, u64)>,
    /// The start or restart that is counting down.
    countdown: Option<Countdown>,
    pub online: Option<Online>,
}

/// A player in the lobby, as the last [`Event::Lobby`] said.
struct Member {
    /// The short form of the key.
    key: String,
    /// What the player is called (see [`Online::labels`]).
    label: String,
    nick: String,
    character: u8,
}

impl Lobby {
    /// The players in the lobby, as the last [`Event::Lobby`] said: the character, the name
    /// (the nickname, or the short key without one) and the short key of each.
    pub fn members(&self) -> impl Iterator<Item = (u8, &str, &str)> {
        self.roster
            .iter()
            .map(|m| (m.character, m.label.as_str(), m.key.as_str()))
    }

    /// The guests that only watch: the name and the short key of each.
    pub fn watchers(&self) -> impl Iterator<Item = (&str, &str)> {
        self.watchers.iter().map(|(n, k)| (n.as_str(), k.as_str()))
    }

    /// Whether we are in a lobby and no race is on: the players wait.
    pub fn is_waiting(&self) -> bool {
        self.session.is_some() && self.online.is_none()
    }

    /// Whether we are in a lobby (ours or another's), or creating one.
    pub fn is_open(&self) -> bool {
        self.session.is_some()
    }

    /// Opens a lobby of our own, for others to join. (They get its address when it is up.)
    pub fn host(&mut self, character: u8, nick: &str) {
        self.session = Some(net::host(character, nick, race::SIM_VERSION));
        self.joined = false;
        self.spectator = false;
        self.roster.clear();
        self.roster_seen = false;
    }

    /// Starts counting `then` down from [`COUNTDOWN_SECONDS`].
    fn count_down(&mut self, then: Then, now: f32) {
        let text = match then {
            Then::Start(_) => "Race starts in",
            Then::Restart(..) => "Race restarts in",
        };
        self.announce(format!("{text} {COUNTDOWN_SECONDS}"));
        self.countdown = Some(Countdown {
            then,
            left: COUNTDOWN_SECONDS,
            at: now,
        });
    }

    /// The race is over, won or stopped: back to a free drive, in the lobby. (The session is told,
    /// so that players who join now do not hear of a race in progress.)
    fn end_race(&mut self) {
        self.online = None;
        if let Some(session) = &self.session {
            session.over();
        }
    }

    /// Leaves the lobby, ours or another's, and the race in it.
    pub fn close(&mut self) {
        *self = Lobby::default();
    }

    /// Joins the lobby at `ticket`.
    pub fn join(&mut self, ticket: Ticket, character: u8, nick: &str) {
        debug!("joining {ticket}");
        self.session = Some(net::join(ticket, character, nick, race::SIM_VERSION));
        self.joined = true;
        self.spectator = false;
        self.roster.clear();
        self.roster_seen = false;
    }

    /// Joins the lobby at `ticket` to watch.
    pub fn spectate(&mut self, ticket: Ticket, nick: &str) {
        debug!("joining {ticket} to watch");
        self.session = Some(net::spectate(ticket, nick, race::SIM_VERSION));
        self.joined = true;
        self.spectator = true;
        self.roster.clear();
        self.roster_seen = false;
    }

    /// Watch instead of driving in the next race, or drive again (a guest, before the race).
    /// Returns whether we watch now.
    pub fn toggle_spectator(&mut self) -> bool {
        self.spectator = !self.spectator;
        if let Some(session) = &self.session {
            session.set_spectator(self.spectator);
        }
        self.spectator
    }

    /// We drive another character now, which the others learn before the race starts.
    pub fn set_character(&self, character: u8) {
        if let Some(session) = &self.session {
            session.set_character(character);
        }
    }

    /// We are called something else now, which the others learn at once, in a race too.
    pub fn set_nickname(&self, nick: &str) {
        if let Some(session) = &self.session {
            session.set_nickname(nick);
        }
    }

    /// Names the slots of the race after the nicknames known now.
    fn refresh_labels(&mut self) {
        if let Some(online) = &mut self.online {
            let players: Vec<(EndpointId, &str)> = online
                .ids
                .iter()
                .map(|id| (*id, self.nicks.get(id).map_or("", String::as_str)))
                .collect();
            online.labels = net::display_names(&players);
        }
    }

    /// Our pad for the tick being played, which goes to the others at once.
    pub fn send(&self, pad: Pad) {
        if let Some(session) = &self.session {
            session.input(pad);
        }
    }

    /// What we hold of the pads of a player that left, from tick `first` (see `Event::Gone`).
    pub fn have(&self, slot: usize, first: u32, pads: Vec<Pad>) {
        if let Some(session) = &self.session {
            session.have(slot, first, pads);
        }
    }

    /// Our hash of the sim before `tick`, for the others to compare.
    pub fn checksum(&self, tick: u32, hash: u64) {
        if let Some(session) = &self.session {
            session.checksum(tick, hash);
        }
    }

    /// Tells everyone in the lobby (the host only).
    pub fn announce(&self, text: impl Into<String>) {
        if let Some(session) = &self.session {
            session.announce(text);
        }
    }

    /// Starts the race over on `track` (the host only).
    pub fn restart(&self, track: u32, hash: u64) {
        if let Some(session) = &self.session {
            self.provide(session, track);
            session.restart(track, hash);
        }
    }

    /// Starts the race on `track` (the host only).
    fn start(&self, track: u32) {
        if let Some(session) = &self.session {
            self.provide(session, track);
            session.start(track, Self::hash_of(track));
        }
    }

    /// Whether we host the lobby we are in.
    pub fn is_host(&self) -> bool {
        self.session.is_some() && !self.joined
    }

    /// Tells the lobby which level to drive on until the next race (the host only): the players
    /// go to it, and so do the ones who join later.
    pub fn set_track(&self, track: u32) {
        if self.is_host()
            && self.online.is_none()
            && let Some(session) = &self.session
        {
            self.provide(session, track);
            session.set_track(track, Self::hash_of(track));
        }
    }

    /// The level the host picked that we have yet to go to, and the hash of its map.
    pub(crate) fn pending_track(&self) -> Option<(u32, u64)> {
        self.pending_track
    }

    /// We went to the level the host picked, or will not.
    pub(crate) fn clear_pending_track(&mut self) {
        self.pending_track = None;
    }

    /// The hash of the contents of the map `track`: the others compare it with theirs, which for
    /// one of the game's levels is made from their disc. (0 for a level we have no map of.)
    fn hash_of(track: u32) -> u64 {
        maps::hash(track).unwrap_or(0)
    }

    /// Makes the map `track` available to the players who lack it, if it is one.
    fn provide(&self, session: &Session, track: u32) {
        if map::is_map_id(track)
            && let Some((hash, bytes)) = maps::file(track)
        {
            session.provide(track, hash, std::sync::Arc::new(bytes));
        }
    }

    /// The race is built and can start, once the others are as well.
    pub fn ready(&self) {
        if let Some(session) = &self.session {
            session.ready();
        }
    }
}

pub struct LobbyPlugin;

impl Plugin for LobbyPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Lobby>()
            .init_resource::<crate::kart::Smoothing>()
            .init_resource::<RollbackToasts>()
            .add_systems(Startup, spawn_waiting)
            .add_systems(
                Update,
                (
                    run,
                    countdown,
                    report,
                    crate::remote::build,
                    finish,
                    rollback_toasts,
                    pace,
                    waiting,
                )
                    .chain()
                    .run_if(crate::drive::driving),
            );
    }
}

/// What the host says to everyone is yellow.
const ANNOUNCE_COLOR: Color = Color::srgb(1.0, 0.9, 0.2);
