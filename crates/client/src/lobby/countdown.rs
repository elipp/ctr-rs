//! The countdown before the host's race starts or starts over.

use super::Lobby;
use crate::config::Config;
use bevy::prelude::*;

/// The seconds of the countdown before the host's race starts, or starts over.
pub(super) const COUNTDOWN_SECONDS: u32 = 5;

/// What the countdown ends in.
#[derive(Clone, Copy)]
pub(super) enum Then {
    /// A race of the series, on this track, with this many laps.
    Start { track: u32, laps: u8 },
    /// The race again from the start, on this track. It is a new start, so it takes the players
    /// and bots there are now.
    Restart { track: u32, laps: u8 },
}

/// The countdown of a start or a restart: the seconds left, and when the last one was announced.
pub(super) struct Countdown {
    pub(super) then: Then,
    pub(super) left: u32,
    pub(super) at: f32,
}

/// The counted part of a countdown, as the lobby was told it: what it ends in, and when
/// (`Time<Real>` seconds). Whatever drops [`Lobby::countdown`] calls it off for everyone here.
#[derive(Clone, Copy, PartialEq)]
pub(super) struct Told {
    track: u32,
    restart: bool,
    end: f32,
}

/// Counts a start or a restart down, which everyone sees (the host only). Closing the lobby
/// cancels it.
pub(super) fn countdown(mut lobby: ResMut<Lobby>, config: Res<Config>, time: Res<Time<Real>>) {
    if lobby.joined || lobby.session.is_none() {
        lobby.countdown = None;
        lobby.countdown_told = None;
        return;
    }
    let now = time.elapsed_secs();
    if let Some(mut c) = lobby.countdown.take() {
        if now - c.at >= 1.0 {
            c.left -= 1;
            c.at += 1.0;
            if c.left == 0 {
                // (The start ends the countdown for everyone: nothing to call off.)
                lobby.countdown_told = None;
                match c.then {
                    Then::Start { track, laps } | Then::Restart { track, laps } => {
                        lobby.start(track, super::rules(laps, &config, now))
                    }
                }
                return;
            }
        }
        lobby.countdown = Some(c);
    }
    let wanted = lobby.countdown.as_ref().filter(|c| c.left <= COUNTDOWN_SECONDS).map(|c| {
        let (track, restart) = match c.then {
            Then::Start { track, .. } => (track, false),
            Then::Restart { track, .. } => (track, true),
        };
        Told { track, restart, end: c.at + c.left as f32 }
    });
    if wanted == lobby.countdown_told {
        return;
    }
    lobby.countdown_told = wanted;
    if let Some(session) = &lobby.session {
        session.count_down(wanted.map(|t| net::Countdown {
            track: t.track,
            ms: ((t.end - now).max(0.0) * 1000.0) as u32,
            restart: t.restart,
        }));
    }
}
