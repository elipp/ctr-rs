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

/// The camera of local player `.0` that draws what goes under the figures of the HUD: the same
/// part of the window as [`HudCamera`]'s (`hud_view::Under`).
#[derive(Component)]
pub struct UnderCamera(pub usize);

/// Makes the players on the screen as many as `/players` says: the others' karts and views, and
/// what each drives with. A pad if there is one that no player has, else (for the second player)
/// the keyboard's second set of keys. Not before the level is up. In an online race the players
/// are the ones the roster gave us (`Online::locals`): the views follow those slots, and the
/// karts are the characters the host said.
pub(super) fn sync(
    wanted: Res<LocalPlayers>,
    chosen: Res<super::Chosen>,
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
    if loading.is_some() {
        return;
    }
    // The slot of each local player (the first is ours) and the character it drives.
    let (slots, characters): (Vec<usize>, Vec<usize>) = match &lobby.online {
        // (Once `remote::build` has put the others on the grid. A spectator has no view of its own.)
        Some(online) if !online.built || online.spectating() => return,
        Some(online) => online
            .locals
            .iter()
            .map(|&slot| (slot, online.characters[slot] as usize))
            .unzip(),
        None => (0..wanted.0.clamp(1, super::MAX_PLAYERS))
            .map(|slot| (slot, chosen.character(slot, drive.character)))
            .unzip(),
    };
    let n = slots.len();
    // Both are there, as they should be: a new level takes the karts with the rest of the scene
    // (and starts a `Drive` of one view), a change of count leaves a mismatch.
    let slots_right = (1..n).all(|view| karts.iter().any(|k| k.0 == slots[view]))
        && drive.views.iter().zip(&slots).skip(1).all(|(view, &slot)| view.slot == slot);
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
        // The window's UI (the pause menu, ...) is over all of them: this one is drawn last on
        // the window, so `pixel` takes it for Bevy's UI.
        builder.commands.spawn((
            Extra,
            crate::pixel::OnWindow,
            Camera2d,
            Camera {
                order: crate::pixel::UI_ORDER,
                clear_color: ClearColorConfig::None,
                ..default()
            },
            config.msaa(),
            RenderLayers::none(),
        ));
        for view in 0..n {
            builder.commands.spawn((
                Extra,
                UnderCamera(view),
                crate::pixel::OwnTarget,
                Camera2d,
                Camera {
                    order: crate::pixel::UNDER_ORDER + view as isize,
                    clear_color: ClearColorConfig::None,
                    ..default()
                },
                config.msaa(),
            ));
            builder.commands.spawn((
                Extra,
                HudCamera(view),
                crate::pixel::OwnTarget,
                Camera2d,
                Camera {
                    order: crate::pixel::HUD_ORDER + view as isize,
                    clear_color: ClearColorConfig::None,
                    ..default()
                },
                config.msaa(),
            ));
        }
    }
    for view in 1..n {
        let (slot, character) = (slots[view], characters[view]);
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
            None if view == 1 => Device::Keys(Controls::second()),
            None => {
                toasts.write(Toast::new(
                    format!("Player {} has no pad: connect one, then /players again", view + 1),
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
            player_layer(view),
            Some((slot, &birth)),
        );
        builder.commands.entity(kart).insert(Extra);
        spawn_cameras(
            &mut builder.commands,
            view,
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
    // After the first player's cameras: the first camera to draw into the picture clears all of
    // it, which wipes what an earlier one drew in its part. Under the present camera and the HUD's
    // (`pixel::PRESENT_ORDER`).
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
            // Its own trail of the clock (`clock.rs`).
            crate::clock::ClockTrail::default(),
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
    ), (Without<HudCamera>, Without<UnderCamera>)>,
    mut huds: Query<(Option<&HudCamera>, Option<&UnderCamera>, &mut Camera), Or<(With<HudCamera>, With<UnderCamera>)>>,
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
    for (hud, under, mut camera) in &mut huds {
        let Some(view) = hud.map(|h| h.0).or(under.map(|u| u.0)) else {
            continue;
        };
        if view >= n {
            continue;
        }
        let (position, size) = rect(view, n, window.physical_size());
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

/// Where local player `view` of `n` is on the window, in its logical pixels: all of it for one.
pub(crate) fn window_rect(view: usize, n: usize, window: &Window) -> Rect {
    if n <= 1 {
        return Rect::new(0.0, 0.0, window.width(), window.height());
    }
    let (position, size) = rect(view, n, window.physical_size());
    let scale = window.scale_factor();
    Rect::from_corners(
        position.as_vec2() / scale,
        (position + size).as_vec2() / scale,
    )
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
