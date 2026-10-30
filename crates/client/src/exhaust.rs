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
use render::particles::{DrawMode, Particle, Rng, Sprite, quarter_units_to_world};

use crate::kart::{Attached, KartSlot};
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

    /// The dirt, grass or snow that the kart's tires threw this game tick (before
    /// [`Particles::tick`]). `timer` is the game's frame counter.
    pub fn spray(&mut self, slot: usize, spray: &Option<vehicle::sparks::Spray>, timer: u32) {
        use render::particles::{
            TERRAIN_DIRT, TERRAIN_GRASS_DARK, TERRAIN_GRASS_LIGHT, TERRAIN_SNOW,
        };
        use vehicle::sparks::Kick;
        let Some(spray) = spray else {
            return;
        };
        let set: &[_] = match spray.kick {
            Kick::Dirt => &TERRAIN_DIRT,
            Kick::LightGrass => &TERRAIN_GRASS_LIGHT,
            Kick::DarkGrass => &TERRAIN_GRASS_DARK,
            Kick::Snow => &TERRAIN_SNOW,
        };
        for &tire in &spray.tires {
            let streak = Particle::terrain_spray(
                set,
                tire,
                |v| vehicle::exhaust::rotate(&spray.matrix, v),
                timer,
                &mut self.rng,
            );
            self.list.push((slot, streak));
        }
    }

    /// The puffs of this game tick of kart `slot` (before [`Particles::tick`]): smoke, or bubbles
    /// where the pipe is under water.
    pub fn puffs(&mut self, slot: usize, puffs: &[vehicle::exhaust::Puff]) {
        for puff in puffs {
            let make = if puff.underwater {
                Particle::bubble
            } else {
                Particle::exhaust
            };
            let smoke = make(puff.position, puff.velocity, puff.dark, &mut self.rng);
            self.list.push((slot, smoke));
        }
    }

    /// The drops that the wake of kart `slot` threw up this game tick (`wake_particles`).
    pub fn splashes(&mut self, slot: usize, count: u8) {
        for _ in 0..count {
            let drop = Particle::splash(&mut self.rng);
            self.list.push((slot, drop));
        }
    }

    /// The clods that kart `slot` threw up in mud this game tick (`mud_particles`).
    pub fn clods(&mut self, slot: usize, count: u8) {
        for _ in 0..count {
            let clod = Particle::mud_splash(&mut self.rng);
            self.list.push((slot, clod));
        }
    }

    /// One game tick: every particle moves, after the karts have puffed (retail updates the
    /// particles after the threads, so a new puff starts at its pipe). `heights` is each kart's
    /// height (`instSelf->matrix.t[1]`), which the bubbles pop at the surface by.
    pub fn tick(&mut self, heights: &[i32]) {
        let rng = &mut self.rng;
        self.list.retain_mut(|(slot, p)| {
            let alive = p.update();
            if let Some(&y) = heights.get(*slot) {
                p.surface(y, rng);
            }
            alive
        });
    }
}

/// The first pictures of the level's icon groups 7, 8, 9 and 13 (`bubble`, `bubblepop`, `drop`,
/// `mud`): the exhaust's bubble, its pop, the splash's drop and the clod of mud. The level keeps
/// them, not the racers.
pub fn water_icons(dram: &loader::dram::DramFile) -> [Option<Icon>; 4] {
    let groups = loader::icons::parse_level_groups(dram).unwrap_or_default();
    [7, 8, 9, 13].map(|id| {
        groups
            .iter()
            .find(|g| g.group_id == id)
            .and_then(|g| g.icons.first().cloned())
    })
}

/// `VEH_EMITTER_GROUND_SPARK_COUNT`
const LANDING_SPARKS: usize = 10;

/// Width of a spark's streak, in game units.
const SPARK_WIDTH: f32 = 1.2;

/// The sprites of one icon and one blend mode (the smoke, a bubble, a pop or a drop). The
/// picture is the whole texture, and its size is in texels.
#[derive(Component)]
pub struct Smoke {
    sprite: Sprite,
    mode: DrawMode,
    width: i32,
    height: i32,
}

/// The picture of an icon as a texture, and its size in texels.
pub fn icon_image(icon: &Icon, vram: &Vram) -> (Image, i32, i32) {
    icon_image_blended(icon, vram, 255)
}

/// The picture of an icon for a sprite that is blended with the picture behind it: only the
/// texels with the semi-transparency bit are (the GPU's blend is per texel), at `alpha`, and the
/// others are drawn solid.
pub fn icon_image_blended(icon: &Icon, vram: &Vram, alpha: u8) -> (Image, i32, i32) {
    let (u0, v0) = icon.layout.uv[0];
    let width = icon.layout.uv[1].0 as i32 - u0 as i32 + 1;
    let height = icon.layout.uv[2].1 as i32 - v0 as i32 + 1;
    let mut rgba = vec![0u8; width as usize * height as usize * 4];
    for y in 0..height as usize {
        for x in 0..width as usize {
            if let Ok(Some((mut c, stp))) = vram.texel_stp(
                &icon.layout,
                u0.wrapping_add(x as u8),
                v0.wrapping_add(y as u8),
            ) {
                if stp {
                    c[3] = (c[3] as u32 * alpha as u32 / 255) as u8;
                }
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

/// Most quads one mesh draws: those of one kart.
const MAX_QUADS: usize = 64;

/// The streaks of the sparks of a kart, all in one mesh.
#[derive(Component)]
pub struct SparkLines {
    /// The streaks that are drawn over the picture (the grass's), not added to it.
    solid: bool,
}

/// The meshes that draw the smoke that adds, the smoke that subtracts and the sparks of `kart`.
/// The smoke is in the kart's render layer.
pub fn spawn(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    subtract_materials: &mut Assets<SubtractMaterial>,
    icon: &Icon,
    water: &[Option<Icon>; 4],
    vram: &Vram,
    kart: Entity,
    layer: usize,
) {
    let quads = MAX_QUADS;
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
    // All of them in the kart's own layer, as in retail (`otIndexOffset` is the kart's
    // `depthBiasNormal`: they are ordered with the kart's parts and wheels, not put behind the
    // water).
    let [bubble, pop, drop, mud] = water;
    let sprites = [
        (Sprite::Smoke, Some(icon)),
        (Sprite::Bubble, bubble.as_ref()),
        (Sprite::BubblePop, pop.as_ref()),
        (Sprite::Splash, drop.as_ref()),
        (Sprite::Mud, mud.as_ref()),
    ];
    for (sprite, icon) in sprites {
        let Some(icon) = icon else { continue };
        let layer = RenderLayers::layer(layer);
        let (image, width, height) = icon_image(icon, vram);
        let image = images.add(image);
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
            Smoke {
                sprite,
                mode: DrawMode::Add,
                width,
                height,
            },
            Mesh3d(mesh()),
            MeshMaterial3d(materials.add(base(AlphaMode::Add))),
            Attached(kart),
            Transform::default(),
            layer.clone(),
            NoFrustumCulling,
        ));
        // Half the sprite and half the picture behind it (the clods of mud), on the texels that
        // are semi-transparent.
        let average = images.add(icon_image_blended(icon, vram, 128).0);
        let blended = StandardMaterial {
            base_color_texture: Some(average),
            ..base(AlphaMode::Blend)
        };
        commands.spawn((
            Smoke {
                sprite,
                mode: DrawMode::Average,
                width,
                height,
            },
            Mesh3d(mesh()),
            MeshMaterial3d(materials.add(blended)),
            Attached(kart),
            Transform::default(),
            layer.clone(),
            NoFrustumCulling,
        ));
        commands.spawn((
            Smoke {
                sprite,
                mode: DrawMode::Subtract,
                width,
                height,
            },
            Mesh3d(mesh()),
            MeshMaterial3d(subtract_materials.add(SubtractMaterial {
                base: base(AlphaMode::Blend),
                extension: subtract::Subtract {},
            })),
            Attached(kart),
            Transform::default(),
            layer,
            NoFrustumCulling,
        ));
    }
    // Sparks start at the ground and the wall and fly into them; retail has no depth buffer to
    // hide them there, so theirs are over the track for the others' karts as well.
    let spark_layer = RenderLayers::layer(crate::KART_LAYER);

    // The sparks: untextured streaks that fade out towards their tails. Those of the grass are
    // drawn over the picture, the others added to it.
    for (solid, alpha_mode) in [(false, AlphaMode::Add), (true, AlphaMode::Opaque)] {
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
            SparkLines { solid },
            Attached(kart),
            Mesh3d(meshes.add(spark_mesh)),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: Color::WHITE,
                unlit: true,
                cull_mode: None,
                double_sided: true,
                alpha_mode,
                ..default()
            })),
            Transform::default(),
            spark_layer.clone(),
            NoFrustumCulling,
        ));
    }
}

/// Lays the particles out as camera-facing quads, between the last two ticks.
pub fn update(
    smoke: Query<(&Smoke, &Attached, &Mesh3d)>,
    karts: Query<(&Transform, &KartSlot), Without<Smoke>>,
    cameras: Query<
        &Transform,
        (
            With<Camera3d>,
            Without<KartSlot>,
            Without<crate::OverlayCamera>,
            Without<crate::sky::SkyCamera>,
        ),
    >,
    particles: Res<Particles>,
    fixed: Res<Time<Fixed>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let Some(camera) = cameras.iter().next() else {
        return;
    };
    let (right, up) = (camera.rotation * Vec3::X, camera.rotation * Vec3::Y);
    let t = fixed.overstep_fraction();
    for (smoke, attached, mesh) in &smoke {
        let Ok((kart, slot)) = karts.get(attached.0) else {
            continue;
        };
        let kart = kart.translation;
        let Some(mut mesh) = meshes.get_mut(&mesh.0) else {
            continue;
        };
        let quads = MAX_QUADS;
        let mut positions = vec![[0.0f32; 3]; 4 * quads];
        let mut colors = vec![[0.0f32; 4]; 4 * quads];
        let wanted = particles
            .list
            .iter()
            .filter(|(of, p)| {
                *of == slot.0
                    && !p.is_line()
                    && p.sprite == smoke.sprite
                    && p.draw_mode() == smoke.mode
            })
            .take(quads);
        for (n, (_, p)) in wanted.enumerate() {
            let Some(corners) = p.corners(smoke.width, smoke.height) else {
                continue;
            };
            // The offsets are from the kart, in 1/256 game units; a particle is
            // `t` of the way to where its next update puts it.
            let (position, velocity) = (p.position(), p.velocity());
            let at = Vec3::from_array(std::array::from_fn(|a| {
                position[a] as f32 + velocity[a] as f32 * t
            }));
            let mut center = kart + at / 256.0 * SCALE;
            // (The mud's and the wake's height is the world's, not the kart's.)
            if p.ignores_kart_height() {
                center.y = at.y / 256.0 * SCALE;
            }
            // (A particle with its own green and blue, like the mud's, is tinted in them.)
            let gain = p.rgb().map(|c| c as f32 / 128.0);
            for (k, [x, y]) in corners.into_iter().enumerate() {
                let corner = center + right * quarter_units_to_world(x as f32)
                    - up * (quarter_units_to_world(y as f32) * SCREEN_PIXEL_ASPECT);
                positions[n * 4 + k] = corner.to_array();
                colors[n * 4 + k] = [gain[0], gain[1], gain[2], 1.0];
            }
        }
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    }
}

/// Lays the sparks out as streaks facing the camera, from a coloured head to a dark tail.
pub fn update_sparks(
    lines: Query<(&SparkLines, &Attached, &Mesh3d)>,
    karts: Query<(&Transform, &KartSlot), Without<SparkLines>>,
    cameras: Query<
        &Transform,
        (
            With<Camera3d>,
            Without<KartSlot>,
            Without<crate::OverlayCamera>,
            Without<crate::sky::SkyCamera>,
        ),
    >,
    particles: Res<Particles>,
    fixed: Res<Time<Fixed>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let Some(camera) = cameras.iter().next() else {
        return;
    };
    let t = fixed.overstep_fraction();
    for (lines, attached, mesh) in &lines {
        let Ok((kart, slot)) = karts.get(attached.0) else {
            continue;
        };
        let kart = kart.translation;
        let Some(mut mesh) = meshes.get_mut(&mesh.0) else {
            continue;
        };
        let quads = MAX_QUADS;
        let mut positions = vec![[0.0f32; 3]; 4 * quads];
        let mut colors = vec![[0.0f32; 4]; 4 * quads];
        let wanted = particles
            .list
            .iter()
            .filter(|(of, p)| *of == slot.0 && p.is_line() && p.semi_transparent() != lines.solid)
            .take(quads);
        for (n, (_, p)) in wanted.enumerate() {
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
            // A line that does not fade keeps its colour to the tail.
            let tail_shade = if p.line_fades() { 0.0 } else { 1.0 };
            for (k, (point, shade)) in [
                (head + side, 1.0),
                (head - side, 1.0),
                (tail + side, tail_shade),
                (tail - side, tail_shade),
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
