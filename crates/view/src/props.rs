//! The models the level places (`struct InstDef`): mushrooms, crates, fruit, the start banner.
//!
//! Each is posed from its definition's position, angles (`ConvertRotToMatrix`) and scale. A
//! model with an animation plays its first one over and over at the game tick. The ones that
//! move by the game's code (the armadillos, the minecarts, the snowball and the barrels) are
//! `hazard::Hazards`: that is ticked here, and these only show it. What the game moves by other
//! code (spinning fruit, bobbing crates, anything that reacts) stands still.

use std::collections::HashMap;

use bevy::prelude::*;
use hazard::Hazards;
use loader::dram::DramFile;
use loader::lev::Level;
use loader::model::{Model, ModelHeader};
use loader::vram::Vram;
use render::mesh::{self, PARTS, SCALE, to_world};

use crate::drive::animation_pose;
use crate::subtract::SubtractMaterial;
use crate::{GameTick, spawn_parts};

/// The state of the level's hazards, and what the last tick of them did.
#[derive(Resource)]
pub struct Sim {
    pub hazards: Hazards,
    pub last: hazard::Tick,
}

impl Sim {
    pub fn new(hazards: Hazards) -> Sim {
        Sim { hazards, last: hazard::Tick::default() }
    }
}

/// One game tick of the hazards.
pub fn tick_hazards(mut sim: ResMut<Sim>) {
    sim.last = sim.hazards.tick();
}

/// A placed model whose pose changes with the tick.
struct Animated {
    header: ModelHeader,
    inst_scale: [i16; 3],
    parts: [Option<Handle<Mesh>>; PARTS],
    shown: Option<i16>,
}

#[derive(Resource, Default)]
pub struct Props(Vec<Animated>);

/// What a hazard shows of itself: its mesh, and where it was and is between ticks.
struct Shown {
    /// Its index among the hazards of its kind.
    index: usize,
    previous: (Vec3, Quat),
    current: (Vec3, Quat),
    header: ModelHeader,
    inst_scale: [i16; 3],
    parts: [Option<Handle<Mesh>>; PARTS],
    /// The animation and frame the mesh has.
    shown: Option<(u8, i16)>,
}

impl Shown {
    /// Takes the pose of the animation `(index, frame)` if the mesh does not have it yet.
    fn pose(&mut self, animation: (u8, i16), meshes: &mut Assets<Mesh>) {
        if self.shown == Some(animation) {
            return;
        }
        self.shown = Some(animation);
        let Some(pose) = animation_pose(&self.header, animation.0, animation.1) else { return };
        let positions = mesh::model_positions(&self.header, &pose, self.inst_scale);
        for (handle, positions) in self.parts.iter().zip(positions) {
            let Some(mut mesh) = handle.as_ref().and_then(|h| meshes.get_mut(h)) else { continue };
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        }
    }

    fn moved(&mut self, pos: [i32; 3], matrix: &vehicle::Matrix) {
        self.previous = self.current;
        self.current = (Vec3::new(pos[0] as f32, pos[1] as f32, pos[2] as f32) * SCALE, quat_of(matrix));
    }
}

#[derive(Resource, Default)]
pub struct Armadillos(Vec<Shown>);

#[derive(Resource, Default)]
pub struct Minecarts(Vec<Shown>);

/// The turtles stay where they are placed; only their animation is the sim's.
#[derive(Resource, Default)]
pub struct Turtles(Vec<Shown>);

/// The entity of a placed armadillo or minecart (its index in [`Armadillos`] or [`Minecarts`]).
#[derive(Component)]
pub struct ArmadilloBody(usize);

#[derive(Component)]
pub struct MinecartBody(usize);

/// A placed wumpa fruit (its index among `Hazards::fruit`).
#[derive(Component)]
pub struct FruitBody(usize);

/// Constant depth bias (see `DEPTH_BIAS_PER_ORDER`) that keeps a crate in front of the ground it sinks into.
const CRATE_DEPTH_BIAS: f32 = 4_000_000.0;

/// A placed crate (its index among `Hazards::crates`).
#[derive(Component)]
pub struct CrateBody(pub usize);

/// Sizes the crates as the sim says: gone when hit, growing back after a while.
pub fn pose_crates(sim: Res<Sim>, mut crates: Query<(&CrateBody, &mut Transform, &mut Visibility)>) {
    for (body, mut transform, mut visibility) in &mut crates {
        let scale = sim.hazards.crates[body.0].sim.scale;
        let want = if scale == 0 { Visibility::Hidden } else { Visibility::Inherited };
        if *visibility != want {
            *visibility = want;
        }
        if scale != 0 {
            transform.scale = Vec3::splat(scale as f32 / hazard::crates::FULL_SCALE as f32);
        }
    }
}

/// Hides the fruit that were picked up.
pub fn hide_fruit(sim: Res<Sim>, mut fruit: Query<(&FruitBody, &mut Visibility)>) {
    for (body, mut visibility) in &mut fruit {
        let collected = sim.hazards.fruit[body.0].sim.collected;
        let want = if collected { Visibility::Hidden } else { Visibility::Inherited };
        if *visibility != want {
            *visibility = want;
        }
    }
}

/// A placed snowball or barrel (its index among `Hazards::rollers`).
#[derive(Component)]
pub struct Rolling(usize);

/// The orientation of an angle triple, as the matrix `ConvertRotToMatrix` makes of it: its
/// columns are where the model's axes go (as for the kart).
fn rotation_of(gte: &mut gte::Gte, rot: loader::lev::SVec3) -> Quat {
    quat_of(&vehicle::rotmat::convert_rot_to_matrix(gte, rot))
}

fn quat_of(matrix: &vehicle::Matrix) -> Quat {
    let column = |c: usize| Vec3::new(matrix.m[0][c] as f32, matrix.m[1][c] as f32, matrix.m[2][c] as f32) / 4096.0;
    Quat::from_mat3(&Mat3::from_cols(column(0), column(1), column(2)))
}

/// The scale an object's init sets on its instance, replacing the level file's (`RB_Plant_Init`,
/// `RB_CtrLetter_Init`, `RB_Snowball_LInB` which the barrels share, `RB_Minecart_LInB`); the other
/// placed models keep theirs. By model name.
fn spawn_scale(model: &str) -> Option<i16> {
    match model {
        "plant" => Some(0x2800),
        "c" | "t" | "r" => Some(0x1800),
        "snowball" | "barrel" | "minecart" => Some(0x1000),
        _ => None,
    }
}

/// The pose of a model at rest: its first animation's first frame, or its only frame.
fn rest_pose(header: &ModelHeader) -> Option<&loader::model::Frame> {
    header.animations.first().and_then(|a| a.frames.first()).or(header.static_frame.as_ref())
}

/// The entity a hazard's model hangs off, which the systems below move; `marker` says which.
fn body(commands: &mut Commands, transform: Transform, marker: impl Bundle) -> Entity {
    commands.spawn((marker, transform, Visibility::default())).id()
}

/// Which hazard of which kind moves an instance.
enum Hazard {
    Armadillo(usize),
    Minecart(usize),
    Roller(usize),
    Turtle(usize),
    Fruit(usize),
    Crate(usize),
    /// Only its sparks show (`fire`).
    Fireball,
}

pub fn spawn(
    level: &Level,
    hazards: &Hazards,
    dram: &DramFile,
    vram: &Vram,
    commands: &mut Commands,
    images: &mut Assets<Image>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    subtract_materials: &mut Assets<SubtractMaterial>,
) {
    let mut by_instance: HashMap<usize, Hazard> = HashMap::new();
    by_instance.extend(hazards.armadillos.iter().enumerate().map(|(i, h)| (h.instance, Hazard::Armadillo(i))));
    by_instance.extend(hazards.minecarts.iter().enumerate().map(|(i, h)| (h.instance, Hazard::Minecart(i))));
    by_instance.extend(hazards.rollers.iter().enumerate().map(|(i, h)| (h.instance, Hazard::Roller(i))));
    by_instance.extend(hazards.crates.iter().enumerate().map(|(i, h)| (h.instance, Hazard::Crate(i))));
    by_instance.extend(hazards.fruit.iter().enumerate().map(|(i, h)| (h.instance, Hazard::Fruit(i))));
    by_instance.extend(hazards.turtles.iter().enumerate().map(|(i, h)| (h.instance, Hazard::Turtle(i))));
    by_instance.extend(hazards.fireballs.iter().map(|h| (h.instance, Hazard::Fireball)));

    let mut props = Props::default();
    let mut armadillos = Armadillos::default();
    let mut minecarts = Minecarts::default();
    let mut turtles = Turtles::default();
    let mut models: HashMap<u32, Option<Model>> = Default::default();
    let mut gte = gte::Gte::default();
    for (n, inst) in level.instances.iter().enumerate().filter(|(_, i)| i.model != 0) {
        // The fireballs hide their model (`HIDE_MODEL`); see `fire`.
        if matches!(by_instance.get(&n), Some(Hazard::Fireball)) {
            continue;
        }
        let model = models.entry(inst.model).or_insert_with(|| Model::parse_at(dram, inst.model).ok());
        let Some(header) = model.as_ref().and_then(|m| m.headers.first()) else { continue };
        let Some(frame) = rest_pose(header) else { continue };
        let inst_scale = match spawn_scale(&inst.model_name) {
            Some(scale) => [scale; 3],
            None => [inst.scale.x, inst.scale.y, inst.scale.z],
        };
        let data = mesh::model_mesh(header, frame, vram, inst_scale);

        let mut atlas = Image::new(
            bevy::render::render_resource::Extent3d {
                width: data.atlas.width as u32,
                height: data.atlas.height as u32,
                depth_or_array_layers: 1,
            },
            bevy::render::render_resource::TextureDimension::D2,
            data.atlas.rgba,
            bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
            bevy::asset::RenderAssetUsages::RENDER_WORLD,
        );
        atlas.sampler = bevy::image::ImageSampler::nearest();
        let base = StandardMaterial {
            base_color: Color::WHITE,
            base_color_texture: Some(images.add(atlas)),
            unlit: true,
            cull_mode: None,
            double_sided: true,
            ..default()
        };

        let rotation = rotation_of(&mut gte, inst.rot);
        let transform = Transform::from_translation(Vec3::from(to_world(inst.pos))).with_rotation(rotation);
        let pose = (transform.translation, transform.rotation);

        // A hazard's model hangs off an entity that the systems below move.
        let shown = |index| Shown {
            index,
            previous: pose,
            current: pose,
            header: header.clone(),
            inst_scale,
            parts: Default::default(),
            shown: None,
        };
        let (transform, parent) = match by_instance.get(&n) {
            Some(&Hazard::Roller(i)) => (Transform::default(), Some(body(commands, transform, Rolling(i)))),
            // A fruit's model hangs off a body that is hidden once it is picked up.
            Some(&Hazard::Fruit(i)) => (Transform::default(), Some(body(commands, transform, FruitBody(i)))),
            // A crate's model hangs off a body that shrinks when it is hit and grows back.
            Some(&Hazard::Crate(i)) => (Transform::default(), Some(body(commands, transform, CrateBody(i)))),
            Some(&Hazard::Armadillo(i)) => {
                let parent = body(commands, transform, ArmadilloBody(i));
                let mut s = shown(i);
                s.parts = spawn_parts(data.parts, &base, Transform::default(), Some(parent), commands, meshes, materials, subtract_materials, 0);
                armadillos.0.push(s);
                continue;
            }
            Some(&Hazard::Minecart(i)) => {
                let parent = body(commands, transform, MinecartBody(i));
                let mut s = shown(i);
                s.parts = spawn_parts(data.parts, &base, Transform::default(), Some(parent), commands, meshes, materials, subtract_materials, 0);
                minecarts.0.push(s);
                continue;
            }
            Some(&Hazard::Turtle(i)) => {
                let mut s = shown(i);
                s.parts = spawn_parts(data.parts, &base, transform, None, commands, meshes, materials, subtract_materials, 0);
                turtles.0.push(s);
                continue;
            }
            _ => (transform, None),
        };
        // The level places crates at whatever height, so they sink into the ground; pull them
        // toward the camera so the ground does not cut them.
        let crate_base;
        let base = if matches!(by_instance.get(&n), Some(Hazard::Crate(_))) {
            crate_base = StandardMaterial { depth_bias: CRATE_DEPTH_BIAS, ..base.clone() };
            &crate_base
        } else {
            &base
        };
        let parts = spawn_parts(data.parts, base, transform, parent, commands, meshes, materials, subtract_materials, 0);
        if header.animations.first().is_some_and(|a| a.frames.len() > 1) {
            props.0.push(Animated { header: header.clone(), inst_scale, parts, shown: Some(0) });
        }
    }
    info!(
        "{} placed models, {} animated, {} armadillos, {} minecarts, {} rollers, {} turtles",
        level.instances.len(),
        props.0.len(),
        armadillos.0.len(),
        minecarts.0.len(),
        hazards.rollers.len(),
        turtles.0.len()
    );
    commands.insert_resource(props);
    commands.insert_resource(armadillos);
    commands.insert_resource(minecarts);
    commands.insert_resource(turtles);
}

/// After a tick of the hazards: the armadillos and the minecarts take their new place and the
/// pose of the animation they play.
pub fn pose_hazards(
    sim: Res<Sim>,
    mut armadillos: ResMut<Armadillos>,
    mut minecarts: ResMut<Minecarts>,
    mut turtles: ResMut<Turtles>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    for t in &mut turtles.0 {
        t.pose((0, sim.hazards.turtles[t.index].sim.anim_frame), &mut meshes);
    }
    for a in &mut armadillos.0 {
        let h = &sim.hazards.armadillos[a.index].sim;
        a.moved(h.pos, &h.matrix());
        a.pose((h.anim_index, h.anim_frame), &mut meshes);
    }
    for c in &mut minecarts.0 {
        let h = &sim.hazards.minecarts[c.index].sim;
        c.moved(h.pos, &h.matrix());
        c.pose((0, h.anim_frame), &mut meshes);
    }
}

/// Puts the armadillos and the minecarts between their last two ticks.
pub fn place_hazards(
    mut armadillos: Query<(&ArmadilloBody, &mut Transform), Without<MinecartBody>>,
    mut minecarts: Query<(&MinecartBody, &mut Transform), Without<ArmadilloBody>>,
    shown: (Res<Armadillos>, Res<Minecarts>),
    fixed: Res<Time<Fixed>>,
) {
    let t = fixed.overstep_fraction();
    let between = |s: &Shown, transform: &mut Transform| {
        // The path of a minecart restarts with a jump, which must not be smoothed over.
        let jump = s.previous.0.distance(s.current.0) > 5.0;
        let from = if jump { s.current } else { s.previous };
        transform.translation = from.0.lerp(s.current.0, t);
        transform.rotation = from.1.slerp(s.current.1, t);
    };
    for (body, mut transform) in &mut armadillos {
        between(&shown.0.0[body.0], &mut transform);
    }
    for (body, mut transform) in &mut minecarts {
        between(&shown.1.0[body.0], &mut transform);
    }
}

/// Puts the rolling objects between the point of their path they are at and the next.
pub fn roll(mut rolling: Query<(&Rolling, &mut Transform)>, sim: Res<Sim>, fixed: Res<Time<Fixed>>) {
    let mut gte = gte::Gte::default();
    let fraction = fixed.overstep_fraction();
    for (rolling, mut transform) in &mut rolling {
        let (from, to) = sim.hazards.roller_frames(rolling.0);
        let mut place = |p: loader::lev::PosRot| (Vec3::from(to_world(p.pos)), rotation_of(&mut gte, p.rot));
        let ((from_pos, from_rot), (to_pos, to_rot)) = (place(from), place(to));
        transform.translation = from_pos.lerp(to_pos, fraction);
        transform.rotation = from_rot.slerp(to_rot, fraction);
    }
}

/// Plays the placed models' first animation, once per game tick.
pub fn animate(mut props: ResMut<Props>, tick: Res<GameTick>, mut meshes: ResMut<Assets<Mesh>>) {
    for prop in &mut props.0 {
        let Some(animation) = prop.header.animations.first() else { continue };
        let frame = (tick.0 % animation.num_frames.max(1) as u32) as i16;
        if prop.shown == Some(frame) {
            continue;
        }
        prop.shown = Some(frame);
        let Some(pose) = animation_pose(&prop.header, 0, frame) else { continue };
        let positions = mesh::model_positions(&prop.header, &pose, prop.inst_scale);
        for (handle, positions) in prop.parts.iter().zip(positions) {
            let Some(mut mesh) = handle.as_ref().and_then(|h| meshes.get_mut(h)) else { continue };
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        }
    }
}
