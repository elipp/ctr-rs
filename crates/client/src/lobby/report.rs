//! What the session reports (`net::Event`), taken in once a frame.

use super::commands::{copy_to_clipboard, net_info_text};
use super::{ANNOUNCE_COLOR, Lobby, Member, Queue};
use crate::console::{Log, console};
use crate::maps;
use crate::race_on::{CurrentRace, NetRace, RaceOn};
use crate::toast::Toast;
use bevy::prelude::*;
use net::Event;

/// How much of a new reading of the clocks' difference is taken in (they come 30 times a second
/// from each player).
pub(super) const LEAD_SMOOTHING: f32 = 0.1;

pub(super) fn report(
    mut lobby: ResMut<Lobby>,
    mut race_on: ResMut<RaceOn>,
    mut lag: ResMut<super::lag::FakeLag>,
    time: Res<Time<Real>>,
    mut toasts: MessageWriter<Toast>,
    mut logs: MessageWriter<Log>,
) {
    if !lobby.joined {
        lobby.tell_lock();
        lobby.tell_bots();
    }
    let Some(session) = &lobby.session else {
        return;
    };
    let mut failed = None;
    let mut promoted = None;
    let mut handed_over = false;
    let mut listed = None;
    let mut wanted_map = None;
    let mut got_map = None;
    let mut map_failed = None;
    let mut roster = None;
    let mut started = None;
    let mut cup_next = None;
    let mut go = false;
    let mut pads = Vec::new();
    let mut sums = Vec::new();
    let mut leads = Vec::new();
    let mut stopped = false;
    let mut paused = None;
    let mut new_standings = None;
    // Whether a race started, and if so whether it is a cup's.
    let mut started_cup = None;
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
                    Toast::success(
                        "Lobby created!\nJoin command copied to clipboard".to_string(),
                        3000,
                    ),
                );
            }
            Event::Promoted { ticket, public } => promoted = Some((ticket, public)),
            Event::Searching { players, waited } => {
                let waited = waited.as_secs() as u32;
                lobby.queue = Some(Queue::Searching { players, waited });
            }
            Event::Found(found) => {
                lobby.queue = Some(Queue::Found {
                    found,
                    at: time.elapsed_secs(),
                    left: score::matching::ACCEPT_SECONDS,
                    accepted: false,
                });
            }
            Event::Next { track, left, votes, ready } => {
                let at = time.elapsed_secs() + left.as_secs_f32();
                lobby.next = Some(super::NextRace { track, at, votes, ready });
            }
            Event::Requeued => {
                lobby.queue = Some(Queue::Searching { players: 0, waited: 0 });
                toasts.write(Toast::new("Someone declined: back in the queue", 3000));
            }
            Event::PeerJoined(id) => debug!("{} joined", id.fmt_short()),
            Event::Lobby {
                players,
                spectators,
                you,
                locked,
                bots,
            } => {
                // The match's lobby is up: the queue is behind us.
                if let Some(Queue::Found { found, .. }) = lobby.queue.take() {
                    lobby.last_match = Some(found.lobby);
                }
                roster = Some((players, spectators, you, locked, bots));
            }
            Event::Announce(text) => announce(&mut toasts, text),
            Event::Countdown(countdown) => {
                lobby.starting = countdown.map(|c| super::Starting {
                    track: c.track,
                    restart: c.restart,
                    at: time.elapsed_secs() + c.ms as f32 / 1000.0,
                });
            }
            // (The scoreboard shows them: `scoreboard.rs`.)
            Event::Standings(standings) => {
                cup_next = standings.next.as_ref().map(|(track, _)| *track);
                new_standings = Some(standings);
            }
            Event::Chat { from, text } => chats.push((from, text)),
            Event::Joined(id) => {
                debug!("joined the lobby of {}", id.fmt_short());
                // We hosted: this is the host we handed the lobby over to.
                handed_over |= !lobby.joined;
            }
            Event::Rtt { id, rtt, relay } => rtts.push((id.fmt_short().to_string(), rtt, relay)),
            Event::PeerLeft(id) => {
                debug!(
                    "{} left{}",
                    id.fmt_short(),
                    if race_on.is_on() {
                        " (their kart idles for the rest of the race)"
                    } else {
                        ""
                    }
                );
                left.push(id.fmt_short().to_string());
            }
            Event::Started {
                slot,
                locals,
                characters,
                players,
                track,
                hash,
                rules,
                bots,
                retail,
            } => {
                debug!(
                    "race started: you are player {slot} of {} ({bots} brain bots), characters {characters:?}, retail bots {retail:?}, track {track:#x}",
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
                // (A character past the roster, from the host, is the first: the players' and the
                // retail bots'.)
                let on_roster = |c: &mut u8| {
                    if *c >= net::CHARACTERS {
                        *c = 0;
                    }
                };
                let mut characters = characters;
                characters.iter_mut().for_each(on_roster);
                let mut retail = retail;
                retail
                    .characters
                    .truncate(race::MAX_KARTS.saturating_sub(characters.len()));
                retail.characters.iter_mut().for_each(on_roster);
                stopped = false;
                started_cup = Some(retail.cup);
                lobby.again = false;
                lobby.next = None;
                lobby.starting = None;
                let net = NetRace::new(players, slot);
                started = Some(CurrentRace::new(
                    slot, locals, characters, bots, track, hash, rules, retail, Some(net),
                ));
            }
            Event::Go => go = true,
            Event::Gone { slot } => gones.push(slot),
            Event::Left { slot, first, pads } => lefts.push((slot, first, pads)),
            // (A stop is of the race before a start that follows it, if there is one.)
            Event::Stopped => {
                started = None;
                stopped = true;
            }
            Event::Paused(at) => paused = Some(at),
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
                    debug!("received track {track:#x} ({} bytes)", bytes.len());
                    got_map = Some(track);
                }
                Err(why) => map_failed = Some(format!("Could not use the track sent: {why}")),
            },
            Event::BlobMissing { track, .. } => {
                // (The host sends custom maps only: the game's own tracks are made from the disc.)
                map_failed = Some(if map::is_map_id(track) {
                    format!("The host cannot send the track {}", maps::name(track))
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
    if let Some(Queue::Found { at, left, .. }) = &mut lobby.queue {
        let gone = (time.elapsed_secs() - *at).max(0.0) as u32;
        *left = score::matching::ACCEPT_SECONDS.saturating_sub(gone);
    }
    // We handed the lobby over, and are a guest of the new host: what only the host has is theirs.
    if handed_over {
        lobby.joined = true;
        lobby.public = None;
        lobby.series = None;
        lobby.countdown = None;
        lobby.countdown_told = None;
        lobby.roster.clear();
        lobby.roster_seen = false;
    }
    // The host left, and the lobby is ours now (the session listed it, if it was public).
    if let Some((ticket, public)) = promoted {
        lobby.joined = false;
        // (The lobby shows the bots of ours now.)
        lobby.bots_told = None;
        lobby.public = public.map(|public| public.name);
        copy_to_clipboard(format!("/lobby join {ticket}"));
        lobby.list("");
        toasts.write(
            Toast::success(
                "You are now the host!\nJoin command copied to clipboard".to_string(),
                4000,
            ),
        );
    }
    for (key, rtt, relay) in rtts {
        lobby
            .average_pings
            .entry(key.clone())
            .and_modify(|old| {
                *old = old.mul_f32(1.0 - PING_SMOOTHING) + rtt.mul_f32(PING_SMOOTHING)
            })
            .or_insert(rtt);
        match relay {
            Some(relay) => lobby.relayed.insert(key.clone(), relay),
            None => lobby.relayed.remove(&key),
        };
        lobby.pings.insert(key, rtt);
    }
    for key in left {
        lobby.pings.remove(&key);
        lobby.average_pings.remove(&key);
        lobby.relayed.remove(&key);
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
                Toast::error(format!("Could not list the lobby: {why}"), 4000),
            );
        }
        None => {}
    }
    if cup_next.is_some() {
        lobby.race_next = cup_next;
    }
    // A race that is not of a cup, or the first of a new one, leaves the old standings behind.
    if let Some(cup) = started_cup
        && (!cup || lobby.standings.as_ref().is_some_and(|s| s.next.is_none()))
    {
        lobby.standings = None;
    }
    // (A race that starts before our podium was shown takes it with it.)
    if started_cup.is_some() {
        lobby.podium = None;
    }
    if let Some(standings) = new_standings {
        lobby.podium = super::cup::podium(&standings);
        lobby.standings = Some(standings);
    }
    if stopped {
        lobby.race_next = None;
        lobby.end_race(&mut race_on);
        toasts.write(Toast::new("Race stopped", 3000).color(ANNOUNCE_COLOR));
    }
    let new_race = started.is_some();
    if new_race {
        // Every race's start brings the flag in (a cup's next one has it from its standings
        // already): the players load at their own speeds, and the faster ones would show the
        // scene before the race has put the karts on the grid, until everyone's go.
        if let Some(race) = started {
            lobby.race_next = Some(race.track);
            race_on.start(race);
        }
    }
    // (The session drops the pauses of older races, so one that came with a start is of the new
    // race.)
    if let Some(at) = paused
        && let Some(net) = race_on.newest_mut().and_then(|race| race.net.as_mut())
    {
        net.pause_at = at;
        let text = if at.is_some() {
            "Race paused by the host"
        } else {
            "Race resumed"
        };
        toasts.write(Toast::new(text, 3000).color(ANNOUNCE_COLOR));
    }
    let nicks_told = roster.is_some();
    if let Some((track, hash)) = wanted_map {
        if let Some(session) = &lobby.session {
            session.fetch(track, hash);
        }
        lobby.fetching = Some(track);
        toasts.write(Toast::new("Asking the host for the track...", 3000));
    }
    if let Some(track) = got_map {
        if lobby.fetching == Some(track) {
            lobby.fetching = None;
        }
        toasts.write(
            Toast::success(format!("Got the track {}", maps::name(track)), 2500),
        );
    }
    if let Some(why) = map_failed {
        warn!("{why}");
        toasts.write(Toast::error(why, 6000));
        // The race cannot start without the map: the others would wait for us for good.
        lobby.close(&mut race_on);
        return;
    }
    if let Some((players, spectators, you, locked, bots)) = roster {
        lobby.characters_locked = locked;
        lobby.lobby_bots = bots;
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
            .enumerate()
            .map(|(n, (p, label))| Member {
                id: p.id,
                key: p.id.fmt_short().to_string(),
                label,
                nick: p.nick.clone(),
                character: p.character,
                extras: p.extras.clone(),
                you: Some(n) == you,
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
                    None => {
                        console!("{who} joined as {name}");
                        continue;
                    }
                    Some(was) if was.character != member.character => {
                        format!("{who} is now {name}")
                    }
                    // (We know what we called ourselves.)
                    Some(was) if was.nick != member.nick && !is_you => {
                        format!("{} is now called {}", was.label, member.label)
                    }
                    Some(_) => continue,
                };
                toasts.write(Toast::success(text, 3000));
            }
            for was in &lobby.roster {
                if members.iter().all(|now| now.key != was.key) {
                    toasts.write(
                        Toast::warning(format!("{} left", was.label), 3000),
                    );
                }
            }
        }
        lobby.roster = members;
        lobby.roster_seen = true;
    }
    // The race is named after the nicknames when it starts, and again when one changes.
    if new_race || nicks_told {
        lobby.refresh_labels(&mut race_on);
    }
    // (After the lobby: a player who joined and spoke at once is known by now.)
    for (from, text) in chats {
        logs.write(lobby.chat_line(from, &text));
    }
    let mut answers = Vec::new();
    if let Some(race) = race_on.newest_mut()
        && let Some(net) = race.net.as_mut()
    {
        race.go |= go;
        // (`/debug lag` holds the pads back a while; with no lag they go through at once.)
        let now = time.elapsed();
        net.delayed.hold(pads, now, &mut lag);
        // The pads of a player we reported on are not taken: the report is where they end.
        let released = net.delayed.release(now, &lag);
        net.inbox.extend(
            released
                .into_iter()
                .filter(|(slot, ..)| !net.frozen.get(*slot).copied().unwrap_or(false)),
        );
        for slot in gones {
            if !race.go {
                // The sim has none of its pads yet, and takes none now.
                net.inbox.retain(|(from, ..)| *from != slot);
                net.delayed.forget(slot);
                if let Some(frozen) = net.frozen.get_mut(slot) {
                    *frozen = true;
                }
                answers.push(slot);
            } else {
                net.asked.push(slot);
            }
        }
        net.lefts.extend(lefts);
        for (slot, ahead) in leads {
            if let Some(smoothed) = net.ahead.get_mut(slot) {
                *smoothed += (ahead - *smoothed) * LEAD_SMOOTHING;
            }
        }
        for (slot, tick, hash) in sums {
            let found = net.checks.theirs(slot, tick, hash);
            net.found(found);
        }
        for d in net.desync.iter_mut().filter(|d| !d.told) {
            d.told = true;
            let who = race.labels.get(d.slot).map_or("?", String::as_str);
            let hashes = d.hashes.map_or_else(
                || "(a pad is missing, not a hash)".to_string(),
                |h| format!("ours {:016x}, theirs {:016x}", h.ours, h.theirs),
            );
            error!(
                "DESYNC: the sim of player {} ({who}) differs from ours at tick {}: {hashes}. \
                 We are player {} of {}; track {:#x}, map hash {:#018x}; characters {:?}; players {:?}",
                d.slot,
                d.tick,
                race.slot,
                race.characters.len(),
                race.track,
                race.hash,
                race.characters,
                net.names,
            );
            toasts.write(
                Toast::error(
                    format!("Out of sync with player {} (tick {})", d.slot, d.tick),
                    8000,
                ),
            );
        }
    }
    for slot in answers {
        lobby.have(slot, 0, Vec::new());
    }
    if let Some(why) = failed {
        // Back to where we started: no lobby, no race.
        lobby.close(&mut race_on);
        toasts.write(
            Toast::error(format!("Left the lobby: {why}"), 5000),
        );
    }
}

/// How much of a new ping goes into the average that is kept.
pub(super) const PING_SMOOTHING: f32 = 0.4;

/// Toasts what the host told everyone. (A cup's standings are several lines, which take longer
/// to read.)
fn announce(toasts: &mut MessageWriter<Toast>, text: String) {
    let ms = 3000 + 1500 * text.lines().filter(|l| !l.is_empty()).count().saturating_sub(1) as u64;
    toasts.write(Toast::new(text, ms).color(ANNOUNCE_COLOR));
}
