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

pub fn spawn_waiting(mut commands: Commands) {
    spawn_label(&mut commands, WaitingHud, "WAITING FOR HOST TO START THE RACE");
    spawn_label(&mut commands, PausedHud, "RACE PAUSED BY THE HOST");
    spawn_label(&mut commands, SearchingHud, "");
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
            CtrText::new(text, Font::Middle, Gradient::ORANGE).fixed(hud::PIXEL / 2.0),
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
    mut label: Query<(&mut Visibility, &Children), With<SearchingHud>>,
    mut texts: Query<&mut CtrText>,
) {
    let text = match &lobby.queue {
        None if !lobby.quick_play || race_on.is_on() => None,
        None if lobby.between_matched_races(&race_on) && !lobby.again => {
            Some("OPEN THE MENU TO PLAY AGAIN".into())
        }
        None => Some("WAITING FOR THE OTHERS".into()),
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
