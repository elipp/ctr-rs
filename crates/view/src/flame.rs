//! The fire of a boost as Bevy entities. How it burns and fades is
//! `vehicle::turbo`; each pipe shows one of eight models (`turbo` .. `turbo7`)
//! of the racer's pack, the second pipe mirrored.

use std::io;

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use loader::dram::DramFile;
use loader::model::{Model, parse_pack_models};
use loader::vram::Vram;
use render::mesh::{self, SCALE};
use vehicle::turbo::{self, ANIM_FRAMES};

use crate::drive::Drive;
use crate::subtract::SubtractMaterial;

/// `STATIC_TURBO_EFFECT`: the model id of the first frame.
const FIRST_MODEL_ID: i16 = 44;

/// `alphaScale` at which the fire is gone.
const ALPHA_GONE: f32 = 0x1000 as f32;

/// The eight frames of the racer pack, in order.
pub fn models(pack: &DramFile) -> io::Result<Vec<Model>> {
    let mut models: Vec<_> = parse_pack_models(pack)?
        .into_iter()
        .filter(|m| (FIRST_MODEL_ID..FIRST_MODEL_ID + ANIM_FRAMES).contains(&m.id))
        .collect();
    models.sort_by_key(|m| m.id);
    if models.len() != ANIM_FRAMES as usize {
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("{} turbo models", models.len())));
    }
    Ok(models)
}

/// One frame of one pipe's fire; only the one the turbo shows is visible.
#[derive(Component)]
pub struct Flame {
    frame: usize,
    pipe: usize,
}

pub fn spawn(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    subtract_materials: &mut Assets<SubtractMaterial>,
    models: &[Model],
    vram: &Vram,
    kart: Entity,
) {
    for (frame, model) in models.iter().enumerate() {
        let header = &model.headers[0];
        let Some(pose) = header.static_frame.as_ref().or_else(|| header.animations.first().and_then(|a| a.frames.first())) else {
            continue;
        };
        for pipe in 0..2 {
            // Unscaled: the fire's own size is set every frame.
            let data = mesh::model_mesh(header, pose, vram, [0x1000; 3]);
            let mut atlas = Image::new(
                Extent3d { width: data.atlas.width as u32, height: data.atlas.height as u32, depth_or_array_layers: 1 },
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
                cull_mode: None,
                double_sided: true,
                ..default()
            };
            let group = commands
                .spawn((Flame { frame, pipe }, Transform::default(), Visibility::Hidden, ChildOf(kart)))
                .id();
            crate::spawn_parts(data.parts, &base, Transform::default(), Some(group), commands, meshes, materials, subtract_materials, crate::KART_LAYER);
        }
    }
}

/// Shows the frames the turbo is on, at its size, at the pipes, dimmed as it
/// fades out.
pub fn update(
    drive: Res<Drive>,
    mut flames: Query<(&Flame, &mut Transform, &mut Visibility, &Children)>,
    parts: Query<(Option<&MeshMaterial3d<StandardMaterial>>, Option<&MeshMaterial3d<SubtractMaterial>>)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut subtract_materials: ResMut<Assets<SubtractMaterial>>,
) {
    let turbo = drive.driver.turbo.filter(|t| !t.hidden);
    let offsets = turbo::pipe_offsets([drive.driver.inst_scale_x, drive.driver.inst_scale_y, drive.driver.inst_scale_z].map(i32::from));
    for (flame, mut transform, mut visibility, children) in &mut flames {
        let shown = turbo.filter(|t| t.frames()[flame.pipe] == flame.frame);
        *visibility = if shown.is_some() { Visibility::Inherited } else { Visibility::Hidden };
        let Some(turbo) = shown else { continue };
        let size = turbo.scale();
        // The second pipe's fire is the first one's mirror image.
        let mirror = if flame.pipe == 0 { 1.0 } else { -1.0 };
        transform.translation = Vec3::from_array(offsets[flame.pipe].map(|v| v as f32)) * SCALE;
        transform.scale = Vec3::new(size * mirror, size, size);

        // Fading in the colour, and in the alpha of the blended parts.
        let fade = (1.0 - turbo.alpha_scale as f32 / ALPHA_GONE).clamp(0.0, 1.0);
        let dim = |m: &mut StandardMaterial| {
            let masked = matches!(m.alpha_mode, AlphaMode::Mask(_));
            m.base_color = Color::linear_rgba(fade, fade, fade, if masked { 1.0 } else { fade });
        };
        for child in children {
            let Ok((standard, subtract)) = parts.get(*child) else { continue };
            if let Some(mut m) = standard.and_then(|h| materials.get_mut(&h.0)) {
                dim(&mut *m);
            }
            if let Some(mut m) = subtract.and_then(|h| subtract_materials.get_mut(&h.0)) {
                dim(&mut m.base);
            }
        }
    }
}
