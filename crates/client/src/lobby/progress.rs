//! The race in progress: its end, the clock's pace and the rollback toasts.

use super::cup::{self, Finisher};
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

/// How long the race goes on after the players who score have finished, in seconds of the
/// race: the winner's confetti and music, and the places of the others, are for these.
pub(super) const AFTERMATH_SECONDS: u32 = 15;

/// How long a toast that says who finished shows, in milliseconds.
const FINISH_TOAST_MS: u64 = 4000;

/// The `place`th (from 0) as a person says it: 1st, 2nd, 3rd, 4th, ... 11th, 12th, 21st.
pub(super) fn ordinal(place: usize) -> String {
    let n = place + 1;
    let suffix = match (n % 10, n % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

#[cfg(test)]
mod tests {
    use super::ordinal;

    #[test]
    fn places_are_said_as_ordinals() {
        let said: Vec<String> = [0, 1, 2, 3, 10, 11, 12, 20, 21].map(ordinal).into();
        assert_eq!(said, ["1st", "2nd", "3rd", "4th", "11th", "12th", "13th", "21st", "22nd"]);
    }
}

/// Says who finished and in which place, as each crosses the line, and ends the race
/// [`AFTERMATH_SECONDS`] after the last of the players who would score in a cup (the first four,
/// or all there are if fewer) has: everyone goes back to a free drive (`drive_tick` puts the
/// kart alone on the grid when there is no race). Both wait until the tick they happened on is
/// settled, so that every client sees the same finishers whatever it predicted, and ends at the
/// same tick. The lobby stays; the host scores the race for its cup and starts the next, or can
/// start another race.
pub(super) fn finish(
    mut lobby: ResMut<Lobby>,
    sim: Res<crate::props::Sim>,
    time: Res<Time<Real>>,
    mut toasts: MessageWriter<Toast>,
) {
    let Some(net) = lobby.online.as_mut().filter(|net| net.go && !net.fresh) else {
        return;
    };
    let settled = sim.race.settled();
    while let Some(&at) = sim.race.finish_ticks.get(net.announced)
        && at < settled
    {
        let place = net.announced;
        net.announced += 1;
        let finished = |kart: &race::Kart| {
            kart.driver.actions_flag_set & vehicle::actions::RACE_FINISHED != 0
                && kart.driver.driver_rank == place as i16
        };
        let Some(slot) = sim.race.karts.iter().position(finished) else {
            continue;
        };
        let name = net.labels.get(slot).map_or("?", String::as_str);
        let text = format!("{name} finished {}", ordinal(place));
        debug!("{text} (tick {at})");
        toasts.write(Toast::new(text, FINISH_TOAST_MS).color(ANNOUNCE_COLOR));
    }
    // The race is over when the players that score have crossed the line (the ones that left
    // will not, and are not waited for), and the aftermath has passed.
    let present = net.gone.iter().filter(|gone| !**gone).count();
    let waited_for = cup::places_waited_for(present);
    let over_at = sim
        .race
        .finish_ticks
        .get(waited_for.saturating_sub(1))
        .map(|&at| at + AFTERMATH_SECONDS * crate::GAME_TICK_HZ as u32);
    if !over_at.is_some_and(|at| at < settled) {
        return;
    }
    // (Spectators have no kart: `characters` has the players only.)
    let finishers: Vec<Finisher> = {
        let mut order: Vec<(i16, usize)> = sim
            .race
            .karts
            .iter()
            .enumerate()
            .filter(|(_, kart)| kart.driver.actions_flag_set & vehicle::actions::RACE_FINISHED != 0)
            .map(|(slot, kart)| (kart.driver.driver_rank, slot))
            .collect();
        order.sort();
        order
            .into_iter()
            .filter_map(|(_, slot)| {
                Some(Finisher {
                    id: *net.ids.get(slot)?,
                    name: net.labels.get(slot)?.clone(),
                })
            })
            .collect()
    };
    let players = net.characters.len();
    lobby.end_race();
    if lobby.is_host() {
        lobby.race_over(players, &finishers, time.elapsed_secs());
    }
}

/// Keeps the clock of an online race with the others': it catches up after a hitch, and runs a bit
/// slow while it is ahead of any of them (the ones behind do not hurry; they play on as they do).
/// Offline it runs as fast as the effects panel says (`/debug fx speed`).
pub(super) fn pace(
    lobby: Res<Lobby>,
    fx: Option<Res<crate::fx::Effects>>,
    mut time: ResMut<Time<Virtual>>,
) {
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
    let speed = match &fx {
        Some(fx) if lobby.online.is_none() => fx.speed,
        _ => (1.0 - SLOWDOWN_PER_TICK * (ahead - LEAD_ALLOWED).max(0.0)).max(SLOWEST),
    };
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
