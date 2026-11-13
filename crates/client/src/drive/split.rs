//! The other local players of a split screen (`/players`): their karts on the grid, and the
//! views and devices that drive and follow them. The first player is ours, as ever.

use bevy::camera::visibility::RenderLayers;
use bevy::camera::{ClearColorConfig, Viewport};
use bevy::prelude::*;
use race::KartSpec;

use super::input::Device;
use super::local::LocalView;
use super::{ChaseCamera, Drive, Extra, LocalPlayers, MainView, Pads, ViewOf};
use crate::pixel::RenderSize;
use crate::sky::SkyCamera;
use crate::{OverlayCamera, Track, player_layer};
use crate::controls::Controls;
use crate::kart::{KartBuilder, KartSlot};
use crate::lobby::Lobby;
use crate::toast::Toast;

/// The camera that draws the HUD of local player `.0`, in the player's part of the window (a UI
/// that has it as its `UiTargetCamera` is laid out there). There are some only while there are
/// several players.
#[derive(Component)]
pub struct HudCamera(pub usize);

/// Makes the players on the screen as many as `/players` says: the others' karts and views, and
/// what each drives with. A pad if there is one that no player has, else (for the second player)
/// the keyboard's second set of keys. Not in a lobby, and not before the level is up.
pub(super) fn sync(
    wanted: Res<LocalPlayers>,
    lobby: Res<Lobby>,
    track: Res<Track>,
    loading: Option<Res<crate::loading::Loading>>,
    mut drive: ResMut<Drive>,
    extras: Query<Entity, With<Extra>>,
    karts: Query<&KartSlot, With<Extra>>,
    main: Query<(&Projection, &Camera), (With<ChaseCamera>, With<MainView>)>,
    skies: Query<(&ViewOf, &Projection, &Camera), With<SkyCamera>>,
    pads: Pads,
    config: Res<crate::config::Config>,
    mut builder: KartBuilder,
    mut toasts: MessageWriter<Toast>,
) {
    if lobby.online.is_some() || loading.is_some() {
        return;
    }
    let n = wanted.0.clamp(1, super::MAX_PLAYERS);
    // Both are there, as they should be: a new level takes the karts with the rest of the scene
    // (and starts a `Drive` of one view), a change of count leaves a mismatch.
    let slots_right = (1..n).all(|slot| karts.iter().any(|k| k.0 == slot));
    if drive.views.len() == n && karts.iter().count() == n - 1 && slots_right {
        return;
    }
    // The cameras of ours are the pattern of the others'. (Not up yet: the next frame.)
    let Some((projection, camera)) = main.iter().next() else {
        return;
    };
    let sky = skies
        .iter()
        .find(|(view, _, _)| view.0 == 0)
        .map(|(_, projection, camera)| (projection, camera.clear_color.clone()));
    for entity in &extras {
        builder.commands.entity(entity).try_despawn();
    }
    drive.views.truncate(1);
    let mut free: Vec<Entity> = pads.entities().map(|(entity, _)| entity).collect();
    free.reverse();
    // The HUD of each, when there are several: over everything else of the window, but not the
    // one that Bevy's UI goes to (`OwnTarget` keeps `pixel` from taking them).
    if n > 1 {
        for view in 0..n {
            builder.commands.spawn((
                Extra,
                HudCamera(view),
                crate::pixel::OwnTarget,
                Camera2d,
                Camera {
                    order: 4 + view as isize,
                    clear_color: ClearColorConfig::None,
                    ..default()
                },
                config.msaa(),
            ));
        }
    }
    for slot in 1..n {
        let character = (drive.character + slot) % crate::CHARACTERS.len();
        let (model, wheel_icons) = crate::load_racer(character);
        let spec = KartSpec {
            character,
            anim_num_frames: crate::scene::anim_num_frames(&model),
            bot: false,
            line: None,
        };
        let birth = race::Kart::birth(&track.0, slot, &spec);
        let device = match free.pop() {
            Some(pad) => Device::Pad(pad),
            None if slot == 1 => Device::Keys(Controls::second()),
            None => {
                toasts.write(Toast::new(
                    format!("Player {} has no pad: connect one, then /players again", slot + 1),
                    4000,
                ));
                Device::Keys(Controls::none())
            }
        };
        let kart = builder.spawn_in(
            &model,
            &wheel_icons,
            Transform::default(),
            false,
            player_layer(slot),
            Some((slot, &birth)),
        );
        builder.commands.entity(kart).insert(Extra);
        spawn_cameras(
            &mut builder.commands,
            slot,
            projection,
            camera.clear_color.clone(),
            sky.clone(),
        );
        let mut view = LocalView::new(&track, &birth.driver, slot, device);
        view.spec = Some(spec);
        drive.views.push(view);
    }
}

/// The cameras that draw the picture of local player `view`: the sky, the main camera (which the
/// view moves, `follow::chase_extra`) and, over it, the one that draws the kart in a layer of its
/// own, with only the depth buffer cleared. Their orders are in between the first player's and
/// the ones of the window's, one set of three for each player.
fn spawn_cameras(
    commands: &mut Commands,
    view: usize,
    projection: &Projection,
    clear: ClearColorConfig,
    sky: Option<(&Projection, ClearColorConfig)>,
) {
    // EXPERIMENT: after the first player's cameras (was `-3 * view`: before them).
    let order = 10 * view as isize;
    if let Some((projection, clear)) = sky {
        commands.spawn((
            Extra,
            crate::sky::camera(view, order - 1, clear, projection.clone()),
        ));
    }
    commands
        .spawn((
            Extra,
            // (Not the one the world's systems follow: see `OverlayCamera`.)
            OverlayCamera,
            MainView(view),
            ViewOf(view),
            Camera3d::default(),
            Camera {
                order,
                clear_color: clear,
                ..default()
            },
            projection.clone(),
            Transform::default(),
            RenderLayers::layer(0),
        ))
        .with_child((
            OverlayCamera,
            ViewOf(view),
            Camera3d::default(),
            Camera {
                order: order + 1,
                clear_color: ClearColorConfig::None,
                ..default()
            },
            projection.clone(),
            Transform::default(),
            RenderLayers::layer(player_layer(view)),
        ));
}

/// Where player `view` of `n` is on a picture of `size`: a part of it, as its position and size.
/// Two players are one above the other, and three or four make a grid of two by two.
fn rect(view: usize, n: usize, size: UVec2) -> (UVec2, UVec2) {
    let (columns, rows) = if n <= 2 { (1, n as u32) } else { (2, 2) };
    let (column, row) = if n <= 2 {
        (0, view as u32)
    } else {
        (view as u32 % 2, view as u32 / 2)
    };
    let edge = |i: u32, of: u32, length: u32| i * length / of;
    let from = UVec2::new(edge(column, columns, size.x), edge(row, rows, size.y));
    let to = UVec2::new(
        edge(column + 1, columns, size.x),
        edge(row + 1, rows, size.y),
    );
    (from, (to - from).max(UVec2::ONE))
}

/// Puts each camera of a player in the player's part of the picture, and has the main cameras draw
/// the others' karts (which are in layers of their own).
pub(super) fn layout(
    drive: Res<Drive>,
    size: Res<RenderSize>,
    mut cameras: Query<(
        Entity,
        &ViewOf,
        &mut Camera,
        Option<&MainView>,
        Option<&RenderLayers>,
    ), Without<HudCamera>>,
    mut huds: Query<(&HudCamera, &mut Camera)>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut commands: Commands,
) {
    let n = drive.views.len();
    for (entity, view, mut camera, main, layers) in &mut cameras {
        if view.0 >= n {
            continue;
        }
        let wanted = (n > 1).then(|| rect(view.0, n, size.physical));
        let have = (camera.viewport.as_ref()).map(|v| (v.physical_position, v.physical_size));
        if have != wanted {
            camera.viewport = wanted.map(|(position, size)| Viewport {
                physical_position: position,
                physical_size: size,
                ..default()
            });
        }
        if main.is_some() {
            let mut wanted = vec![0];
            wanted.extend((0..n).filter(|&other| other != view.0).map(player_layer));
            let wanted = RenderLayers::from_layers(&wanted);
            if layers != Some(&wanted) {
                commands.entity(entity).insert(wanted);
            }
        }
    }
    // The HUD's cameras go by the window, which is not the picture's size (the picture may be
    // pixelated, `pixel.rs`).
    let Ok(window) = windows.single() else {
        return;
    };
    for (hud, mut camera) in &mut huds {
        if hud.0 >= n {
            continue;
        }
        let (position, size) = rect(hud.0, n, window.physical_size());
        let have = (camera.viewport.as_ref()).map(|v| (v.physical_position, v.physical_size));
        if have != Some((position, size)) {
            camera.viewport = Some(Viewport {
                physical_position: position,
                physical_size: size,
                ..default()
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_players_are_one_above_the_other() {
        let size = UVec2::new(1920, 1080);
        assert_eq!(rect(0, 2, size), (UVec2::new(0, 0), UVec2::new(1920, 540)));
        assert_eq!(rect(1, 2, size), (UVec2::new(0, 540), UVec2::new(1920, 540)));
    }

    #[test]
    fn three_and_four_players_make_a_grid_that_covers_the_picture() {
        let size = UVec2::new(1001, 601);
        for n in [3, 4] {
            let mut area = 0;
            for view in 0..n {
                let (position, part) = rect(view, n, size);
                assert!(position.x + part.x <= size.x && position.y + part.y <= size.y);
                area += part.x * part.y;
            }
            // (Three leave a corner empty.)
            assert!(area <= size.x * size.y);
            assert!(n == 3 || area == size.x * size.y);
        }
    }
}
