//! The online lobby (`--drive`): L opens one and prints how to join it, `--join <ticket>` joins
//! one. The host's G starts the race, which is tick 0 for everyone. The connections themselves
//! are `net`'s; this reports what happens to them and hands the race to `drive.rs`.
//!
//! * L: open a lobby (again to close it)
//! * G: start the race, once the others have joined (the host only)
//!
//! The race is lockstep: a tick is played once everyone's pad for it is in, and the pads reach
//! `drive_tick` through [`Online::frames`]. The other players' karts are not simulated yet; the
//! HUD shows their buttons.

use bevy::prelude::*;
use net::{Event, Frame, Pad, Session, Ticket};

/// A race that has started.
pub struct Online {
    /// Our player.
    pub slot: usize,
    pub players: usize,
    /// Everyone's pads, by tick.
    pub frames: Vec<Frame>,
    /// Set by the start and cleared by `drive_tick`, which puts the sim back to tick 0.
    pub fresh: bool,
}

impl Online {
    /// A line for the HUD: where the race is, and the buttons the others held.
    pub fn summary(&self, tick: u32) -> String {
        let others: Vec<String> = self
            .frames
            .get(tick as usize)
            .into_iter()
            .flatten()
            .enumerate()
            .filter(|&(slot, _)| slot != self.slot)
            .map(|(slot, pad)| format!("p{slot} {:x}", pad.held))
            .collect();
        let wait = if self.frames.len() as u32 <= tick { " (waiting)" } else { "" };
        format!("NET {tick} you p{}/{}{wait}  {}", self.slot, self.players, others.join("  "))
    }
}

/// The session this client is in, if any.
#[derive(Resource, Default)]
pub struct Lobby {
    session: Option<Session>,
    pub online: Option<Online>,
}

impl Lobby {
    /// A lobby that is joined from the start, for `--join`.
    pub fn joining(ticket: Ticket) -> Lobby {
        println!("joining {ticket}");
        Lobby { session: Some(net::join(ticket)), online: None }
    }

    /// Our pad for the tick `net::lockstep::INPUT_DELAY` after the one being played.
    pub fn send(&self, pad: Pad) {
        if let Some(session) = &self.session {
            session.input(pad);
        }
    }
}

pub struct LobbyPlugin;

impl Plugin for LobbyPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Lobby>().add_systems(Update, (toggle, start, report));
    }
}

fn toggle(keys: Res<ButtonInput<KeyCode>>, mut lobby: ResMut<Lobby>) {
    if !keys.just_pressed(KeyCode::KeyL) {
        return;
    }
    if lobby.session.take().is_some() {
        lobby.online = None;
        println!("lobby closed");
    } else {
        println!("opening a lobby...");
        lobby.session = Some(net::host());
    }
}

fn start(keys: Res<ButtonInput<KeyCode>>, lobby: Res<Lobby>) {
    if keys.just_pressed(KeyCode::KeyG)
        && lobby.online.is_none()
        && let Some(session) = &lobby.session
    {
        session.start();
    }
}

fn report(mut lobby: ResMut<Lobby>) {
    let Some(session) = &lobby.session else { return };
    let mut failed = false;
    let mut started = None;
    let mut frames = Vec::new();
    for event in session.poll() {
        match event {
            Event::Open(ticket) => {
                println!("lobby open. To join it, run:\n  cargo run -p view -- --drive --join {ticket}\nPress G to start the race once the others have joined.")
            }
            Event::PeerJoined(id) => println!("{} joined", id.fmt_short()),
            Event::Joined(id) => println!("joined the lobby of {}", id.fmt_short()),
            Event::PeerLeft(id) => println!("{} left{}", id.fmt_short(), if lobby.online.is_some() { " (the race waits for them)" } else { "" }),
            Event::Started { slot, players } => {
                println!("race started: you are player {slot} of {players}");
                started = Some(Online { slot, players, frames: Vec::new(), fresh: true });
            }
            Event::Frame { tick, pads } => frames.push((tick, pads)),
            Event::Failed(why) => {
                println!("lobby failed: {why}");
                failed = true;
            }
        }
    }
    if started.is_some() {
        lobby.online = started;
    }
    if let Some(online) = &mut lobby.online {
        for (tick, pads) in frames {
            debug_assert_eq!(tick as usize, online.frames.len());
            online.frames.push(pads);
        }
    }
    if failed {
        lobby.session = None;
        lobby.online = None;
    }
}
