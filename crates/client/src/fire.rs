//! The fire of the fireballs of Mystery Caves (`RB_Fireball_ThTick`) and of the flame jets of
//! Tiger Temple (`RB_FlameJet_Particles`), and the tires the plants of Papu's Pyramid spit
//! (`emSet_PlantTires`). The ball's model is hidden by the game; what shows is the trail of sparks
//! it leaves, in the level's `fireball` icon, which the jets' flames use too. The tires are the
//! wheel icons of the racer, spinning.
//! Where the balls go and what sparks they leave is `instance::Objects`, how a spark moves and
//! looks is `render::particles`; this only shows them.

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use loader::dram::DramFile;
use loader::icons::{Icon, parse_level_groups};
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

/// The tires a plant spits at a time.
const TIRES_A_SPIT: usize = 4;

/// A spark, and how far into its first tick it was born: it is drawn that much further on in
/// its life than the tick it is in says, as one born earlier in the tick has lived longer.
pub(crate) struct Spark {
    pub(crate) particle: Particle,
    /// Ticks (a fraction of one) it has lived beyond the whole ticks it has been updated.
    pub(crate) lead: f32,
    /// Whole ticks it has been updated.
    pub(crate) ticks: u32,
    /// How many ticks it lives, from birth to gone.
    pub(crate) life: f32,
}

impl Spark {
    /// A particle that was born this tick, at the start of its first.
    fn born(particle: Particle) -> Spark {
        let life = particle.frames_left as f32 + 1.0;
        Spark {
            particle,
            lead: 0.0,
            ticks: 0,
            life,
        }
    }
}

#[derive(Resource)]
pub struct Fire {
    sparks: Vec<Spark>,
    /// The tires the plants spat.
    tires: Vec<Spark>,
    /// The heat the flame jets give off, oldest first: lenses that warp the picture (`heat`).
    pub(crate) heat: Vec<Spark>,
    rng: Rng,
    width: i32,
    height: i32,
    /// Where each wheel icon is in the tires' picture, and its size in texels.
    wheels: Vec<Wheel>,
}

/// One wheel icon in the picture of them all, stacked.
struct Wheel {
    width: i32,
    height: i32,
    /// Left, top, right and bottom of it in the picture, 0 to 1.
    uv: [f32; 4],
}

#[derive(Component)]
pub struct FireSprites;

#[derive(Component)]
pub struct TireSprites;

/// What the level has that these show: `emitters` fireballs and flame jets, and `plants`.
pub struct Sources {
    pub emitters: usize,
    pub plants: usize,
}

/// Spawns the sprites of the sparks and of the tires, for what the level has.
pub fn spawn(
    sources: Sources,
    wheel_icons: &[Icon],
    dram: &DramFile,
    vram: &Vram,
    commands: &mut Commands,
    images: &mut Assets<Image>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) {
    let mut sprites = |image: Image, mode: AlphaMode| {
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        set_quads(&mut mesh, &[], &[], &[]);
        (
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: Color::WHITE,
                base_color_texture: Some(images.add(image)),
                unlit: true,
                cull_mode: None,
                double_sided: true,
                alpha_mode: mode,
                ..default()
            })),
            Transform::default(),
            NoFrustumCulling,
        )
    };

    let icon = parse_level_groups(dram)
        .ok()
        .and_then(|groups| groups.into_iter().find(|g| g.group_id == FIREBALL_GROUP))
        .and_then(|group| group.icons.into_iter().next())
        .filter(|_| sources.emitters > 0);
    let (mut width, mut height) = (0, 0);
    if let Some(icon) = &icon {
        let (image, w, h) = icon_image(icon, vram);
        (width, height) = (w, h);
        commands.spawn((FireSprites, sprites(image, AlphaMode::Add)));
    }

    let mut wheels = Vec::new();
    if sources.plants > 0 && !wheel_icons.is_empty() {
        let (image, stacked) = wheel_image(wheel_icons, vram);
        wheels = stacked;
        // A tire is opaque, where it has texels.
        commands.spawn((TireSprites, sprites(image, AlphaMode::Mask(0.5))));
    }
    if icon.is_none() && wheels.is_empty() {
        return;
    }
    info!(
        "{} fireballs and flame jets, {} plants",
        sources.emitters, sources.plants
    );
    commands.insert_resource(Fire {
        sparks: Vec::new(),
        tires: Vec::new(),
        heat: Vec::new(),
        rng: Rng::new(0x1d87_2b41),
        width,
        height,
        wheels,
    });
}

/// The wheel icons one above the other in one picture, and where each is.
fn wheel_image(icons: &[Icon], vram: &Vram) -> (Image, Vec<Wheel>) {
    let pictures: Vec<_> = icons.iter().map(|icon| icon_image(icon, vram)).collect();
    let total_width = pictures.iter().map(|p| p.1).max().unwrap_or(1) as usize;
    let total_height = pictures.iter().map(|p| p.2).sum::<i32>().max(1) as usize;
    let mut rgba = vec![0u8; total_width * total_height * 4];
    let mut wheels = Vec::new();
    let mut top = 0usize;
    for (image, width, height) in &pictures {
        let (w, h) = (*width as usize, *height as usize);
        let data = image.data.as_deref().unwrap_or(&[]);
        for y in 0..h {
            let row = &data[y * w * 4..(y + 1) * w * 4];
            rgba[((top + y) * total_width) * 4..][..w * 4].copy_from_slice(row);
        }
        wheels.push(Wheel {
            width: *width,
            height: *height,
            uv: [
                0.0,
                top as f32 / total_height as f32,
                w as f32 / total_width as f32,
                (top + h) as f32 / total_height as f32,
            ],
        });
        top += h;
    }
    let mut image = Image::new(
        bevy::render::render_resource::Extent3d {
            width: total_width as u32,
            height: total_height as u32,
            depth_or_array_layers: 1,
        },
        bevy::render::render_resource::TextureDimension::D2,
        rgba,
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = bevy::image::ImageSampler::nearest();
    (image, wheels)
}

impl Fire {
    /// Puts the sparks out, for a race that starts again.
    pub fn clear(&mut self) {
        self.sparks.clear();
        self.tires.clear();
        self.heat.clear();
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
    for flame in &sim.race.last.flames {
        let particle = Particle::flame_jet(flame.pos, flame.dir, flame.ticks, &mut fire.rng);
        let heat = Particle::flame_jet_heat(&particle, &mut fire.rng);
        fire.sparks.push(Spark::born(particle));
        fire.heat.push(Spark::born(heat));
    }
    for spit in &sim.race.last.spits {
        if fire.wheels.is_empty() {
            break;
        }
        for _ in 0..TIRES_A_SPIT {
            let particle =
                Particle::plant_tire(spit.pos, spit.facing, fire.wheels.len(), &mut fire.rng);
            fire.tires.push(Spark::born(particle));
        }
    }
    // (As in the game, a new spark is updated once in the tick it is born.)
    for list in [&mut fire.sparks, &mut fire.tires, &mut fire.heat] {
        list.retain_mut(|s| {
            s.ticks += 1;
            s.particle.update() && s.ticks as f32 + s.lead < s.life
        });
    }
}

/// Lays the sparks and the tires out as camera-facing quads, between the last two ticks.
pub fn update(
    sprites: Query<&Mesh3d, With<FireSprites>>,
    tire_sprites: Query<&Mesh3d, With<TireSprites>>,
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
    let (Some(fire), Some(camera)) = (fire, cameras.iter().next()) else {
        return;
    };
    let (right, up) = (camera.rotation * Vec3::X, camera.rotation * Vec3::Y);
    let t = fixed.overstep_fraction();
    if let Some(mesh) = sprites.iter().next()
        && let Some(mut mesh) = meshes.get_mut(&mesh.0)
    {
        let quads = lay_out(&fire.sparks, right, up, t, |_| {
            Some((fire.width, fire.height, [0.0, 0.0, 1.0, 1.0]))
        });
        set_quads(&mut mesh, &quads.0, &quads.1, &quads.2);
    }
    if let Some(mesh) = tire_sprites.iter().next()
        && let Some(mut mesh) = meshes.get_mut(&mesh.0)
    {
        let quads = lay_out(&fire.tires, right, up, t, |p| {
            let wheel = fire.wheels.get(p.icon_frame()?)?;
            Some((wheel.width, wheel.height, wheel.uv))
        });
        set_quads(&mut mesh, &quads.0, &quads.1, &quads.2);
    }
}

/// The corners, colours and picture coordinates of the quads of `sparks`; `picture` says how big
/// a particle's icon is and where it is in the texture (left, top, right and bottom).
fn lay_out(
    sparks: &[Spark],
    right: Vec3,
    up: Vec3,
    t: f32,
    picture: impl Fn(&Particle) -> Option<(i32, i32, [f32; 4])>,
) -> (Vec<[f32; 3]>, Vec<[f32; 4]>, Vec<[f32; 2]>) {
    let mut positions = Vec::with_capacity(4 * sparks.len());
    let mut colors = Vec::with_capacity(4 * sparks.len());
    let mut uvs = Vec::with_capacity(4 * sparks.len());
    for s in sparks {
        // Where it is in its life, between the ticks.
        let age = s.lead + t;
        if s.ticks as f32 + age >= s.life {
            continue;
        }
        let p = s.particle.ahead(age);
        let Some((width, height, [u0, v0, u1, v1])) = picture(&p) else {
            continue;
        };
        let Some(corners) = p.corners(width, height) else {
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
        uvs.extend([[u0, v0], [u1, v0], [u0, v1], [u1, v1]]);
    }
    (positions, colors, uvs)
}

/// Gives the mesh one quad for each four corners (top left, top right, bottom left, bottom
/// right) in `positions`. With none it keeps one that is not seen, as a mesh with no vertices at
/// all is not drawn right.
fn set_quads(mesh: &mut Mesh, positions: &[[f32; 3]], colors: &[[f32; 4]], uvs: &[[f32; 2]]) {
    let hidden = ([[0.0f32; 3]; 4], [[0.0f32; 4]; 4], [[0.0f32; 2]; 4]);
    let (positions, colors, uvs) = if positions.is_empty() {
        (&hidden.0[..], &hidden.1[..], &hidden.2[..])
    } else {
        (positions, colors, uvs)
    };
    let quads = (positions.len() / 4) as u32;
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions.to_vec());
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors.to_vec());
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs.to_vec());
    mesh.insert_indices(Indices::U32(
        (0..quads)
            .flat_map(|q| [0, 2, 1, 1, 2, 3].map(|i| q * 4 + i))
            .collect(),
    ));
}
