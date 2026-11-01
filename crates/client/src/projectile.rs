//! The bombs, missiles and mines in play, and the bursts they leave, as Bevy entities. Where a
//! weapon is, which way it faces and how big it is, is `weapon`'s (`Projectile`, `Mine`); the
//! models are `render::projectile`'s.
//!
//! A bomb rolls along; a missile loops its `thrust` animation; a TNT or Nitro crate is the crate
//! model, which grows where it was dropped and squashes and stretches on a head; a beaker bubbles
//! where it lies, and is flown as it is when thrown. When a bomb or a
//! missile goes off (`RB_Burst_Init`) three models are born under it, all turned to the camera: two
//! copies of the burst, one turned a quarter on the screen, and a shockwave ring. When a mine does
//! (`RB_Blowup_Init`) it is the break-up of a crate, tinted, and a shockwave of the same colour,
//! turned to the camera; a mine set off by another one only breaks up (`RB_Explosion_InitGeneric`).
//! A beaker that shatters (`RB_Explosion_InitPotion`) is the red or green shockwave, half size.
//! Each plays its animation a frame a tick and is gone at the end of it.
//!
//! Not shown: the missile's exhaust, the afterimage that follows a dropped mine, and (in split screen, which there is none of) the
//! smaller burst.

use ::weapon::Kind;
use ::weapon::mine::{MineKind, State};
use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::RenderLayers;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, Face, TextureDimension, TextureFormat};
use loader::model::{Frame, Model, ModelHeader};
use render::explosion::{Playback, tint};
use render::mesh::{self, MeshPart, PARTS, SCALE};
use render::projectile::{BURST_DROP, Models, NITRO_COLOR, TNT_COLOR, loop_frame};

use crate::drive::{ChaseCamera, animation_pose, rotation_of};
use crate::props::Sim;
use crate::subtract::SubtractMaterial;
use crate::{DEPTH_BIAS_PER_ORDER, KART_LAYER, Textures, spawn_parts};

/// The models, and what is made of each once: the faces of its first pose and their texture.
#[derive(Resource)]
pub struct ProjectileModels {
    models: Models,
    built: Option<[Built; 13]>,
    /// The level's wake, if it has one (`None`: not looked for yet).
    wake: Option<Option<Built>>,
}

impl ProjectileModels {
    pub fn new(models: Models) -> ProjectileModels {
        ProjectileModels {
            models,
            built: None,
            wake: None,
        }
    }
}

/// A model made into meshes' worth of faces.
struct Built {
    header: ModelHeader,
    parts: [MeshPart; PARTS],
    atlas: Handle<Image>,
    /// Drawn with back faces culled, for a camera inside it to see through.
    convex: bool,
}

/// The order of [`ProjectileModels::built`].
const BOMB: usize = 0;
const ROCKET: usize = 1;
const BURST: usize = 2;
const SHOCKWAVE: usize = 3;
const TNT: usize = 4;
const NITRO: usize = 5;
const EXPLOSION: usize = 6;
const SHOCKWAVE_GREEN: usize = 7;
const BEAKER_GREEN: usize = 8;
const BEAKER_RED: usize = 9;
const SHIELD_BLUE: usize = 10;
const SHIELD_GREEN: usize = 11;
const CLOUD: usize = 12;
const WAKE: usize = 13;

/// A shield bomb's `scale` and `alphaScale` (0x400 each, `RB_ShieldDark_ThTick_Grow`): 0x1000 is
/// the model's size, and an alpha at which it is gone.
const SHIELD_BOMB_SCALE: f32 = 0x400 as f32 / 4096.0;
const SHIELD_BOMB_ALPHA: f32 = 1.0 - 0x400 as f32 / 4096.0;

/// The pose of a model at `frame`; a model with no animation has its one.
fn pose(header: &ModelHeader, frame: i16) -> Option<Frame> {
    animation_pose(header, 0, frame).or_else(|| header.static_frame.clone())
}

fn build(
    model: &Model,
    convex: bool,
    textures: &Textures,
    images: &mut Assets<Image>,
) -> Option<Built> {
    let header = model.headers.first()?;
    let mut data = mesh::model_mesh(header, &pose(header, 0)?, &textures.vram, [0x1000; 3]);
    if convex {
        mesh::wind_outward(&mut data.parts);
    }
    let mut atlas = Image::new(
        Extent3d {
            width: data.atlas.width as u32,
            height: data.atlas.height as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data.atlas.rgba,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    atlas.sampler = ImageSampler::nearest();
    Some(Built {
        header: header.clone(),
        parts: data.parts,
        atlas: images.add(atlas),
        convex,
    })
}

/// Where a body is, and how big.
#[derive(Clone, Copy)]
struct Placed {
    translation: Vec3,
    rotation: Quat,
    scale: Vec3,
}

/// A weapon in play: its entity follows the sim's by `id`.
#[derive(Component)]
pub struct Body {
    id: u32,
    /// The render layer of its parts.
    layer: usize,
    /// The frame of the animation, for a model that loops one.
    frame: i16,
    /// Turned to the camera each frame (the shield bomb).
    billboard: bool,
    header: ModelHeader,
    parts: [Option<Handle<Mesh>>; PARTS],
    /// Where it was and where it is, at the last two ticks.
    previous: Placed,
    current: Placed,
}

/// Which way an effect faces.
#[derive(Clone, Copy)]
enum Facing {
    /// Turned to the camera (`DRAW_BILLBOARD`).
    Camera,
    /// Turned to the camera and a quarter on the screen.
    CameraQuarter,
    Fixed(Quat),
}

/// One of the models of a burst or a blowup.
#[derive(Component)]
pub struct Effect {
    header: ModelHeader,
    parts: [Option<Handle<Mesh>>; PARTS],
    playback: Playback,
    facing: Facing,
    /// The explosion of a bomb or a missile, which flashes a screen it is close to.
    flashes: bool,
}

/// How far an effect is drawn in front of what is at its depth (`depthBiasNormal -= 2`).
const EFFECT_ORDER: f32 = 2.0;

fn spawn_model(
    built: &Built,
    transform: Transform,
    color: Color,
    order: f32,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    subtract_materials: &mut Assets<SubtractMaterial>,
    layer: usize,
) -> (Entity, [Option<Handle<Mesh>>; PARTS]) {
    let base = StandardMaterial {
        base_color: color,
        base_color_texture: Some(built.atlas.clone()),
        unlit: true,
        cull_mode: built.convex.then_some(Face::Back),
        double_sided: !built.convex,
        depth_bias: order * DEPTH_BIAS_PER_ORDER,
        ..default()
    };
    let entity = commands.spawn((transform, Visibility::default())).id();
    let parts = spawn_parts(
        built.parts.clone(),
        &base,
        Transform::default(),
        Some(entity),
        commands,
        meshes,
        materials,
        subtract_materials,
        layer,
    );
    (entity, parts)
}

/// Puts a model into `frame`, in the meshes `parts`.
fn set_frame(
    header: &ModelHeader,
    parts: &[Option<Handle<Mesh>>; PARTS],
    frame: i16,
    meshes: &mut Assets<Mesh>,
) {
    let Some(pose) = pose(header, frame) else {
        return;
    };
    let positions = mesh::model_positions(header, &pose, [0x1000; 3]);
    for (handle, positions) in parts.iter().zip(positions) {
        let Some(mut mesh) = handle.as_ref().and_then(|h| meshes.get_mut(h)) else {
            continue;
        };
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    }
}

fn world(pos: [i32; 3]) -> Vec3 {
    Vec3::new(pos[0] as f32, pos[1] as f32, pos[2] as f32) * SCALE
}

/// A weapon the sim has: which model, and where.
struct Wanted {
    id: u32,
    model: usize,
    placed: Placed,
    /// The model loops its animation.
    animated: bool,
    /// Turned to the camera, and tinted.
    billboard: bool,
    color: Color,
    /// The render layer: [`KART_LAYER`] for a crate on the head of our kart, so that it is ordered
    /// with the kart by depth instead of being hidden or shown by the track.
    layer: usize,
}

fn wanted(sim: &Sim) -> Vec<Wanted> {
    let projectiles = sim.race.weapons.projectiles().iter().map(|p| Wanted {
        id: p.id,
        model: match p.kind {
            Kind::Bomb => BOMB,
            Kind::Missile => ROCKET,
            Kind::Shield if p.blue() => SHIELD_BLUE,
            Kind::Shield => SHIELD_GREEN,
        },
        placed: Placed {
            translation: world(p.pos),
            rotation: rotation_of(&p.matrix()),
            scale: if p.kind == Kind::Shield {
                Vec3::splat(SHIELD_BOMB_SCALE)
            } else {
                Vec3::ONE
            },
        },
        animated: p.kind == Kind::Missile,
        billboard: p.kind == Kind::Shield,
        color: if p.kind == Kind::Shield {
            Color::linear_rgba(
                SHIELD_BOMB_ALPHA,
                SHIELD_BOMB_ALPHA,
                SHIELD_BOMB_ALPHA,
                SHIELD_BOMB_ALPHA,
            )
        } else {
            Color::WHITE
        },
        layer: 0,
    });
    let mines = sim.race.weapons.mines().iter().map(|m| Wanted {
        id: m.id,
        model: match m.kind {
            MineKind::Tnt => TNT,
            MineKind::Nitro => NITRO,
            MineKind::GreenBeaker => BEAKER_GREEN,
            MineKind::RedBeaker => BEAKER_RED,
        },
        placed: Placed {
            translation: world(m.pos),
            rotation: rotation_of(&m.rot),
            scale: Vec3::new(m.scale_xz as f32, m.scale_y as f32, m.scale_xz as f32) / 4096.0,
        },
        // A beaker bubbles where it lies, not in the air.
        animated: m.kind.is_beaker() && m.state == State::Ground,
        billboard: false,
        color: Color::WHITE,
        layer: match m.state {
            State::Landing { target, .. } | State::Sitting { target, .. } if target == sim.own => {
                KART_LAYER
            }
            _ => 0,
        },
    });
    // The red beakers' rain clouds, over the karts they hit (and shrinking away). A cloud is told
    // from the weapons' ids by the top bit, and from the others by its kart and its birth.
    let clouds = sim.race.karts.iter().enumerate().flat_map(|(slot, kart)| {
        let d = &kart.driver;
        d.cloud
            .iter()
            .map(|c| (c, true))
            .chain(d.fading_clouds.iter().map(|c| (c, false)))
            .map(move |(c, active)| Wanted {
                id: 0x8000_0000 | (slot as u32) << 24 | (c.born & 0xff_ffff),
                model: CLOUD,
                placed: Placed {
                    translation: world(c.pos),
                    rotation: Quat::IDENTITY,
                    scale: Vec3::new(
                        c.scale[0].max(0) as f32,
                        c.scale[1].max(0) as f32,
                        c.scale[2].max(0) as f32,
                    ) / 4096.0,
                },
                animated: active,
                billboard: true,
                color: Color::WHITE,
                layer: 0,
            })
    });
    // The wake of a kart in the water: on the water line under it, turned with its heading. It
    // is drawn with the kart (ours over the track, like the kart), and told from the others by
    // the next bit under the clouds'.
    let wakes = sim
        .race
        .karts
        .iter()
        .enumerate()
        .filter(|(_, kart)| kart.driver.wake_scale != 0)
        .map(|(slot, kart)| {
            let d = &kart.driver;
            let (sin, cos) = (d.angle as f32 * std::f32::consts::TAU / 4096.0).sin_cos();
            let fixed = |v: f32| (v * 4096.0).round() as i16;
            let heading = vehicle::Matrix {
                m: [
                    [fixed(cos), 0, fixed(sin)],
                    [0, 0x1000, 0],
                    [fixed(-sin), 0, fixed(cos)],
                ],
            };
            Wanted {
                id: 0x4000_0000 | slot as u32,
                model: WAKE,
                placed: Placed {
                    translation: world([d.inst_pos[0], vehicle::force::WATERLINE_Y, d.inst_pos[2]]),
                    rotation: rotation_of(&heading),
                    // `wakeScale` is the model's own size at 0x1000.
                    scale: Vec3::new(d.wake_scale as f32, 4096.0, d.wake_scale as f32) / 4096.0,
                },
                animated: true,
                billboard: false,
                color: Color::WHITE,
                layer: if slot == sim.own { KART_LAYER } else { 0 },
            }
        });
    projectiles
        .chain(mines)
        .chain(clouds)
        .chain(wakes)
        .collect()
}

/// One game tick: the weapons the sim has are followed (born, moved, ended), the bursts it made
/// begin, and the bursts there are play a frame.
pub fn tick(
    sim: Res<Sim>,
    mut models: ResMut<ProjectileModels>,
    textures: Res<Textures>,
    track: Res<crate::Track>,
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut subtract_materials: ResMut<Assets<SubtractMaterial>>,
    mut bodies: Query<(Entity, &mut Body, Option<&Children>)>,
    mut effects: Query<(Entity, &mut Effect, &Transform)>,
    cameras: Query<&Transform, (With<ChaseCamera>, Without<Body>, Without<Effect>)>,
    mut flash: ResMut<crate::flash::Flash>,
    mut last: Local<Option<u32>>,
) {
    // Once per tick of the sim, however often the system is scheduled.
    let tick = sim.race.tick;
    let previous = last.replace(tick);
    if previous == Some(tick) {
        return;
    }
    // The race began again: nothing of the old one is left.
    let restarted = previous.is_some_and(|p| tick < p);
    if restarted {
        for (entity, ..) in &bodies {
            commands.entity(entity).despawn();
        }
        for (entity, ..) in &effects {
            commands.entity(entity).despawn();
        }
    }

    if models.built.is_none() {
        let m = &models.models;
        let built = [
            (&m.bomb, false),
            (&m.rocket, false),
            (&m.burst, false),
            (&m.shockwave, false),
            (&m.tnt, true),
            (&m.nitro, true),
            (&m.crate_explosion, false),
            (&m.shockwave_green, false),
            (&m.beaker_green, false),
            (&m.beaker_red, false),
            (&m.shield_blue, false),
            (&m.shield_green, false),
            (&m.cloud, false),
        ]
        .map(|(model, convex)| build(model, convex, &textures, &mut images));
        let Some(built) = built.into_iter().collect::<Option<Vec<_>>>() else {
            return;
        };
        models.built = built.try_into().ok();
    }
    // The wake is the level's own, and not every level has one (retail births none then).
    if track.is_changed() {
        models.wake = None;
    }
    if models.wake.is_none() {
        let wake = track
            .0
            .models
            .iter()
            .filter_map(|&at| loader::model::Model::parse_at(&textures.dram, at).ok())
            .find(|m| m.id == render::projectile::WAKE);
        models.wake = Some(wake.and_then(|m| build(&m, false, &textures, &mut images)));
    }
    let built = models.built.as_ref().unwrap();

    // The bursts there are: the next frame, until the animation is over.
    if !restarted {
        let view = cameras.single().ok().map(|c| c.to_matrix().inverse());
        for (entity, mut effect, transform) in &mut effects {
            if !effect.playback.advance() {
                commands.entity(entity).despawn();
                continue;
            }
            if effect.flashes
                && effect.playback.frame == 1
                && let Some(view) = view
            {
                flash.proximity(view.transform_point3(transform.translation) / SCALE);
            }
            set_frame(
                &effect.header,
                &effect.parts,
                effect.playback.frame,
                &mut meshes,
            );
        }
    }

    // The bursts and the blowups of this tick.
    let mut spawn_effect = |which: usize, at: Vec3, facing: Facing, color: Color, scale: f32| {
        let model = &built[which];
        let (entity, parts) = spawn_model(
            model,
            Transform::from_translation(at).with_scale(Vec3::splat(scale)),
            color,
            EFFECT_ORDER,
            &mut commands,
            &mut meshes,
            &mut materials,
            &mut subtract_materials,
            0,
        );
        let frames = model
            .header
            .animations
            .first()
            .map_or(0, |a| a.num_frames as i16);
        commands.entity(entity).insert(Effect {
            header: model.header.clone(),
            parts,
            playback: Playback::new(frames),
            facing,
            flashes: which == BURST && matches!(facing, Facing::Camera),
        });
    };
    for explosion in &sim.race.last_weapons.explosions {
        let at = world([
            explosion.pos[0],
            explosion.pos[1] - BURST_DROP,
            explosion.pos[2],
        ]);
        spawn_effect(BURST, at, Facing::Camera, Color::WHITE, 1.0);
        spawn_effect(BURST, at, Facing::CameraQuarter, Color::WHITE, 1.0);
        spawn_effect(SHOCKWAVE, at, Facing::Camera, Color::WHITE, 1.0);
    }
    for detonation in &sim.race.last_weapons.detonations {
        let at = world(detonation.pos);
        // A beaker only shatters (`RB_Explosion_InitPotion`): its shockwave, half size.
        let shatter = match detonation.kind {
            MineKind::GreenBeaker => Some(SHOCKWAVE_GREEN),
            MineKind::RedBeaker => Some(SHOCKWAVE),
            _ => None,
        };
        if let Some(shockwave) = shatter {
            spawn_effect(shockwave, at, Facing::Camera, Color::WHITE, 0.5);
            continue;
        }
        let [r, g, b] = tint(match detonation.kind {
            MineKind::Nitro => NITRO_COLOR,
            _ => TNT_COLOR,
        });
        let color = Color::srgb_u8(r, g, b);
        if detonation.blowup {
            spawn_effect(EXPLOSION, at, Facing::Camera, color, 1.0);
            let shockwave = match detonation.kind {
                MineKind::Nitro => SHOCKWAVE_GREEN,
                _ => SHOCKWAVE,
            };
            spawn_effect(shockwave, at, Facing::Camera, Color::WHITE, 1.0);
        } else {
            let facing = Facing::Fixed(rotation_of(&detonation.rot));
            spawn_effect(EXPLOSION, at, facing, color, 1.0);
        }
    }

    // The weapons: the ones the entities have, and the ones the sim has.
    let wanted = wanted(&sim);
    let mut known = Vec::new();
    for (entity, mut body, children) in &mut bodies {
        if restarted {
            break;
        }
        let Some(w) = wanted.iter().find(|w| w.id == body.id) else {
            commands.entity(entity).despawn();
            continue;
        };
        known.push(w.id);
        body.previous = body.current;
        body.current = w.placed;
        if body.layer != w.layer {
            body.layer = w.layer;
            for &child in children.into_iter().flatten() {
                commands.entity(child).insert(RenderLayers::layer(w.layer));
            }
        }
        if w.animated {
            let frames = body
                .header
                .animations
                .first()
                .map_or(0, |a| a.num_frames as i16);
            body.frame = loop_frame(body.frame, frames);
            set_frame(&body.header, &body.parts, body.frame, &mut meshes);
        }
    }
    for w in wanted.iter().filter(|w| !known.contains(&w.id)) {
        let model = if w.model == WAKE {
            let Some(wake) = models.wake.as_ref().and_then(Option::as_ref) else {
                continue;
            };
            wake
        } else {
            &built[w.model]
        };
        let (entity, parts) = spawn_model(
            model,
            Transform {
                translation: w.placed.translation,
                rotation: w.placed.rotation,
                scale: w.placed.scale,
            },
            w.color,
            0.0,
            &mut commands,
            &mut meshes,
            &mut materials,
            &mut subtract_materials,
            w.layer,
        );
        commands.entity(entity).insert(Body {
            id: w.id,
            layer: w.layer,
            frame: 0,
            billboard: w.billboard,
            header: model.header.clone(),
            parts,
            previous: w.placed,
            current: w.placed,
        });
    }
}

/// Puts the weapons between the last two ticks, and turns the bursts to the camera.
pub fn place(
    fixed: Res<Time<Fixed>>,
    cameras: Query<&Transform, (With<ChaseCamera>, Without<Body>, Without<Effect>)>,
    mut bodies: Query<(&Body, &mut Transform), Without<Effect>>,
    mut effects: Query<(&Effect, &mut Transform), Without<Body>>,
) {
    let t = fixed.overstep_fraction();
    for (body, mut transform) in &mut bodies {
        let (a, b) = (body.previous, body.current);
        transform.translation = a.translation.lerp(b.translation, t);
        transform.rotation = a.rotation.slerp(b.rotation, t);
        transform.scale = a.scale.lerp(b.scale, t);
    }
    let camera = cameras.single().ok().map(|c| c.rotation);
    for (body, mut transform) in &mut bodies {
        if let (true, Some(camera)) = (body.billboard, camera) {
            transform.rotation = camera;
        }
    }
    for (effect, mut transform) in &mut effects {
        transform.rotation = match (effect.facing, camera) {
            (Facing::Fixed(rotation), _) => rotation,
            (Facing::Camera, Some(camera)) => camera,
            (Facing::CameraQuarter, Some(camera)) => {
                camera * Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)
            }
            _ => continue,
        };
    }
}
