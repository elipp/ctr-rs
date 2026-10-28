//! The label for the players who wait for the host.

use bevy::prelude::*;
use super::Lobby;

/// The label for the players who wait for the host.
#[derive(Component)]
pub(super) struct WaitingHud;

/// How often the label blinks: a second on, a second off.
pub(super) const BLINK_SECONDS: f32 = 1.0;

pub fn spawn_waiting(mut commands: Commands) {
    commands.spawn((
        WaitingHud,
        Text::new("Waiting for host to start the race"),
        TextColor(Color::srgb(1.0, 0.65, 0.1)),
        TextLayout::justify(Justify::Center),
        Visibility::Hidden,
        Node {
            position_type: PositionType::Absolute,
            top: Val::Percent(20.0),
            width: Val::Percent(100.0),
            ..default()
        },
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

