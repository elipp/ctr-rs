//! The online lobby : `/lobby create` opens one and prints its address, `/lobby join <address>` (`command.rs`)
//! joins one. The host's `/race start` starts the race, which is tick 0 for everyone. The connections themselves
//! are `net`'s; this reports what happens to them and hands the race to `drive.rs`.
//!
//! * `/lobby create [public [name]]`: open a lobby (`/lobby close` closes it); a public one is listed in
//!   the realm under that name, for anyone to find; the command line offers a name (`command.rs`)
//! * the pause menu's "Play online" (`pause.rs`): the realm's list, where a lobby is joined by taking it,
//!   and creating a public or a private lobby
//! * `/lobby join <address>`: join the lobby at an address
//! * `/lobby spectate <address>`: join the lobby at an address to watch its races, not to drive in them
//! * `/cache purge`: delete the maps other players sent us (the `.cache/custom-tracks` folder, see `paths.rs`)
//! * `/race start`: start the race, once the others have joined (the host only). Outside a lobby
//!   it starts a race of our local players alone (`/race restart` and `/race stop` too)
//! * `/cup create <name> <tracks> <laps>`: define a cup (`cup.rs`), kept in the config until
//!   `/cup delete <name>`, and `/cup start <name>` to drive it: its races one after another, points
//!   for the first four to finish, and [`BETWEEN_RACES_SECONDS`] between them. A `/race start` is a
//!   cup of one race with no points.
//! * `/cup pause`: pause the race for everyone, or the countdown to a cup's next race (the host
//!   only); `/cup resume` to go on
//! * `/cup next`, `/cup previous`: stop the cup's race without points, and count down to its next
//!   or previous track (the host only)
//! * `/cup stop`: end the cup and its race, as `/race stop` does (the host only)
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
//! * `/bots [count] [easy|medium|hard] [brain|retail] [ordered|random]`, `/bots off`: fill the
//!   grid of the host's races with bots, up to eight karts: the learned policy's
//!   (`learned.rs`, whose pads the host sends; the difficulty is their rubber band's,
//!   `race::Band`), or retail's (`vehicle::bot`, which every client drives itself). A race has
//!   one kind or the other. In a cup, a change applies from its next race, and bots keep their
//!   points. `/bots pool` limits the characters they may drive (a mask, or names); see
//!   [`BotPick`] for how they're drawn. The pause menu's "Bots" page sets all of it too
//!   (`bots_menu.rs`)
//!
//! The race is a mesh with rollback: every player plays its own pad at once and sends it to all
//! the others, and the pads of the others reach `drive_tick` through [`NetRace::inbox`](crate::race_on::NetRace::inbox). A pad
//! that has not come yet is predicted, and put right when it does (`race::rollback`).

use bevy::prelude::*;
use net::{EndpointId, Pad, Session, Ticket};

use crate::console::Log;
use crate::maps;
use crate::race_on::{CurrentRace, RaceOn};

use commands::{pause_key, run};
use countdown::{COUNTDOWN_SECONDS, Countdown, Then, countdown};
use cup::{BETWEEN_RACES_SECONDS, Finisher, Series};
pub use hud::spawn_waiting;
use hud::waiting;
pub(crate) use progress::SPECTATE_BUFFER;
use placing::placing;
use progress::{RollbackToasts, finish, pace, rollback_toasts};
use report::report;

pub(crate) mod commands;
mod countdown;
pub(crate) mod cup;
mod hud;
pub mod lag;
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
    /// The bots the host's races are filled with, as the last [`Event::Lobby`] said (the count
    /// the most: as many as there is room for).
    lobby_bots: net::LobbyBots,
    /// The `/bots` we, the host, have told the session to show, if we have (see
    /// [`tell_bots`](Self::tell_bots)).
    bots_told: Option<BotSettings>,
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
    /// The bots our races have, when we host (`/bots`). Kept when the lobby closes.
    pub bots: BotSettings,
    /// The track of the race that is about to start, as the standings of the race before told it
    /// (a cup's next), or as its start did: its start brings the loading screen's flag in over
    /// what is shown (`scene::switch_level`).
    pub(crate) cup_next: Option<u32>,
    /// The cup's latest standings (or its final ones), for the scoreboard.
    pub(crate) standings: Option<net::Standings>,
    /// Who stands on the podium of the cup that just ended (`podium.rs`), until it is shown: once
    /// our race is over.
    pub(crate) podium: Option<[Option<u8>; 3]>,
}

/// How many bots the host's races have, of which kind, and how hard they race (`/bots`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BotSettings {
    pub count: u8,
    /// `vehicle::bot::difficulty_level`: the retail bots', or the brain bots' rubber band's.
    pub difficulty: u16,
    pub kind: BotKind,
    /// Bit `c` set: character `c` may be a bot. Never empty (`commands::refusal`; the pause
    /// menu's pool page won't be left empty), which [`pick_bots`] relies on.
    pub pool: u16,
    pub pick: BotPick,
}

/// How the bots' characters are drawn from the pool. Either way the free ones (no player drives
/// them) come first, and once they run out characters repeat, so the count is always met.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BotPick {
    /// In pool order, starting over from the pool's first when it runs out.
    #[default]
    Ordered,
    /// The free ones shuffled, then any of the pool's, with repeats.
    Random,
}

/// Every character in [`BotSettings::pool`].
pub const ALL_CHARACTERS: u16 = u16::MAX;
const _: () = assert!(crate::CHARACTERS.len() == u16::BITS as usize);

/// Who drives the bots of a race.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BotKind {
    /// The learned policy (`learned.rs`): the host makes their pads, and they drive the kart as a
    /// player does.
    #[default]
    Brain,
    /// Retail's (`vehicle::bot`): they slide along the bot paths, and every client drives them.
    Retail,
}


impl Default for BotSettings {
    fn default() -> BotSettings {
        BotSettings {
            count: 0,
            difficulty: vehicle::bot::difficulty_level::MEDIUM as u16,
            kind: BotKind::default(),
            pool: ALL_CHARACTERS,
            pick: BotPick::default(),
        }
    }
}

/// `count` characters from `pool` (see [`BotPick`]), those in `taken` last.
fn pick_bots(pool: u16, pick: BotPick, taken: &[u8], count: usize, mut seed: u64) -> Vec<u8> {
    let pool: Vec<u8> = (0..crate::CHARACTERS.len() as u8)
        .filter(|c| pool & 1 << c != 0)
        .collect();
    let mut free: Vec<u8> = pool.iter().copied().filter(|c| !taken.contains(c)).collect();
    let repeats: Box<dyn Iterator<Item = u8>> = match pick {
        BotPick::Ordered => Box::new(pool.iter().copied().cycle()),
        BotPick::Random => {
            cup::shuffle(&mut free, &mut seed);
            Box::new(std::iter::repeat_with(move || {
                pool[(cup::splitmix(&mut seed) % pool.len() as u64) as usize]
            }))
        }
    };
    free.into_iter().chain(repeats).take(count).collect()
}

/// The difficulty as `/bots` names it.
pub fn difficulty_name(difficulty: u16) -> &'static str {
    use vehicle::bot::difficulty_level::{EASY, HARD};
    match difficulty as i16 {
        EASY => "easy",
        HARD => "hard",
        _ => "medium",
    }
}

/// A kart of the lobby's next race, as [`Lobby::rows`] gives it.
#[derive(Clone, PartialEq, Eq)]
pub struct Row {
    pub character: u8,
    /// What it is called: its player's name, and for a split screen's other players, their number ("name@2"), as in the race.
    pub name: String,
    /// The short key of its machine.
    pub key: String,
    /// Its machine's key.
    pub id: EndpointId,
    /// Which of the machine's players: 0 for the first, 1 for "name@2", and so on.
    pub seat: u8,
    /// Whether it is the host's own.
    pub host: bool,
    /// Whether it is at our machine.
    pub you: bool,
}

/// A player in the lobby, as the last [`Event::Lobby`] said.
struct Member {
    id: EndpointId,
    /// The short form of the key.
    key: String,
    /// What the player is called (see [`CurrentRace::labels`]).
    label: String,
    nick: String,
    character: u8,
    /// What the other players at its machine drive.
    extras: Vec<u8>,
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

    /// The karts the lobby's players have in a race, as the last [`Event::Lobby`] said, one row
    /// each: a player's, then those of the other players at its machine (split screen), as many
    /// as the grid has room for. (The race gives the room to the host's extras first, then
    /// each guest's, in this order.)
    pub fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        for (n, member) in self.roster.iter().enumerate() {
            rows.push(Row {
                character: member.character,
                name: member.label.clone(),
                key: member.key.clone(),
                id: member.id,
                seat: 0,
                host: n == 0,
                you: member.you,
            });
        }
        // (The extras come after all the players, as they do in the race.)
        for member in &self.roster {
            for (n, &character) in member.extras.iter().enumerate() {
                rows.push(Row {
                    character,
                    name: format!("{}@{}", member.label, n + 2),
                    key: member.key.clone(),
                    id: member.id,
                    seat: n as u8 + 1,
                    host: false,
                    you: member.you,
                });
            }
        }
        rows.truncate(race::MAX_KARTS);
        rows
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
    pub fn is_waiting(&self, race: &RaceOn) -> bool {
        self.session.is_some() && !race.is_on()
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
    pub fn host(&mut self, character: u8, extras: &[u8], nick: &str, public: Option<&str>) {
        self.session = Some(net::host(character, extras, nick, race::SIM_VERSION));
        self.bots_told = None;
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

    /// The characters of the bots of a race the players in the lobby now drive in, as many as
    /// `/bots` asks and the grid has room for.
    fn bot_characters(&self, seed: u64) -> Vec<u8> {
        let taken: Vec<u8> = self.roster.iter().map(|m| m.character).collect();
        let room = race::MAX_KARTS.saturating_sub(taken.len().max(1));
        let count = (self.bots.count as usize).min(room);
        pick_bots(self.bots.pool, self.bots.pick, &taken, count, seed)
    }

    /// Starts the series, with the countdown to its first race (the host only).
    fn start_series(&mut self, series: Series, now: f32) {
        let then = Then::Start {
            track: series.track(),
            laps: series.laps,
        };
        self.series = Some(series);
        self.set_series_bots(now);
        self.count_down(then, now, COUNTDOWN_SECONDS);
    }

    /// Gives the series the bots `/bots` asks for now, for the races it starts from here on. A
    /// bot drives the same character from race to race (a random pick is drawn here, once), so
    /// it keeps its points in a cup.
    fn set_series_bots(&mut self, now: f32) {
        let characters = self.bot_characters(cup::random_seed(now));
        let BotSettings { kind, difficulty, .. } = self.bots;
        let Some(series) = &mut self.series else {
            return;
        };
        let (brains, retail) = match kind {
            BotKind::Brain => (characters, Vec::new()),
            BotKind::Retail => (Vec::new(), characters),
        };
        series.brains = brains;
        // (The difficulty goes with the retail bots, none of them for brain bots: it is their
        // rubber band's, `race::Band`.)
        series.retail = net::RetailBots {
            characters: retail,
            difficulty,
            cup: series.scored,
        };
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
            stopped: false,
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

    /// Whether `/cup pause` is in effect, in a race or between the races of a cup.
    pub(crate) fn is_paused(&self, race: &RaceOn) -> bool {
        race.net().is_some_and(|net| net.pause_at.is_some())
            || self.series.as_ref().is_some_and(|s| s.paused)
    }

    /// `/cup pause` (the host only): everyone's race stops at `tick`, our current one. Between
    /// the races of a cup, the countdown to the next one stops instead.
    fn pause(&mut self, race_on: &RaceOn, tick: u32) {
        if race_on.is_on() {
            if let Some(session) = &self.session {
                session.pause(tick);
            }
            return;
        }
        let Some(series) = self.series.as_mut().filter(|s| s.scored) else {
            return;
        };
        series.paused = true;
        if self.countdown.as_ref().is_some_and(|c| matches!(c.then, Then::Start { .. })) {
            self.countdown = None;
        }
        self.announce("Cup paused");
    }

    /// `/cup resume`: the race goes on from where it stopped, or the countdown to the cup's next
    /// race starts.
    fn resume(&mut self, race_on: &RaceOn, now: f32) {
        if race_on.net().is_some_and(|net| net.pause_at.is_some()) {
            if let Some(session) = &self.session {
                session.resume();
            }
            return;
        }
        let Some(series) = self.series.as_mut().filter(|s| s.paused) else {
            return;
        };
        series.paused = false;
        let then = Then::Start {
            track: series.track(),
            laps: series.laps,
        };
        self.announce("Cup resumed");
        self.count_down(then, now, COUNTDOWN_SECONDS);
    }

    /// `/cup next` (`step` 1) and `/cup previous` (-1), the host only: the race in progress stops
    /// without points, and the countdown goes to the track `step` away. Between races, `step`
    /// counts from the race that was coming up.
    fn step_cup(&mut self, race_on: &mut RaceOn, step: isize, now: f32) {
        let Some(series) = self.series.as_mut().filter(|s| s.scored) else {
            return;
        };
        let Some(race) = (series.race.checked_add_signed(step)).filter(|&r| r < series.tracks.len())
        else {
            return;
        };
        series.race = race;
        series.paused = false;
        let track = series.track();
        let then = Then::Start {
            track,
            laps: series.laps,
        };
        let text = format!("Race {} of {}: {}", race + 1, series.tracks.len(), maps::name(track));
        if race_on.is_on() {
            if let Some(session) = &self.session {
                session.stop();
            }
            self.end_race(race_on);
        }
        self.announce(text);
        self.count_down(then, now, COUNTDOWN_SECONDS);
    }

    /// The race is over, won or stopped: back to a free drive, in the lobby. (The session is told,
    /// so that players who join now do not hear of a race in progress.)
    fn end_race(&mut self, race_on: &mut RaceOn) {
        race_on.0 = None;
        if let Some(session) = &self.session {
            session.over();
        }
    }

    /// Leaves the lobby, ours or another's, and the race in it.
    pub fn close(&mut self, race_on: &mut RaceOn) {
        race_on.0 = None;
        let bots = self.bots;
        *self = Lobby::default();
        self.bots = bots;
    }

    /// Joins the lobby at `ticket`.
    pub fn join(&mut self, ticket: Ticket, character: u8, extras: &[u8], nick: &str) {
        debug!("joining {ticket}");
        self.session = Some(net::join(ticket, character, extras, nick, race::SIM_VERSION));
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

    /// Has the session show everyone our `/bots` (the host only).
    fn tell_bots(&mut self) {
        let bots = self.bots;
        if self.bots_told != Some(bots)
            && let Some(session) = &self.session
        {
            session.show_bots(net::LobbyBots {
                count: bots.count,
                retail: bots.kind == BotKind::Retail,
                difficulty: bots.difficulty,
            });
            self.bots_told = Some(bots);
        }
    }

    /// What the host's bots are, as the lobby shows them: "medium, brain", in the order `/bots` takes them.
    pub fn next_bots_kind(&self) -> String {
        let b = self.lobby_bots;
        let kind = if b.retail { "retail" } else { "brain" };
        format!("{}, {kind}", difficulty_name(b.difficulty))
    }

    /// How many bots the next race of the lobby has, as far as we know: as many as the host's
    /// `/bots` asks and the grid has room for after the players.
    pub fn next_bots(&self) -> usize {
        let karts: usize = self.roster.iter().map(|m| 1 + m.extras.len()).sum();
        let room = race::MAX_KARTS.saturating_sub(karts.max(1));
        (self.lobby_bots.count as usize).min(room)
    }

    /// We drive another character now, which the others learn before the race starts.
    pub fn set_character(&self, character: u8) {
        if let Some(session) = &self.session {
            session.set_character(character);
        }
    }

    /// What our other local players drive, as the lobby last said (the host's word on it).
    pub fn own_extras(&self) -> Option<&[u8]> {
        self.roster.iter().find(|m| m.you).map(|m| m.extras.as_slice())
    }

    /// The other local players drive other characters now, which the others learn before the
    /// race starts.
    pub fn set_extras(&self, extras: &[u8]) {
        if let Some(session) = &self.session {
            session.set_extras(extras);
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

    /// Names the slots of a lobby's race after the nicknames known now, and the bots after
    /// their characters.
    fn refresh_labels(&self, race_on: &mut RaceOn) {
        let Some(race) = race_on.get_mut() else {
            return;
        };
        let Some(ids) = race.net.as_ref().map(|net| &net.ids) else {
            return;
        };
        // One name for each endpoint (its first slot): the others of its slots are its extra
        // players, "name@2", "name@3", ...
        let first = |slot: usize| ids.iter().position(|id| *id == ids[slot]) == Some(slot);
        let players: Vec<(EndpointId, &str)> = (ids.iter().enumerate())
            .filter(|&(slot, _)| !race.is_bot(slot) && first(slot))
            .map(|(_, id)| (*id, self.nicks.get(id).map_or("", String::as_str)))
            .collect();
        let names = net::display_names(&players);
        let name_of = |id: EndpointId| {
            let at = players.iter().position(|(other, _)| *other == id);
            at.map(|at| names[at].clone()).unwrap_or_default()
        };
        let labels = (0..ids.len())
            .map(|slot| match race.characters.get(slot) {
                Some(&c) if race.is_bot(slot) => crate::race_on::bot_label(c),
                _ if first(slot) => name_of(ids[slot]),
                _ => {
                    let nth = (0..=slot).filter(|&s| !race.is_bot(s) && ids[s] == ids[slot]).count();
                    format!("{}@{nth}", name_of(ids[slot]))
                }
            })
            .collect();
        race.labels = labels;
    }

    /// Our pad for the tick being played, which goes to the others at once.
    pub fn send(&self, pad: Pad) {
        if let Some(session) = &self.session {
            session.input(pad);
        }
    }

    /// The pads of our other local players (split screen) for the tick being played, in the order
    /// of their slots (`CurrentRace::locals`, after the first). They go before ours
    /// ([`send`](Self::send)).
    pub fn send_extras(&self, pads: Vec<Pad>) {
        if let Some(session) = &self.session {
            session.extra_input(pads);
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

    /// Starts the race on `track` (the host only), with the bots of the series: its brain bots
    /// (the session takes as many as there is room for), or its retail ones.
    fn start(&self, track: u32, rules: net::Rules) {
        if let Some(session) = &self.session {
            self.provide(session, track);
            let (brains, retail) = self
                .series
                .as_ref()
                .map(|s| (s.brains.clone(), s.retail.clone()))
                .unwrap_or_default();
            session.set_bots(brains);
            session.start(track, Self::hash_of(track), rules, retail);
            self.list(&maps::name(track));
        }
    }

    /// The pads of our bots for the tick being played, in the order of their slots (see
    /// [`CurrentRace::bot_slots`]). They go before ours ([`send`](Self::send)).
    pub fn send_bots(&self, pads: Vec<Pad>) {
        if let Some(session) = &self.session {
            session.bot_input(pads);
        }
    }

    /// Whether we host the lobby we are in.
    pub fn is_host(&self) -> bool {
        self.session.is_some() && !self.joined
    }

    /// The races we, the host, are running, if any.
    pub(crate) fn series(&self) -> Option<&Series> {
        self.series.as_ref()
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

    /// Our part of the race is built, and it can start once the others' are as well.
    pub fn ready(&self) {
        if let Some(session) = &self.session {
            session.ready();
        }
    }

    /// Starts a race of our local players alone on `track`, or starts ours over: no lobby, no
    /// countdown (the race has its lights). A lobby's own start comes from its session instead
    /// (`report`).
    fn race_alone(
        &mut self,
        race_on: &mut RaceOn,
        characters: Vec<u8>,
        nick: &str,
        track: u32,
        rules: net::Rules,
        solo: Option<race::solo::Kind>,
    ) {
        debug_assert!(self.session.is_none());
        // As a lobby's start does (`report`): the flag comes in over what is shown.
        self.cup_next = Some(track);
        race_on.0 = Some(CurrentRace::alone(characters, nick, track, rules, solo));
    }
}

/// The rules of a race we host with `laps` laps. Read at each start, so the wizard's weapons
/// apply from the next race on, a cup's included. Without a seed every race's RNG would start at
/// `RANDOM_NUMBER_INIT`.
fn rules(laps: u8, config: &crate::config::Config, now: f32) -> net::Rules {
    net::Rules {
        laps,
        masked_weapons: vehicle::item::mask(&config.masked_weapons),
        even_odds: config.even_weapon_odds,
        seed: cup::random_seed(now) as u16,
    }
}

pub struct LobbyPlugin;

impl Plugin for LobbyPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<crate::learned::Brains>()
            .init_resource::<RaceOn>();
        app.init_resource::<Lobby>()
            .init_resource::<crate::kart::Smoothing>()
            .init_resource::<RollbackToasts>()
            .init_resource::<lag::FakeLag>()
            .add_systems(Startup, spawn_waiting)
            .add_systems(
                Update,
                (
                    pause_key,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_free_characters_go_first_then_the_pool_repeats() {
        let pool = 1 << 2 | 1 << 5 | 1 << 9;
        assert_eq!(pick_bots(pool, BotPick::Ordered, &[5], 6, 0), [2, 9, 2, 5, 9, 2]);
        assert_eq!(pick_bots(pool, BotPick::Ordered, &[], 2, 0), [2, 5]);
        for seed in 0..20 {
            let picked = pick_bots(pool, BotPick::Random, &[5], 6, seed);
            assert_eq!(picked.len(), 6);
            let mut first = picked[..2].to_vec();
            first.sort();
            assert_eq!(first, [2, 9]);
            assert!(picked.iter().all(|&c| pool & 1 << c != 0));
        }
    }
}
