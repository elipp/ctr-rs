//! The models the level places (`struct InstDef`): mushrooms, crates, fruit, the start banner.
//!
//! The snowball and the barrels roll along a path recorded in the level file, and the armadillos
//! and the minecarts run `hazard::armadillo` and `hazard::minecart`.
//!
//! Each is posed from its definition's position, angles (`ConvertRotToMatrix`) and scale. A
//! model with an animation plays its first one over and over at the game tick; the objects the
//! game moves by code (spinning fruit, bobbing crates, anything that reacts) stand still here.

use bevy::prelude::*;
use loader::dram::DramFile;
use loader::lev::Level;
use loader::model::{Model, ModelHeader};
use loader::vram::Vram;
use render::mesh::{self, PARTS, to_world};

use crate::drive::animation_pose;
use crate::subtract::SubtractMaterial;
use crate::{GameTick, spawn_parts};

/// A placed model whose pose changes with the tick.
struct Animated {
    header: ModelHeader,
    inst_scale: [i16; 3],
    parts: [Option<Handle<Mesh>>; PARTS],
    shown: Option<i16>,
}

#[derive(Resource, Default)]
pub struct Props(Vec<Animated>);

/// A placed armadillo: the simulation, where it was and is, and the pose its mesh shows.
struct ArmadilloProp {
    sim: hazard::armadillo::Armadillo,
    previous: (Vec3, Quat),
    current: (Vec3, Quat),
    header: ModelHeader,
    inst_scale: [i16; 3],
    parts: [Option<Handle<Mesh>>; PARTS],
    shown: Option<(u8, i16)>,
}

#[derive(Resource, Default)]
pub struct Armadillos(Vec<ArmadilloProp>);

/// A placed minecart, like [`ArmadilloProp`].
struct MinecartProp {
    sim: hazard::minecart::Minecart,
    previous: (Vec3, Quat),
    current: (Vec3, Quat),
    header: ModelHeader,
    inst_scale: [i16; 3],
    parts: [Option<Handle<Mesh>>; PARTS],
    shown: Option<i16>,
}

#[derive(Resource, Default)]
pub struct Minecarts(Vec<MinecartProp>);

/// The entity of a placed minecart (its index in [`Minecarts`]).
#[derive(Component)]
pub struct MinecartBody(usize);

/// The entity of a placed armadillo (its index in [`Armadillos`]), which [`armadillo_place`] moves.
#[derive(Component)]
pub struct ArmadilloBody(usize);

/// A placed model that rolls along `Level::paths[path]` (`RB_Snowball_ThTick`).
#[derive(Component)]
pub struct Rolling {
    path: usize,
}

/// The path of a rolling object: the digit its instance name ends in.
fn path_of(model: &str, name: &str, level: &Level) -> Option<usize> {
    if !matches!(model, "snowball" | "barrel") {
        return None;
    }
    let path = name.chars().last()?.to_digit(10)? as usize;
    (level.paths.get(path)?.len() > 1).then_some(path)
}

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
/// `RB_CtrLetter_Init`); the other placed models keep theirs. By model name.
fn spawn_scale(model: &str) -> Option<i16> {
    match model {
        "plant" => Some(0x2800),
        "c" | "t" | "r" => Some(0x1800),
        // `RB_Snowball_LInB`, which the barrels share.
        "snowball" | "barrel" => Some(0x1000),
        // `RB_Minecart_LInB`
        "minecart" => Some(0x1000),
        _ => None,
    }
}

/// The pose of a model at rest: its first animation's first frame, or its only frame.
fn rest_pose(header: &ModelHeader) -> Option<&loader::model::Frame> {
    header.animations.first().and_then(|a| a.frames.first()).or(header.static_frame.as_ref())
}

pub fn spawn(
    level: &Level,
    dram: &DramFile,
    vram: &Vram,
    commands: &mut Commands,
    images: &mut Assets<Image>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    subtract_materials: &mut Assets<SubtractMaterial>,
) {
    let mut props = Props::default();
    let mut armadillos = Armadillos::default();
    let mut minecarts = Minecarts::default();
    let mut models: std::collections::HashMap<u32, Option<Model>> = Default::default();
    let mut gte = gte::Gte::default();
    for inst in level.instances.iter().filter(|i| i.model != 0) {
        let model = models.entry(inst.model).or_insert_with(|| Model::parse_at(dram, inst.model).ok());
        let Some(header) = model.as_ref().and_then(|m| m.headers.first()) else { continue };
        let Some(frame) = rest_pose(header) else { continue };
        // The fireballs hide their model (`HIDE_MODEL`); see `fire`.
        if model.as_ref().is_some_and(|m| m.name == "fireball") {
            continue;
        }
        let inst_scale = match model.as_ref().and_then(|m| spawn_scale(&m.name)) {
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

        // A rolling object hangs off an entity that `roll` moves along its path, an armadillo off
        // one that `armadillo_place` moves.
        let name = &model.as_ref().unwrap().name;
        let (transform, parent) = match path_of(name, &inst.name, level) {
            Some(path) => {
                let parent = commands.spawn((Rolling { path }, transform, Visibility::default())).id();
                (Transform::default(), Some(parent))
            }
            None if name == "armadillo" => {
                let delay = inst.name.chars().last().and_then(|c| c.to_digit(10)).and_then(|d| level.spawn_delays.get(d as usize));
                let sim = hazard::armadillo::Armadillo::new(inst, delay.copied().unwrap_or(0));
                let parent = commands.spawn((ArmadilloBody(armadillos.0.len()), transform, Visibility::default())).id();
                let pose = (transform.translation, transform.rotation);
                let parts = spawn_parts(data.parts, &base, Transform::default(), Some(parent), commands, meshes, materials, subtract_materials, 0);
                armadillos.0.push(ArmadilloProp {
                    sim,
                    previous: pose,
                    current: pose,
                    header: header.clone(),
                    inst_scale,
                    parts,
                    shown: None,
                });
                continue;
            }
            None if name == "minecart" => {
                let Some(sim) = level.position_paths.first().and_then(|path| hazard::minecart::Minecart::new(inst, path)) else {
                    continue;
                };
                let parent = commands.spawn((MinecartBody(minecarts.0.len()), transform, Visibility::default())).id();
                let pose = (transform.translation, transform.rotation);
                let parts = spawn_parts(data.parts, &base, Transform::default(), Some(parent), commands, meshes, materials, subtract_materials, 0);
                minecarts.0.push(MinecartProp { sim, previous: pose, current: pose, header: header.clone(), inst_scale, parts, shown: None });
                continue;
            }
            None => (transform, None),
        };
        let parts = spawn_parts(data.parts, &base, transform, parent, commands, meshes, materials, subtract_materials, 0);
        if header.animations.first().is_some_and(|a| a.frames.len() > 1) {
            props.0.push(Animated { header: header.clone(), inst_scale, parts, shown: Some(0) });
        }
    }
    info!(
        "{} placed models, {} animated, {} armadillos, {} minecarts",
        level.instances.len(),
        props.0.len(),
        armadillos.0.len(),
        minecarts.0.len()
    );
    commands.insert_resource(props);
    commands.insert_resource(armadillos);
    commands.insert_resource(minecarts);
}

/// Where a rolling object is `tick` ticks into its path: it goes along the recorded points and
/// back again, a point per tick (`pointIndex` runs to twice the last one). Between two ticks it
/// is `fraction` of the way to the next point.
pub fn roll(
    mut rolling: Query<(&Rolling, &mut Transform)>,
    track: Res<crate::Track>,
    tick: Res<GameTick>,
    fixed: Res<Time<Fixed>>,
) {
    let mut gte = gte::Gte::default();
    let fraction = fixed.overstep_fraction();
    for (rolling, mut transform) in &mut rolling {
        let path = &track.0.paths[rolling.path];
        let at = |tick: u32| path[hazard::roller::frame_index(path.len(), tick).expect("a path of two points or more")];
        let (from, to) = (at(tick.0), at(tick.0 + 1));
        let mut place = |p: loader::lev::PosRot| (Vec3::from(to_world(p.pos)), rotation_of(&mut gte, p.rot));
        let ((from_pos, from_rot), (to_pos, to_rot)) = (place(from), place(to));
        transform.translation = from_pos.lerp(to_pos, fraction);
        transform.rotation = from_rot.slerp(to_rot, fraction);
    }
}

/// One game tick of the armadillos: they move, and their mesh takes the pose of the animation
/// they play.
pub fn armadillo_tick(mut armadillos: ResMut<Armadillos>, mut meshes: ResMut<Assets<Mesh>>) {
    for a in &mut armadillos.0 {
        let frames = [0, 1].map(|i| a.header.animations.get(i).map_or(0, |anim| anim.num_frames as i32));
        a.sim.tick(frames);
        a.previous = a.current;
        let at = Vec3::new(a.sim.pos[0] as f32, a.sim.pos[1] as f32, a.sim.pos[2] as f32) * render::mesh::SCALE;
        a.current = (at, quat_of(&a.sim.matrix()));

        let now = (a.sim.anim_index, a.sim.anim_frame);
        if a.shown == Some(now) {
            continue;
        }
        a.shown = Some(now);
        let Some(pose) = animation_pose(&a.header, now.0, now.1) else { continue };
        let positions = mesh::model_positions(&a.header, &pose, a.inst_scale);
        for (handle, positions) in a.parts.iter().zip(positions) {
            let Some(mut mesh) = handle.as_ref().and_then(|h| meshes.get_mut(h)) else { continue };
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        }
    }
}

/// One game tick of the minecarts: they move along the level's first position path and their
/// mesh takes the frame of their animation.
pub fn minecart_tick(mut minecarts: ResMut<Minecarts>, track: Res<crate::Track>, mut meshes: ResMut<Assets<Mesh>>) {
    let Some(path) = track.0.position_paths.first() else { return };
    for c in &mut minecarts.0 {
        let frames = c.header.animations.first().map_or(0, |anim| anim.num_frames as i32);
        c.sim.tick(path, frames);
        c.previous = c.current;
        let at = Vec3::new(c.sim.pos[0] as f32, c.sim.pos[1] as f32, c.sim.pos[2] as f32) * render::mesh::SCALE;
        c.current = (at, quat_of(&c.sim.matrix()));

        if c.shown == Some(c.sim.anim_frame) {
            continue;
        }
        c.shown = Some(c.sim.anim_frame);
        let Some(pose) = animation_pose(&c.header, 0, c.sim.anim_frame) else { continue };
        let positions = mesh::model_positions(&c.header, &pose, c.inst_scale);
        for (handle, positions) in c.parts.iter().zip(positions) {
            let Some(mut mesh) = handle.as_ref().and_then(|h| meshes.get_mut(h)) else { continue };
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        }
    }
}

/// Puts the minecarts between their last two ticks.
pub fn minecart_place(mut bodies: Query<(&MinecartBody, &mut Transform)>, minecarts: Res<Minecarts>, fixed: Res<Time<Fixed>>) {
    let t = fixed.overstep_fraction();
    for (body, mut transform) in &mut bodies {
        let c = &minecarts.0[body.0];
        // The path restarts with a jump, which must not be smoothed over.
        let jump = c.previous.0.distance(c.current.0) > 5.0;
        let from = if jump { c.current } else { c.previous };
        transform.translation = from.0.lerp(c.current.0, t);
        transform.rotation = from.1.slerp(c.current.1, t);
    }
}

/// Puts the armadillos between their last two ticks.
pub fn armadillo_place(
    mut bodies: Query<(&ArmadilloBody, &mut Transform)>,
    armadillos: Res<Armadillos>,
    fixed: Res<Time<Fixed>>,
) {
    let t = fixed.overstep_fraction();
    for (body, mut transform) in &mut bodies {
        let a = &armadillos.0[body.0];
        transform.translation = a.previous.0.lerp(a.current.0, t);
        // A turn is a few angle units a tick; the product of the last two ticks' facing is the
        // short way round.
        transform.rotation = a.previous.1.slerp(a.current.1, t);
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

