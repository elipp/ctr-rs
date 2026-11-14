//! The labels for the players who wait for the host: to start the race, or to resume it.

use super::Lobby;
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

/// How often the label blinks: a second on, a second off.
pub(super) const BLINK_SECONDS: f32 = 1.0;

pub fn spawn_waiting(mut commands: Commands) {
    spawn_label(&mut commands, WaitingHud, "WAITING FOR HOST TO START THE RACE");
    spawn_label(&mut commands, PausedHud, "RACE PAUSED BY THE HOST");
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
    time: Res<Time<Real>>,
    mut waiting_label: Query<&mut Visibility, (With<WaitingHud>, Without<PausedHud>)>,
    mut paused_label: Query<&mut Visibility, With<PausedHud>>,
) {
    let waiting = lobby.joined && lobby.session.is_some() && lobby.online.is_none();
    let paused = lobby.online.as_ref().is_some_and(|net| net.pause_at.is_some());
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
