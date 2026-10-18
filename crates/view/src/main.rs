//! Fly-through viewer for a track's geometry (textured).
//!
//! `view [LEVEL_ID] [--lo]` — level id as in `loader::level::LevelId` (default 0);
//! `--lo` shows the `color_lo` vertex colours; `--show-red` keeps the
//! faces painted pure red, which are hidden by default. The disc image is `CTR_BIN`
//! or `assets/ctr-u.bin`.
//!
//! A collision probe from the camera is drawn and described on screen; see
//! `probe.rs` for its keys.
//!
//! Controls: hold the right mouse button to look, WASD to move, Space/Shift
//! up/down, Ctrl to go fast, mouse wheel to change the base speed.

mod mesh;
mod probe;
mod subtract;

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
use loader::lev::Level;
use loader::level::LevelId;
use loader::vram::{Vram, parse_vram_file};
use mesh::{PARTS, Part};
use subtract::SubtractMaterial;

/// A level's geometry plus what its texture pointers need.
struct Loaded {
    level: Level,
    dram: DramFile,
    vram: Vram,
}

fn load_level(id: u32) -> Loaded {
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
    Loaded {
        level,
        dram,
        vram,
    }
}

#[derive(Resource)]
pub struct Track(pub Level);

#[derive(Resource)]
struct Textures {
    dram: DramFile,
    vram: Vram,
}

#[derive(Resource)]
struct LowColors(bool);

#[derive(Resource)]
struct ShowRed(bool);

#[derive(Component)]
struct FlyCamera {
    yaw: f32,
    pitch: f32,
    speed: f32,
}

fn main() {
    let mut id = 0;
    let mut low = false;
    let mut show_red = false;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--lo" => low = true,
            "--show-red" => show_red = true,
            n => {
                id = n.parse().unwrap_or_else(|_| {
                    panic!("usage: view [LEVEL_ID] [--lo] [--show-red], got `{n}`")
                })
            }
        }
    }

    let Loaded {
        level,
        dram,
        vram,
    } = load_level(id);
    let loaded = Track(level);
    let textures = Textures { dram, vram };

    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: format!("ctr-rs view: level {id}"),
                ..default()
            }),
            ..default()
        }))
        .insert_resource(ClearColor(Color::srgb(0.05, 0.06, 0.1)))
        .insert_resource(loaded)
        .insert_resource(textures)
        .insert_resource(LowColors(low))
        .insert_resource(ShowRed(show_red))
        .add_systems(Startup, setup)
        .add_plugins(MaterialPlugin::<SubtractMaterial>::default())
        .add_plugins(probe::ProbePlugin)
        .insert_resource(Time::<Fixed>::from_hz(GAME_TICK_HZ))
        .add_systems(FixedUpdate, animate)
        .add_systems(Update, fly)
        .run();
}

fn setup(
    mut commands: Commands,
    track: Res<Track>,
    textures: Res<Textures>,
    mut images: ResMut<Assets<Image>>,
    low: Res<LowColors>,
    show_red: Res<ShowRed>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut subtract_materials: ResMut<Assets<SubtractMaterial>>,
) {
    let level = &track.0;
    let data = mesh::level_mesh(level, &textures.dram, &textures.vram, low.0, !show_red.0);
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

    let mut handles: [Option<Handle<Mesh>>; PARTS] = default();
    let mut uvs: [Vec<[f32; 2]>; PARTS] = default();
    let base = StandardMaterial {
        base_color: Color::WHITE,
        base_color_texture: Some(atlas),
        unlit: true,
        cull_mode: None,
        double_sided: true,
        ..default()
    };
    for (i, part) in data.parts.into_iter().enumerate() {
        if part.indices.is_empty() {
            continue;
        }
        uvs[i] = part.uvs.clone();
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
        let mut entity = commands.spawn(Mesh3d(handle));
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
    commands.insert_resource(AnimState {
        shown: vec![usize::MAX; data.animations.len()],
        animations: data.animations,
        faces: data.animated_faces,
        uvs,
        handles,
        tick: 0,
    });

    let spawn = level.driver_spawns[0].pos;
    let [x, y, z] = mesh::to_world(spawn);
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
    /// Game ticks elapsed (`gGT->timer`).
    tick: i32,
}

/// `CTR_CycleTex_LEV`, once per game tick.
fn animate(mut state: ResMut<AnimState>, mut meshes: ResMut<Assets<Mesh>>) {
    let state = &mut *state;
    let mut dirty = [false; PARTS];
    for (i, anim) in state.animations.iter().enumerate() {
        let frame = anim.texture.frame_at(state.tick);
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
    state.tick += 1;
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
