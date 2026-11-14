//! The countdown before the host's race starts or starts over.

use super::Lobby;
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

/// Counts a start or a restart down, which everyone sees (the host only). Closing the lobby
/// cancels it.
pub(super) fn countdown(mut lobby: ResMut<Lobby>, time: Res<Time<Real>>) {
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
                Then::Start { track, laps } | Then::Restart { track, laps } => {
                    lobby.start(track, laps)
                }
            }
            return;
        }
        if c.left <= COUNTDOWN_SECONDS {
            lobby.announce(format!("{}...", c.left));
        }
    }
    lobby.countdown = Some(c);
}
