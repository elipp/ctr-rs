//! What the HUD elements of a split screen have in common. Each element is made once for every
//! local player ([`HudView`]): its top node carries the view, and goes to the HUD camera of that
//! view (`drive::HudCamera`), which lays it out in the player's part of the window. With one
//! player there are no such cameras, and the nodes are the window's, as they always were.
//!
//! An element's system does the three things that differ: it spawns what is [`missing`], it finds
//! the driver of each view ([`driver_of`]) and it shows that. This module does the rest: the
//! targeting, and the despawning of the elements of a view that is gone.

use bevy::prelude::*;

use crate::drive::{Drive, HudCamera, UnderCamera};
use crate::props::Sim;

/// The local player (the index in [`Drive::views`], ours 0) that an element of the HUD is for. On
/// the top node, which goes to the player's camera, and on the parts of it that are told from the
/// other players' by it.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub struct HudView(pub usize);

/// How many views the HUD has elements for: the ones whose kart the race has. (The players are
/// added to the views first, and the race starts over with them a tick later: `drive_tick`.)
pub fn views(sim: &Sim, drive: &Drive) -> usize {
    drive.views().len().min(sim.race.karts.len())
}

/// The views that have none of an element yet, given the `have` the element's top nodes carry.
pub fn missing(sim: &Sim, drive: &Drive, have: impl IntoIterator<Item = HudView>) -> Vec<usize> {
    let have: Vec<usize> = have.into_iter().map(|v| v.0).collect();
    (0..views(sim, drive))
        .filter(|view| !have.contains(view))
        .collect()
}

/// The kart that `view` (of those [`views`] counts) is of: the one the camera follows for ours,
/// which is another kart for a spectator.
pub fn slot_of(sim: &Sim, drive: &Drive, view: usize) -> usize {
    match view {
        0 => sim.own,
        _ => drive.views()[view].slot(),
    }
}

/// The driver of the kart that `view` is of.
pub fn driver_of<'a>(sim: &'a Sim, drive: &Drive, view: usize) -> &'a vehicle::Driver {
    &sim.race.karts[slot_of(sim, drive, view)].driver
}

/// On an element that is drawn under the figures of the HUD (the fruit and the tracker), as the
/// weapon's glow is: it goes to the camera under them.
#[derive(Component)]
pub struct Under;

pub struct HudViewPlugin;

impl Plugin for HudViewPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, target.run_if(crate::drive::driving));
    }
}

/// Sends each element to its view's HUD camera, and takes the ones of a view that is gone away.
/// With one player the elements are the window's, but for the ones that go under the figures:
/// they go to the camera that draws on the window first, which is the one that puts the picture on
/// it (when the pixel scale is on) or else the world's own.
fn target(
    mut commands: Commands,
    drive: Res<Drive>,
    cameras: Query<(Entity, &HudCamera)>,
    unders: Query<(Entity, &UnderCamera)>,
    presenting: Query<(Entity, &Camera), With<crate::pixel::Present>>,
    scene: Query<(Entity, &Camera), With<Camera3d>>,
    roots: Query<(Entity, &HudView, Has<Under>, Option<&UiTargetCamera>), Without<ChildOf>>,
) {
    let players = drive.views().len();
    for (entity, view, under, target) in &roots {
        if view.0 >= players {
            commands.entity(entity).despawn();
            continue;
        }
        let wanted = if players > 1 {
            // (Not up yet: the next frame.)
            if under {
                unders.iter().find(|(_, c)| c.0 == view.0).map(|(e, _)| e)
            } else {
                cameras.iter().find(|(_, c)| c.0 == view.0).map(|(e, _)| e)
            }
        } else if under {
            // (The scene may have been replaced since: it is looked for every frame.)
            presenting
                .iter()
                .find(|(_, c)| c.is_active)
                .or_else(|| scene.iter().find(|(_, c)| c.order == 0))
                .map(|(e, _)| e)
        } else {
            None
        };
        match (wanted, target) {
            (Some(camera), target) if target.is_none_or(|t| t.0 != camera) => {
                commands.entity(entity).try_insert(UiTargetCamera(camera));
            }
            (None, Some(_)) if players == 1 || !under => {
                commands.entity(entity).try_remove::<UiTargetCamera>();
            }
            _ => {}
        }
    }
}
