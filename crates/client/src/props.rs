//! The models the level places (`struct InstDef`): mushrooms, crates, fruit, the start banner.
//!
//! Each is posed from its definition's position, angles (`ConvertRotToMatrix`) and scale. A
//! model with an animation plays its first one over and over at the game tick. The ones that
//! move by the game's code (the armadillos, the minecarts, the spiders, the seals, the plants,
//! the snowball and the barrels) are `instance::Objects`: that is ticked here, and these only
//! show it. What the game moves by other
//! code (bobbing crates, anything that reacts) stands still. The fruit spin on their own
//! axis and bob, at a speed of our own (DIVERGENCE.md).

use std::collections::HashMap;

use bevy::prelude::*;
use instance::{Objects, Slot};
use loader::dram::DramFile;
use loader::lev::Level;
use loader::level::LevelId;
use loader::model::{Model, ModelHeader};
use loader::vram::Vram;
use race::rollback::Rollback;
use race::{KartSpec, Race};
use render::mesh::{self, PARTS, SCALE, to_world};

use crate::banner::{Banner, Banners};
use crate::drive::animation_pose;
use crate::subtract::SubtractMaterial;
use crate::{GameTick, Track, spawn_parts};

/// The ticks that are kept to be played again when a pad arrives late: 4 seconds.
const ROLLBACK_WINDOW: usize = 120;

/// How often the sims of an online race are hashed and compared: every second.
const CHECK_EVERY: u32 = 30;

/// The race that is being played: the karts, the level's objects and everything else the next
/// tick depends on (`race::Race`). Out of a drive session it has no karts, only the objects.
#[derive(Resource)]
pub struct Sim {
    /// The race as far as it has been played; it reads as a `Race`.
    pub race: Rollback<Race>,
    /// The objects at tick 0, to start a race over.
    start: Objects,
    /// Our kart among the race's: the one the camera and the HUD follow (a spectator's changes).
    pub own: usize,
}

impl Sim {
    pub fn new(level: &Level, objects: Objects, specs: &[KartSpec]) -> Sim {
        Sim {
            race: Sim::rollback(level, objects.clone(), specs, 0, false, false),
            start: objects,
            own: 0,
        }
    }

    /// The race at tick 0 with the karts `specs`, ours being kart `own`; `online` for a race of
    /// a lobby, even if we are alone in it. A `spectator` drives none: `own` is the kart it is
    /// following, and no pad of it is waited for.
    fn rollback(
        level: &Level,
        objects: Objects,
        specs: &[KartSpec],
        own: usize,
        online: bool,
        spectator: bool,
    ) -> Rollback<Race> {
        let mut race = Race::new(level, objects, specs, crate::remote::kart_collisions());
        // An online race starts with the ceremony; offline the kart drives off at once.
        if online {
            race.start_ceremony(level);
        } else {
            race.laps = race::FREE_DRIVE_LAPS;
        }
        let rollback = Rollback::new(
            race,
            specs.len(),
            (!specs.is_empty() && !spectator).then_some(own),
            ROLLBACK_WINDOW,
        );
        // Nobody to compare with alone.
        if specs.len() > 1 {
            rollback.check_every(CHECK_EVERY)
        } else {
            rollback
        }
    }

    /// Puts the race back to tick 0 with the karts `specs`, ours being kart `own` (or, for a
    /// `spectator`, the one that is followed); every object is as it was at the start.
    pub fn restart(
        &mut self,
        level: &Level,
        specs: &[KartSpec],
        own: usize,
        online: bool,
        spectator: bool,
    ) {
        self.race = Sim::rollback(level, self.start.clone(), specs, own, online, spectator);
        self.own = own;
    }

    /// Our kart.
    pub fn kart(&self) -> &race::Kart {
        &self.race.karts[self.own]
    }

    /// Our driver.
    pub fn driver(&self) -> &vehicle::Driver {
        &self.kart().driver
    }
}

/// One game tick of the objects, in a viewer that has no karts.
pub fn tick_objects(mut sim: ResMut<Sim>, track: Res<Track>) {
    sim.race.advance(&track.0);
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
    /// Where the sim keeps it.
    slot: Slot,
    previous: (Vec3, Quat),
    current: (Vec3, Quat),
    header: ModelHeader,
    inst_scale: [i16; 3],
    parts: [Option<Handle<Mesh>>; PARTS],
    /// The animation and frame the mesh has.
    shown: Option<(u8, i16)>,
    /// The sim does not draw it (a door that is fully open).
    hidden: bool,
}

impl Shown {
    /// Takes the pose of the animation `(index, frame)` if the mesh does not have it yet.
    fn pose(&mut self, animation: (u8, i16), meshes: &mut Assets<Mesh>) {
        if self.shown == Some(animation) {
            return;
        }
        self.shown = Some(animation);
        let Some(pose) = animation_pose(&self.header, animation.0, animation.1) else {
            return;
        };
        let positions = mesh::model_positions(&self.header, &pose, self.inst_scale);
        for (handle, positions) in self.parts.iter().zip(positions) {
            let Some(mut mesh) = handle.as_ref().and_then(|h| meshes.get_mut(h)) else {
                continue;
            };
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        }
    }

    fn moved(&mut self, pos: [i32; 3], matrix: &vehicle::Matrix) {
        self.previous = self.current;
        self.current = (
            Vec3::new(pos[0] as f32, pos[1] as f32, pos[2] as f32) * SCALE,
            quat_of(matrix),
        );
    }
}

/// The objects that move or animate by the sim and have a pose to show for it
/// (`Objects::pose`): armadillos, minecarts, spiders, seals, plants and the temple door.
#[derive(Resource, Default)]
pub struct Hazards(Vec<Shown>);

/// The turtles stay where they are placed; only their animation is the sim's.
#[derive(Resource, Default)]
pub struct Turtles(Vec<Shown>);

/// The entity of a placed hazard (its index in [`Hazards`]).
#[derive(Component)]
pub struct HazardBody(usize);

/// A placed wumpa fruit (its index among `Objects::fruit`), and the way it was placed.
#[derive(Component)]
pub struct FruitBody {
    index: usize,
    /// Where and how it was placed.
    home: Vec3,
    rest: Quat,
}

/// The ticks a fruit takes to turn once around its own axis. Not retail's (see DIVERGENCE.md).
pub(crate) const FRUIT_SPIN_TICKS: f32 = 60.0;

/// The ticks of a bob up and down, and how far it goes either way (in level units). Only the
/// drawing bobs: the hitbox stays where the level has it.
const FRUIT_BOB_TICKS: f32 = 55.0;
const FRUIT_BOB_HEIGHT: f32 = 10.0;

/// Turns the fruit around their own vertical axis and bobs them, by the tick and how far the
/// render is through it. The angle only depends on the tick, so every peer sees the same spin; it never goes back
/// when ticks wait (a pause, the network) and the render keeps passing through the same tick.
pub fn spin_fruit(
    sim: Res<Sim>,
    fixed: Res<Time<Fixed>>,
    mut last: Local<(u32, f32)>,
    mut fruit: Query<(&FruitBody, &mut Transform)>,
) {
    let ticks = sim.race.objects.ticks();
    let mut turns = ticks as f32 + fixed.overstep_fraction();
    if ticks >= last.0 {
        turns = turns.max(last.1);
    }
    *last = (ticks, turns);
    let angle = turns / FRUIT_SPIN_TICKS * std::f32::consts::TAU;
    for (body, mut transform) in &mut fruit {
        transform.rotation = body.rest * Quat::from_rotation_y(angle);
        // Each fruit is a little ahead of the one before, so they do not bob as one.
        let phase = (turns / FRUIT_BOB_TICKS + body.index as f32 * 0.17) * std::f32::consts::TAU;
        transform.translation = body.home + Vec3::Y * (phase.sin() * FRUIT_BOB_HEIGHT * SCALE);
    }
}

/// How far to raise instance `n` of a level, in level units, when the level places a crate inside
/// the ground (see DIVERGENCE.md). Only what is drawn moves; the sim keeps the crate where the
/// level has it.
///
/// Each is how far the drawn ground is above the crate's base at its center (`ctr-query crates`)
/// plus 15, which is how high the level puts its other crates. Crates the ground only cuts at a
/// corner, on a slope, are left to `CrateBias`.
fn crate_fixup(level: LevelId, n: usize) -> i16 {
    match (level.0, n) {
        // Dingo Canyon.
        (0, 33) => 38,
        (0, 37 | 47) => 26,
        // Dragon Mines.
        (1, 7) => 23,
        // Blizzard Bluff: the weapon crates over quad blocks 881 and 882 (ids 878 and 877) sit at
        // 339 in ground at 360.
        (2, 23..=26) => 36,
        (2, 36) => 34,
        // Crash Cove.
        (3, 11) => 23,
        (3, 12) => 25,
        // Tiger Temple.
        (4, 6) => 24,
        (4, 7) => 27,
        // Papu's Pyramid.
        (5, 15) => 24,
        (5, 16) => 25,
        // Roo's Tubes.
        (6, 29) => 23,
        (6, 31) => 32,
        // Sewer Speedway.
        (8, 27) => 28,
        // Mystery Caves.
        (9, 8) => 29,
        (9, 9) => 27,
        (9, 13 | 14) => 23,
        (9, 15) => 42,
        // Polar Pass.
        (12, 10) => 32,
        // Oxide Station.
        (13, 7) => 24,
        (13, 24) => 26,
        // Tiny Arena.
        (15, 7) => 31,
        (15, 28 | 31) => 23,
        _ => 0,
    }
}

/// A placed crate (its index among `Objects::crates`).
#[derive(Component)]
pub struct CrateBody(pub usize);

/// `/debug crate bias`: sets the depth bias of every crate's materials, and what the next level gets.
pub(crate) fn set_crate_bias(
    mut actions: MessageReader<crate::command::Action>,
    mut bias: ResMut<crate::CrateBias>,
    bodies: Query<&Children, With<CrateBody>>,
    parts: Query<&MeshMaterial3d<StandardMaterial>>,
    subtract_parts: Query<&MeshMaterial3d<crate::SubtractMaterial>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut subtract_materials: ResMut<Assets<crate::SubtractMaterial>>,
    mut toasts: MessageWriter<crate::toast::Toast>,
) {
    for action in actions.read() {
        let crate::command::Action::DebugCrateBias(value) = action else {
            continue;
        };
        let Some(value) = *value else {
            toasts.write(crate::toast::Toast::new(
                format!("Crate depth bias: {}", bias.0),
                3000,
            ));
            continue;
        };
        bias.0 = value;
        for child in bodies.iter().flat_map(|c| c.iter()) {
            if let Ok(m) = parts.get(child)
                && let Some(mut m) = materials.get_mut(&m.0)
            {
                m.depth_bias = value;
            }
            if let Ok(m) = subtract_parts.get(child)
                && let Some(mut m) = subtract_materials.get_mut(&m.0)
            {
                m.base.depth_bias = value;
            }
        }
        toasts.write(crate::toast::Toast::new(
            format!("Crate depth bias set to {value}"),
            2000,
        ));
    }
}

/// Sizes the crates as the sim says: gone when hit, growing back after a while.
pub fn pose_crates(
    sim: Res<Sim>,
    mut crates: Query<(&CrateBody, &mut Transform, &mut Visibility)>,
) {
    for (body, mut transform, mut visibility) in &mut crates {
        let scale = sim.race.objects.crates[body.0].sim.scale;
        let want = if scale == 0 {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
        if *visibility != want {
            *visibility = want;
        }
        if scale != 0 {
            transform.scale =
                Vec3::splat(scale as f32 / instance::pickup::crates::FULL_SCALE as f32);
        }
    }
}

/// Hides the fruit that were picked up.
pub fn hide_fruit(sim: Res<Sim>, mut fruit: Query<(&FruitBody, &mut Visibility)>) {
    for (body, mut visibility) in &mut fruit {
        let collected = sim.race.objects.fruit[body.index].sim.collected;
        let want = if collected {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
        if *visibility != want {
            *visibility = want;
        }
    }
}

/// A placed snowball or barrel (`Slot::Roller`), or a barrel of N. Gin Labs (`Slot::Drum`).
#[derive(Component)]
pub struct Rolling(Slot);

/// The orientation of an angle triple, as the matrix `ConvertRotToMatrix` makes of it: its
/// columns are where the model's axes go (as for the kart).
pub(crate) fn rotation_of(rot: loader::lev::SVec3) -> Quat {
    quat_of(&vehicle::rotmat::rot_to_matrix(rot))
}

fn quat_of(matrix: &vehicle::Matrix) -> Quat {
    let column = |c: usize| {
        Vec3::new(
            matrix.m[0][c] as f32,
            matrix.m[1][c] as f32,
            matrix.m[2][c] as f32,
        ) / 4096.0
    };
    Quat::from_mat3(&Mat3::from_cols(column(0), column(1), column(2)))
}

/// The scale an object's init sets on its instance, replacing the level file's (`RB_Plant_Init`,
/// `RB_CtrLetter_Init`, `RB_Snowball_LInB` which the barrels share, `RB_Baron_LInB`, `RB_Minecart_LInB`,
/// `RB_Spider_LInB`, `RB_Seal_LInB`); the other
/// placed models keep theirs. By model name.
fn spawn_scale(model: &str) -> Option<i16> {
    match model {
        "plant" => Some(0x2800),
        "spider" => Some(0x1c00),
        "seal" => Some(0x2000),
        "c" | "t" | "r" => Some(0x1800),
        "snowball" | "barrel" | "drum" | "minecart" => Some(0x1000),
        _ => None,
    }
}

/// The light of `RB_FlameJet_ThTick` (`fjLightDir`), which the jet's stone is shaded with.
const FLAME_JET_LIGHT: [i32; 3] = [0x8b8, 0xd6a, 0];
/// How much of its colour the stone keeps in the dark.
const FLAME_JET_AMBIENT: f32 = 0.4;

/// The model of a flame jet with its colours lit. They are the faces' normals (a byte for each
/// axis) and not colours: the game shades the model with a light that does not turn with it
/// (`Vector_SpecLightNoSpin3D`, the light turned into the model's space by its angles `rot`).
fn flame_jet_lit(header: &ModelHeader, rot: loader::lev::SVec3) -> ModelHeader {
    let m = vehicle::rotmat::rot_to_matrix(rot).m;
    // The matrix transposed, applied to the light.
    let light: [f32; 3] = std::array::from_fn(|i| {
        (0..3)
            .map(|j| m[j][i] as i32 * FLAME_JET_LIGHT[j])
            .sum::<i32>() as f32
            / 4096.0
            / 4096.0
    });
    let mut lit = header.clone();
    for color in &mut lit.colors {
        let normal = [*color & 0xff, *color >> 8 & 0xff, *color >> 16 & 0xff]
            .map(|c| c as u8 as i8 as f32 / 127.0);
        let facing: f32 = normal.iter().zip(light).map(|(n, l)| n * l).sum();
        let grey = (FLAME_JET_AMBIENT + (1.0 - FLAME_JET_AMBIENT) * facing.max(0.0)) * 128.0;
        let grey = grey.round() as u32;
        *color = grey | grey << 8 | grey << 16;
    }
    lit
}

/// The pose of a model at rest: its first animation's first frame, or its only frame.
pub(crate) fn rest_pose(header: &ModelHeader) -> Option<&loader::model::Frame> {
    header
        .animations
        .first()
        .and_then(|a| a.frames.first())
        .or(header.static_frame.as_ref())
}

/// The entity a hazard's model hangs off, which the systems below move; `marker` says which.
fn body(commands: &mut Commands, transform: Transform, marker: impl Bundle) -> Entity {
    commands
        .spawn((marker, transform, Visibility::default()))
        .id()
}

pub fn spawn(
    level: &Level,
    objects: &Objects,
    dram: &DramFile,
    vram: &Vram,
    commands: &mut Commands,
    images: &mut Assets<Image>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    subtract_materials: &mut Assets<SubtractMaterial>,
    crate_bias: f32,
    level_id: LevelId,
    fix_crates: bool,
) {
    // Which object of which kind moves an instance.
    let by_instance: HashMap<usize, Slot> = objects.slots().collect();

    let mut props = Props::default();
    let mut hazards = Hazards::default();
    let mut turtles = Turtles::default();
    let mut banners = Banners::default();
    let mut models: HashMap<u32, Option<Model>> = Default::default();
    // The C, T and R letters are hidden unless `CTR_LETTERS` is set.
    let show_letters = std::env::var_os("CTR_LETTERS").is_some();
    for (n, inst) in level
        .instances
        .iter()
        .enumerate()
        .filter(|(_, i)| i.model != 0)
    {
        if !show_letters && matches!(inst.model_name.as_str(), "c" | "t" | "r") {
            continue;
        }
        // The fireballs hide their model (`HIDE_MODEL`); see `fire`.
        if matches!(by_instance.get(&n), Some(Slot::Fireball(_))) {
            continue;
        }
        let model = models
            .entry(inst.model)
            .or_insert_with(|| Model::parse_at(dram, inst.model).ok());
        let Some(header) = model.as_ref().and_then(|m| m.headers.first()) else {
            continue;
        };
        let lit;
        let header = if inst.model_name == "flamejet" {
            lit = flame_jet_lit(header, inst.rot);
            &lit
        } else {
            header
        };
        let Some(frame) = rest_pose(header) else {
            continue;
        };
        let inst_scale = match spawn_scale(&inst.model_name) {
            Some(scale) => [scale; 3],
            None => [inst.scale.x, inst.scale.y, inst.scale.z],
        };
        // The banner waves: its commands are made over for it, and its mesh is posed each tick.
        let mut banner = (inst.model_name == "startbanner")
            .then(|| Banner::new(header, frame, inst_scale))
            .flatten();
        if inst.model_name == "startbanner" {
            match &banner {
                Some(b) => debug!("start banner waves: {} vertices", b.vertex_count()),
                None => warn!(
                    "start banner does not wave: {} colours, {} animations",
                    header.colors.len(),
                    header.animations.len()
                ),
            }
        }
        let mut data = mesh::model_mesh(
            banner.as_ref().map_or(header, |b| &b.header),
            frame,
            vram,
            inst_scale,
        );

        // The camera can be inside a crate or a roller, which then must not be there: they are
        // culled like the level.
        let convex = matches!(
            by_instance.get(&n),
            Some(Slot::Crate(_) | Slot::Roller(_) | Slot::Drum(_) | Slot::Fruit(_))
        );
        if convex {
            mesh::wind_outward(&mut data.parts);
        }
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
            cull_mode: convex.then_some(bevy::render::render_resource::Face::Back),
            double_sided: !convex,
            ..default()
        };

        let rotation = rotation_of(inst.rot);
        let transform =
            Transform::from_translation(Vec3::from(to_world(inst.pos))).with_rotation(rotation);
        let pose = (transform.translation, transform.rotation);

        // A hazard's model hangs off an entity that the systems below move.
        let shown = |slot| Shown {
            slot,
            previous: pose,
            current: pose,
            header: header.clone(),
            inst_scale,
            parts: Default::default(),
            shown: None,
            hidden: false,
        };
        let (transform, parent) = match by_instance.get(&n) {
            Some(&slot @ (Slot::Roller(_) | Slot::Drum(_))) => (
                Transform::default(),
                Some(body(commands, transform, Rolling(slot))),
            ),
            // A fruit's model hangs off a body that is hidden once it is picked up.
            Some(&Slot::Fruit(i)) => (
                Transform::default(),
                Some(body(
                    commands,
                    transform,
                    FruitBody {
                        index: i,
                        home: transform.translation,
                        rest: rotation,
                    },
                )),
            ),
            // A crate's model hangs off a body that shrinks when it is hit and grows back.
            Some(&Slot::Crate(i)) => {
                let mut pos = inst.pos;
                if fix_crates {
                    pos.y += crate_fixup(level_id, n);
                }
                let fixed =
                    Transform::from_translation(Vec3::from(to_world(pos))).with_rotation(rotation);
                (
                    Transform::default(),
                    Some(body(commands, fixed, CrateBody(i))),
                )
            }
            // What the sim moves or animates hangs off a body that is placed each tick.
            Some(&slot) if objects.pose(slot).is_some() => {
                let parent = body(commands, transform, HazardBody(hazards.0.len()));
                let mut s = shown(slot);
                s.parts = spawn_parts(
                    data.parts,
                    &base,
                    Transform::default(),
                    Some(parent),
                    commands,
                    meshes,
                    materials,
                    subtract_materials,
                    0,
                );
                hazards.0.push(s);
                if let Slot::Spider(i) = slot {
                    crate::spider::spawn_shadow(
                        i,
                        level,
                        objects,
                        dram,
                        vram,
                        commands,
                        images,
                        meshes,
                        materials,
                        subtract_materials,
                    );
                }
                continue;
            }
            Some(&slot @ Slot::Turtle(_)) => {
                let mut s = shown(slot);
                s.parts = spawn_parts(
                    data.parts,
                    &base,
                    transform,
                    None,
                    commands,
                    meshes,
                    materials,
                    subtract_materials,
                    0,
                );
                turtles.0.push(s);
                continue;
            }
            _ => (transform, None),
        };
        // The level places crates at whatever height, so they sink into the ground; pull them
        // toward the camera so the ground does not cut them.
        let crate_base;
        let base = if matches!(by_instance.get(&n), Some(Slot::Crate(_))) {
            crate_base = StandardMaterial {
                depth_bias: crate_bias,
                ..base.clone()
            };
            &crate_base
        } else {
            &base
        };
        let parts = spawn_parts(
            data.parts,
            base,
            transform,
            parent,
            commands,
            meshes,
            materials,
            subtract_materials,
            0,
        );
        if let Some(mut banner) = banner.take() {
            banner.parts = parts;
            banners.0.push(banner);
            continue;
        }
        if header
            .animations
            .first()
            .is_some_and(|a| a.frames.len() > 1)
        {
            props.0.push(Animated {
                header: header.clone(),
                inst_scale,
                parts,
                shown: Some(0),
            });
        }
    }
    debug!(
        "{} placed models, {} animated, {} hazards, {} rollers, {} turtles",
        level.instances.len(),
        props.0.len(),
        hazards.0.len(),
        objects.rollers.len(),
        turtles.0.len()
    );
    commands.insert_resource(props);
    commands.insert_resource(hazards);
    commands.insert_resource(turtles);
    commands.insert_resource(banners);
}

/// After a tick of the hazards: the ones the sim moves or animates take their new place and the
/// pose of the animation they play.
pub fn pose_hazards(
    sim: Res<Sim>,
    mut hazards: ResMut<Hazards>,
    mut turtles: ResMut<Turtles>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let objects = &sim.race.objects;
    for t in &mut turtles.0 {
        if let Slot::Turtle(i) = t.slot {
            t.pose((0, objects.turtles[i].sim.anim_frame), &mut meshes);
        }
    }
    for h in &mut hazards.0 {
        let Some(pose) = objects.pose(h.slot) else {
            continue;
        };
        h.moved(pose.pos, &pose.matrix);
        h.hidden = pose.hidden;
        h.pose(pose.animation, &mut meshes);
    }
}

/// Puts the hazards between their last two ticks.
pub fn place_hazards(
    mut bodies: Query<(&HazardBody, &mut Transform, &mut Visibility)>,
    hazards: Res<Hazards>,
    fixed: Res<Time<Fixed>>,
) {
    let t = fixed.overstep_fraction();
    for (body, mut transform, mut visibility) in &mut bodies {
        let s = &hazards.0[body.0];
        let want = if s.hidden {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
        if *visibility != want {
            *visibility = want;
        }
        // The path of a minecart restarts with a jump, which must not be smoothed over.
        let jump = s.previous.0.distance(s.current.0) > 5.0;
        let from = if jump { s.current } else { s.previous };
        transform.translation = from.0.lerp(s.current.0, t);
        transform.rotation = from.1.slerp(s.current.1, t);
    }
}

/// Puts the rolling objects between the point of their path they are at and the next.
pub fn roll(
    mut rolling: Query<(&Rolling, &mut Transform)>,
    sim: Res<Sim>,
    fixed: Res<Time<Fixed>>,
) {
    let fraction = fixed.overstep_fraction();
    for (rolling, mut transform) in &mut rolling {
        let objects = &sim.race.objects;
        let frames = match rolling.0 {
            Slot::Roller(i) => Some(objects.roller_frames(i)),
            Slot::Drum(i) => objects.drum_frames(i),
            _ => None,
        };
        let Some((from, to)) = frames else { continue };
        let place = |p: loader::lev::PosRot| (Vec3::from(to_world(p.pos)), rotation_of(p.rot));
        let ((from_pos, from_rot), (to_pos, to_rot)) = (place(from), place(to));
        transform.translation = from_pos.lerp(to_pos, fraction);
        transform.rotation = from_rot.slerp(to_rot, fraction);
    }
}

/// Plays the placed models' first animation, once per game tick.
pub fn animate(mut props: ResMut<Props>, tick: Res<GameTick>, mut meshes: ResMut<Assets<Mesh>>) {
    for prop in &mut props.0 {
        let Some(animation) = prop.header.animations.first() else {
            continue;
        };
        let frame = (tick.0 % animation.num_frames.max(1) as u32) as i16;
        if prop.shown == Some(frame) {
            continue;
        }
        prop.shown = Some(frame);
        let Some(pose) = animation_pose(&prop.header, 0, frame) else {
            continue;
        };
        let positions = mesh::model_positions(&prop.header, &pose, prop.inst_scale);
        for (handle, positions) in prop.parts.iter().zip(positions) {
            let Some(mut mesh) = handle.as_ref().and_then(|h| meshes.get_mut(h)) else {
                continue;
            };
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        }
    }
}
