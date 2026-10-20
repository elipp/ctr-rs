//! The models the level places (`struct InstDef`): mushrooms, crates, fruit, the start banner.
//!
//! Each is posed from its definition's position, angles (`ConvertRotToMatrix`) and scale. A
//! model with an animation plays its first one over and over at the game tick; the objects the
//! game moves by code (spinning fruit, bobbing crates, anything that reacts) stand still here.

use bevy::prelude::*;
use loader::dram::DramFile;
use loader::lev::Level;
use loader::model::{Model, ModelHeader};
use loader::vram::Vram;
use render::mesh::{self, PARTS, to_world};

use crate::drive::animation_pose;
use crate::subtract::SubtractMaterial;
use crate::{GameTick, spawn_parts};

/// A placed model whose pose changes with the tick.
struct Animated {
    header: ModelHeader,
    inst_scale: [i16; 3],
    parts: [Option<Handle<Mesh>>; PARTS],
    shown: Option<i16>,
}

#[derive(Resource, Default)]
pub struct Props(Vec<Animated>);

/// The scale an object's init sets on its instance, replacing the level file's (`RB_Plant_Init`,
/// `RB_CtrLetter_Init`); the other placed models keep theirs. By model name.
fn spawn_scale(model: &str) -> Option<i16> {
    match model {
        "plant" => Some(0x2800),
        "c" | "t" | "r" => Some(0x1800),
        _ => None,
    }
}

/// The pose of a model at rest: its first animation's first frame, or its only frame.
fn rest_pose(header: &ModelHeader) -> Option<&loader::model::Frame> {
    header.animations.first().and_then(|a| a.frames.first()).or(header.static_frame.as_ref())
}

pub fn spawn(
    level: &Level,
    dram: &DramFile,
    vram: &Vram,
    commands: &mut Commands,
    images: &mut Assets<Image>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    subtract_materials: &mut Assets<SubtractMaterial>,
) {
    let mut props = Props::default();
    let mut models: std::collections::HashMap<u32, Option<Model>> = Default::default();
    let mut gte = gte::Gte::default();
    for inst in level.instances.iter().filter(|i| i.model != 0) {
        let model = models.entry(inst.model).or_insert_with(|| Model::parse_at(dram, inst.model).ok());
        let Some(header) = model.as_ref().and_then(|m| m.headers.first()) else { continue };
        let Some(frame) = rest_pose(header) else { continue };
        let inst_scale = match model.as_ref().and_then(|m| spawn_scale(&m.name)) {
            Some(scale) => [scale; 3],
            None => [inst.scale.x, inst.scale.y, inst.scale.z],
        };
        let data = mesh::model_mesh(header, frame, vram, inst_scale);

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

        // The matrix's columns are where the model's axes go (as for the kart).
        let matrix = vehicle::rotmat::convert_rot_to_matrix(&mut gte, inst.rot);
        let column = |c: usize| Vec3::new(matrix.m[0][c] as f32, matrix.m[1][c] as f32, matrix.m[2][c] as f32) / 4096.0;
        let rotation = Quat::from_mat3(&Mat3::from_cols(column(0), column(1), column(2)));
        let transform = Transform::from_translation(Vec3::from(to_world(inst.pos))).with_rotation(rotation);

        let parts = spawn_parts(data.parts, &base, transform, None, commands, meshes, materials, subtract_materials, 0);
        if header.animations.first().is_some_and(|a| a.frames.len() > 1) {
            props.0.push(Animated { header: header.clone(), inst_scale, parts, shown: Some(0) });
        }
    }
    info!("{} placed models, {} animated", level.instances.len(), props.0.len());
    commands.insert_resource(props);
}

/// Plays the placed models' first animation, once per game tick.
pub fn animate(mut props: ResMut<Props>, tick: Res<GameTick>, mut meshes: ResMut<Assets<Mesh>>) {
    for prop in &mut props.0 {
        let Some(animation) = prop.header.animations.first() else { continue };
        let frame = (tick.0 % animation.num_frames.max(1) as u32) as i16;
        if prop.shown == Some(frame) {
            continue;
        }
        prop.shown = Some(frame);
        let Some(pose) = animation_pose(&prop.header, 0, frame) else { continue };
        let positions = mesh::model_positions(&prop.header, &pose, prop.inst_scale);
        for (handle, positions) in prop.parts.iter().zip(positions) {
            let Some(mut mesh) = handle.as_ref().and_then(|h| meshes.get_mut(h)) else { continue };
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        }
    }
}

