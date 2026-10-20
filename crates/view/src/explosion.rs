//! The break-up of a crate as Bevy entities: when the sim breaks a crate, the explosion model
//! (`render::explosion`) plays its animation at the crate, tinted as `hazard::crates` says.

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use loader::model::{Model, ModelHeader};
use render::explosion::{Playback, break_yaw, tint};
use render::mesh::{self, MeshPart, PARTS};

use crate::drive::animation_pose;
use crate::props::{CrateBody, Sim};
use crate::subtract::SubtractMaterial;
use crate::{GameTick, Textures, spawn_parts};

/// The model, and what is made of it once: the faces of its first frame and their texture.
#[derive(Resource)]
pub struct CrateExplosion {
    model: Option<Model>,
    built: Option<Built>,
}

struct Built {
    parts: [MeshPart; PARTS],
    atlas: Handle<Image>,
}

impl CrateExplosion {
    pub fn new(model: Option<Model>) -> CrateExplosion {
        CrateExplosion { model, built: None }
    }
}

/// One playing explosion.
#[derive(Component)]
pub struct Explosion {
    header: ModelHeader,
    parts: [Option<Handle<Mesh>>; PARTS],
    playback: Playback,
}

/// Starts an explosion at each crate the last tick broke.
pub fn spawn(
    sim: Res<Sim>,
    mut explosion: ResMut<CrateExplosion>,
    textures: Res<Textures>,
    bodies: Query<(&CrateBody, &Transform)>,
    mut seen: Local<Vec<u32>>,
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut subtract_materials: ResMut<Assets<SubtractMaterial>>,
) {
    let breaks: Vec<u32> = sim.hazards.crates.iter().map(|c| c.sim.breaks).collect();
    // A new level: whatever was broken before does not count.
    if seen.len() != breaks.len() {
        *seen = breaks.clone();
    }
    let Some(model) = explosion.model.clone() else { return };
    let Some(header) = model.headers.first() else { return };
    let Some(pose) = animation_pose(header, 0, 0) else { return };
    let frames = header.animations.first().map_or(0, |a| a.num_frames as i16);

    for (body, transform) in &bodies {
        let i = body.0;
        if breaks[i] == seen[i] {
            continue;
        }
        seen[i] = breaks[i];
        let crate_box = &sim.hazards.crates[i].sim;

        // The model's faces and texture are made once.
        if explosion.built.is_none() {
            let data = mesh::model_mesh(header, &pose, &textures.vram, [0x1000; 3]);
            let mut atlas = Image::new(
                Extent3d { width: data.atlas.width as u32, height: data.atlas.height as u32, depth_or_array_layers: 1 },
                TextureDimension::D2,
                data.atlas.rgba,
                TextureFormat::Rgba8UnormSrgb,
                RenderAssetUsages::RENDER_WORLD,
            );
            atlas.sampler = ImageSampler::nearest();
            explosion.built = Some(Built { parts: data.parts, atlas: images.add(atlas) });
        }
        let built = explosion.built.as_ref().unwrap();

        let base = StandardMaterial {
            base_color: {
                let [r, g, b] = tint(crate_box.kind.explosion_color());
                Color::srgb_u8(r, g, b)
            },
            base_color_texture: Some(built.atlas.clone()),
            unlit: true,
            cull_mode: None,
            double_sided: true,
            ..default()
        };
        // The weapon and time crates' break-up is turned by a random angle around the vertical.
        let yaw = if crate_box.kind.explosion_turned() { break_yaw(crate_box.breaks, i) * std::f32::consts::TAU } else { 0.0 };
        let place = Transform {
            translation: transform.translation,
            rotation: transform.rotation * Quat::from_rotation_y(yaw),
            scale: Vec3::ONE,
        };
        let entity = commands.spawn((place, Visibility::default())).id();
        let parts = spawn_parts(built.parts.clone(), &base, Transform::default(), Some(entity), &mut commands, &mut meshes, &mut materials, &mut subtract_materials, 0);
        commands.entity(entity).insert(Explosion { header: header.clone(), parts, playback: Playback::new(frames) });
    }
}

/// One game tick of every explosion: the next frame, until the animation is over.
pub fn animate(mut commands: Commands, tick: Res<GameTick>, mut explosions: Query<(Entity, &mut Explosion)>, mut meshes: ResMut<Assets<Mesh>>, mut last: Local<u32>) {
    // Only once per game tick, however often the system is scheduled.
    if std::mem::replace(&mut *last, tick.0) == tick.0 {
        return;
    }
    for (entity, mut explosion) in &mut explosions {
        if !explosion.playback.advance() {
            commands.entity(entity).despawn();
            continue;
        }
        let Some(pose) = animation_pose(&explosion.header, 0, explosion.playback.frame) else { continue };
        let positions = mesh::model_positions(&explosion.header, &pose, [0x1000; 3]);
        for (handle, positions) in explosion.parts.iter().zip(positions) {
            let Some(mut mesh) = handle.as_ref().and_then(|h| meshes.get_mut(h)) else { continue };
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        }
    }
}
