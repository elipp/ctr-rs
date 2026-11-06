//! The online lobby : `/lobby create` opens one and prints its address, `/lobby join <address>` (`command.rs`)
//! joins one. The host's `/race start` starts the race, which is tick 0 for everyone. The connections themselves
//! are `net`'s; this reports what happens to them and hands the race to `drive.rs`.
//!
//! * `/lobby create [public [name]]`: open a lobby (`/lobby close` closes it); a public one is listed in
//!   the realm under that name, for anyone to find; the command line offers a name (`command.rs`)
//! * the pause menu's "Public lobbies" (`pause.rs`): the realm's list, where a lobby is joined by taking it
//! * `/lobby join <address>`: join the lobby at an address
//! * `/lobby spectate <address>`: join the lobby at an address to watch its races, not to drive in them
//! * `/cache purge`: delete the maps other players sent us (the `.cache/custom-tracks` folder, see `paths.rs`)
//! * `/race start`: start the race, once the others have joined (the host only)
//! * `/cup create <name> <tracks> <laps>`: define a cup (`cup.rs`), kept in the config until
//!   `/cup delete <name>`, and `/cup start <name>` to drive it: its races one after another, points
//!   for the first four to finish, and [`BETWEEN_RACES_SECONDS`] between them. A `/race start` is a
//!   cup of one race with no points.
//! * `/race restart`: start the race in progress over (the host only)
//! * `/promote <player>`: hand the lobby over to a player, between races (the host only): they host it
//!   from then on, a public lobby stays listed, and we are a guest of theirs
//! * `/net info`: how each peer is reached (direct or relay), the ping and the packets lost
//! * `/net smoothing ping|replay|<duration>`: how long the drawing takes to ease a kart that a late
//!   pad put right, by the ping, by the ticks the rollbacks replay, or fixed (`kart::Smoothing`), at
//!   any time
//! * `/net rollbacks on|off`: a toast for each rollback (on at first)
//! * `/net relay add <url>`: use that relay too, besides the public ones (kept in the config; `clear` drops them all)
//! * `/race stop`: end the race in progress for everyone, with no winner (the host only)
//!
//! The race is a mesh with rollback: every player plays its own pad at once and sends it to all
//! the others, and the pads of the others reach `drive_tick` through [`Online::inbox`]. A pad
//! that has not come yet is predicted, and put right when it does (`race::rollback`).

use bevy::prelude::*;
use net::{EndpointId, Pad, Session, Ticket};

use crate::console::Log;
use crate::maps;

use commands::run;
use countdown::{COUNTDOWN_SECONDS, Countdown, Then, countdown};
use cup::{BETWEEN_RACES_SECONDS, Finisher, Series};
pub use hud::spawn_waiting;
use hud::waiting;
pub use online::Online;
pub(crate) use progress::SPECTATE_BUFFER;
use placing::placing;
use progress::{RollbackToasts, finish, pace, rollback_toasts};
use report::report;

pub(crate) mod commands;
mod countdown;
pub(crate) mod cup;
mod hud;
pub mod lag;
mod online;
mod placing;
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
    /// The name our lobby is listed under in the realm, if it is public (`/lobby create public`).
    public: Option<String>,
    /// Who is in the lobby and the character each drives (the short key and the character), as
    /// the last [`Event::Lobby`] said, and whether there has been one: what changes in the next is
    /// what the players are told.
    roster: Vec<Member>,
    roster_seen: bool,
    /// The host has fixed what everyone drives, as the last [`Event::Lobby`] said.
    characters_locked: bool,
    /// Whether we, the host, have told the session to fix them (for the cup, see
    /// [`tell_lock`](Self::tell_lock)).
    lock_told: bool,
    /// The guests that only watch, as the same said: the name and the short key of each.
    watchers: Vec<(String, String)>,
    /// The nickname of every player the lobby has told of (empty for none).
    nicks: std::collections::HashMap<EndpointId, String>,
    /// The last round trip to each player in the lobby, by short key, from the pings, and an
    /// average of them (which the smoothing follows, not to be moved by one slow packet).
    pub pings: std::collections::HashMap<String, std::time::Duration>,
    pub average_pings: std::collections::HashMap<String, std::time::Duration>,
    /// Whether the way to each player in the lobby, by short key, is a relay (`true`) or direct.
    pub relayed: std::collections::HashMap<String, bool>,
    /// The map the race is on that we are waiting for the host to send.
    fetching: Option<u32>,
    /// The races the host is running (a cup, or the one of `/race start`), until the last is over.
    series: Option<Series>,
    /// How fast we have been talking.
    chat_limit: net::RateLimit,
    /// The start or restart that is counting down.
    countdown: Option<Countdown>,
    pub online: Option<Online>,
}

/// A player in the lobby, as the last [`Event::Lobby`] said.
struct Member {
    id: EndpointId,
    /// The short form of the key.
    key: String,
    /// What the player is called (see [`Online::labels`]).
    label: String,
    nick: String,
    character: u8,
    /// Whether it is us.
    you: bool,
}

impl Lobby {
    /// The player of the lobby, other than us (the host, when we are), that `name` stands for: the
    /// nickname, the name as the lobby shows it, or the short key, whatever its case.
    pub fn player_named(&self, name: &str) -> Result<(EndpointId, &str), String> {
        let name = name.trim();
        let is = |s: &str| s.eq_ignore_ascii_case(name);
        // (The host is first, and is us when we host: the others are the ones to hand it to.)
        let mut found = self
            .roster
            .iter()
            .skip(1)
            .filter(|m| is(&m.nick) || is(&m.label) || is(&m.key));
        match (found.next(), found.next()) {
            (Some(m), None) => Ok((m.id, m.label.as_str())),
            (Some(_), Some(_)) => Err(format!("More than one player is called {name}: use the key")),
            _ if self.roster.first().is_some_and(|m| is(&m.nick) || is(&m.label) || is(&m.key)) => {
                Err("You are the host already".into())
            }
            _ => Err(format!("No player called {name} in the lobby")),
        }
    }

    /// The players in the lobby, as the last [`Event::Lobby`] said: the character, the name
    /// (the nickname, or the short key without one) and the short key of each.
    pub fn members(&self) -> impl Iterator<Item = (u8, &str, &str)> {
        self.roster
            .iter()
            .map(|m| (m.character, m.label.as_str(), m.key.as_str()))
    }

    /// Our place in [`members`](Self::members), if we drive (not if we only watch, or the lobby
    /// has not told yet).
    pub fn you(&self) -> Option<usize> {
        self.roster.iter().position(|m| m.you)
    }

    /// Who the lobby could be handed to, the character and the name of each: the others, if we
    /// are its host.
    pub fn promotable(&self) -> impl Iterator<Item = (u8, &str)> {
        let hosting = self.is_open() && !self.joined;
        // (As in `player_named`: the first is us.)
        self.roster
            .iter()
            .skip(1)
            .filter(move |_| hosting)
            .map(|m| (m.character, m.label.as_str()))
    }

    /// The guests that only watch: the name and the short key of each.
    pub fn watchers(&self) -> impl Iterator<Item = (&str, &str)> {
        self.watchers.iter().map(|(n, k)| (n.as_str(), k.as_str()))
    }

    /// Whether we are in a lobby and no race is on: the players wait.
    pub fn is_waiting(&self) -> bool {
        self.session.is_some() && self.online.is_none()
    }

    /// What our lobby is called in the realm's list, if it is public.
    pub fn public_name(&self) -> Option<&str> {
        self.public.as_deref()
    }

    /// Whether we are in a lobby (ours or another's), or creating one.
    pub fn is_open(&self) -> bool {
        self.session.is_some()
    }

    /// Opens a lobby of our own, for others to join. (They get its address when it is up.) A
    /// `public` one is also listed in the realm, under that name (the default if it is empty;
    /// `net::lobby_name` has checked it).
    /// The name is the lobby's, not ours: it stays as it is when we change nickname.
    pub fn host(&mut self, character: u8, nick: &str, public: Option<&str>) {
        self.session = Some(net::host(character, nick, race::SIM_VERSION));
        self.public = public.map(|name| {
            Some(name)
                .filter(|name| !name.is_empty())
                .map_or_else(|| net::DEFAULT_LOBBY_NAME.to_string(), str::to_string)
        });
        self.joined = false;
        self.spectator = false;
        self.roster.clear();
        self.roster_seen = false;
    }

    /// Starts counting `then` down from [`COUNTDOWN_SECONDS`].
    fn count_down(&mut self, then: Then, now: f32, seconds: u32) {
        // (A long wait is announced by whoever starts it; the last seconds are counted.)
        if seconds <= COUNTDOWN_SECONDS {
            let text = match then {
                Then::Start { .. } => "Race starts in",
                Then::Restart { .. } => "Race restarts in",
            };
            self.announce(format!("{text} {seconds}"));
        }
        self.countdown = Some(Countdown {
            then,
            left: seconds,
            at: now,
        });
    }

    /// Starts the series, with the countdown to its first race (the host only).
    fn start_series(&mut self, series: Series, now: f32) {
        let then = Then::Start {
            track: series.track(),
            laps: series.laps,
        };
        self.series = Some(series);
        self.count_down(then, now, COUNTDOWN_SECONDS);
    }

    /// The race of the series ended with these players over the line, in order, out of `players`
    /// (the host only). A cup scores it and, if it has another race, starts it in
    /// [`BETWEEN_RACES_SECONDS`]; the series is over after its last.
    fn race_over(&mut self, players: usize, finishers: &[Finisher], now: f32) {
        let Some(mut series) = self.series.take().filter(|s| s.scored) else {
            return;
        };
        let gained = series.score(players, finishers);
        let race = series.race;
        let next = series.has_next().then(|| {
            series.race += 1;
            series.track()
        });
        let standings = net::Standings {
            cup: series.name.clone().unwrap_or_else(|| "Cup".into()),
            race: race as u8,
            races: series.tracks.len() as u8,
            gained,
            totals: series.totals(),
            next: next.map(|track| (track, maps::name(track))),
        };
        if let Some(session) = &self.session {
            session.standings(standings);
        }
        if let Some(track) = next {
            let laps = series.laps;
            self.series = Some(series);
            self.count_down(Then::Start { track, laps }, now, BETWEEN_RACES_SECONDS);
        }
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

    /// Whether we cannot drive another character now: in a cup, from its countdown to its last
    /// race, the players keep the ones they started it with.
    pub fn characters_locked(&self) -> bool {
        self.characters_locked || self.series.as_ref().is_some_and(|s| s.scored)
    }

    /// Has the session fix what everyone drives while our cup is on, and free it after (the host
    /// only; the guests learn of it in [`Event::Lobby`]).
    fn tell_lock(&mut self) {
        let locked = self.series.as_ref().is_some_and(|s| s.scored);
        if locked != self.lock_told
            && let Some(session) = &self.session
        {
            session.lock_characters(locked);
            self.lock_told = locked;
        }
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

    /// Says that to everyone in the lobby, us too: false if we are talking too fast for it (see
    /// `net::RateLimit`; the host holds us to the same).
    pub fn say(&mut self, text: &str) -> bool {
        if net::clean_chat(text).is_empty() {
            return true;
        }
        if !self.chat_limit.allow(net::Instant::now()) {
            return false;
        }
        if let Some(session) = &self.session {
            session.say(text);
        }
        true
    }

    /// What a player said as a line of the console: the portrait of its character, its name in
    /// the colour of the character, and the text. A player the lobby does not know (it has left,
    /// or only watches: there is no character then) has no portrait.
    fn chat_line(&self, from: EndpointId, text: &str) -> Log {
        let key = from.fmt_short().to_string();
        let said = |name: &str, color| {
            Log::colored(name.to_string(), color).then(format!(": {text}"), Color::WHITE)
        };
        if let Some(member) = self.roster.iter().find(|m| m.key == key) {
            let character = (member.character as usize).min(crate::CHARACTER_COLORS.len() - 1);
            return said(&member.label, crate::CHARACTER_COLORS[character]).icon(character as u8);
        }
        let name = self
            .watchers
            .iter()
            .find(|(_, k)| *k == key)
            .map_or(key.as_str(), |(name, _)| name.as_str());
        said(name, crate::console::DIM_COLOR)
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
    pub fn restart(&self, track: u32, hash: u64, laps: u8) {
        if let Some(session) = &self.session {
            self.provide(session, track);
            session.restart(track, hash, laps);
        }
    }

    /// Starts the race on `track` (the host only).
    fn start(&self, track: u32, laps: u8) {
        if let Some(session) = &self.session {
            self.provide(session, track);
            session.start(track, Self::hash_of(track), laps);
            self.list(&maps::name(track));
        }
    }

    /// Whether we host the lobby we are in.
    pub fn is_host(&self) -> bool {
        self.session.is_some() && !self.joined
    }

    /// Lists the lobby in the realm, if it is public, with the track it is on (empty for none).
    fn list(&self, track: &str) {
        if let (Some(name), Some(session)) = (&self.public, &self.session) {
            session.set_public(Some(net::Public {
                realm: net::default_realm(),
                name: name.clone(),
                track: track.to_string(),
            }));
        }
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
            .init_resource::<lag::FakeLag>()
            .add_systems(Startup, spawn_waiting)
            .add_systems(
                Update,
                (
                    run,
                    countdown,
                    report,
                    crate::remote::build,
                    finish,
                    placing,
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
