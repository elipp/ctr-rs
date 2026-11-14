//! The wumpa counter's fruit: the spinning 3D wumpa of the levels, at the top of the screen right
//! of the weapon, next to the count. A camera of its own draws it over the picture, an orthographic one: the
//! game's projection is 90 degrees wide, which stretches anything near a corner of the screen.
//! One world unit of that camera is a pixel of the window.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use loader::model::Model;
use render::mesh;

use crate::hud_view::{HudView, HudViews, ViewKart};
use crate::subtract::SubtractMaterial;
use crate::{Textures, Track};

/// The turning fruit; its child is the model, moved so that its middle is the pivot.
#[derive(Component)]
struct WumpaIcon {
    /// The model's largest size in world units, to size it for the screen.
    diameter: f32,
    /// How the level places the fruit, which it turns around its own vertical axis from.
    rest: Quat,
}

/// The render layer of the fruit, which only the HUD camera (`scene.rs`) draws. (0 is the world's, `KART_LAYER` 1
/// and the sky's 2.)
pub(crate) const LAYER: usize = 3;

/// How far in front of the camera it hangs, in pixels: anything between its near and far plane.
const DEPTH: f32 = 100.0;

/// Where the middle of the fruit is: right of the middle of the screen in pixels of retail's
/// screen, and down from its top in lines. As in retail screenshots, right of the weapon and
/// level with it. And how tall the fruit is, in logical pixels.
pub const FRUIT_X: f32 = 68.0;
pub const MIDDLE_Y: f32 = 19.0;
const HEIGHT_PIXELS: f32 = 58.0;

/// Where the middle of the fruit's mesh is, down from the top of the screen in lines: a little
/// above `MIDDLE_Y`, which the counter and the weapon share.
const ICON_MIDDLE_Y: f32 = 17.0;

/// Where the counter's "x" starts, right of the middle of the screen in pixels of retail's
/// screen; the count grows to the right from there.
pub const COUNTER_X: f32 = 84.0;

/// The texels between the "x" and the count.
pub const COUNTER_GAP: usize = 2;

/// The counter's fruit from the racer pack, for a level that has no fruit of its own (a map).
#[derive(Resource)]
pub struct FruitDisplay(pub Model);

pub struct WumpaIconPlugin;

impl Plugin for WumpaIconPlugin {
    fn build(&self, app: &mut App) {
        // Per system: the viewer can be switched on at the sync point between two of them.
        app.add_systems(
            Update,
            (build, show, fly)
                .chain()
                .distributive_run_if(crate::drive::driving),
        );
    }
}

/// Puts the fruit up on the first frame of a level, which takes the old one away with the
/// camera. The fruit is the level's own, or the pack's if the level has none.
fn build(
    mut commands: Commands,
    track: Res<Track>,
    textures: Res<Textures>,
    display: Res<FruitDisplay>,
    hud: HudViews,
    icon: Query<&HudView, With<WumpaIcon>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut subtract_materials: ResMut<Assets<SubtractMaterial>>,
    mut none: Local<bool>,
) {
    if track.is_changed() {
        *none = false;
    }
    let missing = hud.missing(icon.iter().copied());
    if missing.is_empty() || *none {
        return;
    }
    let placed = track
        .0
        .instances
        .iter()
        .find(|i| i.model_name == "fruit")
        .and_then(|inst| {
            let (header, frame) = Model::parse_at(&textures.dram, inst.model)
                .ok()
                .and_then(|m| m.headers.into_iter().next())
                .and_then(|h| crate::props::rest_pose(&h).cloned().map(|f| (h, f)))?;
            let scale = [inst.scale.x, inst.scale.y, inst.scale.z];
            Some((header, frame, scale, crate::props::rotation_of(inst.rot)))
        });
    let fallback = || {
        let header = display.0.headers.first()?.clone();
        let frame = crate::props::rest_pose(&header)?.clone();
        Some((header, frame, [0x1000; 3], Quat::IDENTITY))
    };
    let Some((header, frame, scale, rest)) = placed.or_else(fallback) else {
        *none = true;
        return;
    };
    let data = mesh::model_mesh(&header, &frame, &textures.vram, scale);
    let (mut low, mut high) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for position in data.parts.iter().flat_map(|p| &p.positions) {
        low = low.min(Vec3::from_array(*position));
        high = high.max(Vec3::from_array(*position));
    }
    let middle = (low + high) / 2.0;
    let mut atlas = Image::new(
        bevy::render::render_resource::Extent3d {
            width: data.atlas.width as u32,
            height: data.atlas.height as u32,
            depth_or_array_layers: 1,
        },
        bevy::render::render_resource::TextureDimension::D2,
        data.atlas.rgba,
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::RENDER_WORLD,
    );
    atlas.sampler = bevy::image::ImageSampler::nearest();
    let base = StandardMaterial {
        base_color: Color::WHITE,
        base_color_texture: Some(images.add(atlas)),
        unlit: true,
        cull_mode: None,
        double_sided: true,
        ..default()
    };
    // One for each view, all drawn by the camera over the whole window.
    for view in missing {
        let pivot = commands
            .spawn((
                WumpaIcon {
                    diameter: (high - low).max_element(),
                    rest,
                },
                HudView(view),
                Transform::default(),
                Visibility::default(),
            ))
            .id();
        crate::spawn_parts(
            data.parts.clone(),
            &base,
            Transform::from_translation(-middle),
            Some(pivot),
            &mut commands,
            &mut meshes,
            &mut materials,
            &mut subtract_materials,
            LAYER,
        );
    }
}

/// Puts the fruit in the corner of the window, as tall as the HUD wants it, and turns it as
/// the fruit of the level turn.
fn show(
    sim: Res<crate::props::Sim>,
    hud: HudViews,
    fixed: Res<Time<Fixed>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut icon: Query<(&WumpaIcon, &HudView, &mut Transform)>,
    mut last: Local<(u32, f32)>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    // The spin is the level fruit's: it only depends on the tick, and never goes back.
    let ticks = sim.race.objects.ticks();
    let mut turns = ticks as f32 + fixed.overstep_fraction();
    if ticks >= last.0 {
        turns = turns.max(last.1);
    }
    *last = (ticks, turns);
    let angle = turns / crate::props::FRUIT_SPIN_TICKS * std::f32::consts::TAU;
    for (icon, &HudView(view), mut transform) in &mut icon {
        let Some(kart) = hud.get(view) else {
            continue;
        };
        // The camera looks at the middle of the window, with y up; the fruit is in the player's
        // part of it.
        let rect = kart.rect(window);
        transform.translation = Vec3::new(
            rect.center().x - window.width() / 2.0 + FRUIT_X * crate::hud::PIXEL.x,
            window.height() / 2.0 - rect.min.y - ICON_MIDDLE_Y * crate::hud::PIXEL.y,
            -DEPTH,
        );
        transform.scale = Vec3::splat(HEIGHT_PIXELS / icon.diameter);
        transform.rotation = icon.rest * Quat::from_rotation_y(angle);
    }
}

/// `wumpaposter` (`gGT->iconGroup[0xB]`), the icon group of the fruit that flies to the counter.
const POSTER_GROUP: i16 = 0xB;

/// How far above the kart's projected spot the flight starts, in lines of retail's screen
/// (`RB_Fruit_ThCollide`'s `- 0x14`).
const START_LIFT: f32 = 20.0;

/// The fruit picked up, flying from where it was grabbed to the counter (`UI_RenderFrame`): the
/// sim's queue (`vehicle::wumpa::hud_tick`) says how far along it is.
#[derive(Component)]
struct FlyingWumpa {
    /// The picture's size on screen.
    size: Vec2,
}

/// Where the flight started (the spot the last fruit was picked up at, as the screen had it) and
/// how many fruit were queued last frame, to see a new one come in.
#[derive(Default, Clone, Copy)]
struct Flight {
    start: Vec2,
    queued: i32,
}

/// Keeps the flying fruit of each view where the queue has it. The picture is a node of the UI
/// over the 3D fruit of the counter, which it lands on; the node is in its view's part of the
/// window.
fn fly(
    mut commands: Commands,
    hud: HudViews,
    fixed: Res<Time<Fixed>>,
    textures: Res<Textures>,
    windows: Query<&Window, With<PrimaryWindow>>,
    screen: Res<crate::pixel::RenderSize>,
    cameras: Query<(&crate::drive::MainView, &Camera, &GlobalTransform)>,
    karts: Query<(&crate::kart::KartSlot, &Transform)>,
    mut images: ResMut<Assets<Image>>,
    mut flying: Query<(&FlyingWumpa, &HudView, &mut Node, &mut Visibility)>,
    mut flights: Local<Vec<Flight>>,
    mut missing: Local<bool>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    // (The views have a part of the window each; only the ones whose kart the race has are shown.)
    let players = hud.len();
    let absent = hud.missing(flying.iter().map(|(_, v, ..)| *v));
    if !absent.is_empty() {
        // The first frame of a level, which took the node away.
        if *missing {
            return;
        }
        let groups = crate::hud::pack_groups();
        let Some(atlas) = render::hud_icons::group_icon(&groups, &textures.vram, POSTER_GROUP)
        else {
            *missing = true;
            return;
        };
        let size = Vec2::new(atlas.width as f32, atlas.height as f32) * crate::hud::PIXEL;
        let image = crate::hud::atlas_image(atlas, &mut images);
        for view in absent {
            commands.spawn((
                FlyingWumpa { size },
                HudView(view),
                ImageNode::new(image.clone()).with_mode(NodeImageMode::Stretch),
                Visibility::Hidden,
                Node {
                    position_type: PositionType::Absolute,
                    width: Val::Px(size.x),
                    height: Val::Px(size.y),
                    ..default()
                },
                Pickable::IGNORE,
            ));
        }
        return;
    }
    flights.resize(players, Flight::default());
    for (wumpa, &HudView(view), mut node, mut visibility) in &mut flying {
        let Some(kart) = hud.get(view) else {
            continue;
        };
        let ViewKart { slot, driver, .. } = kart;
        let rect = kart.rect(window);
        let flight = &mut flights[view];
        // A fruit came in: the flight starts from the kart.
        let queued = driver.pickup_wumpa_collected;
        if queued > flight.queued
            && let Some((_, camera, camera_transform)) = cameras.iter().find(|(m, ..)| m.0 == view)
            && let Some((_, kart)) = karts.iter().find(|(k, _)| k.0 == slot)
            && let Ok(at) = camera.world_to_viewport(camera_transform, kart.translation)
        {
            // (The camera draws into a part of the picture of the scene, which may be smaller
            // than the window; the node is in the player's part of the window.)
            let origin = camera
                .viewport
                .as_ref()
                .map_or(Vec2::ZERO, |v| v.physical_position.as_vec2() / screen.scale_factor);
            flight.start = screen.to_window(origin + at)
                - rect.min
                - Vec2::new(0.0, START_LIFT * crate::hud::PIXEL.y);
        }
        flight.queued = queued;
        if queued == 0 {
            *visibility = Visibility::Hidden;
            continue;
        }
        *visibility = Visibility::Inherited;
        let end = Vec2::new(
            rect.width() / 2.0 + FRUIT_X * crate::hud::PIXEL.x,
            ICON_MIDDLE_Y * crate::hud::PIXEL.y,
        );
        // The tick's countdown, run on between ticks.
        let left = (driver.pickup_wumpa_cooldown as f32 - fixed.overstep_fraction()).max(0.0);
        let at = end + (flight.start - end) * (left / vehicle::wumpa::FLIGHT_TICKS as f32);
        node.left = Val::Px(at.x - wumpa.size.x / 2.0);
        node.top = Val::Px(at.y - wumpa.size.y / 2.0);
    }
}
