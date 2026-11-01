//! The label for the players who wait for the host.

use super::Lobby;
use bevy::prelude::*;
use render::font::{Font, Gradient};

use crate::font::CtrText;
use crate::hud;

/// The label for the players who wait for the host.
#[derive(Component)]
pub(super) struct WaitingHud;

/// How often the label blinks: a second on, a second off.
pub(super) const BLINK_SECONDS: f32 = 1.0;

pub fn spawn_waiting(mut commands: Commands) {
    commands.spawn((
        WaitingHud,
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
            CtrText::new(
                "WAITING FOR HOST TO START THE RACE",
                Font::Middle,
                Gradient::ORANGE
            )
            .fixed(hud::PIXEL / 2.0),
            Pickable::IGNORE,
        )],
    ));
}

/// Shows the label, blinking, to a player who is in a lobby and has not been started yet.
pub(super) fn waiting(
    lobby: Res<Lobby>,
    time: Res<Time<Real>>,
    mut label: Query<&mut Visibility, With<WaitingHud>>,
) {
    let waiting = lobby.joined && lobby.session.is_some() && lobby.online.is_none();
    let on = ((time.elapsed_secs() / BLINK_SECONDS) as u32).is_multiple_of(2);
    for mut visibility in &mut label {
        *visibility = if waiting && on {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
}
