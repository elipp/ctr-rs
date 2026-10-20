//! The fire of the fireballs of Mystery Caves (`RB_Fireball_ThTick`). The ball's model is hidden
//! by the game; what shows is the trail of sparks it leaves, in the level's `fireball` icon.
//! Where the balls go and what sparks they leave is `hazard::Hazards`, how a spark moves and
//! looks is `render::particles`; this only shows them.

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use loader::dram::DramFile;
use loader::icons::parse_level_groups;
use loader::vram::Vram;
use render::mesh::SCALE;
use render::particles::{Particle, Rng, quarter_units_to_world};

use crate::exhaust::{SCREEN_PIXEL_ASPECT, icon_image};

/// `IconGroup::group_id` of the fire's sprite in the level's own icons.
const FIREBALL_GROUP: i16 = 10;

/// Most sparks drawn at once.
const MAX_QUADS: usize = 64;

#[derive(Resource)]
pub struct Fire {
    sparks: Vec<Particle>,
    rng: Rng,
    width: i32,
    height: i32,
}

#[derive(Component)]
pub struct FireSprites;

/// Spawns the sprites of the sparks, if the level has fireballs.
pub fn spawn(
    fireballs: usize,
    dram: &DramFile,
    vram: &Vram,
    commands: &mut Commands,
    images: &mut Assets<Image>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) {
    let icon = parse_level_groups(dram)
        .ok()
        .and_then(|groups| groups.into_iter().find(|g| g.group_id == FIREBALL_GROUP))
        .and_then(|group| group.icons.into_iter().next());
    let (true, Some(icon)) = (fireballs > 0, icon) else { return };

    let (image, width, height) = icon_image(&icon, vram);
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0f32; 3]; 4 * MAX_QUADS]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[0.0f32; 4]; 4 * MAX_QUADS]);
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_UV_0,
        (0..MAX_QUADS).flat_map(|_| [[0.0f32, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]]).collect::<Vec<_>>(),
    );
    mesh.insert_indices(Indices::U32((0..MAX_QUADS as u32).flat_map(|q| [0, 2, 1, 1, 2, 3].map(|i| q * 4 + i)).collect()));
    commands.spawn((
        FireSprites,
        Mesh3d(meshes.add(mesh)),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::WHITE,
            base_color_texture: Some(images.add(image)),
            unlit: true,
            cull_mode: None,
            double_sided: true,
            alpha_mode: AlphaMode::Add,
            ..default()
        })),
        Transform::default(),
        NoFrustumCulling,
    ));
    info!("{fireballs} fireballs");
    commands.insert_resource(Fire { sparks: Vec::new(), rng: Rng::new(0x1d87_2b41), width, height });
}

/// One game tick, after the hazards': the sparks they left start, then every spark moves.
pub fn tick(fire: Option<ResMut<Fire>>, sim: Res<crate::props::Sim>) {
    let Some(mut fire) = fire else { return };
    let fire = &mut *fire;
    for spark in &sim.last.sparks {
        fire.sparks.push(Particle::fireball(spark.pos, spark.vel_y, &mut fire.rng));
    }
    fire.sparks.retain_mut(|p| p.update());
}

/// Lays the sparks out as camera-facing quads, between the last two ticks.
pub fn update(
    sprites: Query<&Mesh3d, With<FireSprites>>,
    cameras: Query<&Transform, (With<Camera3d>, Without<crate::OverlayCamera>, Without<crate::sky::SkyCamera>)>,
    fire: Option<Res<Fire>>,
    fixed: Res<Time<Fixed>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let (Some(fire), Some(camera), Some(mesh)) = (fire, cameras.iter().next(), sprites.iter().next()) else { return };
    let Some(mut mesh) = meshes.get_mut(&mesh.0) else { return };
    let (right, up) = (camera.rotation * Vec3::X, camera.rotation * Vec3::Y);
    let t = fixed.overstep_fraction();
    let mut positions = vec![[0.0f32; 3]; 4 * MAX_QUADS];
    let mut colors = vec![[0.0f32; 4]; 4 * MAX_QUADS];
    for (n, p) in fire.sparks.iter().take(MAX_QUADS).enumerate() {
        let Some(corners) = p.corners(fire.width, fire.height) else { continue };
        // The positions are absolute, in 1/256 game units.
        let (position, velocity) = (p.position(), p.velocity());
        let at = Vec3::from_array(std::array::from_fn(|a| position[a] as f32 + velocity[a] as f32 * t));
        let center = at / 256.0 * SCALE;
        let [r, g, b] = p.rgb().map(|c| c as f32 / 128.0);
        for (k, [x, y]) in corners.into_iter().enumerate() {
            let corner = center + right * quarter_units_to_world(x as f32) - up * (quarter_units_to_world(y as f32) * SCREEN_PIXEL_ASPECT);
            positions[n * 4 + k] = corner.to_array();
            colors[n * 4 + k] = [r, g, b, 1.0];
        }
    }
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
}
