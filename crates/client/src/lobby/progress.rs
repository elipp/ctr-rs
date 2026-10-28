//! The race in progress: its end, the clock's pace and the rollback toasts.

use super::{ANNOUNCE_COLOR, Lobby};
use crate::toast::Toast;
use bevy::prelude::*;

/// The ticks we may be ahead of the others before the clock is slowed.
pub(super) const LEAD_ALLOWED: f32 = 1.0;

/// How much slower the clock runs for each tick ahead past that, and the slowest it gets.
pub(super) const SLOWDOWN_PER_TICK: f32 = 0.03;
pub(super) const SLOWEST: f32 = 0.9;

/// The longest frame the clock takes in during an online race: after a hitch the ticks that were
/// missed are played at once, instead of being lost (Bevy's own limit is a quarter second).
pub(super) const CATCH_UP: std::time::Duration = std::time::Duration::from_secs(3);

/// Bevy's own limit on a frame, for everything but an online race.
pub(super) const NORMAL_MAX_DELTA: std::time::Duration = std::time::Duration::from_millis(250);

/// How long the race goes on after it has been won, in seconds of the race, if the players still
/// here have not all finished by then.
pub(super) const AFTERMATH_SECONDS: u32 = 5;

/// Announces the winner of the race, and [`AFTERMATH_SECONDS`] later ends it: everyone goes back to
/// a free drive (`drive_tick` puts the kart alone on the grid when there is no race). Both are
/// counted in ticks from the win, once the tick it happened on is settled, so that every client
/// sees the same winner whatever it predicted, and ends at the same tick. The lobby stays, and
/// the host can start another race.
pub(super) fn finish(
    mut lobby: ResMut<Lobby>,
    sim: Res<crate::props::Sim>,
    mut toasts: MessageWriter<Toast>,
) {
    let Some(net) = lobby.online.as_mut().filter(|net| net.go && !net.fresh) else {
        return;
    };
    let Some((slot, tick)) = sim
        .race
        .winner
        .filter(|&(_, tick)| tick < sim.race.settled())
    else {
        return;
    };
    if !net.won {
        net.won = true;
        let name = net.labels.get(slot).map_or("?", String::as_str);
        let text = format!("Race finished, player {name} won");
        debug!("{text} (tick {tick})");
        toasts.write(Toast::new(text, AFTERMATH_SECONDS as u64 * 1000).color(ANNOUNCE_COLOR));
    }
    // The race is over when the last of the players still here crosses the line (the ones that
    // left will not), or [`AFTERMATH_SECONDS`] after the win. The state is final at that point,
    // so every client sees the same.
    let all_over = sim.race.unsettled() == 0
        && sim.race.karts.iter().enumerate().all(|(slot, kart)| {
            net.gone.get(slot).copied().unwrap_or(false)
                || kart.driver.actions_flag_set & vehicle::actions::RACE_FINISHED != 0
        });
    if all_over || sim.race.settled() >= tick + AFTERMATH_SECONDS * crate::GAME_TICK_HZ as u32 {
        lobby.end_race();
    }
}

/// Keeps the clock of an online race with the others': it catches up after a hitch, and runs a bit
/// slow while it is ahead of any of them (the ones behind do not hurry; they play on as they do).
pub(super) fn pace(lobby: Res<Lobby>, mut time: ResMut<Time<Virtual>>) {
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

/// Whether a toast shows each rollback (`/net rollbacks on|off`); off to start with.
#[derive(Resource)]
pub struct RollbackToasts(pub(super) bool);

impl Default for RollbackToasts {
    fn default() -> RollbackToasts {
        RollbackToasts(false)
    }
}

/// A rollback of this many ticks or more is shown in a warmer color.
pub(super) const ROLLBACK_WARN_TICKS: u32 = 4;

/// Shows a short toast for the rollbacks of the last frame: the ticks they replayed, and how
/// many there were if more than one.
pub(super) fn rollback_toasts(
    lobby: Res<Lobby>,
    on: Res<RollbackToasts>,
    mut seen: Local<(u32, u32)>,
    mut toasts: MessageWriter<Toast>,
) {
    let Some(net) = &lobby.online else {
        *seen = (0, 0);
        return;
    };
    // A race that starts over counts from 0 again.
    if net.corrections < seen.0 || net.replayed < seen.1 {
        *seen = (0, 0);
    }
    let (count, ticks) = (net.corrections - seen.0, net.replayed - seen.1);
    *seen = (net.corrections, net.replayed);
    if !on.0 || count == 0 {
        return;
    }
    let text = if count > 1 {
        format!("Rollback: {ticks} ticks (x{count})")
    } else {
        format!("Rollback: {ticks} ticks")
    };
    let color = if ticks >= ROLLBACK_WARN_TICKS {
        Color::srgb(1.0, 0.6, 0.3)
    } else {
        Color::srgb(0.8, 0.8, 0.8)
    };
    toasts.write(Toast::new(text, 700).size(16.0).color(color));
}
