//! The lobby's commands (`/lobby`, `/race`, `/net`, `/cache`) and what they answer.

use super::countdown::Then;
use super::progress::RollbackToasts;
use super::cup::{CupDef, Cups, Finisher, Series};
use super::{ANNOUNCE_COLOR, Lobby};
use crate::command::Action;
use crate::console::{Log, console};
use crate::maps;
use crate::toast::Toast;
use bevy::prelude::*;

/// Puts `text` on the system clipboard. It is done on a thread of its own: on Linux the clipboard's
/// owner has to stay up to serve the paste (until something else is copied), and a failure is only
/// reported on the console.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn copy_to_clipboard(text: String) {
    std::thread::spawn(move || {
        let result = arboard::Clipboard::new().and_then(|mut clipboard| {
            #[cfg(target_os = "linux")]
            {
                use arboard::SetExtLinux;
                clipboard.set().wait().text(text)
            }
            #[cfg(not(target_os = "linux"))]
            {
                clipboard.set_text(text)
            }
        });
        if let Err(e) = result {
            warn!("could not copy to the clipboard: {e}");
        }
    });
}

/// The page's clipboard, in a browser.
#[cfg(target_arch = "wasm32")]
pub(super) fn copy_to_clipboard(text: String) {
    crate::web::copy(&text);
}

/// Why `action` cannot be done now, if it cannot.
pub(super) fn refusal(action: &Action, lobby: &Lobby) -> Option<&'static str> {
    Some(match action {
        Action::CreateLobby { .. } | Action::JoinLobby(_) | Action::SpectateLobby(_)
            if lobby.is_open() =>
        {
            "Close the lobby first (/lobby close)"
        }
        Action::ToggleSpectator if !lobby.is_open() || !lobby.joined => {
            "Only a guest of a lobby can do that (the host drives)"
        }
        Action::ToggleSpectator if lobby.online.is_some() => "Not during a race",
        Action::CloseLobby if !lobby.is_open() => "No lobby to close",
        Action::PurgeCache if lobby.online.is_some() => "Not during a race",
        Action::NetInfo if !lobby.is_open() => "No lobby (/lobby create)",
        Action::Say(_) if !lobby.is_open() => "Not in a lobby to talk to (/lobby create)",
        Action::StartRace | Action::RestartRace if !lobby.is_open() || lobby.joined => {
            "Only the host of a lobby can do that"
        }
        Action::StartRace if lobby.online.is_some() => "The race has started (/race restart)",
        Action::RestartRace if lobby.online.is_none() => "No race to restart (/race start)",
        Action::DebugRanking if !lobby.is_open() || lobby.joined || lobby.online.is_none() => {
            "Only the host of a lobby can do that, in a race"
        }
        Action::StopRace if !lobby.is_open() || lobby.joined => {
            "Only the host of a lobby can do that"
        }
        Action::StopRace if lobby.online.is_none() && lobby.series.is_none() => "No race to stop",
        Action::CupStart(_) if !lobby.is_open() || lobby.joined => {
            "Only the host of a lobby can do that"
        }
        Action::CupStart(_) if lobby.online.is_some() || lobby.series.is_some() => {
            "A race or a cup is on (/race stop)"
        }
        Action::StartRace | Action::RestartRace | Action::CupStart(_)
            if lobby.countdown.is_some() =>
        {
            "A countdown is already running"
        }
        Action::StartRace if lobby.series.is_some() => "A cup is on (/race stop)",
        _ => return None,
    })
}

/// Puts `items` in a random order (for the debug ranking; the std hasher is random enough).
fn shuffle<T>(items: &mut [T]) {
    use std::hash::BuildHasher;
    let state = std::collections::hash_map::RandomState::new();
    for i in (1..items.len()).rev() {
        items.swap(i, state.hash_one(i) as usize % (i + 1));
    }
}

/// Does what the keys and the command line ask (`command.rs`).
pub(super) fn run(
    mut actions: MessageReader<Action>,
    mut lobby: ResMut<Lobby>,
    drive: Res<crate::drive::Drive>,
    track: Res<crate::Track>,
    time: Res<Time<Real>>,
    mut smoothing: ResMut<crate::kart::Smoothing>,
    mut rollbacks: ResMut<RollbackToasts>,
    config: Res<crate::config::Config>,
    mut cups: ResMut<Cups>,
    mut toasts: MessageWriter<Toast>,
    mut logs: MessageWriter<Log>,
) {
    for action in actions.read() {
        if let Some(why) = refusal(action, &lobby) {
            toasts.write(Toast::new(why, 2500).color(Color::srgb(1.0, 0.4, 0.4)));
            continue;
        }
        let now = time.elapsed_secs();
        let say = match action {
            Action::CreateLobby { public } => {
                lobby.host(drive.character as u8, &config.nickname, public.as_deref());
                if public.is_some() {
                    "Creating public lobby..."
                } else {
                    "Creating lobby..."
                }
            }
            Action::JoinLobby(ticket) => {
                lobby.join(ticket.clone(), drive.character as u8, &config.nickname);
                "Joining lobby..."
            }
            Action::SpectateLobby(ticket) => {
                lobby.spectate(ticket.clone(), &config.nickname);
                "Joining lobby to watch..."
            }
            Action::ToggleSpectator => {
                if lobby.toggle_spectator() {
                    "You will watch the next race"
                } else {
                    "You will drive in the next race"
                }
            }
            Action::CloseLobby => {
                lobby.close();
                "Lobby closed!"
            }
            Action::StartRace => {
                lobby.start_series(Series::single(track.1.0, race::DEFAULT_LAPS), now);
                continue;
            }
            Action::CupCreate { name, tracks, laps } => {
                let cup = CupDef {
                    name: name.clone(),
                    tracks: tracks.clone(),
                    laps: *laps,
                };
                console!(
                    "Cup {name} {}: {} tracks, {laps} laps each",
                    if cups.set(cup) { "replaced" } else { "created" },
                    tracks.len()
                );
                for (n, &id) in tracks.iter().enumerate() {
                    console!("  {}. {} ({})", n + 1, maps::name(id), maps::slug(id));
                }
                console!("Start it in a lobby with /cup start {name}");
                continue;
            }
            Action::CupStart(name) => {
                let Some(def) = cups.get(name) else {
                    logs.write(Log::error(format!("No cup is called {name} (/cup create)")));
                    continue;
                };
                lobby.start_series(Series::cup(def), now);
                continue;
            }
            Action::PurgeCache => match maps::purge() {
                Ok(0) => "The cache is empty",
                Ok(n) => {
                    debug!("deleted {n} cached map(s)");
                    toasts.write(Toast::new(format!("Deleted {n} cached map(s)"), 2500));
                    continue;
                }
                Err(why) => {
                    toasts.write(Toast::new(why, 4000).color(Color::srgb(1.0, 0.4, 0.4)));
                    continue;
                }
            },
            Action::NetInfo => {
                // The answer comes as an event (`report`).
                if let Some(session) = &lobby.session {
                    session.net_info();
                }
                continue;
            }
            Action::NetSmoothing(how) => {
                // Takes hold at the next tick, in a race or out of one.
                *smoothing = *how;
                toasts.write(Toast::new(format!("Smoothing: {how}"), 2500));
                continue;
            }
            Action::NetRollbacks(on) => {
                rollbacks.0 = *on;
                toasts.write(Toast::new(
                    format!("Rollback toasts {}", if *on { "on" } else { "off" }),
                    2000,
                ));
                continue;
            }
            Action::Say(text) => {
                if !lobby.say(text) {
                    logs.write(Log::error("You are talking too fast"));
                }
                continue;
            }
            // (Not the lobby's.)
            Action::DebugRanking => {
                // The others are told the race stopped; the host scores it like a finished one.
                let Some(net) = &lobby.online else { continue };
                let mut finishers: Vec<Finisher> = net
                    .ids
                    .iter()
                    .zip(&net.labels)
                    .take(net.characters.len())
                    .map(|(id, name)| Finisher {
                        id: *id,
                        name: name.clone(),
                    })
                    .collect();
                shuffle(&mut finishers);
                let players = finishers.len();
                if let Some(session) = &lobby.session {
                    session.stop();
                }
                lobby.countdown = None;
                lobby.end_race();
                lobby.race_over(players, &finishers, now);
                continue;
            }
            Action::DebugAddWeapon(_)
            | Action::DebugSetWumpa(_)
            | Action::DebugStall(_)
            | Action::DebugViewer
            | Action::DebugHud(_)
            | Action::DebugLevel(_)
            | Action::DebugCrateBias(_)
            | Action::DebugFx(_)
            | Action::SetConfig(_)
            | Action::Version => continue,
            Action::StopRace => {
                // Like a race that has been won, without the wait and the toast.
                if let Some(session) = lobby.session.as_ref().filter(|_| lobby.online.is_some()) {
                    session.stop();
                }
                lobby.countdown = None;
                let cup = lobby.series.take().is_some_and(|s| s.scored);
                lobby.end_race();
                let text = if cup { "Cup stopped" } else { "Race stopped" };
                toasts.write(Toast::new(text, 3000).color(ANNOUNCE_COLOR));
                continue;
            }
            Action::RestartRace => {
                if let Some(net) = &lobby.online {
                    let then = Then::Restart {
                        track: net.track,
                        hash: net.hash,
                        laps: net.laps,
                    };
                    lobby.count_down(then, now, super::COUNTDOWN_SECONDS);
                }
                continue;
            }
        };
        toasts.write(Toast::new(say, 2500));
    }
}

/// What `/net info` shows: a line per peer with the way our data goes to it (direct or by
/// relay), the round trip time and the packets lost, and a line per other path that is open.
pub(super) fn net_info_text(peers: &[net::PeerInfo]) -> String {
    if peers.is_empty() {
        return "No peers connected".into();
    }
    let ms = |d: std::time::Duration| d.as_secs_f32() * 1000.0;
    let mut lines = Vec::new();
    for peer in peers {
        let loss = if peer.sent_packets == 0 {
            0.0
        } else {
            100.0 * peer.lost_packets as f32 / peer.sent_packets as f32
        };
        let used = peer.paths.iter().find(|p| p.selected);
        let how = match used {
            Some(p) if p.relay => "relay",
            Some(_) => "direct",
            None => "no path",
        };
        let rtt = used.map_or(String::new(), |p| format!(", path {:.0} ms", ms(p.rtt)));
        let ping = peer.ping.map_or("no ping yet".to_string(), |d| {
            format!("ping {:.0} ms", ms(d))
        });
        lines.push(format!(
            "{}: {how}{rtt}, {ping}, lost {}/{} ({loss:.1}%)",
            peer.id.fmt_short(),
            peer.lost_packets,
            peer.sent_packets,
        ));
        for path in peer.paths.iter().filter(|p| !p.selected) {
            let kind = if path.relay { "relay" } else { "direct" };
            lines.push(format!(
                "  also {kind} {} ({:.0} ms)",
                path.addr,
                ms(path.rtt)
            ));
        }
    }
    lines.join("\n")
}
