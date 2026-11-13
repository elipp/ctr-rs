//! The powerslide meter under the minimap, as retail draws it with one player
//! (`UI_DrawSlideMeter`): a grey bar in a black outline, filled from its right end as a drift
//! nears its boost. The fill is green until the meter's room left drops to the low-room warning,
//! and red from then on. An engine that revs (the grid, a mask grab) fills it too: the sim keeps
//! the rev in the same meter. Like retail's, it is gone once our race is finished, unless the
//! speedometer is showing (`speedometer.rs`), which keeps it.

use bevy::prelude::*;

use crate::hud;
use crate::speedometer::ShowSpeedometer;

/// Where the bar's bottom right corner is on retail's screen (`UI_HUD_SLOT_SLIDE_METER`), and its
/// size (`UI_SLIDE_METER_BAR_W`, `UI_SLIDE_METER_BAR_H_FULL`). The outline goes round it, on its
/// first column and row and just past its last.
const RIGHT: i32 = 490;
const BOTTOM: i32 = 206;
const WIDTH: i32 = 49;
const HEIGHT: i32 = 7;

/// Over the speedometer's dial, and under its needle.
pub const Z: i32 = 1;

/// `ELAPSED_MS`: the meter's room is in milliseconds, its stats in frames.
const ELAPSED_MS: i32 = 32;

/// The bar's colours: empty, filling, and past the low-room warning
/// (`UI_SLIDE_METER_EMPTY_COLOR`, `_WAIT_COLOR`, `_READY_COLOR`).
const EMPTY: Color = Color::srgb_u8(0x80, 0x80, 0x80);
const FILLING: Color = Color::srgb_u8(0, 0xff, 0);
const READY: Color = Color::srgb_u8(0xff, 0, 0);

/// How far the fill reaches from the bar's right end, in pixels of retail's screen: none while
/// the meter is untouched (its room left is 0), the whole bar when its room is all used.
fn fill(room_left: i32, max_room: i32) -> i32 {
    if room_left == 0 {
        return 0;
    }
    WIDTH
        - (room_left * WIDTH)
            .checked_div(max_room * ELAPSED_MS)
            .unwrap_or(0)
}

/// The bar of local player `view`, outline and all, and the HUD camera it is laid out in (none
/// while there is one player: the window's).
#[derive(Component)]
struct SlideMeter {
    view: usize,
    camera: Option<Entity>,
}

/// The fill of the bar of local player `view`.
#[derive(Component)]
struct Fill(usize);

pub struct SlideMeterPlugin;

impl Plugin for SlideMeterPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, show.run_if(crate::drive::driving));
    }
}

/// Shows the meter of each local player, in the player's part of the screen when there are
/// several (each has a HUD camera then, `drive::HudCamera`).
fn show(
    mut commands: Commands,
    sim: Res<crate::props::Sim>,
    drive: Res<crate::drive::Drive>,
    showing: Res<ShowSpeedometer>,
    cameras: Query<(Entity, &crate::drive::HudCamera)>,
    mut meters: Query<(Entity, &SlideMeter, &mut Visibility)>,
    mut fills: Query<(&Fill, &mut Node, &mut BackgroundColor)>,
) {
    let players = drive.views();
    for (entity, meter, _) in &meters {
        if meter.view >= players.len() {
            commands.entity(entity).despawn();
        }
    }
    for (view, local) in players.iter().enumerate() {
        let camera = (players.len() > 1)
            .then(|| cameras.iter().find(|(_, hud)| hud.0 == view).map(|(e, _)| e))
            .flatten();
        if players.len() > 1 && camera.is_none() {
            // Its camera is not up yet.
            continue;
        }
        let Some((entity, meter, mut visibility)) = meters
            .iter_mut()
            .find(|(_, meter, _)| meter.view == view)
        else {
            // The first frame, or of a new level (which took the nodes away).
            spawn(&mut commands, view, camera);
            continue;
        };
        if meter.camera != camera {
            commands.entity(entity).despawn();
            continue;
        }
        // (Ours is the one the camera follows, which is another kart for a spectator.)
        let d = if view == 0 {
            sim.driver()
        } else {
            &sim.race.karts[local.slot()].driver
        };
        let finished = d.actions_flag_set & vehicle::actions::RACE_FINISHED != 0;
        visibility.set_if_neq(if finished && !showing.0 {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        });
        let room_left = d.turbo_meter_room_left as i32;
        let length = fill(room_left, d.stats.turbo_max_room as i32);
        let color = if (d.stats.turbo_low_room_warning as i32) * ELAPSED_MS < room_left {
            FILLING
        } else {
            READY
        };
        for (_, mut node, mut background) in fills.iter_mut().filter(|(f, _, _)| f.0 == view) {
            // The outline covers the bar's first column.
            let width = hud::x(length.clamp(0, WIDTH - 1) as f32);
            if node.width != width {
                node.width = width;
            }
            background.set_if_neq(BackgroundColor(color));
        }
    }
}

fn spawn(commands: &mut Commands, view: usize, camera: Option<Entity>) {
    // The outline is a line on the bar's first column and row, and one just past its last: the
    // node is a pixel wider and taller than the bar, its border those lines.
    let mut meter = commands.spawn((
        SlideMeter { view, camera },
        Node {
            position_type: PositionType::Absolute,
            right: hud::x((hud::WIDTH - RIGHT - 1) as f32),
            bottom: hud::y((hud::HEIGHT - BOTTOM - 1) as f32),
            width: hud::x((WIDTH + 1) as f32),
            height: hud::y((HEIGHT + 1) as f32),
            border: UiRect::axes(hud::x(1.0), hud::y(1.0)),
            ..default()
        },
        BorderColor::all(Color::BLACK),
        BackgroundColor(EMPTY),
        ZIndex(Z),
        Pickable::IGNORE,
        children![(
            Fill(view),
            Node {
                position_type: PositionType::Absolute,
                right: Val::ZERO,
                top: Val::ZERO,
                bottom: Val::ZERO,
                width: Val::ZERO,
                ..default()
            },
            BackgroundColor(FILLING),
            Pickable::IGNORE,
        )],
    ));
    if let Some(camera) = camera {
        meter.insert(UiTargetCamera(camera));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fill_grows_as_the_room_runs_out() {
        // Untouched, then just started, half way and nearly out of room (30 frames of it).
        let max = 30;
        assert_eq!(fill(0, max), 0);
        assert_eq!(fill(30 * ELAPSED_MS, max), 0);
        assert_eq!(fill(15 * ELAPSED_MS, max), 25);
        assert_eq!(fill(1, max), 49);
        // No room to measure by.
        assert_eq!(fill(100, 0), WIDTH);
    }
}
