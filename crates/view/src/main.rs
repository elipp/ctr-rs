//! Fly-through viewer for a track's geometry (textured).
//!
//! `view [LEVEL_ID] [--lo] [--racer N | --no-racer]` — level id as in
//! `loader::level::LevelId` (default 0); `--racer N` puts racer model N (0 Crash,
//! 1 Cortex, ... as `GAME_CHARACTER_IDS`; default 0) at the first starting grid slot;
//! `--lo` shows the `color_lo` vertex colours; `--show-hidden` keeps the
//! blocks without textures and the faces painted pure red, which are hidden by default. The disc image is `CTR_BIN`
//! or `assets/ctr-u.bin`.
//!
//! A collision probe from the camera is drawn and described on screen; see
//! `probe.rs` for its keys.
//!
//! Controls: hold the right mouse button to look, WASD to move, Space/Shift
//! up/down, Ctrl to go fast, mouse wheel to change the base speed.
//!
//! `--drive` drives the racer instead, see `drive.rs`.

mod drive;
mod probe;
mod exhaust;
mod shadow;
mod subtract;
mod wheels;

use std::path::PathBuf;

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use loader::bigindex::{BigIndex, LevFile, Lod};
use loader::disc::open_bigfile;
use loader::dram::DramFile;
use loader::icons::{Icon, parse_groups, parse_icons};
use loader::lev::Level;
use loader::level::LevelId;
use loader::model::Model;
use loader::vram::{Vram, parse_vram_file};
use render::mesh::{self, PARTS, Part};
use subtract::SubtractMaterial;

/// `NITROS_OXIDE` in `GAME_CHARACTER_IDS` order.
const NITROS_OXIDE: u32 = 15;

/// A level's geometry plus what its texture pointers need.
struct Loaded {
    level: Level,
    dram: DramFile,
    vram: Vram,
    racer: Option<Model>,
    /// The icons of the racer's wheel sprites (`tireAnim`); none for Oxide, who has no wheels.
    wheel_icons: Vec<Icon>,
    /// Global icons 0 and 1 of the racer's pack (`shadow1`, `shadow2`).
    shadow_icons: Option<[Icon; 2]>,
    /// The first picture of the `dustpuff` icon group: the exhaust smoke.
    smoke_icon: Option<Icon>,
}

/// The sprite and texture page file shared by every track (`shared.vrm`):
/// the racers' textures are in it.
fn load_level(id: u32, racer: Option<u32>) -> Loaded {
    let path = std::env::var_os("CTR_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/ctr-u.bin")
        });
    let (mut disc, big) = open_bigfile(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut read = |f| {
        let index = BigIndex::level(LevelId(id), Lod::OnePlayer, f);
        big.read(&mut disc, index.get())
            .expect("read level file from BIGFILE")
    };
    let dram = DramFile::parse(&read(LevFile::Lev)).expect("parse DRAM file");
    let level = Level::parse(&dram).expect("parse LEV");
    let mut vram = Vram::default();
    for img in parse_vram_file(&read(LevFile::Vram)).expect("parse VRAM file") {
        vram.upload(&img).expect("upload VRAM image");
    }
    let shared = big
        .read(&mut disc, BigIndex::SHARED_MPK_VRM.get())
        .expect("read shared VRAM file");
    for img in parse_vram_file(&shared).expect("parse shared VRAM file") {
        vram.upload(&img).expect("upload shared VRAM image");
    }
    let racer_character = racer;
    let racer = racer.map(|character| {
        let file = big
            .read(&mut disc, BigIndex::RACER_MODEL_HI.nth(character).get())
            .expect("read racer model");
        Model::parse(&DramFile::parse(&file).expect("parse DRAM file")).expect("parse racer model")
    });
    let shadow_icons = racer_character.and_then(|character| {
        let pack = big
            .read(&mut disc, BigIndex::ARCADE_1P_PACK.nth(character).get())
            .expect("read racer pack");
        let icons = parse_icons(&DramFile::parse(&pack).expect("parse racer pack")).expect("parse racer pack icons");
        let find = |index| icons.iter().find(|i| i.global_index == index).cloned();
        Some([find(0)?, find(1)?])
    });
    let smoke_icon = racer_character.and_then(|character| {
        let pack = big
            .read(&mut disc, BigIndex::ARCADE_1P_PACK.nth(character).get())
            .expect("read racer pack");
        let groups = parse_groups(&DramFile::parse(&pack).expect("parse racer pack")).expect("parse racer pack icons");
        groups.into_iter().find(|g| g.group_id == 1)?.icons.into_iter().next()
    });
    let wheel_icons = match racer_character {
        Some(character) if character != NITROS_OXIDE => {
            let pack = big
                .read(&mut disc, BigIndex::ARCADE_1P_PACK.nth(character).get())
                .expect("read racer pack");
            parse_groups(&DramFile::parse(&pack).expect("parse racer pack"))
                .expect("parse racer pack icons")
                .into_iter()
                .find(|g| g.group_id == 0)
                .expect("racer pack has the tire icon group")
                .icons
        }
        _ => Vec::new(),
    };
    Loaded {
        level,
        dram,
        vram,
        racer,
        wheel_icons,
        shadow_icons,
        smoke_icon,
    }
}

#[derive(Resource)]
pub struct Track(pub Level);

#[derive(Resource)]
struct Textures {
    dram: DramFile,
    vram: Vram,
}

/// The racer model shown on the grid.
#[derive(Resource)]
struct Racer(Option<Model>);

#[derive(Resource)]
struct WheelIcons(Vec<Icon>);

#[derive(Resource)]
struct ShadowIcons(Option<[Icon; 2]>);

#[derive(Resource)]
struct SmokeIcon(Option<Icon>);

#[derive(Resource)]
struct LowColors(bool);

#[derive(Resource)]
struct ShowHidden(bool);

#[derive(Component)]
struct FlyCamera {
    yaw: f32,
    pitch: f32,
    speed: f32,
}

fn main() {
    let mut id = 0;
    let mut racer = Some(0);
    let mut low = false;
    let mut show_hidden = false;
    let mut driving = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--lo" => low = true,
            "--drive" => driving = true,
            "--show-hidden" => show_hidden = true,
            "--no-racer" => racer = None,
            "--racer" => {
                racer = Some(
                    args.next()
                        .and_then(|n| n.parse().ok())
                        .filter(|&n| n < 16)
                        .expect("--racer takes a number below 16"),
                )
            }
            n => {
                id = n.parse().unwrap_or_else(|_| {
                    panic!("usage: view [LEVEL_ID] [--lo] [--show-hidden] [--drive] [--racer N | --no-racer], got `{n}`")
                })
            }
        }
    }

    let character = racer.unwrap_or(0) as usize;
    let Loaded {
        level,
        dram,
        vram,
        racer,
        wheel_icons,
        shadow_icons,
        smoke_icon,
    } = load_level(id, racer);
    let loaded = Track(level);
    let textures = Textures { dram, vram };
    let drive = driving.then(|| {
        let model = racer.as_ref().expect("--drive needs a racer");
        let mut anim_num_frames = [0; 8];
        for (n, anim) in anim_num_frames.iter_mut().zip(&model.headers[0].animations) {
            *n = anim.num_frames;
        }
        drive::Drive::new(&loaded, character, anim_num_frames)
    });

    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: format!("ctr-rs view: level {id}"),
            ..default()
        }),
        ..default()
    }))
    .insert_resource(ClearColor(Color::srgb(0.05, 0.06, 0.1)))
    .insert_resource(loaded)
    .insert_resource(textures)
    .insert_resource(Racer(racer))
    .insert_resource(WheelIcons(wheel_icons))
    .insert_resource(ShadowIcons(shadow_icons))
    .insert_resource(SmokeIcon(smoke_icon))
    .insert_resource(LowColors(low))
    .insert_resource(ShowHidden(show_hidden))
    .add_systems(Startup, setup)
    // After the cameras and karts have moved this frame, before their transforms propagate.
    .add_systems(PostUpdate, wheels::update.before(TransformSystems::Propagate))
    .add_plugins(MaterialPlugin::<SubtractMaterial>::default())
    .insert_resource(Time::<Fixed>::from_hz(GAME_TICK_HZ))
    .add_systems(FixedUpdate, (animate, animate_water, advance_tick).chain());
    if let Some(drive) = drive {
        app.insert_resource(drive).add_plugins(drive::DrivePlugin);
    } else {
        app.add_plugins(probe::ProbePlugin).add_systems(Update, fly);
    }
    app.run();
}

/// Spawns one entity per non-empty part, with the material its blend mode needs.
fn spawn_parts(
    parts: [mesh::MeshPart; PARTS],
    base: &StandardMaterial,
    transform: Transform,
    parent: Option<Entity>,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    subtract_materials: &mut Assets<SubtractMaterial>,
) -> [Option<Handle<Mesh>>; PARTS] {
    let mut handles: [Option<Handle<Mesh>>; PARTS] = default();
    for (i, part) in parts.into_iter().enumerate() {
        if part.indices.is_empty() {
            continue;
        }
        let mut m = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        m.insert_attribute(Mesh::ATTRIBUTE_POSITION, part.positions);
        m.insert_attribute(Mesh::ATTRIBUTE_COLOR, part.colors);
        m.insert_attribute(Mesh::ATTRIBUTE_UV_0, part.uvs);
        m.insert_indices(Indices::U32(part.indices));
        let handle = meshes.add(m);
        handles[i] = Some(handle.clone());
        let with = |alpha_mode| StandardMaterial {
            alpha_mode,
            ..base.clone()
        };
        let mut entity = commands.spawn((Mesh3d(handle), transform));
        if let Some(parent) = parent {
            entity.insert(ChildOf(parent));
        }
        match i {
            i if i == Part::Opaque as usize => {
                entity.insert(MeshMaterial3d(materials.add(with(AlphaMode::Mask(0.5)))))
            }
            i if i == Part::Average as usize => {
                entity.insert(MeshMaterial3d(materials.add(with(AlphaMode::Blend))))
            }
            i if i == Part::Add as usize => {
                entity.insert(MeshMaterial3d(materials.add(with(AlphaMode::Add))))
            }
            _ => entity.insert(MeshMaterial3d(subtract_materials.add(SubtractMaterial {
                base: with(AlphaMode::Blend),
                extension: subtract::Subtract {},
            }))),
        };
    }
    handles
}

fn setup(
    mut commands: Commands,
    track: Res<Track>,
    textures: Res<Textures>,
    racer: Res<Racer>,
    wheel_icons: Res<WheelIcons>,
    shadow_icons: Res<ShadowIcons>,
    smoke_icon: Res<SmokeIcon>,
    mut images: ResMut<Assets<Image>>,
    low: Res<LowColors>,
    show_hidden: Res<ShowHidden>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut subtract_materials: ResMut<Assets<SubtractMaterial>>,
    driving: Option<Res<drive::Drive>>,
) {
    let level = &track.0;
    let data = mesh::level_mesh(level, &textures.dram, &textures.vram, low.0, !show_hidden.0);
    info!(
        "{} quad blocks, vertices per part {:?}, {} animated faces",
        level.quad_blocks.len(),
        data.parts.each_ref().map(|p| p.positions.len()),
        data.animated_faces.len()
    );

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
    let atlas = images.add(atlas);

    let base = StandardMaterial {
        base_color: Color::WHITE,
        base_color_texture: Some(atlas),
        unlit: true,
        cull_mode: None,
        double_sided: true,
        ..default()
    };
    let uvs = data.parts.each_ref().map(|p| p.uvs.clone());
    let handles = spawn_parts(
        data.parts,
        &base,
        Transform::default(),
        None,
        &mut commands,
        &mut meshes,
        &mut materials,
        &mut subtract_materials,
    );
    let water_state = WaterState {
        water: level.water.clone(),
        corners: data.water_corners,
        atlas_size: data.atlas_size,
        colors: data.water.each_ref().map(|p| p.colors.clone()),
        uvs: data.water.each_ref().map(|p| p.uvs.clone()),
        handles: spawn_parts(
            data.water,
            &base,
            Transform::default(),
            None,
            &mut commands,
            &mut meshes,
            &mut materials,
            &mut subtract_materials,
        ),
    };
    commands.insert_resource(water_state);
    commands.insert_resource(GameTick(0));
    commands.insert_resource(AnimState {
        shown: vec![usize::MAX; data.animations.len()],
        animations: data.animations,
        faces: data.animated_faces,
        uvs,
        handles,
    });

    if let Some(model) = &racer.0 {
        spawn_racer(
            model,
            level.driver_spawns[0],
            driving.is_some(),
            &textures.vram,
            &wheel_icons.0,
            &mut commands,
            &mut images,
            &mut meshes,
            &mut materials,
            &mut subtract_materials,
        );
    }

    if let (Some(icons), true) = (&shadow_icons.0, driving.is_some()) {
        shadow::spawn(
            &mut commands,
            &mut images,
            &mut meshes,
            &mut subtract_materials,
            [&icons[0], &icons[1]],
            &textures.vram,
        );
    }

    if let (Some(icon), true) = (&smoke_icon.0, driving.is_some()) {
        exhaust::spawn(
            &mut commands,
            &mut images,
            &mut meshes,
            &mut materials,
            &mut subtract_materials,
            icon,
            &textures.vram,
        );
    }

    let spawn = level.driver_spawns[0].pos;
    let [x, y, z] = mesh::to_world(spawn);
    if driving.is_some() {
        commands.spawn((
            Camera3d::default(),
            // The game projects with a screen distance of 256 for a 512 pixel wide
            // screen: 90 degrees across, 73.7 degrees up at 4:3.
            Projection::Perspective(PerspectiveProjection {
                fov: 2.0 * (0.75f32).atan(),
                ..default()
            }),
            Transform::default(),
            drive::ChaseCamera::new(),
        ));
    } else {
        commands.spawn((
            Camera3d::default(),
            Transform::from_xyz(x, y + 3.0, z + 6.0),
            FlyCamera {
                yaw: 0.0,
                pitch: -0.3,
                speed: 10.0,
            },
        ));
    }
}

/// The instance scale `VehBirth` gives drivers (`VEH_BIRTH_DRIVER_INSTANCE_SCALE`).
const DRIVER_INSTANCE_SCALE: i16 = render::wheels::DRIVER_INSTANCE_SCALE as i16;

/// Racer models face along +X, while a driver's heading (`spawn.rot.y`) points along
/// +Z: a quarter turn clockwise seen from above (Bevy's +Y is up, so negative).
const MODEL_FORWARD_YAW: f32 = std::f32::consts::FRAC_PI_2;

/// Puts the model's neutral pose (the middle frame of its first animation,
/// which is the steering one) at a spawn point.
fn spawn_racer(
    model: &Model,
    spawn: loader::lev::SpawnPoint,
    driving: bool,
    vram: &Vram,
    wheel_icons: &[Icon],
    commands: &mut Commands,
    images: &mut Assets<Image>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    subtract_materials: &mut Assets<SubtractMaterial>,
) {
    let header = &model.headers[0];
    let frame = header
        .animations
        .first()
        .map(|a| &a.frames[a.frames.len() / 2])
        .or(header.static_frame.as_ref())
        .expect("racer model has a pose");
    let data = mesh::model_mesh(header, frame, vram, [DRIVER_INSTANCE_SCALE; 3]);

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
    let base = StandardMaterial {
        base_color: Color::WHITE,
        base_color_texture: Some(images.add(atlas)),
        unlit: true,
        cull_mode: None,
        double_sided: true,
        ..default()
    };

    let [x, y, z] = mesh::to_world(spawn.pos);
    let yaw = spawn.rot.y as f32 / 4096.0 * std::f32::consts::TAU;
    let placement = Transform::from_xyz(x, y, z)
        .with_rotation(Quat::from_rotation_y(yaw) * Quat::from_rotation_y(MODEL_FORWARD_YAW));
    // Driving: the parts hang off an entity the sim moves, and the model is
    // posed by that (the model itself faces along +Z, like the heading).
    let (placement, parent, anchor) = if driving {
        let kart = commands
            .spawn((drive::Kart, placement, Visibility::default()))
            .id();
        (Transform::default(), Some(kart), wheels::Anchor::Kart(kart))
    } else {
        (placement, None, wheels::Anchor::Fixed(placement))
    };
    if !wheel_icons.is_empty() {
        wheels::spawn(commands, images, meshes, materials, wheel_icons, vram, anchor);
    }
    spawn_parts(
        data.parts,
        &base,
        placement,
        parent,
        commands,
        meshes,
        materials,
        subtract_materials,
    );
}

/// The game logic and its texture animation run at this rate; rendering may
/// run at any multiple of it.
const GAME_TICK_HZ: f64 = 30.0;

/// Animated level textures: which frame each shows, and the UVs to patch into
/// the part meshes when it changes.
#[derive(Resource)]
struct AnimState {
    animations: Vec<mesh::Animation>,
    faces: Vec<mesh::AnimatedFace>,
    uvs: [Vec<[f32; 2]>; PARTS],
    handles: [Option<Handle<Mesh>>; PARTS],
    shown: Vec<usize>,
}

/// Game ticks elapsed (`gGT->timer`).
#[derive(Resource)]
struct GameTick(u32);

/// The water faces: their meshes, and what the per-tick water animation patches into them.
#[derive(Resource)]
struct WaterState {
    water: loader::water::Water,
    corners: std::collections::HashMap<usize, Vec<mesh::WaterCorner>>,
    atlas_size: [f32; 2],
    colors: [Vec<[f32; 4]>; PARTS],
    uvs: [Vec<[f32; 2]>; PARTS],
    handles: [Option<Handle<Mesh>>; PARTS],
}

/// `CTR_CycleTex_LEV`, once per game tick.
fn animate(mut state: ResMut<AnimState>, tick: Res<GameTick>, mut meshes: ResMut<Assets<Mesh>>) {
    let state = &mut *state;
    let mut dirty = [false; PARTS];
    for (i, anim) in state.animations.iter().enumerate() {
        let frame = anim.texture.frame_at(tick.0 as i32);
        if std::mem::replace(&mut state.shown[i], frame) == frame {
            continue;
        }
        for face in state.faces.iter().filter(|f| f.animation == i) {
            for (k, &from) in face.uv_order.iter().enumerate() {
                state.uvs[face.part][face.first_vertex + k] = anim.uvs[frame][from];
            }
            dirty[face.part] = true;
        }
    }
    for (part, _) in dirty.iter().enumerate().filter(|(_, d)| **d) {
        if let Some(mut mesh) = state.handles[part].as_ref().and_then(|h| meshes.get_mut(h)) {
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, state.uvs[part].clone());
        }
    }
}

/// `AnimateWater1P`, once per game tick, for every water vertex.
fn animate_water(
    mut state: ResMut<WaterState>,
    tick: Res<GameTick>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let state = &mut *state;
    let [width, height] = state.atlas_size;
    for c in state.water.animate(tick.0, |_| true) {
        // Textured primitives get their colour doubled, as everywhere else.
        let gray = c.gray as f32 * 2.0 / 255.0;
        for corner in state.corners.get(&c.vertex).into_iter().flatten() {
            state.colors[corner.part][corner.slot] = [gray, gray, gray, 1.0];
            state.uvs[corner.part][corner.slot] = [
                (corner.origin[0] + c.uv[0] as f32) / width,
                (corner.origin[1] + c.uv[1] as f32) / height,
            ];
        }
    }
    for part in 0..PARTS {
        if let Some(mut mesh) = state.handles[part].as_ref().and_then(|h| meshes.get_mut(h)) {
            mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, state.colors[part].clone());
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, state.uvs[part].clone());
        }
    }
}

fn advance_tick(mut tick: ResMut<GameTick>) {
    tick.0 += 1;
}

fn fly(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    mut cameras: Query<(&mut Transform, &mut FlyCamera)>,
) {
    for (mut transform, mut cam) in &mut cameras {
        if buttons.pressed(MouseButton::Right) {
            cam.yaw -= motion.delta.x * 0.003;
            cam.pitch = (cam.pitch - motion.delta.y * 0.003).clamp(-1.55, 1.55);
        }
        cam.speed = (cam.speed * (1.0 + scroll.delta.y * 0.1)).clamp(0.5, 500.0);
        transform.rotation = Quat::from_euler(EulerRot::YXZ, cam.yaw, cam.pitch, 0.0);

        let axis = |pos: KeyCode, neg: KeyCode| {
            keys.pressed(pos) as i32 as f32 - keys.pressed(neg) as i32 as f32
        };
        let wish = transform.forward() * axis(KeyCode::KeyW, KeyCode::KeyS)
            + transform.right() * axis(KeyCode::KeyD, KeyCode::KeyA)
            + Vec3::Y * axis(KeyCode::Space, KeyCode::ShiftLeft);
        let boost = if keys.pressed(KeyCode::ControlLeft) {
            4.0
        } else {
            1.0
        };
        transform.translation += wish.normalize_or_zero() * cam.speed * boost * time.delta_secs();
    }
}
