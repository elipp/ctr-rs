//! The sky dome (`DrawSky_Full`): gouraud triangles around the camera.
//!
//! Retail projects it with the view's rotation only and links it into the farthest slot of the
//! ordering table, so it is always behind everything and never gets closer. Here a camera of its
//! own draws it first, at the origin, with the main camera's rotation, and the main camera then
//! draws over it without clearing. Retail only draws the 4 of 8 segments nearest the heading; all
//! 8 are drawn here, the depth buffer and the frustum do the rest.
//!
//! Not done: the glow gradients (`CAM_SkyboxGlow`, `Level::glowGradient`), the level's clear color.

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::RenderLayers;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use loader::lev::Skybox;
use render::mesh::to_world;

/// The render layer of the sky; the main cameras are not on it.
const SKY_LAYER: usize = 2;

#[derive(Component)]
pub struct SkyCamera;

pub struct SkyPlugin;

impl Plugin for SkyPlugin {
    fn build(&self, app: &mut App) {
        // After the cameras moved, before their transforms propagate (as `wheels::update`).
        app.add_systems(PostUpdate, follow.before(TransformSystems::Propagate));
    }
}

/// Spawns the sky mesh and the camera that draws it. `projection` is the main camera's.
pub fn spawn(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    sky: &Skybox,
    mut projection: PerspectiveProjection,
) {
    let mut positions = Vec::new();
    let mut colors = Vec::new();
    let mut radius = 0.0f32;
    for tri in sky.segments.iter().flatten() {
        for &i in tri {
            let v = sky.vertices[i as usize];
            let p = to_world(v.pos);
            radius = radius.max(Vec3::from(p).length());
            positions.push(p);
            let [r, g, b] = v.color.map(|c| c as f32 / 255.0);
            colors.push([r, g, b, 1.0]);
        }
    }
    if positions.is_empty() {
        return;
    }
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    let indices = (0..positions.len() as u32).collect();
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_indices(Indices::U32(indices));
    commands.spawn((
        Mesh3d(meshes.add(mesh)),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::WHITE,
            unlit: true,
            // The game does not cull it: no NCLIP in `DrawSky_Piece`.
            cull_mode: None,
            double_sided: true,
            ..default()
        })),
        RenderLayers::layer(SKY_LAYER),
    ));
    projection.far = projection.far.max(radius * 2.0);
    commands.spawn((
        SkyCamera,
        Camera3d::default(),
        Camera { order: -1, ..default() },
        Projection::Perspective(projection),
        Transform::default(),
        RenderLayers::layer(SKY_LAYER),
    ));
}

fn follow(
    main: Query<(&Camera, &GlobalTransform), (With<Camera3d>, Without<SkyCamera>)>,
    mut sky: Query<&mut Transform, With<SkyCamera>>,
) {
    let Some((_, at)) = main.iter().find(|(camera, _)| camera.order == 0) else { return };
    for mut transform in &mut sky {
        transform.rotation = at.rotation();
    }
}
