//! The place our kart finished in, "You came in 2nd", which stays on screen until the race is
//! over.

use super::Lobby;
use super::progress::ordinal;
use crate::font::CtrText;
use bevy::prelude::*;
use render::font::Font;

/// The row across the window that the text is centered in.
#[derive(Component)]
pub(super) struct Placing;

/// Shows our place from the moment our kart crosses the line, and takes it away with the race.
pub(super) fn placing(
    mut commands: Commands,
    lobby: Res<Lobby>,
    sim: Res<crate::props::Sim>,
    row: Query<(Entity, &Children), With<Placing>>,
    mut texts: Query<&mut CtrText>,
) {
    let place = lobby
        .online
        .as_ref()
        .filter(|net| net.go && !net.fresh && !net.spectating())
        .map(|_| sim.driver())
        .filter(|d| d.actions_flag_set & vehicle::actions::RACE_FINISHED != 0)
        .map(|d| d.driver_rank.max(0) as usize);
    let Some(place) = place else {
        for (entity, _) in &row {
            commands.entity(entity).try_despawn();
        }
        return;
    };
    let text = format!("You came in {}", ordinal(place));
    // (It follows the sim, which a rollback can put right.)
    if let Ok((_, children)) = row.single() {
        for &child in children {
            if let Ok(mut shown) = texts.get_mut(child)
                && shown.text != text
            {
                shown.text = text.clone();
            }
        }
        return;
    }
    commands
        .spawn((
            Placing,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Percent(30.0),
                width: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                ..default()
            },
            GlobalZIndex(1),
            Pickable::IGNORE,
        ))
        .with_child(
            CtrText::new(
                text,
                Font::Middle,
                crate::toast::gradient(Color::srgb(1.0, 0.9, 0.2)),
            )
            .fixed(crate::toast::texel_size(48.0)),
        );
}
