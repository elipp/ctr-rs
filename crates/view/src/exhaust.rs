//! The kart's exhaust smoke as Bevy entities. Whether and where it puffs is
//! `vehicle::exhaust`, how a particle moves and looks is `render::particles`.

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::image::ImageSampler;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use loader::icons::Icon;
use loader::vram::Vram;
use render::mesh::SCALE;
use render::particles::{DrawMode, Particle, Rng, quarter_units_to_world};

use crate::drive::Kart;
use crate::props::Sim;
use crate::remote::Remotes;
use crate::subtract::{self, SubtractMaterial};

/// The live particles, each with the slot of the kart it belongs to; they tick with the game.
/// (They are only drawn, so unlike the karts they are not part of the rollback: a puff from a
/// predicted pad that turns out wrong is just a puff that was not quite right.)
#[derive(Resource)]
pub struct Particles {
    pub list: Vec<(usize, Particle)>,
    pub rng: Rng,
}

impl Particles {
    pub fn new() -> Particles {
        Particles {
            list: Vec::new(),
            rng: Rng::new(0x2545_f491),
        }
    }

    /// The sparks of this game tick (before [`Particles::tick`]) of kart `slot`: the landing and
    /// the scrape of a wall that the sim saw.
    pub fn sparks(
        &mut self,
        slot: usize,
        landing: Option<vehicle::sparks::Landing>,
        wall: Option<vehicle::sparks::Wall>,
    ) {
        if let Some(l) = landing {
            for _ in 0..LANDING_SPARKS {
                let spark = Particle::landing_spark(l.back, l.side, l.push, &mut self.rng);
                self.list.push((slot, spark));
            }
        }
        if let Some(w) = wall {
            let m = w.matrix;
            let spark =
                Particle::wall_spark(w.tire, |v| vehicle::exhaust::rotate(&m, v), &mut self.rng);
            self.list.push((slot, spark));
        }
    }

    /// The puffs of this game tick of kart `slot` (before [`Particles::tick`]).
    pub fn puffs(&mut self, slot: usize, puffs: &[vehicle::exhaust::Puff]) {
        for puff in puffs {
            let smoke = Particle::exhaust(puff.position, puff.velocity, puff.dark, &mut self.rng);
            self.list.push((slot, smoke));
        }
    }

    /// One game tick: every particle moves, after the karts have puffed (retail updates the
    /// particles after the threads, so a new puff starts at its pipe).
    pub fn tick(&mut self) {
        self.list.retain_mut(|(_, p)| p.update());
    }
}

/// `VEH_EMITTER_GROUND_SPARK_COUNT`
const LANDING_SPARKS: usize = 10;

/// Width of a spark's streak, in game units.
const SPARK_WIDTH: f32 = 1.2;

/// The smoke's sprite size in texels; the picture is the whole texture.
#[derive(Component)]
pub struct Smoke {
    /// Whether this is the smoke of the other players' karts, which is in the world's layer.
    remote: bool,
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
            if let Ok(Some((c, _))) = vram.texel_stp(
                &icon.layout,
                u0.wrapping_add(x as u8),
                v0.wrapping_add(y as u8),
            ) {
                rgba[(y * width as usize + x) * 4..][..4].copy_from_slice(&c);
            }
        }
    }
    let mut image = Image::new(
        Extent3d {
            width: width as u32,
            height: height as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        rgba,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::nearest();
    (image, width, height)
}

/// How much taller than wide a retail screen pixel is: the picture is 512 by 216 pixels (`MainMain`'s
/// draw environments) shown at 4:3. The sprites are twice as wide in the camera's space to make up
/// for it, while the viewer's picture has square pixels, so their height needs the same stretch.
pub(crate) const SCREEN_PIXEL_ASPECT: f32 = (512.0 * 3.0 / 4.0) / 216.0;

/// Most quads one mesh draws: ours, and the other players' together.
const MAX_QUADS: usize = 64;
const MAX_REMOTE_QUADS: usize = 256;

fn max_quads(remote: bool) -> usize {
    if remote { MAX_REMOTE_QUADS } else { MAX_QUADS }
}

/// The streaks of the sparks, all in one mesh (`remote`: the other players' karts').
#[derive(Component)]
pub struct SparkLines {
    remote: bool,
}

/// Where a kart's particles are anchored: the kart of slot `slot`, as drawn. Ours is the
/// [`Kart`], the others' are in `remotes`; none while the kart is not up.
fn anchor(
    slot: usize,
    own: usize,
    kart: &Query<&Transform, With<Kart>>,
    remotes: &Remotes,
    transforms: &Query<&Transform, Without<Kart>>,
) -> Option<Vec3> {
    if slot == own {
        kart.iter().next().map(|t| t.translation)
    } else {
        let entity = remotes.entities.get(slot).copied().flatten()?;
        transforms.get(entity).ok().map(|t| t.translation)
    }
}

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
    for remote in [false, true] {
        spawn_set(
            commands,
            meshes,
            materials,
            subtract_materials,
            &image,
            (width, height),
            remote,
        );
    }
}

/// The meshes of one set: the smoke that adds, the smoke that subtracts and the sparks, for our
/// kart or for the others'.
fn spawn_set(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    subtract_materials: &mut Assets<SubtractMaterial>,
    image: &Handle<Image>,
    (width, height): (i32, i32),
    remote: bool,
) {
    let quads = max_quads(remote);
    let mut mesh = || {
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0f32; 3]; 4 * quads]);
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[0.0f32; 4]; 4 * quads]);
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_UV_0,
            (0..quads)
                .flat_map(|_| [[0.0f32, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]])
                .collect::<Vec<_>>(),
        );
        mesh.insert_indices(Indices::U32(
            (0..quads as u32)
                .flat_map(|q| [0, 2, 1, 1, 2, 3].map(|i| q * 4 + i))
                .collect(),
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
    let layer = RenderLayers::layer(if remote { 0 } else { crate::KART_LAYER });
    // Sparks start at the ground and the wall and fly into them; retail has no depth buffer to
    // hide them there, so theirs are over the track for the others' karts as well.
    let spark_layer = RenderLayers::layer(crate::KART_LAYER);
    commands.spawn((
        Smoke {
            remote,
            mode: DrawMode::Add,
            width,
            height,
        },
        Mesh3d(mesh()),
        MeshMaterial3d(materials.add(base(AlphaMode::Add))),
        Transform::default(),
        layer.clone(),
        NoFrustumCulling,
    ));
    commands.spawn((
        Smoke {
            remote,
            mode: DrawMode::Subtract,
            width,
            height,
        },
        Mesh3d(mesh()),
        MeshMaterial3d(subtract_materials.add(SubtractMaterial {
            base: base(AlphaMode::Blend),
            extension: subtract::Subtract {},
        })),
        Transform::default(),
        layer.clone(),
        NoFrustumCulling,
    ));

    // The sparks: untextured streaks that fade out towards their tails.
    let mut spark_mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    spark_mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0f32; 3]; 4 * quads]);
    spark_mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[0.0f32; 4]; 4 * quads]);
    spark_mesh.insert_indices(Indices::U32(
        (0..quads as u32)
            .flat_map(|q| [0, 2, 1, 1, 2, 3].map(|i| q * 4 + i))
            .collect(),
    ));
    commands.spawn((
        SparkLines { remote },
        Mesh3d(meshes.add(spark_mesh)),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::WHITE,
            unlit: true,
            cull_mode: None,
            double_sided: true,
            alpha_mode: AlphaMode::Add,
            ..default()
        })),
        Transform::default(),
        spark_layer,
        NoFrustumCulling,
    ));
}

/// Lays the particles out as camera-facing quads, between the last two ticks.
pub fn update(
    smoke: Query<(&Smoke, &Mesh3d)>,
    karts: Query<&Transform, With<Kart>>,
    transforms: Query<&Transform, Without<Kart>>,
    cameras: Query<
        &Transform,
        (
            With<Camera3d>,
            Without<Kart>,
            Without<crate::OverlayCamera>,
            Without<crate::sky::SkyCamera>,
        ),
    >,
    particles: Res<Particles>,
    remotes: Res<Remotes>,
    sim: Res<Sim>,
    fixed: Res<Time<Fixed>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let Some(camera) = cameras.iter().next() else {
        return;
    };
    let (right, up) = (camera.rotation * Vec3::X, camera.rotation * Vec3::Y);
    let t = fixed.overstep_fraction();
    for (smoke, mesh) in &smoke {
        let Some(mut mesh) = meshes.get_mut(&mesh.0) else {
            continue;
        };
        let quads = max_quads(smoke.remote);
        let mut positions = vec![[0.0f32; 3]; 4 * quads];
        let mut colors = vec![[0.0f32; 4]; 4 * quads];
        let wanted = particles
            .list
            .iter()
            .filter(|(slot, p)| {
                (*slot != sim.own) == smoke.remote && !p.is_line() && p.draw_mode() == smoke.mode
            })
            .take(quads);
        for (n, (slot, p)) in wanted.enumerate() {
            let Some(corners) = p.corners(smoke.width, smoke.height) else {
                continue;
            };
            let Some(kart) = anchor(*slot, sim.own, &karts, &remotes, &transforms) else {
                continue;
            };
            // The offsets are from the kart, in 1/256 game units; a particle is
            // `t` of the way to where its next update puts it.
            let (position, velocity) = (p.position(), p.velocity());
            let at = Vec3::from_array(std::array::from_fn(|a| {
                position[a] as f32 + velocity[a] as f32 * t
            }));
            let center = kart + at / 256.0 * SCALE;
            let gain = p.tint() as f32 / 128.0;
            for (k, [x, y]) in corners.into_iter().enumerate() {
                let corner = center + right * quarter_units_to_world(x as f32)
                    - up * (quarter_units_to_world(y as f32) * SCREEN_PIXEL_ASPECT);
                positions[n * 4 + k] = corner.to_array();
                colors[n * 4 + k] = [gain, gain, gain, 1.0];
            }
        }
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    }
}

/// Lays the sparks out as streaks facing the camera, from a coloured head to a dark tail.
pub fn update_sparks(
    lines: Query<(&SparkLines, &Mesh3d)>,
    karts: Query<&Transform, With<Kart>>,
    transforms: Query<&Transform, Without<Kart>>,
    cameras: Query<
        &Transform,
        (
            With<Camera3d>,
            Without<Kart>,
            Without<crate::OverlayCamera>,
            Without<crate::sky::SkyCamera>,
        ),
    >,
    particles: Res<Particles>,
    remotes: Res<Remotes>,
    sim: Res<Sim>,
    fixed: Res<Time<Fixed>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let Some(camera) = cameras.iter().next() else {
        return;
    };
    let t = fixed.overstep_fraction();
    for (lines, mesh) in &lines {
        let Some(mut mesh) = meshes.get_mut(&mesh.0) else {
            continue;
        };
        let quads = max_quads(lines.remote);
        let mut positions = vec![[0.0f32; 3]; 4 * quads];
        let mut colors = vec![[0.0f32; 4]; 4 * quads];
        let wanted = particles
            .list
            .iter()
            .filter(|(slot, p)| (*slot != sim.own) == lines.remote && p.is_line())
            .take(quads);
        for (n, (slot, p)) in wanted.enumerate() {
            let Some(kart) = anchor(*slot, sim.own, &karts, &remotes, &transforms) else {
                continue;
            };
            let world = |v: Vec3| v / 256.0 * SCALE;
            let (position, velocity) = (p.position(), p.velocity());
            let head = kart
                + world(Vec3::from_array(std::array::from_fn(|a| {
                    position[a] as f32 + velocity[a] as f32 * t
                })));
            let tail = head + world(Vec3::from_array(p.line_tail().map(|v| v as f32)));
            let along = tail - head;
            let toward_camera = camera.translation - head;
            let side = along.cross(toward_camera).normalize_or_zero() * (SPARK_WIDTH * SCALE / 2.0);
            let color = p.rgb().map(|c| c as f32 / 255.0);
            for (k, (point, shade)) in [
                (head + side, 1.0),
                (head - side, 1.0),
                (tail + side, 0.0),
                (tail - side, 0.0),
            ]
            .into_iter()
            .enumerate()
            {
                positions[n * 4 + k] = point.to_array();
                colors[n * 4 + k] = [color[0] * shade, color[1] * shade, color[2] * shade, 1.0];
            }
        }
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    }
}
