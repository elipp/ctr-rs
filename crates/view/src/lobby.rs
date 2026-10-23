//! The online lobby (`--drive`): L opens one and prints how to join it, `--join <ticket>` joins
//! one. The host's G starts the race, which is tick 0 for everyone. The connections themselves
//! are `net`'s; this reports what happens to them and hands the race to `drive.rs`.
//!
//! * L: open a lobby (again to close it)
//! * G: start the race, once the others have joined (the host only)
//!
//! The race is a mesh with rollback: every player plays its own pad at once and sends it to all
//! the others, and the pads of the others reach `drive_tick` through [`Online::inbox`]. A pad
//! that has not come yet is predicted, and put right when it does (`race::rollback`).

use bevy::prelude::*;
use net::{Event, Pad, Session, Ticket};

use crate::toast::Toast;

/// A race that has started.
pub struct Online {
    /// Our player.
    pub slot: usize,
    /// The character each slot drives, which sets its kart's stats.
    pub characters: Vec<u8>,
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
    pub online: Option<Online>,
}

impl Lobby {
    /// A lobby that is joined from the start, for `--join`.
    pub fn joining(ticket: Ticket, character: u8) -> Lobby {
        println!("joining {ticket}");
        Lobby {
            session: Some(net::join(ticket, character)),
            online: None,
        }
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
            .init_resource::<crate::remote::Remotes>()
            .add_systems(
                Update,
                (toggle, start, report, crate::remote::build).chain(),
            );
    }
}

fn toggle(
    keys: Res<ButtonInput<KeyCode>>,
    mut lobby: ResMut<Lobby>,
    drive: Res<crate::drive::Drive>,
    mut toasts: MessageWriter<Toast>,
) {
    if !keys.just_pressed(KeyCode::KeyL) {
        return;
    }
    if lobby.session.take().is_some() {
        lobby.online = None;
        toasts.write(Toast::new(
            "Lobby closed!",
            2000,
            Color::srgb(1.0, 1.0, 1.0),
        ));
    } else {
        lobby.session = Some(net::host(drive.character as u8));
        toasts.write(Toast::new(
            "Opening lobby...",
            3000,
            Color::srgb(1.0, 1.0, 1.0),
        ));
    }
}

fn start(keys: Res<ButtonInput<KeyCode>>, lobby: Res<Lobby>, track: Res<crate::Track>) {
    if keys.just_pressed(KeyCode::KeyG)
        && lobby.online.is_none()
        && let Some(session) = &lobby.session
    {
        session.start(track.1.0);
    }
}

fn report(mut lobby: ResMut<Lobby>, mut toasts: MessageWriter<Toast>) {
    let Some(session) = &lobby.session else {
        return;
    };
    let mut failed = false;
    let mut started = None;
    let mut go = false;
    let mut pads = Vec::new();
    let mut sums = Vec::new();
    for event in session.poll() {
        match event {
            Event::Open(ticket) => {
                println!(
                    "lobby open. To join it, run:\n  cargo run -p view -- --drive --join {ticket}\nPress G to start the race once the others have joined."
                )
            }
            Event::PeerJoined(id) => {
                println!("{} joined", id.fmt_short());
                toasts.write(Toast::new(
                    format!("{} joined", id.fmt_short()),
                    3000,
                    Color::srgb(0.4, 1.0, 0.4),
                ));
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
                track,
            } => {
                println!(
                    "race started: you are player {slot} of {}, characters {characters:?}, track {track}",
                    characters.len()
                );
                let characters_len = characters.len();
                started = Some(Online {
                    slot,
                    characters,
                    track,
                    inbox: Vec::new(),
                    fresh: true,
                    built: false,
                    go: false,
                    specs: Vec::new(),
                    corrections: 0,
                    replayed: 0,
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
    if let Some(online) = &mut lobby.online {
        online.go |= go;
        online.inbox.extend(pads);
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
            toasts.write(Toast::new(
                format!("Out of sync with player {} (tick {})", d.slot, d.tick),
                8000,
                Color::srgb(1.0, 0.3, 0.3),
            ));
        }
    }
    if failed {
        lobby.session = None;
        lobby.online = None;
    }
}
