//! The lobby's commands (`/lobby`, `/race`, `/net`, `/cache`) and what they answer.

use super::countdown::Then;
use super::cup::{CupDef, Finisher, Series};
use super::progress::RollbackToasts;
use super::{ANNOUNCE_COLOR, BotKind, Lobby};
use crate::command::{Action, PolicyChoice};
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
pub(crate) fn refusal(action: &Action, lobby: &Lobby) -> Option<&'static str> {
    Some(match action {
        Action::CreateLobby { .. } | Action::JoinLobby(_) | Action::SpectateLobby(_)
            if lobby.is_open() =>
        {
            "Close the lobby first (/lobby close)"
        }
        Action::ToggleSpectator if !lobby.is_open() || !lobby.joined => {
            "Only a guest of a lobby can do that (the host drives)"
        }
        Action::ToggleSpectator if lobby.online.is_some() => net::NOT_NOW,
        Action::CloseLobby if !lobby.is_open() => "No lobby to close",
        Action::Promote(_) if !lobby.is_open() || lobby.joined => {
            "Only the host of a lobby can do that"
        }
        Action::Promote(_)
            if lobby.online.is_some() || lobby.series.is_some() || lobby.countdown.is_some() =>
        {
            net::NOT_NOW
        }
        Action::Players(Some(_)) if lobby.is_open() || lobby.online.is_some() => {
            "The players are set before joining: close the lobby first"
        }
        Action::PurgeCache if lobby.online.is_some() => net::NOT_NOW,
        Action::Character(_) if lobby.online.is_some() || lobby.characters_locked() => net::NOT_NOW,
        // (A host's countdown is to the track it is on.)
        Action::Track(_) if lobby.online.is_some() || lobby.countdown.is_some() => net::NOT_NOW,
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
        Action::CupStart(_) if lobby.online.is_some() || lobby.series.is_some() => net::NOT_NOW,
        Action::StartRace | Action::RestartRace | Action::CupStart(_)
            if lobby.countdown.is_some() =>
        {
            net::NOT_NOW
        }
        Action::StartRace if lobby.series.is_some() => net::NOT_NOW,
        Action::Bots { .. } | Action::BotPolicy(_) if lobby.joined => {
            "Only the host of a lobby can do that"
        }
        Action::BotPolicy(_) if cfg!(target_arch = "wasm32") => {
            "Not on the web: the brain bots drive with the built-in policy"
        }
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
    chosen: Res<crate::drive::Chosen>,
    track: Res<crate::Track>,
    time: Res<Time<Real>>,
    mut smoothing: ResMut<crate::kart::Smoothing>,
    mut rollbacks: ResMut<RollbackToasts>,
    mut lag: ResMut<super::lag::FakeLag>,
    mut config: ResMut<crate::config::Config>,
    mut brains: ResMut<crate::learned::Brains>,
    mut players: ResMut<crate::drive::LocalPlayers>,
    mut toasts: MessageWriter<Toast>,
    mut logs: MessageWriter<Log>,
) {
    for action in actions.read() {
        if let Some(why) = refusal(action, &lobby) {
            toasts.write(Toast::new(why, 2500).color(Color::srgb(1.0, 0.4, 0.4)));
            continue;
        }
        // Whoever watches has no kart of its own: the other players go.
        if matches!(action, Action::SpectateLobby(_)) && players.0 > 1 {
            players.0 = 1;
            toasts.write(Toast::new("Split screen is off while watching", 2500));
        }
        // The other local players join the lobby along with the first, each with the racer picked
        // in its menu (by default the one that follows the one before). Their menus change it
        // later, through `Lobby::set_extras`.
        let extras: Vec<u8> = (1..players.0)
            .map(|slot| chosen.character(slot, drive.character) as u8)
            .collect();
        let now = time.elapsed_secs();
        let say = match action {
            Action::CreateLobby { public } => {
                lobby.host(drive.character as u8, &extras, &config.nickname, public.as_deref());
                if public.is_some() {
                    "Creating public lobby..."
                } else {
                    "Creating lobby..."
                }
            }
            Action::JoinLobby(ticket) => {
                lobby.join(ticket.clone(), drive.character as u8, &extras, &config.nickname);
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
            Action::Promote(name) => {
                // The answer, if the session turns it down, comes as an event (`report`).
                match lobby.player_named(name) {
                    Ok((id, label)) => {
                        let text = format!("Promoting {label}...");
                        if let Some(session) = &lobby.session {
                            session.promote(id);
                        }
                        toasts.write(Toast::new(text, 2500));
                    }
                    Err(why) => {
                        toasts.write(Toast::new(why, 3000).color(Color::srgb(1.0, 0.4, 0.4)));
                    }
                }
                continue;
            }
            Action::StartRace => {
                lobby.start_series(Series::single(track.1.0, race::DEFAULT_LAPS), now);
                continue;
            }
            Action::Bots {
                count,
                difficulty,
                kind,
            } => {
                let changed = count.is_some() || difficulty.is_some() || kind.is_some();
                if let Some(count) = count {
                    lobby.bots.count = *count;
                }
                if let Some(difficulty) = difficulty {
                    lobby.bots.difficulty = *difficulty;
                }
                if let Some(kind) = kind {
                    lobby.bots.kind = *kind;
                }
                let bots = lobby.bots;
                let what = match bots.kind {
                    BotKind::Brain => format!("{} brain", bots.difficulty_name()),
                    BotKind::Retail => format!("{} retail", bots.difficulty_name()),
                };
                let mut text = match bots.count {
                    0 => format!("No bots ({what} when there are)"),
                    1 => format!("1 {what} bot"),
                    n => format!("{n} {what} bots"),
                };
                if bots.count > 0 {
                    text += ", as many as the grid has room for";
                }
                if changed && (lobby.online.is_some() || lobby.series.is_some()) {
                    text += " (from the next race you start)";
                }
                toasts.write(Toast::new(text, 3000));
                continue;
            }
            Action::BotPolicy(choice) => {
                let set = match choice {
                    PolicyChoice::Show => Ok(brains.name()),
                    PolicyChoice::Default => brains.set_path(None),
                    PolicyChoice::File(path) => brains.set_path(Some(path.into())),
                };
                match set {
                    Ok(name) => {
                        let mut text = format!("Brain bots drive with the policy\n{name}");
                        if *choice != PolicyChoice::Show
                            && (lobby.online.is_some() || lobby.series.is_some())
                        {
                            text += "\n(from the next race you start)";
                        }
                        console!("{text}");
                        toasts.write(Toast::new(text, 3000));
                    }
                    Err(e) => {
                        console!("{e}");
                        toasts.write(Toast::new(e, 4000).color(Color::srgb(1.0, 0.4, 0.4)));
                    }
                }
                continue;
            }
            Action::Players(wanted) => {
                if let Some(n) = wanted {
                    players.0 = (*n as usize).clamp(1, crate::drive::MAX_PLAYERS);
                }
                let text = format!("{} player(s) on the screen", players.0);
                console!("{text}");
                toasts.write(Toast::new(text, 2500));
                continue;
            }
            Action::Autopilot(on) => {
                let on = on.unwrap_or(!brains.autopilot);
                // (The policy is read now, to say at once what is wrong with it.)
                match on.then(|| brains.ensure_loaded()).transpose() {
                    Ok(name) => {
                        brains.autopilot = on;
                        let text = match name {
                            Some(name) => format!("Autopilot on, the policy\n{name}"),
                            None => "Autopilot off".to_string(),
                        };
                        console!("{text}");
                        toasts.write(Toast::new(text, 2500));
                    }
                    Err(e) => {
                        console!("{e}");
                        toasts.write(Toast::new(e, 4000).color(Color::srgb(1.0, 0.4, 0.4)));
                    }
                }
                continue;
            }
            Action::CupCreate { name, tracks, laps } => {
                let cup = CupDef {
                    name: name.clone(),
                    tracks: tracks.iter().map(|&id| maps::stable_name(id)).collect(),
                    laps: *laps,
                };
                console!(
                    "Cup {name} {}: {} tracks, {laps} laps each",
                    if config.cups.set(cup) { "replaced" } else { "created" },
                    tracks.len()
                );
                // (It holds for this session even if the file cannot be written.)
                if let Err(why) = config.save() {
                    logs.write(Log::error(why));
                }
                for (n, &id) in tracks.iter().enumerate() {
                    console!("  {}. {} ({})", n + 1, maps::name(id), maps::slug(id));
                }
                console!("Start it in a lobby with /cup start {name}");
                continue;
            }
            Action::CupStart(name) => {
                let Some(def) = config.cups.get(name) else {
                    logs.write(Log::error(format!("No cup is called {name} (/cup create)")));
                    continue;
                };
                match def.resolve() {
                    Ok(tracks) => lobby.start_series(Series::cup(def, tracks), now),
                    Err(why) => {
                        logs.write(Log::error(format!("Cup {}: {why}", def.name)));
                    }
                }
                continue;
            }
            Action::CupDelete(name) => {
                let Some(cup) = config.cups.remove(name) else {
                    logs.write(Log::error(format!("No cup is called {name}")));
                    continue;
                };
                console!("Cup {} deleted", cup.name);
                if let Err(why) = config.save() {
                    logs.write(Log::error(why));
                }
                continue;
            }
            Action::ListTracks => {
                for line in maps::listing() {
                    console!("{line}");
                }
                continue;
            }
            Action::PurgeCache => match maps::purge() {
                Ok(0) => "The cache is empty",
                Ok(n) => {
                    debug!("deleted {n} cached track(s)");
                    toasts.write(Toast::new(format!("Deleted {n} cached track(s)"), 2500));
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
            Action::DebugLag(how) => {
                // The pads held back now keep their time; with the lag off they all go at once.
                lag.lag = *how;
                let text = match how {
                    Some(how) => format!("Pads of the others held back {how} (here only)"),
                    None => "No lag on the pads of the others".to_string(),
                };
                toasts.write(Toast::new(text, 2500));
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
                let mut finishers: Vec<Finisher> = (0..net.karts())
                    .filter_map(|slot| {
                        Some(Finisher {
                            who: net.who(slot)?,
                            name: net.kart_label(slot).to_string(),
                        })
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
            | Action::DebugToast(_)
            | Action::DebugLevel(_)
            | Action::DebugCrateBias(_)
            | Action::DebugFx(_)
            | Action::DebugNavPaths
            | Action::DebugLoadingScreen(_)
            | Action::SetConfig(_)
            | Action::Character(_)
            | Action::Track(_)
            | Action::Version
            | Action::Clear => continue,
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
    let mut lines = Vec::new();
    let relays = net::extra_relays();
    if !relays.is_empty() {
        lines.push("Extra relays:".to_string());
        lines.extend(relays.iter().map(|url| format!("  {url}")));
    }
    if peers.is_empty() {
        lines.push("No peers connected".into());
        return lines.join("\n");
    }
    let ms = |d: std::time::Duration| d.as_secs_f32() * 1000.0;
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
