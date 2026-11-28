//! The place our kart finished in, "You came in 2nd", which stays on screen until the race is
//! over.

use super::progress::ordinal;
use crate::font::CtrText;
use bevy::prelude::*;
use render::font::Font;

/// The row across the window that the text is centered in.
#[derive(Component)]
pub(super) struct Placing;

/// How often the place changes colour, a second: slower than the countdown's, as it stays up
/// (10 hurt the eyes).
const FLASH_HZ: f32 = 4.0;

/// Shows our place from the moment our kart crosses the line, and takes it away with the race.
pub(super) fn placing(
    mut commands: Commands,
    race_on: Res<crate::race_on::RaceOn>,
    sim: Res<crate::props::Sim>,
    time: Res<Time<Real>>,
    row: Query<(Entity, &Children), With<Placing>>,
    mut texts: Query<&mut CtrText>,
) {
    let place = race_on
        .get()
        // Alone against the clock, a place says nothing.
        .filter(|race| race.racing() && !race.spectating() && race.solo.is_none())
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
    let on = crate::blink::on(time.elapsed_secs(), 1.0 / FLASH_HZ);
    let gradient = crate::blink::flash(on);
    // (It follows the sim, which a rollback can put right.)
    if let Ok((_, children)) = row.single() {
        for &child in children {
            let Ok(mut shown) = texts.get_mut(child) else {
                continue;
            };
            if shown.text != text {
                shown.text = text.clone();
            }
            if shown.gradient != gradient {
                shown.gradient = gradient;
            }
        }
        return;
    }
    commands.spawn((
            Placing,
            GlobalZIndex(1),
            crate::bands::line(
                crate::bands::Band::Placing,
                CtrText::new(text, Font::Large, gradient)
                    .fixed(crate::hud::PIXEL),
            ),
        ));
}
