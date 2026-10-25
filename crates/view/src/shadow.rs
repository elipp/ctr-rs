//! The kart's ground shadow as a Bevy entity; the geometry is `render::shadow`
//! and whether there is one is `vehicle::shadow`.

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::image::ImageSampler;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use loader::icons::Icon;
use loader::vram::Vram;
use render::shadow::{GAIN, QUADS, shadow_atlas, shadow_quads};

use crate::kart::{Attached, KartMotion, KartSlot};
use crate::subtract::{self, SubtractMaterial};

/// The mesh of four quads, rewritten each frame.
#[derive(Component)]
pub struct Shadow {
    uvs: [[[f32; 2]; 4]; 2],
}

pub fn spawn(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<SubtractMaterial>,
    icons: [&Icon; 2],
    vram: &Vram,
    kart: Entity,
) {
    let (atlas, uvs) = shadow_atlas(icons, vram);
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
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0f32; 3]; 4 * QUADS]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0f32; 2]; 4 * QUADS]);
    // The primitive's colour scales the texels that are subtracted.
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_COLOR,
        vec![[GAIN, GAIN, GAIN, 1.0]; 4 * QUADS],
    );
    mesh.insert_indices(Indices::U32(
        (0..QUADS as u32)
            .flat_map(|q| [0, 2, 1, 1, 2, 3].map(|i| q * 4 + i))
            .collect(),
    ));
    commands.spawn((
        Shadow { uvs },
        Attached(kart),
        Mesh3d(meshes.add(mesh)),
        MeshMaterial3d(materials.add(SubtractMaterial {
            base: StandardMaterial {
                base_color: Color::WHITE,
                base_color_texture: Some(images.add(image)),
                unlit: true,
                cull_mode: None,
                double_sided: true,
                alpha_mode: AlphaMode::Blend,
                ..default()
            },
            extension: subtract::Subtract {},
        })),
        Transform::default(),
        Visibility::Hidden,
        NoFrustumCulling,
    ));
}

/// Poses the shadows between the last two ticks, or hides them.
pub fn update(
    mut shadows: Query<(&Shadow, &Attached, &Mesh3d, &mut Visibility)>,
    karts: Query<&KartMotion, With<KartSlot>>,
    fixed: Res<Time<Fixed>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let t = fixed.overstep_fraction();
    for (shadow, attached, mesh, mut visibility) in &mut shadows {
        let pose = karts
            .get(attached.0)
            .ok()
            .and_then(|motion| motion.shadow(t));
        let (Some((center, axes, height)), Some(mut mesh)) = (pose, meshes.get_mut(&mesh.0)) else {
            *visibility = Visibility::Hidden;
            continue;
        };
        *visibility = Visibility::Inherited;
        let mut positions = Vec::with_capacity(4 * QUADS);
        let mut uvs = Vec::with_capacity(4 * QUADS);
        for quad in shadow_quads(center, axes, height) {
            positions.extend(quad.corners.map(|c| c.to_array()));
            uvs.extend(shadow.uvs[quad.icon]);
        }
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    }
}
