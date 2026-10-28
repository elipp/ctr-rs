//! The fire of the fireballs of Mystery Caves (`RB_Fireball_ThTick`). The ball's model is hidden
//! by the game; what shows is the trail of sparks it leaves, in the level's `fireball` icon.
//! Where the balls go and what sparks they leave is `instance::Objects`, how a spark moves and
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

/// How many sparks a tick's stretch of the ball's path is filled with. The game leaves one a
/// tick, which for a ball at full speed is a dot every 200 units; these are what it would leave
/// had it run faster.
const SUBSTEPS: u32 = 1;

/// A spark, and how far into its first tick it was born: it is drawn that much further on in
/// its life than the tick it is in says, as one born earlier in the tick has lived longer.
struct Spark {
    particle: Particle,
    /// Ticks (a fraction of one) it has lived beyond the whole ticks it has been updated.
    lead: f32,
    /// Whole ticks it has been updated.
    ticks: u32,
    /// How many ticks it lives, from birth to gone.
    life: f32,
}

#[derive(Resource)]
pub struct Fire {
    sparks: Vec<Spark>,
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
    let (true, Some(icon)) = (fireballs > 0, icon) else {
        return;
    };

    let (image, width, height) = icon_image(&icon, vram);
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    set_quads(&mut mesh, &[], &[]);
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
    commands.insert_resource(Fire {
        sparks: Vec::new(),
        rng: Rng::new(0x1d87_2b41),
        width,
        height,
    });
}

impl Fire {
    /// Puts the sparks out, for a race that starts again.
    pub fn clear(&mut self) {
        self.sparks.clear();
    }
}

/// One game tick, after the hazards': the sparks they left start, then every spark moves. A
/// tick of the sim is taken once, however many steps of the clock pass while it waits.
pub fn tick(fire: Option<ResMut<Fire>>, sim: Res<crate::props::Sim>, mut seen: Local<Option<u32>>) {
    let Some(mut fire) = fire else { return };
    if *seen == Some(sim.race.tick) {
        return;
    }
    *seen = Some(sim.race.tick);
    let fire = &mut *fire;
    // The ball went from `prev_y` to `pos` in the tick: the sparks are spread along that, the
    // first where it is now, and each later one a step further back and a step older.
    for spark in &sim.race.last.sparks {
        for step in 0..SUBSTEPS {
            let back = step as f32 / SUBSTEPS as f32;
            let mut pos = spark.pos;
            pos[1] -= ((pos[1] - spark.prev_y) as f32 * back).round() as i32;
            let particle = Particle::fireball(pos, spark.vel_y, &mut fire.rng);
            // It is updated `frames_left + 1` times before it is gone, the first in this tick.
            let life = particle.frames_left as f32 + 1.0;
            fire.sparks.push(Spark {
                particle,
                lead: back,
                ticks: 0,
                life,
            });
        }
    }
    // (As in the game, a new spark is updated once in the tick it is born.)
    fire.sparks.retain_mut(|s| {
        s.ticks += 1;
        s.particle.update() && s.ticks as f32 + s.lead < s.life
    });
}

/// Lays the sparks out as camera-facing quads, between the last two ticks.
pub fn update(
    sprites: Query<&Mesh3d, With<FireSprites>>,
    cameras: Query<
        &Transform,
        (
            With<Camera3d>,
            Without<crate::OverlayCamera>,
            Without<crate::sky::SkyCamera>,
        ),
    >,
    fire: Option<Res<Fire>>,
    fixed: Res<Time<Fixed>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let (Some(fire), Some(camera), Some(mesh)) =
        (fire, cameras.iter().next(), sprites.iter().next())
    else {
        return;
    };
    let Some(mut mesh) = meshes.get_mut(&mesh.0) else {
        return;
    };
    let (right, up) = (camera.rotation * Vec3::X, camera.rotation * Vec3::Y);
    let t = fixed.overstep_fraction();
    let mut positions = Vec::with_capacity(4 * fire.sparks.len());
    let mut colors = Vec::with_capacity(4 * fire.sparks.len());
    for s in &fire.sparks {
        // Where it is in its life, between the ticks.
        let age = s.lead + t;
        if s.ticks as f32 + age >= s.life {
            continue;
        }
        let p = s.particle.ahead(age);
        let Some(corners) = p.corners(fire.width, fire.height) else {
            continue;
        };
        // The positions are absolute, in 1/256 game units.
        let center = Vec3::from_array(p.position().map(|c| c as f32)) / 256.0 * SCALE;
        let [r, g, b] = p.rgb().map(|c| c as f32 / 128.0);
        for [x, y] in corners {
            let corner = center + right * quarter_units_to_world(x as f32)
                - up * (quarter_units_to_world(y as f32) * SCREEN_PIXEL_ASPECT);
            positions.push(corner.to_array());
            colors.push([r, g, b, 1.0]);
        }
    }
    set_quads(&mut mesh, &positions, &colors);
}

/// Gives the mesh one quad for each four corners (top left, top right, bottom left, bottom
/// right) in `positions`. With none it keeps one that is not seen, as a mesh with no vertices at
/// all is not drawn right.
fn set_quads(mesh: &mut Mesh, positions: &[[f32; 3]], colors: &[[f32; 4]]) {
    let hidden = ([[0.0f32; 3]; 4], [[0.0f32; 4]; 4]);
    let (positions, colors) = if positions.is_empty() {
        (&hidden.0[..], &hidden.1[..])
    } else {
        (positions, colors)
    };
    let quads = (positions.len() / 4) as u32;
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions.to_vec());
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors.to_vec());
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_UV_0,
        (0..quads)
            .flat_map(|_| [[0.0f32, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]])
            .collect::<Vec<_>>(),
    );
    mesh.insert_indices(Indices::U32(
        (0..quads)
            .flat_map(|q| [0, 2, 1, 1, 2, 3].map(|i| q * 4 + i))
            .collect(),
    ));
}
