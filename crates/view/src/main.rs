//! Fly-through viewer for a track's geometry (vertex colours only, no textures yet).
//!
//! `view [LEVEL_ID] [--lo] [--screenshot FILE]` — level id as in `loader::level::LevelId` (default 0);
//! `--lo` shows the `color_lo` vertex colours; `--screenshot` saves the first
//! frames' view to FILE and exits. The disc image is `CTR_BIN`
//! or `assets/ctr-u.bin`.
//!
//! Controls: hold the right mouse button to look, WASD to move, Q/E down/up,
//! Shift to go fast, mouse wheel to change the base speed.

mod mesh;

use std::path::PathBuf;

use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use loader::bigindex::{BigIndex, LevFile, Lod};
use loader::disc::open_bigfile;
use loader::dram::DramFile;
use loader::lev::Level;
use loader::level::LevelId;

fn load_level(id: u32) -> Level {
    let path = std::env::var_os("CTR_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/ctr-u.bin"));
    let (mut disc, big) = open_bigfile(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let index = BigIndex::level(LevelId(id), Lod::OnePlayer, LevFile::Lev);
    let file = big.read(&mut disc, index.get()).expect("read LEV from BIGFILE");
    Level::parse(&DramFile::parse(&file).expect("parse DRAM file")).expect("parse LEV")
}

#[derive(Resource)]
struct Track(Level);

#[derive(Resource)]
struct LowColors(bool);

/// Where to save a screenshot, then quit.
#[derive(Resource)]
struct ScreenshotPath(PathBuf);

#[derive(Component)]
struct FlyCamera {
    yaw: f32,
    pitch: f32,
    speed: f32,
}

fn main() {
    let mut id = 0;
    let mut low = false;
    let mut screenshot = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--lo" => low = true,
            "--screenshot" => screenshot = Some(PathBuf::from(args.next().expect("--screenshot needs a file"))),
            n => id = n.parse().unwrap_or_else(|_| panic!("usage: view [LEVEL_ID] [--lo], got `{n}`")),
        }
    }

    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: format!("ctr-rs view: level {id}"),
                ..default()
            }),
            ..default()
        }))
    .insert_resource(ClearColor(Color::srgb(0.05, 0.06, 0.1)))
    .insert_resource(Track(load_level(id)))
    .insert_resource(LowColors(low))
    .add_systems(Startup, setup)
    .add_systems(Update, fly);
    if let Some(path) = screenshot {
        app.insert_resource(ScreenshotPath(path)).add_systems(Update, screenshot_and_quit);
    }
    app.run();
}

fn screenshot_and_quit(
    mut commands: Commands,
    path: Res<ScreenshotPath>,
    mut frame: Local<u32>,
    mut exit: MessageWriter<AppExit>,
) {
    *frame += 1;
    // Give the mesh a few frames to reach the GPU before capturing.
    if *frame == 30 {
        commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path.0.clone()));
    }
    if *frame == 90 {
        exit.write(AppExit::Success);
    }
}

fn setup(
    mut commands: Commands,
    track: Res<Track>,
    low: Res<LowColors>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let level = &track.0;
    let data = mesh::level_mesh(level, low.0);
    info!(
        "{} vertices, {} quad blocks, {} triangles",
        data.positions.len(),
        level.quad_blocks.len(),
        data.indices.len() / 3
    );

    let mut m = Mesh::new(PrimitiveTopology::TriangleList, default());
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, data.positions);
    m.insert_attribute(Mesh::ATTRIBUTE_COLOR, data.colors);
    m.insert_indices(Indices::U32(data.indices));

    commands.spawn((
        Mesh3d(meshes.add(m)),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::WHITE,
            unlit: true,
            cull_mode: None,
            double_sided: true,
            ..default()
        })),
    ));

    let spawn = level.driver_spawns[0].pos;
    let [x, y, z] = mesh::to_world(spawn);
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(x, y + 3.0, z + 6.0),
        FlyCamera { yaw: 0.0, pitch: -0.3, speed: 10.0 },
    ));
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

        let axis = |pos: KeyCode, neg: KeyCode| keys.pressed(pos) as i32 as f32 - keys.pressed(neg) as i32 as f32;
        let wish = transform.forward() * axis(KeyCode::KeyW, KeyCode::KeyS)
            + transform.right() * axis(KeyCode::KeyD, KeyCode::KeyA)
            + Vec3::Y * axis(KeyCode::KeyE, KeyCode::KeyQ);
        let boost = if keys.pressed(KeyCode::ShiftLeft) { 4.0 } else { 1.0 };
        transform.translation += wish.normalize_or_zero() * cam.speed * boost * time.delta_secs();
    }
}
