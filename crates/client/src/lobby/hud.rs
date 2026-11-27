//! The labels for the players who wait for the host: to start the race, or to resume it; and
//! quick play's, while it queues.

use super::{Lobby, Queue};
use bevy::prelude::*;
use render::font::{Font, Gradient};

use crate::font::CtrText;
use crate::hud;

/// The label for the players who wait for the host.
#[derive(Component)]
pub(super) struct WaitingHud;

/// The label for a race the host paused (`/cup pause`).
#[derive(Component)]
pub(super) struct PausedHud;

/// The label of a quick play session, in the waiting label's place: while it queues, and
/// between the match's races.
#[derive(Component)]
pub(super) struct SearchingHud;

/// How often the label blinks: a second on, a second off.
pub(super) const BLINK_SECONDS: f32 = 1.0;

/// The countdown to a race of the lobby ([`Lobby::starting`]), in the searching label's place.
#[derive(Component)]
pub(super) struct StartingHud;

/// Its lines: the seconds, then the track.
#[derive(Component)]
pub(super) enum StartingLine {
    Seconds,
    Track,
}

pub fn spawn_waiting(mut commands: Commands) {
    spawn_label(&mut commands, WaitingHud, "WAITING FOR HOST TO START THE RACE");
    spawn_label(&mut commands, PausedHud, "RACE PAUSED BY THE HOST");
    spawn_label(&mut commands, SearchingHud, "");
    commands.spawn((
        StartingHud,
        Visibility::Hidden,
        Node {
            position_type: PositionType::Absolute,
            top: Val::Percent(20.0),
            width: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            row_gap: Val::Px(8.0),
            ..default()
        },
        Pickable::IGNORE,
        children![
            (
                StartingLine::Seconds,
                CtrText::new("", Font::Large, Gradient::ORANGE).fixed(hud::PIXEL),
                Pickable::IGNORE,
            ),
            (
                StartingLine::Track,
                CtrText::new("", Font::Middle, Gradient::WHITE).fixed(hud::PIXEL),
                Pickable::IGNORE,
            ),
        ],
    ));
}

/// How often the countdown changes colour, a second: retail's 30 (every frame) looks off here.
const FLASH_HZ: f32 = 10.0;

/// Shows the countdown to a race, flashing orange and white as retail's text does.
pub(super) fn starting(
    lobby: Res<Lobby>,
    time: Res<Time<Real>>,
    mut label: Query<&mut Visibility, With<StartingHud>>,
    mut lines: Query<(&StartingLine, &mut CtrText)>,
) {
    let now = time.elapsed_secs();
    let left = lobby.starting.map(|s| (s, (s.at - now).ceil())).filter(|&(_, left)| left > 0.0);
    for mut visibility in &mut label {
        visibility.set_if_neq(match left {
            Some(_) => Visibility::Inherited,
            None => Visibility::Hidden,
        });
    }
    let Some((starting, left)) = left else {
        return;
    };
    let flash = if ((now * FLASH_HZ) as u32).is_multiple_of(2) {
        Gradient::ORANGE
    } else {
        Gradient::WHITE
    };
    for (line, mut text) in &mut lines {
        let (wanted, gradient) = match line {
            StartingLine::Seconds => {
                let what = if starting.restart { "RACE RESTARTS IN" } else { "RACE STARTS IN" };
                (format!("{what} {left}"), flash)
            }
            StartingLine::Track => (crate::maps::name(starting.track).to_uppercase(), Gradient::WHITE),
        };
        if text.text != wanted {
            text.text = wanted;
        }
        if text.gradient != gradient {
            text.gradient = gradient;
        }
    }
}

fn spawn_label(commands: &mut Commands, marker: impl Component, text: &str) {
    commands.spawn((
        marker,
        Visibility::Hidden,
        Node {
            position_type: PositionType::Absolute,
            top: Val::Percent(20.0),
            width: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            ..default()
        },
        Pickable::IGNORE,
        children![(
            CtrText::new(text, Font::Middle, Gradient::ORANGE).fixed(hud::PIXEL),
            Pickable::IGNORE,
        )],
    ));
}

/// Shows a label, blinking: to a player who is in a lobby and has not been started yet, or to
/// everyone in a race the host paused.
pub(super) fn waiting(
    lobby: Res<Lobby>,
    race_on: Res<crate::race_on::RaceOn>,
    time: Res<Time<Real>>,
    mut waiting_label: Query<&mut Visibility, (With<WaitingHud>, Without<PausedHud>)>,
    mut paused_label: Query<&mut Visibility, With<PausedHud>>,
) {
    let waiting = lobby.joined && !lobby.quick_play && lobby.is_waiting(&race_on);
    let paused = race_on.net().is_some_and(|net| net.pause_at.is_some());
    let on = ((time.elapsed_secs() / BLINK_SECONDS) as u32).is_multiple_of(2);
    let visibility_of = |show: bool| {
        if show && on {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        }
    };
    for mut visibility in &mut waiting_label {
        *visibility = visibility_of(waiting);
    }
    for mut visibility in &mut paused_label {
        *visibility = visibility_of(paused);
    }
}

/// What the quick play label says, steady (it is the only news while queued).
pub(super) fn searching(
    lobby: Res<Lobby>,
    race_on: Res<crate::race_on::RaceOn>,
    time: Res<Time<Real>>,
    mut label: Query<(&mut Visibility, &Children), With<SearchingHud>>,
    mut texts: Query<&mut CtrText>,
) {
    let between = lobby.between_matched_races(&race_on);
    let next = lobby.next.as_ref().filter(|_| between).map(|next| {
        let track = next.track;
        let name = map::builtin::display_name(track).unwrap_or("A DISC TRACK").to_uppercase();
        let left = (next.at - time.elapsed_secs()).max(0.0).ceil() as u32;
        let votes = match next.votes.iter().find(|&&(t, _)| t == track).map_or(0, |&(_, n)| n) {
            0 => String::new(),
            1 => " (1 VOTE)".into(),
            n => format!(" ({n} VOTES)"),
        };
        format!("NEXT: {name}{votes} IN {left}")
    });
    let counting = lobby.starting.is_some_and(|s| s.at > time.elapsed_secs());
    let text = match &lobby.queue {
        // `newest`, not `is_on`: a race started is not on until the flag covers the window, and
        // the label would show while the flag comes in.
        None if !lobby.quick_play || race_on.newest().is_some() || counting => None,
        // Else between races (ready is `quick_ready.rs`'s) or gathering (the
        // scoreboard lists who came: `scoreboard::gathering`).
        None => next,
        Some(Queue::Searching { players, waited }) => {
            let clock = format!("{}:{:02}", waited / 60, waited % 60);
            Some(match players {
                0 | 1 => format!("SEARCHING FOR A MATCH  {clock}"),
                n => format!("SEARCHING FOR A MATCH  {clock}  {n} IN QUEUE"),
            })
        }
        Some(Queue::Found { accepted: false, .. }) => Some("MATCH FOUND".into()),
        Some(Queue::Found { accepted: true, .. }) => Some("WAITING FOR THE OTHERS".into()),
    };
    for (mut visibility, children) in &mut label {
        *visibility = match text {
            Some(_) => Visibility::Inherited,
            None => Visibility::Hidden,
        };
        if let Some(text) = &text {
            for &child in children {
                if let Ok(mut shown) = texts.get_mut(child)
                    && shown.text != *text
                {
                    shown.text = text.clone();
                }
            }
        }
    }
}
