//! The lobby's commands (`/lobby`, `/race`, `/net`, `/cache`) and what they answer.

use super::countdown::Then;
use super::cup::{CupDef, Finisher, Series};
use super::progress::RollbackToasts;
use super::{ANNOUNCE_COLOR, BotKind, Lobby};
use crate::ghost::Source;
use crate::race_on::{CurrentRace, RaceOn};
use crate::command::{Action, PolicyChoice};
use crate::console::{Log, console};
use crate::controls::Control;
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
pub(crate) fn refusal(action: &Action, lobby: &Lobby, race_on: &RaceOn) -> Option<&'static str> {
    let racing = race_on.is_on();
    Some(match action {
        Action::CreateLobby { .. } | Action::JoinLobby(_) | Action::SpectateLobby(_) | Action::QuickPlay { .. }
            if lobby.is_open() =>
        {
            "Close the lobby first (/lobby close)"
        }
        // (A race alone has no session to carry over into the lobby.)
        Action::CreateLobby { .. } | Action::JoinLobby(_) | Action::SpectateLobby(_) | Action::QuickPlay { .. }
            if racing =>
        {
            "Stop the race first (/race stop)"
        }
        Action::ToggleSpectator if !lobby.is_open() || !lobby.joined => {
            "Only a guest of a lobby can do that (the host drives)"
        }
        Action::ToggleSpectator if racing => net::NOT_NOW,
        Action::CloseLobby if !lobby.is_open() => "No lobby to close",
        Action::AnswerMatch(_) if lobby.unanswered().is_none() => "No match to answer",
        Action::QuickPlay { rejoin: true } if lobby.last_match.is_none() => "No match to rejoin",
        Action::QuickReady | Action::VoteTrack(_) if !lobby.between_matched_races(race_on) => {
            "Only after a quick play race"
        }
        Action::QuickReady if lobby.again => "You are ready already: waiting for the others",
        Action::VoteTrack(track) if *track >= race::RACING_TRACKS => "Vote for one of the game's tracks",
        Action::Promote(_) if !lobby.is_open() || lobby.joined => {
            "Only the host of a lobby can do that"
        }
        Action::Promote(_)
            if racing || lobby.series.is_some() || lobby.countdown.is_some() =>
        {
            net::NOT_NOW
        }
        Action::Players(Some(_)) if lobby.is_open() => {
            "The players are set before joining: close the lobby first"
        }
        Action::Players(Some(_)) if racing => "Stop the race first (/race stop)",
        Action::PurgeCache if racing => net::NOT_NOW,
        Action::Character(_) if racing || lobby.characters_locked() => net::NOT_NOW,
        // (A host's countdown is to the track it is on.)
        Action::Track(_) if racing || lobby.countdown.is_some() => net::NOT_NOW,
        Action::NetInfo if !lobby.is_open() => "No lobby (/lobby create)",
        Action::Say(_) if !lobby.is_open() => "Not in a lobby to talk to (/lobby create)",
        // (Outside a lobby they are of a race alone.)
        Action::StartRace | Action::RestartRace if lobby.joined => {
            "Only the host of a lobby can do that"
        }
        Action::StartRace | Action::StartSolo(_) if racing => "The race has started (/race restart)",
        Action::StartSolo(_) if lobby.is_open() => "A solo run is alone: close the lobby first (/lobby close)",
        Action::RestartRace if !racing => "No race to restart (/race start)",
        Action::Watch(_) if lobby.is_open() => "A replay is alone: close the lobby first (/lobby close)",
        Action::Watch(_) if race_on.get().is_none_or(|race| race.solo.is_none()) => {
            "Only in a solo race"
        }
        Action::DebugRanking if !lobby.is_open() || lobby.joined || !racing => {
            "Only the host of a lobby can do that, in a race"
        }
        Action::StopRace if lobby.joined => {
            "Only the host of a lobby can do that"
        }
        Action::StopRace if !racing && lobby.series.is_none() => "No race to stop",
        Action::CupStart(_) | Action::CupRandom { .. } | Action::CupDrive { .. } if !lobby.is_open() || lobby.joined => {
            "Only the host of a lobby can do that"
        }
        Action::CupStart(_) | Action::CupRandom { .. } | Action::CupDrive { .. }
            if lobby.series.as_ref().is_some_and(|s| s.paused) =>
        {
            "A cup is paused (/cup resume, or /cup stop to end it)"
        }
        Action::CupStart(_) | Action::CupRandom { .. } | Action::CupDrive { .. }
            if racing || lobby.series.is_some() =>
        {
            net::NOT_NOW
        }
        Action::CupPause | Action::CupResume if !lobby.is_open() || lobby.joined => {
            "Only the host of a lobby can do that"
        }
        Action::CupPause | Action::CupResume
            if !racing && !lobby.series.as_ref().is_some_and(|s| s.scored) =>
        {
            "No race or cup in progress (/cup start)"
        }
        Action::CupPause if lobby.is_paused(race_on) => "Paused already (/cup resume)",
        Action::CupNext | Action::CupPrevious if !lobby.is_open() || lobby.joined => {
            "Only the host of a lobby can do that"
        }
        Action::CupNext | Action::CupPrevious
            if !lobby.series.as_ref().is_some_and(|s| s.scored) =>
        {
            "No cup in progress (/cup start)"
        }
        Action::CupNext if !lobby.series.as_ref().is_some_and(|s| s.has_next()) => {
            "This is the cup's last track"
        }
        Action::CupPrevious if lobby.series.as_ref().is_some_and(|s| s.race == 0) => {
            "This is the cup's first track"
        }
        Action::CupResume if !lobby.is_paused(race_on) => "Not paused (/cup pause)",
        Action::StartRace | Action::RestartRace | Action::CupStart(_) | Action::CupRandom { .. } | Action::CupDrive { .. }
            if lobby.countdown.is_some() =>
        {
            net::NOT_NOW
        }
        Action::StartRace if lobby.series.is_some() => net::NOT_NOW,
        Action::Bots { .. } | Action::BotPolicy(_) if lobby.joined => {
            "Only the host of a lobby can do that"
        }
        // (Every bit is a character, so any other mask has one.)
        Action::Bots { pool: Some(0), .. } => "The pool needs at least one character",
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

/// [`Control::PauseRace`] (G by default) is `/cup pause`, or `/cup resume` when paused. Only in a
/// lobby; a guest gets the usual host-only refusal.
pub(super) fn pause_key(
    keys: Res<ButtonInput<KeyCode>>,
    config: Res<crate::config::Config>,
    menu: Res<crate::pause::PauseMenu>,
    lobby: Res<Lobby>,
    race_on: Res<RaceOn>,
    mut actions: MessageWriter<Action>,
) {
    if menu.takes_keys()
        || !lobby.is_open()
        || !config.controls.just_pressed(Control::PauseRace, &keys)
    {
        return;
    }
    actions.write(if lobby.is_paused(&race_on) {
        Action::CupResume
    } else {
        Action::CupPause
    });
}

/// Does what the keys and the command line ask (`command.rs`).
pub(super) fn run(
    mut actions: MessageReader<Action>,
    mut lobby: ResMut<Lobby>,
    mut race_on: ResMut<RaceOn>,
    (options, mode, world_record): (
        Res<crate::Options>,
        Res<State<crate::Mode>>,
        Res<crate::world_record::WorldRecord>,
    ),
    chosen: Res<crate::drive::Chosen>,
    track: Res<crate::Track>,
    time: Res<Time<Real>>,
    mut smoothing: ResMut<crate::kart::Smoothing>,
    mut rollbacks: ResMut<RollbackToasts>,
    mut lag: ResMut<super::lag::FakeLag>,
    mut config: ResMut<crate::config::Config>,
    mut brains: ResMut<crate::learned::Brains>,
    mut players: ResMut<crate::drive::LocalPlayers>,
    sim: Res<crate::props::Sim>,
    mut toasts: MessageWriter<Toast>,
    mut logs: MessageWriter<Log>,
) {
    for action in actions.read() {
        // (`say`'s.)
        if matches!(action, Action::Say(_)) {
            continue;
        }
        if let Some(why) = refusal(action, &lobby, &race_on) {
            toasts.write(Toast::new(why, 2500).color(Color::srgb(1.0, 0.4, 0.4)));
            continue;
        }
        // The track shown is the podium's. (`/track` is not refused: it leaves the podium,
        // `scene::switch_level`.)
        let needs_track = matches!(
            action,
            Action::StartRace
                | Action::StartSolo(_)
                | Action::CupStart(_)
                | Action::CupRandom { .. }
                | Action::CupDrive { .. }
        );
        if needs_track && *mode.get() == crate::Mode::Podium {
            toasts.write(Toast::new("Not on the podium", 2500).color(Color::srgb(1.0, 0.4, 0.4)));
            continue;
        }
        let character = options.racer as usize;
        // Whoever watches has no kart of its own: the other players go.
        if matches!(action, Action::SpectateLobby(_)) && players.0 > 1 {
            players.0 = 1;
            toasts.write(Toast::new("Split screen is off while watching", 2500));
        }
        // The other local players join the lobby along with the first, each with the racer picked
        // in its menu (by default the one that follows the one before). Their menus change it
        // later, through `Lobby::set_extras`.
        let extras: Vec<u8> = (1..players.0)
            .map(|slot| chosen.character(slot, character) as u8)
            .collect();
        let now = time.elapsed_secs();
        let say = match action {
            Action::CreateLobby { public } => {
                lobby.host(character as u8, &extras, &config.nickname, public.as_deref());
                if public.is_some() {
                    "Creating public lobby..."
                } else {
                    "Creating lobby..."
                }
            }
            Action::JoinLobby(ticket) => {
                lobby.join(ticket.clone(), character as u8, &extras, &config.nickname);
                "Joining lobby..."
            }
            Action::QuickPlay { rejoin } => {
                let rejoin = lobby.last_match.filter(|_| *rejoin);
                let queued = crate::player::key().and_then(|key| {
                    let server = net::score::default_score().ok_or("No score server to queue at")?;
                    lobby.quick_play(key, server, character as u8, &extras, &config.nickname, rejoin);
                    Ok(())
                });
                match queued {
                    Ok(()) if rejoin.is_some() => "Rejoining the match...",
                    Ok(()) => "Searching for a match...",
                    Err(why) => {
                        toasts.write(Toast::new(why, 3000).color(Color::srgb(1.0, 0.4, 0.4)));
                        continue;
                    }
                }
            }
            Action::QuickReady => {
                lobby.play_again();
                "Ready: waiting for the others..."
            }
            Action::VoteTrack(track) => {
                if let Some(session) = &lobby.session {
                    session.vote(*track);
                }
                "Vote taken"
            }
            Action::AnswerMatch(accept) => {
                lobby.answer_match(*accept);
                // (Whoever of the match hosts is the session's business: to the game every
                // player of it is a guest, with none of a host's say.)
                if *accept {
                    if let Some(super::Queue::Found { accepted, .. }) = &mut lobby.queue {
                        *accepted = true;
                    }
                    "Accepted: waiting for the others..."
                } else {
                    "Declined"
                }
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
                lobby.close(&mut race_on);
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
                if lobby.is_open() {
                    lobby.start_series(Series::single(track.1.0, race::DEFAULT_LAPS), now);
                } else {
                    let characters = std::iter::once(character as u8).chain(extras).collect();
                    let rules = super::rules(race::DEFAULT_LAPS, &config, now);
                    let nick = &config.nickname;
                    lobby.race_alone(&mut race_on, characters, nick, track.1.0, rules, None);
                }
                continue;
            }
            Action::Watch(source) => {
                let Some(race) = race_on.get() else { continue };
                use crate::world_record::{date, is_ours, our_name, short_name};
                // (A best from before they were dated has the record's date, if it is the record.)
                let watched = match source {
                    Source::Best => crate::pb::run(race).map(|run| {
                        let date = crate::pb::set_at(race)
                            .map(|at| date(at.as_second()))
                            .or_else(|| {
                                (world_record.of(race))
                                    .filter(|_| world_record.is_best(race))
                                    .map(|(entry, ..)| date(entry.at))
                            });
                        (run, our_name(&config.nickname), date)
                    }),
                    Source::World => world_record.of(race).zip(world_record.run(race)).map(
                        |((entry, ..), run)| {
                            let who = if is_ours(entry) {
                                our_name(&entry.nick)
                            } else {
                                short_name(entry)
                            };
                            (run.clone(), who, Some(date(entry.at)))
                        },
                    ),
                };
                let Some((run, who, date)) = watched else {
                    toasts.write(Toast::new("No run to watch", 2000));
                    continue;
                };
                let rules = super::rules(run.laps, &config, now);
                let mut replay = CurrentRace::alone(
                    vec![run.demo.character as u8],
                    &who,
                    race.track,
                    rules,
                    race.solo,
                );
                replay.replay = Some(crate::replay::Replay::new(run, *source, who, date));
                lobby.start_alone(&mut race_on, replay);
                continue;
            }
            Action::StartSolo(kind) => {
                if !extras.is_empty() {
                    let why = "A solo run is for one player (/players 1)";
                    toasts.write(Toast::new(why, 2500).color(Color::srgb(1.0, 0.4, 0.4)));
                    continue;
                }
                let characters = vec![character as u8];
                let rules = super::rules(race::DEFAULT_LAPS, &config, now);
                let solo = Some(*kind);
                lobby.race_alone(&mut race_on, characters, &config.nickname, track.1.0, rules, solo);
                continue;
            }
            Action::Bots {
                count,
                difficulty,
                kind,
                pool,
                pick,
                quiet,
            } => {
                let changed = count.is_some()
                    || difficulty.is_some()
                    || kind.is_some()
                    || pool.is_some()
                    || pick.is_some();
                if let Some(count) = count {
                    lobby.bots.count = *count;
                }
                if let Some(difficulty) = difficulty {
                    lobby.bots.difficulty = *difficulty;
                }
                if let Some(kind) = kind {
                    lobby.bots.kind = *kind;
                }
                if let Some(pool) = pool {
                    lobby.bots.pool = *pool;
                }
                if let Some(pick) = pick {
                    lobby.bots.pick = *pick;
                }
                let bots = lobby.bots;
                let difficulty = super::difficulty_name(bots.difficulty);
                let what = match bots.kind {
                    BotKind::Brain => format!("{difficulty} brain"),
                    BotKind::Retail => format!("{difficulty} retail"),
                };
                let mut text = match bots.count {
                    0 => format!("No bots ({what} when there are)"),
                    1 => format!("1 {what} bot"),
                    n => format!("{n} {what} bots"),
                };
                if bots.count > 0 {
                    text += ", as many as the grid has room for";
                }
                if bots.pool != super::ALL_CHARACTERS {
                    text += &match bots.pool.count_ones() {
                        1 => ", of 1 character".to_string(),
                        n => format!(", of {n} characters"),
                    };
                }
                if bots.pick == super::BotPick::Random {
                    text += ", picked at random";
                }
                if changed {
                    lobby.set_series_bots(now);
                    if race_on.is_on() {
                        let cup = lobby.series.as_ref().is_some_and(|s| s.scored);
                        text += if cup {
                            " (from the cup's next race)"
                        } else {
                            " (from the next race you start)"
                        };
                    }
                }
                if !quiet {
                    toasts.write(Toast::new(text, 3000));
                }
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
                            && (race_on.is_on() || lobby.series.is_some())
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
            Action::CupRandom { tracks, laps } => {
                let tracks = super::cup::random_tracks(*tracks, super::cup::random_seed(now));
                let names: Vec<String> = tracks.iter().map(|&id| maps::name(id)).collect();
                console!("Random cup, {laps} laps each: {}", names.join(", "));
                let def = CupDef {
                    name: "Random".into(),
                    tracks: tracks.iter().map(|&id| maps::stable_name(id)).collect(),
                    laps: *laps,
                };
                lobby.start_series(Series::cup(&def, tracks), now);
                continue;
            }
            Action::CupDrive { name, tracks, laps } => {
                let def = CupDef {
                    name: name.clone(),
                    tracks: tracks.iter().map(|&id| maps::stable_name(id)).collect(),
                    laps: *laps,
                };
                lobby.start_series(Series::cup(&def, tracks.clone()), now);
                continue;
            }
            Action::CupPause => {
                lobby.pause(&race_on, sim.race.tick());
                continue;
            }
            Action::CupResume => {
                lobby.resume(&race_on, now);
                continue;
            }
            Action::CupNext => {
                lobby.step_cup(&mut race_on, 1, now);
                continue;
            }
            Action::CupPrevious => {
                lobby.step_cup(&mut race_on, -1, now);
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
            Action::Say(_) => continue,
            // (Not the lobby's.)
            Action::DebugRanking => {
                // The others are told the race stopped; the host scores it like a finished one.
                let Some(race) = race_on.get() else { continue };
                let mut finishers: Vec<Finisher> =
                    (0..race.karts()).filter_map(|slot| race.finisher(slot)).collect();
                shuffle(&mut finishers);
                let players = finishers.len();
                if let Some(session) = &lobby.session {
                    session.stop();
                }
                lobby.countdown = None;
                lobby.end_race(&mut race_on);
                lobby.race_over(players, &finishers, now);
                continue;
            }
            Action::Practice(_)
            | Action::DebugAddWeapon(_)
            | Action::DebugSetWumpa(_)
            | Action::DebugStall(_)
            | Action::DebugViewer
            | Action::DebugPodium(_)
            | Action::DebugHud(_)
            | Action::DebugToast(_)
            | Action::DebugLevel(_)
            | Action::DebugCrateBias(_)
            | Action::DebugFx(_)
            | Action::DebugNavPaths
            | Action::SetConfig(_)
            | Action::Character(_)
            | Action::Track(_)
            | Action::Version
            | Action::ScoreKeyExport
            | Action::ScoreKeyImport(_)
            | Action::ScoreSubmit { .. }
            | Action::ScoreTop { .. }
            | Action::ScoreForget(_)
            | Action::Clear => continue,
            Action::StopRace => {
                // Like a race that has been won, without the wait and the toast.
                if let Some(session) = lobby.session.as_ref().filter(|_| race_on.is_on()) {
                    session.stop();
                }
                lobby.countdown = None;
                let series = lobby.series.take();
                let cup = series.as_ref().is_some_and(|s| s.scored);
                lobby.end_race(&mut race_on);
                // (The guests hear of a stopped race from the session; of a cup, only from us.)
                if cup {
                    lobby.announce("Cup stopped");
                    if let Some(standings) = series.and_then(|s| s.final_standings())
                        && let Some(session) = &lobby.session
                    {
                        session.standings(standings);
                    }
                } else {
                    toasts.write(Toast::new("Race stopped", 3000).color(ANNOUNCE_COLOR));
                }
                continue;
            }
            Action::RestartRace if !lobby.is_open() => {
                if let Some(race) = race_on.get() {
                    // Out of a replay, back to an attempt: with our racer, not the run's.
                    let characters = match race.replay {
                        Some(_) => vec![character as u8],
                        None => race.characters.clone(),
                    };
                    let (track, solo) = (race.track, race.solo);
                    let rules = super::rules(race.rules.laps, &config, now);
                    lobby.race_alone(&mut race_on, characters, &config.nickname, track, rules, solo);
                }
                continue;
            }
            Action::RestartRace => {
                if let Some(race) = race_on.get() {
                    let then = Then::Restart {
                        track: race.track,
                        laps: race.rules.laps,
                    };
                    lobby.count_down(then, now, super::COUNTDOWN_SECONDS);
                }
                continue;
            }
        };
        toasts.write(Toast::new(say, 2500));
    }
}

/// Chat, apart from [`run`]: it goes on where there is no `Drive` (the podium), which `run` needs
/// for the rest.
pub(super) fn say(
    mut actions: MessageReader<Action>,
    mut lobby: ResMut<Lobby>,
    race_on: Res<RaceOn>,
    mut toasts: MessageWriter<Toast>,
    mut logs: MessageWriter<Log>,
) {
    for action in actions.read() {
        let Action::Say(text) = action else {
            continue;
        };
        if let Some(why) = refusal(action, &lobby, &race_on) {
            toasts.write(Toast::new(why, 2500).color(Color::srgb(1.0, 0.4, 0.4)));
        } else if !lobby.say(text) {
            logs.write(Log::error("You are talking too fast"));
        }
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
