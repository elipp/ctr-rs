//! The wheels of a kart as Bevy entities; the placement is `render::wheels`.

use crate::kart::KartMotion;
use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::image::ImageSampler;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use loader::icons::Icon;
use loader::vram::Vram;
use render::wheels::{
    DRIVER_INSTANCE_SCALE, WHEEL_SIZE, WHEELS, radius, sprite_atlas, wheel_quads,
};

/// The four wheels of a kart: one mesh of four squares, rewritten every frame
/// for the camera's position.
#[derive(Component)]
pub struct Wheels {
    uvs: Vec<[[f32; 2]; 4]>,
    /// The kart entity, whose pose and [`KartMotion`] the wheels follow.
    kart: Entity,
    wheel_size: i32,
}

/// Wheel squares are pushed this many radii towards the camera when they are on
/// the camera's side of the kart, so that the kart's body does not cut into them
/// (retail sorts the sprites among the body's polygons by depth instead).
const NEAR_SIDE_PUSH: f32 = 1.5;

pub fn spawn(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    icons: &[Icon],
    vram: &Vram,
    kart: Entity,
    layer: usize,
) {
    let (atlas, uvs) = sprite_atlas(icons, vram);
    let mut image = Image::new(
        Extent3d {
            width: atlas.width as u32,
            height: atlas.height as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        atlas.rgba,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::nearest();

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0f32; 3]; 4 * WHEELS]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0f32; 2]; 4 * WHEELS]);
    // Corners are in UV order (top left, top right, bottom left, bottom right).
    mesh.insert_indices(Indices::U32(
        (0..WHEELS as u32)
            .flat_map(|w| [0, 2, 1, 1, 2, 3].map(|i| w * 4 + i))
            .collect(),
    ));
    commands.spawn((
        Wheels {
            uvs,
            kart,
            wheel_size: WHEEL_SIZE,
        },
        Mesh3d(meshes.add(mesh)),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::WHITE,
            base_color_texture: Some(images.add(image)),
            unlit: true,
            cull_mode: None,
            double_sided: true,
            alpha_mode: AlphaMode::Blend,
            ..default()
        })),
        Transform::default(),
        RenderLayers::layer(layer),
        // The squares move with the camera; the mesh's bounds are those of its first frame.
        NoFrustumCulling,
    ));
}

/// Poses every wheel mesh for the camera, with the steering of its kart (a kart that stands
/// still has none).
pub fn update(
    wheels: Query<(Entity, &Wheels, &Mesh3d)>,
    karts: Query<(&Transform, Option<&KartMotion>), Without<Wheels>>,
    cameras: Query<
        &Transform,
        (
            With<Camera3d>,
            Without<crate::OverlayCamera>,
            Without<crate::sky::SkyCamera>,
        ),
    >,
    mut meshes: ResMut<Assets<Mesh>>,
    mut commands: Commands,
) {
    let Some(eye) = cameras.iter().next().map(|t| t.translation) else {
        return;
    };
    for (entity, wheels, mesh) in &wheels {
        let Ok((pose, motion)) = karts.get(wheels.kart) else {
            // The kart is gone (another player left).
            commands.entity(entity).despawn();
            continue;
        };
        let Some(mut mesh) = meshes.get_mut(&mesh.0) else {
            continue;
        };
        // The tires sit at the kart's squashed and stretched size.
        let (wheel_rotation, hazard_timer, scale) = motion
            .map_or((0, 0, [DRIVER_INSTANCE_SCALE; 3]), |m| {
                (m.wheel_rotation, m.hazard_timer, m.inst_scale)
            });
        let quads = wheel_quads(
            pose.translation,
            Mat3::from_quat(pose.rotation),
            scale,
            wheels.wheel_size,
            wheel_rotation,
            hazard_timer,
            eye,
        );
        let radius = radius(wheels.wheel_size);
        let mut positions = Vec::with_capacity(4 * WHEELS);
        let mut uvs = Vec::with_capacity(4 * WHEELS);
        for quad in quads {
            // Moving along the line of sight and shrinking about the eye by the same
            // factor leaves the square looking the same.
            let distance = (quad.center - eye).length();
            let shrink = if quad.near_side && distance > 0.0 {
                (1.0 - NEAR_SIDE_PUSH * radius / distance).max(0.1)
            } else {
                1.0
            };
            positions.extend(quad.corners.map(|c| (eye + (c - eye) * shrink).to_array()));
            uvs.extend(wheels.uvs[quad.sprite.min(wheels.uvs.len() - 1)]);
        }
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    }
}
