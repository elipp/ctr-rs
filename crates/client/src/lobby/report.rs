//! What the session reports (`net::Event`), taken in once a frame.

use super::commands::{copy_to_clipboard, net_info_text};
use super::online::Online;
use super::{ANNOUNCE_COLOR, Lobby, Member};
use crate::console::{Log, console};
use crate::maps;
use crate::toast::Toast;
use bevy::prelude::*;
use net::Event;

/// How much of a new reading of the clocks' difference is taken in (they come 30 times a second
/// from each player).
pub(super) const LEAD_SMOOTHING: f32 = 0.1;

pub(super) fn report(
    mut lobby: ResMut<Lobby>,
    mut toasts: MessageWriter<Toast>,
    mut logs: MessageWriter<Log>,
) {
    let Some(session) = &lobby.session else {
        return;
    };
    let mut failed = None;
    let mut promoted = None;
    let mut listed = None;
    let mut wanted_map = None;
    let mut got_map = None;
    let mut map_failed = None;
    let mut roster = None;
    let mut started = None;
    let mut go = false;
    let mut pads = Vec::new();
    let mut sums = Vec::new();
    let mut leads = Vec::new();
    let mut stopped = false;
    let mut rtts = Vec::new();
    let mut left = Vec::new();
    let mut gones = Vec::new();
    let mut lefts = Vec::new();
    let mut chats = Vec::new();
    for event in session.poll() {
        match event {
            Event::Open(ticket) => {
                copy_to_clipboard(format!("/lobby join {ticket}"));
                lobby.list("");
                toasts.write(
                    Toast::new(
                        "Lobby created!\nJoin command copied to clipboard".to_string(),
                        3000,
                    )
                    .color(Color::linear_rgb(0.4, 1.0, 0.4)),
                );
            }
            Event::Promoted { ticket, public } => promoted = Some((ticket, public)),
            Event::PeerJoined(id) => debug!("{} joined", id.fmt_short()),
            Event::Lobby {
                players,
                spectators,
                you,
            } => roster = Some((players, spectators, you)),
            Event::Announce(text) => {
                // (A cup's results are several lines, which take longer to read.)
                let ms = 3000 + 1500 * text.lines().count().saturating_sub(1) as u64;
                toasts.write(Toast::new(text, ms).color(ANNOUNCE_COLOR));
            }
            Event::Chat { from, text } => chats.push((from, text)),
            Event::Joined(id) => debug!("joined the lobby of {}", id.fmt_short()),
            Event::Rtt { id, rtt } => rtts.push((id.fmt_short().to_string(), rtt)),
            Event::PeerLeft(id) => {
                debug!(
                    "{} left{}",
                    id.fmt_short(),
                    if lobby.online.is_some() {
                        " (their kart idles for the rest of the race)"
                    } else {
                        ""
                    }
                );
                left.push(id.fmt_short().to_string());
            }
            Event::Started {
                slot,
                characters,
                players,
                track,
                hash,
                laps,
            } => {
                debug!(
                    "race started: you are player {slot} of {}, characters {characters:?}, track {track:#x}",
                    characters.len()
                );
                // The host chose a map we do not have: it sends it, and the race waits.
                if !maps::has(track, hash) && lobby.fetching != Some(track) {
                    wanted_map = Some((track, hash));
                }
                // Whatever came before belongs to the race before this one.
                pads.clear();
                sums.clear();
                leads.clear();
                let characters_len = characters.len();
                stopped = false;
                let names: Vec<String> = players
                    .iter()
                    .map(|id| id.fmt_short().to_string())
                    .collect();
                started = Some(Online {
                    slot,
                    characters,
                    // (Named after the nicknames below.)
                    labels: names.clone(),
                    names,
                    ids: players,
                    track,
                    hash,
                    laps,
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
                    announced: 0,
                    gone: vec![false; characters_len],
                    watching: if slot < characters_len { slot } else { 0 },
                    asked: Vec::new(),
                    frozen: vec![false; characters_len],
                    lefts: Vec::new(),
                });
            }
            Event::Go => go = true,
            Event::Gone { slot } => gones.push(slot),
            Event::Left { slot, first, pads } => lefts.push((slot, first, pads)),
            // (A stop is of the race before a start that follows it, if there is one.)
            Event::Stopped => {
                started = None;
                stopped = true;
            }
            Event::Pads {
                slot,
                first,
                pads: p,
            } => pads.push((slot, first, p)),
            Event::Lead { slot, ahead } => leads.push((slot, ahead)),
            Event::Checksum { slot, tick, hash } => sums.push((slot, tick, hash)),
            Event::ChecksumDropped {
                from,
                tick,
                sent,
                epoch,
                planned,
                linked,
            } => warn!(
                "dropped the hash of tick {tick} from {}: it is of race {sent}, ours is {epoch}; \
                 in the roster: {planned}, linked: {linked}",
                from.fmt_short()
            ),
            Event::Blob { track, hash, bytes } => match maps::receive(track, hash, &bytes) {
                Ok(()) => {
                    debug!("received map {track:#x} ({} bytes)", bytes.len());
                    got_map = Some(track);
                }
                Err(why) => map_failed = Some(format!("Could not use the map sent: {why}")),
            },
            Event::BlobMissing { track, .. } => {
                // (The host sends custom maps only: the game's own tracks are made from the disc.)
                map_failed = Some(if map::is_map_id(track) {
                    format!("The host cannot send the map {}", maps::name(track))
                } else {
                    format!(
                        "Your {} is not the host's: another disc or version",
                        maps::name(track)
                    )
                });
            }
            Event::NetInfo(peers) => {
                let text = net_info_text(&peers);
                for line in text.lines() {
                    console!("{line}");
                }
            }
            Event::Listed(result) => listed = Some(result),
            Event::Failed(why) => {
                warn!("lobby failed: {why}");
                failed = Some(why);
            }
        }
    }
    // The host left, and the lobby is ours now (the session listed it, if it was public).
    if let Some((ticket, public)) = promoted {
        lobby.joined = false;
        lobby.public = public.map(|public| public.name);
        copy_to_clipboard(format!("/lobby join {ticket}"));
        lobby.list("");
        toasts.write(
            Toast::new(
                "You are now the host!\nJoin command copied to clipboard".to_string(),
                4000,
            )
            .color(Color::linear_rgb(0.4, 1.0, 0.4)),
        );
    }
    for (key, rtt) in rtts {
        lobby
            .average_pings
            .entry(key.clone())
            .and_modify(|old| {
                *old = old.mul_f32(1.0 - PING_SMOOTHING) + rtt.mul_f32(PING_SMOOTHING)
            })
            .or_insert(rtt);
        lobby.pings.insert(key, rtt);
    }
    for key in left {
        lobby.pings.remove(&key);
        lobby.average_pings.remove(&key);
    }
    match listed {
        Some(Ok(())) => {
            let name = lobby.public_name().unwrap_or_default();
            toasts.write(Toast::new(
                format!("Lobby \"{name}\" listed in the realm"),
                2500,
            ));
        }
        Some(Err(why)) => {
            warn!("could not list the lobby in the realm: {why}");
            toasts.write(
                Toast::new(format!("Could not list the lobby: {why}"), 4000)
                    .color(Color::srgb(1.0, 0.4, 0.4)),
            );
        }
        None => {}
    }
    if stopped {
        lobby.end_race();
        toasts.write(Toast::new("Race stopped", 3000).color(ANNOUNCE_COLOR));
    }
    let new_race = started.is_some();
    if new_race {
        lobby.online = started;
    }
    let nicks_told = roster.is_some();
    if let Some((track, hash)) = wanted_map {
        if let Some(session) = &lobby.session {
            session.fetch(track, hash);
        }
        lobby.fetching = Some(track);
        toasts.write(Toast::new("Asking the host for the map...", 3000));
    }
    if let Some(track) = got_map {
        if lobby.fetching == Some(track) {
            lobby.fetching = None;
        }
        toasts.write(
            Toast::new(format!("Got the map {}", maps::name(track)), 2500)
                .color(Color::linear_rgb(0.4, 1.0, 0.4)),
        );
    }
    if let Some(why) = map_failed {
        warn!("{why}");
        toasts.write(Toast::new(why, 6000).color(Color::srgb(1.0, 0.3, 0.3)));
        // The race cannot start without the map: the others would wait for us for good.
        lobby.close();
        return;
    }
    if let Some((players, spectators, you)) = roster {
        // The names are told apart among everyone in the lobby, those who only watch too.
        let mut labels = net::display_names(
            &players
                .iter()
                .chain(&spectators)
                .map(|p| (p.id, p.nick.as_str()))
                .collect::<Vec<_>>(),
        );
        let watching = labels.split_off(players.len());
        lobby.watchers = spectators
            .iter()
            .zip(watching)
            .map(|(p, label)| (label, p.id.fmt_short().to_string()))
            .collect();
        let members: Vec<Member> = players
            .iter()
            .zip(labels)
            .map(|(p, label)| Member {
                key: p.id.fmt_short().to_string(),
                label,
                nick: p.nick.clone(),
                character: p.character,
            })
            .collect();
        for p in players.iter().chain(&spectators) {
            lobby.nicks.insert(p.id, p.nick.clone());
        }
        // A guest that has just joined meets the lobby as it is, which is not news.
        if lobby.roster_seen || !lobby.joined {
            for (n, member) in members.iter().enumerate() {
                let name = crate::CHARACTERS
                    .get(member.character as usize)
                    .copied()
                    .unwrap_or("?");
                let is_you = Some(n) == you;
                let who = if is_you { "you" } else { member.label.as_str() };
                let before = lobby.roster.iter().find(|known| known.key == member.key);
                let text = match before {
                    // We know that we joined.
                    None if is_you => continue,
                    None => format!("{who} joined as {name}"),
                    Some(was) if was.character != member.character => {
                        format!("{who} is now {name}")
                    }
                    // (We know what we called ourselves.)
                    Some(was) if was.nick != member.nick && !is_you => {
                        format!("{} is now called {}", was.label, member.label)
                    }
                    Some(_) => continue,
                };
                toasts.write(Toast::new(text, 3000).color(Color::srgb(0.4, 1.0, 0.4)));
            }
            for was in &lobby.roster {
                if members.iter().all(|now| now.key != was.key) {
                    toasts.write(
                        Toast::new(format!("{} left", was.label), 3000)
                            .color(Color::srgb(1.0, 0.6, 0.3)),
                    );
                }
            }
        }
        lobby.roster = members;
        lobby.roster_seen = true;
    }
    // The race is named after the nicknames when it starts, and again when one changes.
    if new_race || nicks_told {
        lobby.refresh_labels();
    }
    // (After the lobby: a player who joined and spoke at once is known by now.)
    for (from, text) in chats {
        logs.write(lobby.chat_line(from, &text));
    }
    let mut answers = Vec::new();
    if let Some(online) = &mut lobby.online {
        online.go |= go;
        // The pads of a player we reported on are not taken: the report is where they end.
        online.inbox.extend(
            pads.into_iter()
                .filter(|(slot, ..)| !online.frozen.get(*slot).copied().unwrap_or(false)),
        );
        for slot in gones {
            if !online.go {
                // The sim has none of its pads yet, and takes none now.
                online.inbox.retain(|(from, ..)| *from != slot);
                if let Some(frozen) = online.frozen.get_mut(slot) {
                    *frozen = true;
                }
                answers.push(slot);
            } else {
                online.asked.push(slot);
            }
        }
        online.lefts.extend(lefts);
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
            let who = online.labels.get(d.slot).map_or("?", String::as_str);
            let hashes = d.hashes.map_or_else(
                || "(a pad is missing, not a hash)".to_string(),
                |h| format!("ours {:016x}, theirs {:016x}", h.ours, h.theirs),
            );
            error!(
                "DESYNC: the sim of player {} ({who}) differs from ours at tick {}: {hashes}. \
                 We are player {} of {}; track {:#x}, map hash {:#018x}; characters {:?}; players {:?}",
                d.slot,
                d.tick,
                online.slot,
                online.characters.len(),
                online.track,
                online.hash,
                online.characters,
                online.names,
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
    for slot in answers {
        lobby.have(slot, 0, Vec::new());
    }
    if let Some(why) = failed {
        // Back to where we started: no lobby, no race.
        lobby.close();
        toasts.write(
            Toast::new(format!("Left the lobby: {why}"), 5000).color(Color::srgb(1.0, 0.3, 0.3)),
        );
    }
}

/// How much of a new ping goes into the average that is kept.
pub(super) const PING_SMOOTHING: f32 = 0.4;
