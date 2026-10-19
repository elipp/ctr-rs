//! Drive mode (`--drive`): the kart on the keyboard, simulated by the
//! `vehicle` crate once per game tick (30 Hz), drawn between the last two
//! ticks, with a chase camera.
//!
//! * W / Up: gas (Cross), S / Down: brake and reverse (Square, stick back)
//! * A / D, Left / Right: steer
//! * Space / Shift: hop (R1)
//! * R: back to the starting grid
//!
//! Not in the sim yet: drifting (the hop does nothing but jump), items, the
//! start-of-race countdown, other karts. Falling off the track puts the kart
//! back on the grid (the original's mask grab is not ported).

use bevy::prelude::*;
use coll::Collision;
use vehicle::sim::{spawn, tick};
use vehicle::{Driver, Env, Input, buttons};

use crate::Track;
use crate::mesh::SCALE;

/// The kart model's entity: its pose is the interpolated pose of the sim.
#[derive(Component)]
pub struct Kart;

#[derive(Component)]
pub struct ChaseCamera {
    /// Horizontal direction the camera looks along, smoothed.
    heading: Vec3,
}

impl ChaseCamera {
    pub fn new() -> ChaseCamera {
        ChaseCamera { heading: Vec3::Z }
    }
}

#[derive(Component)]
struct Hud;

/// Where the kart is drawn.
#[derive(Clone, Copy)]
struct Pose {
    translation: Vec3,
    rotation: Quat,
}

#[derive(Resource)]
pub struct Drive {
    driver: Driver,
    coll: Collision,
    character: usize,
    anim_num_frames: [u16; 8],
    tick: u32,
    held: u32,
    previous: Pose,
    current: Pose,
}

impl Drive {
    pub fn new(track: &Track, character: usize, anim_num_frames: [u16; 8]) -> Drive {
        let (driver, coll) = Drive::birth(track, character, anim_num_frames);
        let pose = pose_of(&driver);
        Drive { driver, coll, character, anim_num_frames, tick: 0, held: 0, previous: pose, current: pose }
    }

    fn birth(track: &Track, character: usize, anim_num_frames: [u16; 8]) -> (Driver, Collision) {
        let mut coll = Collision::default();
        let mut driver = Driver::new(0, character);
        driver.anim_num_frames = anim_num_frames;
        spawn(&mut driver, &mut coll, &track.0, 0);
        (driver, coll)
    }
}

/// The pose of the model: its position, and the orientation the sim builds from
/// the surface normal and the heading (`matrixFacingDir`; its second column is
/// up, the third forwards).
fn pose_of(d: &Driver) -> Pose {
    let m = &d.matrix_facing_dir.m;
    let column = |c: usize| Vec3::new(m[0][c] as f32, m[1][c] as f32, m[2][c] as f32) / 4096.0;
    let up = column(1).normalize_or_zero();
    let forward = column(2).normalize_or_zero();
    let rotation = if up == Vec3::ZERO || forward == Vec3::ZERO {
        Quat::IDENTITY
    } else {
        Quat::from_mat3(&Mat3::from_cols(up.cross(forward), up, forward))
    };
    // `pos_curr` has 8 fractional bits; the model sits a little higher when it bounces.
    let offset_y = (d.screen_offset_y as i32 * 3) >> 3;
    let translation = Vec3::new(
        (d.pos_curr.x >> 8) as f32,
        ((d.pos_curr.y >> 8) + offset_y) as f32,
        (d.pos_curr.z >> 8) as f32,
    ) * SCALE;
    Pose { translation, rotation }
}

pub struct DrivePlugin;

impl Plugin for DrivePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_hud)
            .add_systems(FixedUpdate, drive_tick)
            .add_systems(Update, (show_kart, chase, hud).chain());
    }
}

fn spawn_hud(mut commands: Commands) {
    commands.spawn((
        Hud,
        Text::new(""),
        Node { position_type: PositionType::Absolute, bottom: Val::Px(8.0), left: Val::Px(8.0), ..default() },
    ));
}

fn read_input(keys: &ButtonInput<KeyCode>, previous_held: u32) -> Input {
    let down = |codes: &[KeyCode]| codes.iter().any(|&k| keys.pressed(k));
    let mut held = 0;
    if down(&[KeyCode::KeyW, KeyCode::ArrowUp]) {
        held |= buttons::CROSS;
    }
    let reverse = down(&[KeyCode::KeyS, KeyCode::ArrowDown]);
    if reverse {
        held |= buttons::SQUARE;
    }
    if down(&[KeyCode::Space, KeyCode::ShiftLeft, KeyCode::ShiftRight]) {
        held |= buttons::R1;
    }
    let left = down(&[KeyCode::KeyA, KeyCode::ArrowLeft]);
    let right = down(&[KeyCode::KeyD, KeyCode::ArrowRight]);
    Input {
        buttons_held: held,
        buttons_tapped: held & !previous_held,
        // Sticks: 0 is left, 0x80 the middle, 0xff right (and back for LY).
        stick_lx: match (left, right) {
            (true, false) => 0,
            (false, true) => 0xff,
            _ => 0x80,
        },
        stick_ly: if reverse { 0xff } else { 0x80 },
        stick_ry: 0x80,
    }
}

/// One game tick of the sim.
fn drive_tick(keys: Res<ButtonInput<KeyCode>>, track: Res<Track>, mut drive: ResMut<Drive>) {
    let drive = &mut *drive;
    if keys.pressed(KeyCode::KeyR) {
        let (driver, coll) = Drive::birth(&track, drive.character, drive.anim_num_frames);
        (drive.driver, drive.coll, drive.tick) = (driver, coll, 0);
    }

    let input = read_input(&keys, drive.held);
    drive.held = input.buttons_held;
    let env = Env { elapsed_event_time: drive.tick as i32 * 32, timer: drive.tick, ..Env::default() };
    let result = tick(&mut drive.driver, &mut drive.coll, &track.0, &env, &input, &mut Vec::new());
    drive.tick += 1;

    if result.mask_grab {
        let (driver, coll) = Drive::birth(&track, drive.character, drive.anim_num_frames);
        (drive.driver, drive.coll) = (driver, coll);
    }

    drive.previous = drive.current;
    drive.current = pose_of(&drive.driver);
}

/// The kart between the last two ticks.
fn interpolated(drive: &Drive, fixed: &Time<Fixed>) -> Pose {
    let t = fixed.overstep_fraction();
    Pose {
        translation: drive.previous.translation.lerp(drive.current.translation, t),
        rotation: drive.previous.rotation.slerp(drive.current.rotation, t),
    }
}

fn show_kart(drive: Res<Drive>, fixed: Res<Time<Fixed>>, mut karts: Query<&mut Transform, With<Kart>>) {
    let pose = interpolated(&drive, &fixed);
    for mut transform in &mut karts {
        transform.translation = pose.translation;
        transform.rotation = pose.rotation;
    }
}

/// Behind and above the kart, looking along where it goes.
fn chase(
    drive: Res<Drive>,
    fixed: Res<Time<Fixed>>,
    time: Res<Time>,
    mut cameras: Query<(&mut Transform, &mut ChaseCamera)>,
) {
    const DISTANCE: f32 = 6.0;
    const HEIGHT: f32 = 2.5;
    let pose = interpolated(&drive, &fixed);
    let forward = pose.rotation * Vec3::Z;
    let flat = Vec3::new(forward.x, 0.0, forward.z).normalize_or_zero();
    for (mut transform, mut camera) in &mut cameras {
        if flat != Vec3::ZERO {
            let follow = 1.0 - (-6.0 * time.delta_secs()).exp();
            camera.heading = camera.heading.lerp(flat, follow).normalize_or_zero();
            if camera.heading == Vec3::ZERO {
                camera.heading = flat;
            }
        }
        transform.translation = pose.translation - camera.heading * DISTANCE + Vec3::Y * HEIGHT;
        transform.look_at(pose.translation + Vec3::Y * 0.8, Vec3::Y);
    }
}

fn hud(drive: Res<Drive>, mut text: Query<&mut Text, With<Hud>>) {
    let d = &drive.driver;
    let grounded = d.actions_flag_set & vehicle::actions::TOUCH_GROUND != 0;
    for mut text in &mut text {
        text.0 = format!(
            "speed {:5}  {:?}{}{}\nW/S gas/brake, A/D steer, Space hop, R restart",
            d.speed_approx,
            d.phys_state,
            if grounded { "" } else { "  airborne" },
            if d.wall_rub_timer != 0 { "  wall" } else { "" },
        );
    }
}
