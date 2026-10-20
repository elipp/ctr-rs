//! The kart's exhaust smoke as Bevy entities. Whether and where it puffs is
//! `vehicle::exhaust`, how a particle moves and looks is `render::particles`.

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::image::ImageSampler;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use loader::icons::Icon;
use loader::vram::Vram;
use render::mesh::SCALE;
use render::particles::{DrawMode, Particle, Rng, quarter_units_to_world};

use crate::drive::Kart;
use crate::subtract::{self, SubtractMaterial};

/// The live particles; they tick with the game.
#[derive(Resource)]
pub struct Particles {
    pub list: Vec<Particle>,
    pub rng: Rng,
}

impl Particles {
    pub fn new() -> Particles {
        Particles { list: Vec::new(), rng: Rng::new(0x2545_f491) }
    }

    /// One game tick: the kart puffs, then every particle moves (retail
    /// updates the particles after the threads, so a new puff starts at its pipe).
    pub fn tick(&mut self, puffs: &[vehicle::exhaust::Puff]) {
        for puff in puffs {
            self.list.push(Particle::exhaust(puff.position, puff.velocity, puff.dark, &mut self.rng));
        }
        self.list.retain_mut(|p| p.update());
    }
}

/// The smoke's sprite size in texels; the picture is the whole texture.
#[derive(Component)]
pub struct Smoke {
    mode: DrawMode,
    width: i32,
    height: i32,
}

/// The picture of an icon as a texture, and its size in texels.
pub fn icon_image(icon: &Icon, vram: &Vram) -> (Image, i32, i32) {
    let (u0, v0) = icon.layout.uv[0];
    let width = icon.layout.uv[1].0 as i32 - u0 as i32 + 1;
    let height = icon.layout.uv[2].1 as i32 - v0 as i32 + 1;
    let mut rgba = vec![0u8; width as usize * height as usize * 4];
    for y in 0..height as usize {
        for x in 0..width as usize {
            if let Ok(Some((c, _))) = vram.texel_stp(&icon.layout, u0.wrapping_add(x as u8), v0.wrapping_add(y as u8)) {
                rgba[(y * width as usize + x) * 4..][..4].copy_from_slice(&c);
            }
        }
    }
    let mut image = Image::new(
        Extent3d { width: width as u32, height: height as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        rgba,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::nearest();
    (image, width, height)
}

/// Most quads one mesh draws.
const MAX_QUADS: usize = 64;

pub fn spawn(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    subtract_materials: &mut Assets<SubtractMaterial>,
    icon: &Icon,
    vram: &Vram,
) {
    let (image, width, height) = icon_image(icon, vram);
    let image = images.add(image);

    let mut mesh = || {
        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0f32; 3]; 4 * MAX_QUADS]);
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[0.0f32; 4]; 4 * MAX_QUADS]);
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_UV_0,
            (0..MAX_QUADS).flat_map(|_| [[0.0f32, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]]).collect::<Vec<_>>(),
        );
        mesh.insert_indices(Indices::U32(
            (0..MAX_QUADS as u32).flat_map(|q| [0, 2, 1, 1, 2, 3].map(|i| q * 4 + i)).collect(),
        ));
        meshes.add(mesh)
    };
    let base = |alpha_mode| StandardMaterial {
        base_color: Color::WHITE,
        base_color_texture: Some(image.clone()),
        unlit: true,
        cull_mode: None,
        double_sided: true,
        alpha_mode,
        ..default()
    };
    commands.spawn((
        Smoke { mode: DrawMode::Add, width, height },
        Mesh3d(mesh()),
        MeshMaterial3d(materials.add(base(AlphaMode::Add))),
        Transform::default(),
        NoFrustumCulling,
    ));
    commands.spawn((
        Smoke { mode: DrawMode::Subtract, width, height },
        Mesh3d(mesh()),
        MeshMaterial3d(subtract_materials.add(SubtractMaterial {
            base: base(AlphaMode::Blend),
            extension: subtract::Subtract {},
        })),
        Transform::default(),
        NoFrustumCulling,
    ));
}

/// Lays the particles out as camera-facing quads, between the last two ticks.
pub fn update(
    smoke: Query<(&Smoke, &Mesh3d)>,
    karts: Query<&Transform, With<Kart>>,
    cameras: Query<&Transform, (With<Camera3d>, Without<Kart>)>,
    particles: Res<Particles>,
    fixed: Res<Time<Fixed>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let (Some(kart), Some(camera)) = (karts.iter().next(), cameras.iter().next()) else {
        return;
    };
    let (right, up) = (camera.rotation * Vec3::X, camera.rotation * Vec3::Y);
    let t = fixed.overstep_fraction();
    for (smoke, mesh) in &smoke {
        let Some(mut mesh) = meshes.get_mut(&mesh.0) else {
            continue;
        };
        let mut positions = vec![[0.0f32; 3]; 4 * MAX_QUADS];
        let mut colors = vec![[0.0f32; 4]; 4 * MAX_QUADS];
        let wanted = particles.list.iter().filter(|p| p.draw_mode() == smoke.mode).take(MAX_QUADS);
        for (n, p) in wanted.enumerate() {
            let Some(corners) = p.corners(smoke.width, smoke.height) else {
                continue;
            };
            // The offsets are from the kart, in 1/256 game units; a particle is
            // `t` of the way to where its next update puts it.
            let (position, velocity) = (p.position(), p.velocity());
            let at = Vec3::from_array(std::array::from_fn(|a| position[a] as f32 + velocity[a] as f32 * t));
            let center = kart.translation + at / 256.0 * SCALE;
            let gain = p.tint() as f32 / 128.0;
            for (k, [x, y]) in corners.into_iter().enumerate() {
                let corner = center + right * quarter_units_to_world(x as f32) - up * quarter_units_to_world(y as f32);
                positions[n * 4 + k] = corner.to_array();
                colors[n * 4 + k] = [gain, gain, gain, 1.0];
            }
        }
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    }
}
