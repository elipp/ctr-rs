//! The white flash over the screen of a kart that a blast went off on (`RB_Burst_CollThread`: the
//! victim's `pushBuffer` fade jumps to `0x1fff` and runs back to the neutral `0x1000` by
//! `-0x88` a tick, which `PushBuffer_FadeOneWindow` draws as a white quad over the view, as
//! bright as `(value - 0x1000) >> 4` of 255). It takes a second to fade out. A bomb or a missile
//! bursting close to the camera flashes it as well, by how close (`RB_Burst_DrawAll`, see
//! [`Flash::proximity`]).
//!
//! DIVERGENCE: retail adds the white to what is under it; this lays it over, which is the same
//! on the dark parts and a little less bright on the light ones (docs/notes/DIVERGENCE.md).

use bevy::prelude::*;

use crate::props::Sim;

/// `fadeFromBlack_currentValue` above the neutral `0x1000`, at a blast.
const FULL: i16 = 0xfff;
/// `fade_step`, the other way.
const STEP: i16 = 0x88;

/// How far above neutral the fade is, at the last two ticks.
#[derive(Resource, Default)]
pub struct Flash {
    previous: i16,
    current: i16,
}

impl Flash {
    /// `RB_Burst_DrawAll`: a burst that plays its second frame at `local` (level units, in the
    /// camera's axes: right, up, back) flashes the screen if it is right in front of the camera,
    /// the more the nearer the axis. It sets the fade, whatever it was.
    pub fn proximity(&mut self, local: Vec3) {
        let [x, y, z] = local.abs().to_array().map(|a| a as i32);
        if x < PROXIMITY_XY && y < PROXIMITY_XY && z < PROXIMITY_Z {
            let excess = (FULL as i32 - ((x + y) << 3)) as i16;
            *self = Flash {
                previous: excess,
                current: excess,
            };
        }
    }
}

/// How near the axis of the camera a burst is for the flash, and how deep (twice
/// `distanceToScreen`, which is `0x100` with one player).
const PROXIMITY_XY: i32 = 0x100;
const PROXIMITY_Z: i32 = 0x200;

/// The quad that covers the screen.
#[derive(Component)]
pub struct Overlay;

/// One game tick: the fade runs on, and a blast on our kart puts it back to full.
pub fn step(sim: Res<Sim>, mut flash: ResMut<Flash>, mut last: Local<Option<u32>>) {
    let tick = sim.race.tick;
    let previous = last.replace(tick);
    if previous == Some(tick) {
        return;
    }
    // The race began again.
    if previous.is_some_and(|p| tick < p) {
        *flash = Flash::default();
    }
    flash.previous = flash.current;
    if sim.race.last_weapons.flashes.contains(&sim.own) {
        // It starts at once, not over the tick.
        *flash = Flash {
            previous: FULL,
            current: FULL,
        };
    } else {
        flash.current = (flash.current - STEP).max(0);
    }
}

/// How bright the quad is, 0 to 1, for a fade `excess` above neutral.
fn brightness(excess: i16) -> f32 {
    (excess >> 4) as u8 as f32 / 255.0
}

pub fn show(
    mut commands: Commands,
    fixed: Res<Time<Fixed>>,
    flash: Res<Flash>,
    mut overlay: Query<&mut BackgroundColor, With<Overlay>>,
) {
    let Ok(mut color) = overlay.single_mut() else {
        // The first frame of a race, or of a new level (which took the nodes away).
        commands.spawn((
            Overlay,
            BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.0)),
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                ..default()
            },
            // Over the picture, under the rest of the screen's pieces (the weapon, the counters).
            GlobalZIndex(-2),
            Pickable::IGNORE,
        ));
        return;
    };
    let t = fixed.overstep_fraction();
    let alpha = brightness(flash.previous) * (1.0 - t) + brightness(flash.current) * t;
    color.0 = Color::srgba(1.0, 1.0, 1.0, alpha);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blast_flashes_at_full_and_is_gone_after_a_second() {
        assert_eq!(brightness(FULL), 1.0);
        let (mut excess, mut ticks) = (FULL, 0);
        while excess > 0 {
            excess = (excess - STEP).max(0);
            ticks += 1;
        }
        assert_eq!(ticks, 31);
        assert_eq!(brightness(0), 0.0);
    }
}
