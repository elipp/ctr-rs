//! The wumpa counter's fruit: the spinning 3D wumpa of the levels, at the top of the screen right
//! of the weapon, next to the count. A camera of its own draws it over the picture, an orthographic one: the
//! game's projection is 90 degrees wide, which stretches anything near a corner of the screen.
//! One world unit of that camera is a pixel of the window.

use bevy::camera::visibility::RenderLayers;
use bevy::camera::{ClearColorConfig, ScalingMode};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use loader::model::Model;
use render::mesh;

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

/// The render layer of the fruit, which only its camera draws. (0 is the world's, `KART_LAYER` 1
/// and the sky's 2.)
const LAYER: usize = 3;

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
        app.add_systems(Update, (build, show).chain().run_if(crate::drive::driving));
    }
}

/// Puts the fruit up on the first frame of a level, which takes the old one away with the
/// camera. The fruit is the level's own, or the pack's if the level has none.
fn build(
    mut commands: Commands,
    track: Res<Track>,
    textures: Res<Textures>,
    display: Res<FruitDisplay>,
    icon: Query<(), With<WumpaIcon>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut subtract_materials: ResMut<Assets<SubtractMaterial>>,
    mut none: Local<bool>,
) {
    if track.is_changed() {
        *none = false;
    }
    if !icon.is_empty() || *none {
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
    let camera = commands
        .spawn((
            // Not the camera that the wheels and the smoke face.
            crate::OverlayCamera,
            Camera3d::default(),
            // After the overlay camera, over its picture, with only the depth buffer cleared.
            Camera {
                order: 3,
                clear_color: ClearColorConfig::None,
                ..default()
            },
            Projection::Orthographic(OrthographicProjection {
                scaling_mode: ScalingMode::WindowSize,
                ..OrthographicProjection::default_3d()
            }),
            Transform::default(),
            RenderLayers::layer(LAYER),
        ))
        .id();
    let pivot = commands
        .spawn((
            WumpaIcon {
                diameter: (high - low).max_element(),
                rest,
            },
            Transform::default(),
            Visibility::default(),
            ChildOf(camera),
        ))
        .id();
    crate::spawn_parts(
        data.parts,
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

/// Puts the fruit in the corner of the window, as tall as the HUD wants it, and turns it as
/// the fruit of the level turn.
fn show(
    sim: Res<crate::props::Sim>,
    fixed: Res<Time<Fixed>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut icon: Query<(&WumpaIcon, &mut Transform)>,
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
    for (icon, mut transform) in &mut icon {
        // The camera looks at the middle of the window, with y up.
        transform.translation = Vec3::new(
            FRUIT_X * crate::hud::PIXEL.x,
            window.height() / 2.0 - ICON_MIDDLE_Y * crate::hud::PIXEL.y,
            -DEPTH,
        );
        transform.scale = Vec3::splat(HEIGHT_PIXELS / icon.diameter);
        transform.rotation = icon.rest * Quat::from_rotation_y(angle);
    }
}
