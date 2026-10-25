//! The online lobby (`--drive`): L opens one and prints its address, `/join-lobby <address>` (`command.rs`)
//! joins one. The host's G starts the race, which is tick 0 for everyone. The connections themselves
//! are `net`'s; this reports what happens to them and hands the race to `drive.rs`.
//!
//! * L or `/create-lobby`: open a lobby (L again, or `/close-lobby`, to close it)
//! * `/join-lobby <address>`: join the lobby at an address
//! * G or `/start-race`: start the race, once the others have joined (the host only)
//! * F9 or `/restart-race`: start the race in progress over (the host only)
//!
//! The race is a mesh with rollback: every player plays its own pad at once and sends it to all
//! the others, and the pads of the others reach `drive_tick` through [`Online::inbox`]. A pad
//! that has not come yet is predicted, and put right when it does (`race::rollback`).

use bevy::prelude::*;
use net::{Event, Pad, Session, Ticket};

use crate::command::Action;
use crate::toast::Toast;

/// A race that has started.
pub struct Online {
    /// Our player.
    pub slot: usize,
    /// The character each slot drives, which sets its kart's stats.
    pub characters: Vec<u8>,
    /// The short form of each slot's iroh key (`fmt_short`), as the players are known by.
    pub names: Vec<String>,
    /// The `LevelId` it is on, which the viewer shows before anything is played.
    pub track: u32,
    /// The pads of the others that have come in and are not in the sim yet: (slot, first tick,
    /// the pads).
    pub inbox: Vec<(usize, u32, Vec<Pad>)>,
    /// Set by the start and cleared by `drive_tick`, which puts the sim back to tick 0.
    pub fresh: bool,
    /// Set once `remote::build` has put the other karts on the grid.
    pub built: bool,
    /// Tick 0 has come: everyone is ready.
    pub go: bool,
    /// The kart of each slot, as the race takes them: set by `remote::build`, which has their
    /// models.
    pub specs: Vec<race::KartSpec>,
    /// How many times a late pad differed from its prediction, and the ticks played again for it.
    pub corrections: u32,
    pub replayed: u32,
    /// The hashes of the sims, ours and the others'.
    pub checks: race::desync::Checks,
    /// How many ticks our clock is ahead of each player's (smoothed; ours is 0), from `net`.
    pub ahead: Vec<f32>,
    /// The players whose sim differs from ours, with the first tick it was seen at.
    pub desync: Vec<Desync>,
}

/// A player whose sim has parted from ours.
pub struct Desync {
    pub slot: usize,
    pub tick: u32,
    /// Whether the player has been told of it (by a toast).
    pub told: bool,
}

impl Online {
    /// A line for the HUD: where the race is, and how many ticks were played on a prediction, and how often (and how far back) a
    /// wrong one had to be put right.
    pub fn summary(&self, tick: u32, predicted: u32) -> String {
        let wait = if !self.go { " (waiting)" } else { "" };
        let mut line = format!(
            "NET {tick} you p{}/{}{wait}  predicting {predicted}  rollbacks {} ({} ticks)",
            self.slot,
            self.characters.len(),
            self.corrections,
            self.replayed,
        );
        for d in &self.desync {
            line += &format!("  DESYNC with p{} at {}", d.slot, d.tick);
        }
        line
    }

    /// Notes the players whose hash differs from ours. A sim that has parted stays apart, so
    /// only the first is kept for each player.
    pub fn found(&mut self, mismatches: impl IntoIterator<Item = race::desync::Mismatch>) {
        for m in mismatches {
            if self.desync.iter().all(|d| d.slot != m.slot) {
                self.desync.push(Desync {
                    slot: m.slot,
                    tick: m.tick,
                    told: false,
                });
            }
        }
    }
}

/// The session this client is in, if any.
#[derive(Resource, Default)]
pub struct Lobby {
    session: Option<Session>,
    /// We joined someone else's lobby (`/join-lobby`) instead of opening one: the host starts the race.
    joined: bool,
    /// Who is in the lobby and the character each drives (the short key and the character), as
    /// the last [`Event::Lobby`] said, and whether there has been one: what changes in the next is
    /// what the players are told.
    roster: Vec<(String, u8)>,
    roster_seen: bool,
    /// The start or restart that is counting down.
    countdown: Option<Countdown>,
    pub online: Option<Online>,
}

impl Lobby {
    /// Whether we are in a lobby (ours or another's), or opening one.
    pub fn is_open(&self) -> bool {
        self.session.is_some()
    }

    /// Opens a lobby of our own, for others to join. (They get its address when it is up.)
    pub fn host(&mut self, character: u8) {
        self.session = Some(net::host(character));
        self.joined = false;
        self.roster.clear();
        self.roster_seen = false;
    }

    /// Starts counting `then` down from [`COUNTDOWN_SECONDS`].
    fn count_down(&mut self, then: Then, now: f32) {
        let text = match then {
            Then::Start(_) => "Race starts in",
            Then::Restart(_) => "Race restarts in",
        };
        self.announce(format!("{text} {COUNTDOWN_SECONDS}"));
        self.countdown = Some(Countdown {
            then,
            left: COUNTDOWN_SECONDS,
            at: now,
        });
    }

    /// Leaves the lobby, ours or another's, and the race in it.
    pub fn close(&mut self) {
        self.session = None;
        self.countdown = None;
        self.online = None;
        self.roster.clear();
        self.roster_seen = false;
    }

    /// Joins the lobby at `ticket`.
    pub fn join(&mut self, ticket: Ticket, character: u8) {
        println!("joining {ticket}");
        self.session = Some(net::join(ticket, character));
        self.joined = true;
        self.roster.clear();
        self.roster_seen = false;
    }

    /// We drive another character now, which the others learn before the race starts.
    pub fn set_character(&self, character: u8) {
        if let Some(session) = &self.session {
            session.set_character(character);
        }
    }

    /// Our pad for the tick being played, which goes to the others at once.
    pub fn send(&self, pad: Pad) {
        if let Some(session) = &self.session {
            session.input(pad);
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
    pub fn restart(&self, track: u32) {
        if let Some(session) = &self.session {
            session.restart(track);
        }
    }

    /// The race is built and can start, once the others are as well.
    pub fn ready(&self) {
        if let Some(session) = &self.session {
            session.ready();
        }
    }
}

/// How much of a new reading of the clocks' difference is taken in (they come 30 times a second
/// from each player).
const LEAD_SMOOTHING: f32 = 0.1;

/// The ticks we may be ahead of the others before the clock is slowed.
const LEAD_ALLOWED: f32 = 1.0;

/// How much slower the clock runs for each tick ahead past that, and the slowest it gets.
const SLOWDOWN_PER_TICK: f32 = 0.03;
const SLOWEST: f32 = 0.9;

/// The longest frame the clock takes in during an online race: after a hitch the ticks that were
/// missed are played at once, instead of being lost (Bevy's own limit is a quarter second).
const CATCH_UP: std::time::Duration = std::time::Duration::from_secs(3);

/// Bevy's own limit on a frame, for everything but an online race.
const NORMAL_MAX_DELTA: std::time::Duration = std::time::Duration::from_millis(250);

pub struct LobbyPlugin;

impl Plugin for LobbyPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Lobby>()
            .add_systems(Startup, spawn_waiting)
            .add_systems(
                Update,
                (run, countdown, report, crate::remote::build, pace, waiting).chain(),
            );
    }
}

/// Why `action` cannot be done now, if it cannot.
fn refusal(action: &Action, lobby: &Lobby) -> Option<&'static str> {
    Some(match action {
        Action::CreateLobby | Action::JoinLobby(_) if lobby.is_open() => {
            "Close the lobby first (/close-lobby)"
        }
        Action::CloseLobby if !lobby.is_open() => "No lobby to close",
        Action::StartRace | Action::RestartRace if !lobby.is_open() || lobby.joined => {
            "Only the host of a lobby can do that"
        }
        Action::StartRace if lobby.online.is_some() => "The race has started (/restart-race)",
        Action::RestartRace if lobby.online.is_none() => "No race to restart (/start-race)",
        Action::StartRace | Action::RestartRace if lobby.countdown.is_some() => {
            "A countdown is already running"
        }
        _ => return None,
    })
}

/// Does what the keys and the command line ask (`command.rs`).
fn run(
    mut actions: MessageReader<Action>,
    mut lobby: ResMut<Lobby>,
    drive: Res<crate::drive::Drive>,
    track: Res<crate::Track>,
    time: Res<Time<Real>>,
    mut toasts: MessageWriter<Toast>,
) {
    for action in actions.read() {
        if let Some(why) = refusal(action, &lobby) {
            toasts.write(Toast::new(why, 2500).color(Color::srgb(1.0, 0.4, 0.4)));
            continue;
        }
        let now = time.elapsed_secs();
        let say = match action {
            Action::CreateLobby => {
                lobby.host(drive.character as u8);
                "Opening lobby..."
            }
            Action::JoinLobby(ticket) => {
                lobby.join(ticket.clone(), drive.character as u8);
                "Joining lobby..."
            }
            Action::CloseLobby => {
                lobby.close();
                "Lobby closed!"
            }
            Action::StartRace => {
                lobby.count_down(Then::Start(track.1.0), now);
                continue;
            }
            // (Not the lobby's.)
            Action::DebugAddWeapon(_) => continue,
            Action::RestartRace => {
                if let Some(net) = &lobby.online {
                    let then = Then::Restart(net.track);
                    lobby.count_down(then, now);
                }
                continue;
            }
        };
        toasts.write(Toast::new(say, 2500));
    }
}

fn report(mut lobby: ResMut<Lobby>, mut toasts: MessageWriter<Toast>) {
    let Some(session) = &lobby.session else {
        return;
    };
    let mut failed = false;
    let mut roster = None;
    let mut started = None;
    let mut go = false;
    let mut pads = Vec::new();
    let mut sums = Vec::new();
    let mut leads = Vec::new();
    for event in session.poll() {
        match event {
            Event::Open(ticket) => {
                println!(
                    "lobby open. To join it, type this in another viewer:\n  /join-lobby {ticket}\nPress G to start the race once the others have joined."
                );
                toasts.write(
                    Toast::new("Lobby opened!".to_string(), 2000)
                        .color(Color::linear_rgb(0.4, 1.0, 0.4)),
                );
            }
            Event::PeerJoined(id) => println!("{} joined", id.fmt_short()),
            Event::Lobby { players, you } => roster = Some((players, you)),
            Event::Announce(text) => {
                toasts.write(Toast::new(text, 3000).color(ANNOUNCE_COLOR));
            }
            Event::Joined(id) => println!("joined the lobby of {}", id.fmt_short()),
            Event::PeerLeft(id) => println!(
                "{} left{}",
                id.fmt_short(),
                if lobby.online.is_some() {
                    " (the race goes on with their last pad)"
                } else {
                    ""
                }
            ),
            Event::Started {
                slot,
                characters,
                players,
                track,
            } => {
                println!(
                    "race started: you are player {slot} of {}, characters {characters:?}, track {track}",
                    characters.len()
                );
                // Whatever came before belongs to the race before this one.
                pads.clear();
                sums.clear();
                leads.clear();
                let characters_len = characters.len();
                started = Some(Online {
                    slot,
                    characters,
                    names: players
                        .iter()
                        .map(|id| id.fmt_short().to_string())
                        .collect(),
                    track,
                    inbox: Vec::new(),
                    fresh: true,
                    built: false,
                    go: false,
                    specs: Vec::new(),
                    corrections: 0,
                    replayed: 0,
                    ahead: vec![0.0; characters_len],
                    checks: race::desync::Checks::new(characters_len, slot),
                    desync: Vec::new(),
                });
            }
            Event::Go => go = true,
            Event::Pads {
                slot,
                first,
                pads: p,
            } => pads.push((slot, first, p)),
            Event::Lead { slot, ahead } => leads.push((slot, ahead)),
            Event::Checksum { slot, tick, hash } => sums.push((slot, tick, hash)),
            Event::Failed(why) => {
                println!("lobby failed: {why}");
                failed = true;
            }
        }
    }
    if started.is_some() {
        lobby.online = started;
    }
    if let Some((players, you)) = roster {
        let players: Vec<(String, u8)> = players
            .iter()
            .map(|(id, c)| (id.fmt_short().to_string(), *c))
            .collect();
        // A guest that has just joined meets the lobby as it is, which is not news.
        if lobby.roster_seen || !lobby.joined {
            for (n, (id, character)) in players.iter().enumerate() {
                let name = crate::CHARACTERS
                    .get(*character as usize)
                    .copied()
                    .unwrap_or("?");
                let who = if Some(n) == you { "you" } else { id.as_str() };
                let before = lobby.roster.iter().find(|(known, _)| known == id);
                let text = match before {
                    // We know that we joined.
                    None if Some(n) == you => continue,
                    None => format!("{who} joined as {name}"),
                    Some((_, was)) if was != character => format!("{who} is now {name}"),
                    Some(_) => continue,
                };
                toasts.write(Toast::new(text, 3000).color(Color::srgb(0.4, 1.0, 0.4)));
            }
            for (id, _) in &lobby.roster {
                if players.iter().all(|(now, _)| now != id) {
                    toasts.write(
                        Toast::new(format!("{id} left"), 3000).color(Color::srgb(1.0, 0.6, 0.3)),
                    );
                }
            }
        }
        lobby.roster = players;
        lobby.roster_seen = true;
    }
    if let Some(online) = &mut lobby.online {
        online.go |= go;
        online.inbox.extend(pads);
        for (slot, ahead) in leads {
            if let Some(smoothed) = online.ahead.get_mut(slot) {
                *smoothed += (ahead - *smoothed) * LEAD_SMOOTHING;
            }
        }
        for (slot, tick, hash) in sums {
            let found = online.checks.theirs(slot, tick, hash);
            online.found(found);
        }
        for d in online.desync.iter_mut().filter(|d| !d.told) {
            d.told = true;
            println!(
                "DESYNC: the sim of player {} differs from ours at tick {}",
                d.slot, d.tick
            );
            toasts.write(
                Toast::new(
                    format!("Out of sync with player {} (tick {})", d.slot, d.tick),
                    8000,
                )
                .color(Color::srgb(1.0, 0.3, 0.3)),
            );
        }
    }
    if failed {
        lobby.session = None;
        lobby.online = None;
        lobby.roster.clear();
        lobby.roster_seen = false;
    }
}

/// Keeps the clock of an online race with the others': it catches up after a hitch, and runs a bit
/// slow while it is ahead of any of them (the ones behind do not hurry; they play on as they do).
fn pace(lobby: Res<Lobby>, mut time: ResMut<Time<Virtual>>) {
    let running = lobby.online.as_ref().filter(|net| net.go);
    let max_delta = if running.is_some() {
        CATCH_UP
    } else {
        NORMAL_MAX_DELTA
    };
    if time.max_delta() != max_delta {
        time.set_max_delta(max_delta);
    }
    let ahead = running.map_or(0.0, |net| {
        net.ahead
            .iter()
            .enumerate()
            .filter(|&(slot, _)| slot != net.slot)
            .fold(0.0_f32, |most, (_, &a)| most.max(a))
    });
    let speed = (1.0 - SLOWDOWN_PER_TICK * (ahead - LEAD_ALLOWED).max(0.0)).max(SLOWEST);
    if time.relative_speed() != speed {
        time.set_relative_speed(speed);
    }
}

/// What the host says to everyone is yellow.
const ANNOUNCE_COLOR: Color = Color::srgb(1.0, 0.9, 0.2);

/// The seconds of the countdown before the host's race starts, or starts over.
const COUNTDOWN_SECONDS: u32 = 5;

/// What the countdown ends in.
#[derive(Clone, Copy)]
enum Then {
    /// The first race of the lobby, on this track.
    Start(u32),
    /// The same players again, on this track.
    Restart(u32),
}

/// The countdown of a start or a restart: the seconds left, and when the last one was announced.
struct Countdown {
    then: Then,
    left: u32,
    at: f32,
}

/// Counts a start or a restart down, which everyone sees (the host only). Closing the lobby
/// cancels it.
fn countdown(mut lobby: ResMut<Lobby>, time: Res<Time<Real>>) {
    if lobby.joined || lobby.session.is_none() {
        lobby.countdown = None;
        return;
    }
    let now = time.elapsed_secs();
    let Some(mut c) = lobby.countdown.take() else {
        return;
    };
    if now - c.at >= 1.0 {
        c.left -= 1;
        c.at += 1.0;
        if c.left == 0 {
            match c.then {
                Then::Start(track) => {
                    if let Some(session) = &lobby.session {
                        session.start(track);
                    }
                }
                Then::Restart(track) => lobby.restart(track),
            }
            return;
        }
        lobby.announce(format!("{}...", c.left));
    }
    lobby.countdown = Some(c);
}

/// The label for the players who wait for the host.
#[derive(Component)]
struct WaitingHud;

/// How often the label blinks: a second on, a second off.
const BLINK_SECONDS: f32 = 1.0;

pub fn spawn_waiting(mut commands: Commands) {
    commands.spawn((
        WaitingHud,
        Text::new("Waiting for host to start the race"),
        TextLayout::justify(Justify::Center),
        Visibility::Hidden,
        Node {
            position_type: PositionType::Absolute,
            top: Val::Percent(20.0),
            width: Val::Percent(100.0),
            ..default()
        },
    ));
}

/// Shows the label, blinking, to a player who is in a lobby and has not been started yet.
fn waiting(
    lobby: Res<Lobby>,
    time: Res<Time<Real>>,
    mut label: Query<&mut Visibility, With<WaitingHud>>,
) {
    let waiting = lobby.joined && lobby.session.is_some() && lobby.online.is_none();
    let on = ((time.elapsed_secs() / BLINK_SECONDS) as u32).is_multiple_of(2);
    for mut visibility in &mut label {
        *visibility = if waiting && on {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
}
