//! What the spiders of Cortex Castle leave to the drawing: the web each hangs from
//! (`RB_Spider_DrawWebs`) and the shadow on the ground under it (`spidershadow`). Where they are
//! is `instance::hazard::spider`'s.
//!
//! Retail adds the web to the picture as a line of grey that fades out between 0xa00 and 0x1200
//! units of depth; here it is a translucent white gizmo line.

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use instance::Objects;
use loader::dram::DramFile;
use loader::lev::{Level, SVec3};
use loader::model::Model;
use loader::vram::Vram;
use render::mesh::{self, SCALE};

use crate::props::{Sim, rotation_of};
use crate::spawn_parts;
use crate::subtract::SubtractMaterial;

/// The colour of the web where it is not faded (`0x3f` of 255).
const BRIGHTNESS: f32 = 0x3f as f32 / 255.0;
/// The depth the web starts to fade at, and the one it is gone at.
const FADE_START: f32 = 0xa00 as f32;
const FADE_END: f32 = 0x1200 as f32;

/// The shadow of the spider at this index of `Objects::spiders`.
#[derive(Component)]
pub struct SpiderShadow(pub usize);

/// Scales the shadows by the spiders' drop, once per frame.
pub fn shadows(mut shadows: Query<(&SpiderShadow, &mut Transform)>, sim: Res<Sim>) {
    for (shadow, mut transform) in &mut shadows {
        let (_, scale) = sim.race.objects.spiders[shadow.0].sim.shadow();
        let wide = scale as f32 / instance::hazard::spider::SHADOW_SCALE as f32;
        transform.scale = Vec3::new(wide, 1.0, wide);
    }
}

pub fn draw_webs(
    sim: Res<Sim>,
    cameras: Query<&GlobalTransform, With<Camera3d>>,
    mut gizmos: Gizmos,
) {
    let at = |p: [i32; 3]| Vec3::new(p[0] as f32, p[1] as f32, p[2] as f32) * SCALE;
    for spider in &sim.race.objects.spiders {
        let (top, bottom) = spider.sim.web();
        let (top, bottom) = (at(top), at(bottom));
        // The web is rejected by the depth of its lower end, in the view that sees it best.
        let brightness = cameras
            .iter()
            .filter_map(|camera| {
                let depth = (bottom - camera.translation()).dot(*camera.forward()) / SCALE;
                (1.0..FADE_END).contains(&depth).then(|| {
                    if depth > FADE_START {
                        BRIGHTNESS * (FADE_END - depth) / (FADE_END - FADE_START)
                    } else {
                        BRIGHTNESS
                    }
                })
            })
            .fold(0.0, f32::max);
        if brightness > 0.0 {
            gizmos.line(top, bottom, Color::srgba(1.0, 1.0, 1.0, brightness));
        }
    }
}

/// Pulls the shadow toward the camera. It is a flat quad 8 units over a floor that is not flat,
/// which cuts it into wedges; retail draws it after the level, so that it is always on top of it.
const SHADOW_DEPTH_BIAS: f32 = 1_000_000.0;

/// The model the level keeps under `DYNAMIC_SPIDERSHADOW`.
const SHADOW_MODEL_ID: i16 = 0x53;

/// Puts the shadow of the spider `index` on the ground: `spidershadow` turned a quarter, flat.
pub fn spawn_shadow(
    index: usize,
    level: &Level,
    objects: &Objects,
    dram: &DramFile,
    vram: &Vram,
    commands: &mut Commands,
    images: &mut Assets<Image>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    subtract_materials: &mut Assets<SubtractMaterial>,
) {
    let model = level
        .models
        .iter()
        .filter_map(|&at| Model::parse_at(dram, at).ok())
        .find(|m| m.id == SHADOW_MODEL_ID);
    let Some(header) = model.as_ref().and_then(|m| m.headers.first()) else {
        warn!("level has no spider shadow model");
        return;
    };
    let Some(frame) = crate::props::rest_pose(header) else {
        return;
    };
    let scale = instance::hazard::spider::SHADOW_SCALE as i16;
    let data = mesh::model_mesh(header, frame, vram, [scale; 3]);
    let mut atlas = Image::new(
        Extent3d {
            width: data.atlas.width as u32,
            height: data.atlas.height as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data.atlas.rgba,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    atlas.sampler = ImageSampler::nearest();
    let base = StandardMaterial {
        base_color: Color::WHITE,
        base_color_texture: Some(images.add(atlas)),
        unlit: true,
        // The winding of a model's triangles means nothing, so none are culled.
        cull_mode: None,
        double_sided: true,
        depth_bias: SHADOW_DEPTH_BIAS,
        ..default()
    };
    let (pos, _) = objects.spiders[index].sim.shadow();
    let transform =
        Transform::from_translation(Vec3::new(pos[0] as f32, pos[1] as f32, pos[2] as f32) * SCALE)
            .with_rotation(rotation_of(SVec3 {
                x: 0,
                y: 0x200,
                z: 0,
            }));
    let body = commands
        .spawn((SpiderShadow(index), transform, Visibility::default()))
        .id();
    spawn_parts(
        data.parts,
        &base,
        Transform::default(),
        Some(body),
        commands,
        meshes,
        materials,
        subtract_materials,
        0,
    );
}
