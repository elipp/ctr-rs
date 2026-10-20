//! Drive mode (`--drive`): the kart on the keyboard, simulated by the
//! `vehicle` crate once per game tick (30 Hz), drawn between the last two
//! ticks, with a chase camera.
//!
//! * W / Up: gas (Cross), S / Down: brake and reverse (Square, stick back)
//! * A / D, Left / Right: steer
//! * Space / Shift: hop (R1), Q / Ctrl: hop (L1). Land while steering hard
//!   with one still held to drift; tap the other when the meter is nearly
//!   empty for the boost
//! * R: back to the starting grid
//!
//! Not in the sim yet: turbo pads (they are applied in kart-against-kart), items, the
//! start-of-race countdown, other karts. Falling off the track puts the kart
//! back on the grid (the original's mask grab is not ported).

use bevy::prelude::*;
use coll::Collision;
use vehicle::shadow::{GroundShadow, ground_shadow};
use vehicle::sim::{spawn, tick};
use vehicle::{Driver, Effect, Env, Input, buttons};

use crate::Track;
use render::mesh::SCALE;

/// The kart model's entity: its pose is the interpolated pose of the sim.
#[derive(Component)]
pub struct Kart;

/// Marks the camera that shows the sim's `camera::CameraDc`.
#[derive(Component)]
pub struct ChaseCamera;

impl ChaseCamera {
    pub fn new() -> ChaseCamera {
        ChaseCamera
    }
}

/// Where the camera is, what it looks at, and how it rolls.
#[derive(Clone, Copy)]
struct View {
    eye: Vec3,
    target: Vec3,
    /// Turns, around the view direction.
    roll: f32,
}

fn view_of(camera: &camera::CameraDc) -> View {
    let at = |x: i32, y: i32, z: i32| Vec3::new(x as f32, y as f32, z as f32) * SCALE;
    let (p, t) = (camera.pb_pos, camera.look_at_pos);
    View {
        eye: at(p.x as i32, p.y as i32, p.z as i32),
        target: at(t.x, t.y, t.z),
        roll: camera.pb_rot.z as f32 / 4096.0,
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
    shadow_previous: Option<GroundShadow>,
    shadow_current: Option<GroundShadow>,
    camera: camera::CameraDc,
    previous_view: View,
    current_view: View,
}

impl Drive {
    pub fn new(track: &Track, character: usize, anim_num_frames: [u16; 8]) -> Drive {
        let (driver, coll) = Drive::birth(track, character, anim_num_frames);
        let pose = pose_of(&driver);
        let mut camera = camera::CameraDc::default();
        camera.follow_driver(&driver, &track.0, &camera::Input { elapsed_time_ms: 32, ..Default::default() });
        let view = view_of(&camera);
        Drive {
            camera,
            previous_view: view,
            current_view: view,
            driver,
            coll,
            character,
            anim_num_frames,
            tick: 0,
            held: 0,
            previous: pose,
            current: pose,
            shadow_previous: None,
            shadow_current: None,
        }
    }

    /// The ground shadow between the last two ticks (`t` of the way to the
    /// latest): its centre, orientation and the height that scales it.
    pub fn shadow(&self, t: f32) -> Option<(Vec3, Mat3, f32)> {
        let now = self.shadow_current?;
        let before = self.shadow_previous.unwrap_or(now);
        let place = |s: &GroundShadow| Vec3::new(s.x as f32, s.y as f32, s.z as f32) * SCALE;
        let orient = |s: &GroundShadow| {
            let column = |c: usize| Vec3::new(s.axes.m[0][c] as f32, s.axes.m[1][c] as f32, s.axes.m[2][c] as f32) / 4096.0;
            let (up, forward) = (column(1).normalize_or_zero(), column(2).normalize_or_zero());
            Quat::from_mat3(&Mat3::from_cols(up.cross(forward), up, forward))
        };
        Some((
            place(&before).lerp(place(&now), t),
            Mat3::from_quat(orient(&before).slerp(orient(&now), t)),
            before.height as f32 + (now.height - before.height) as f32 * t,
        ))
    }

    /// `Driver::wheelRotation`: the front wheels' steering.
    pub fn wheel_rotation(&self) -> i16 {
        self.driver.wheel_rotation
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
    Pose {
        translation,
        rotation,
    }
}

pub struct DrivePlugin;

impl Plugin for DrivePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(crate::exhaust::Particles::new())
            .add_systems(Startup, spawn_hud)
            .add_systems(FixedUpdate, drive_tick)
            .add_systems(Update, (show_kart, chase, hud).chain())
            .add_systems(Update, crate::shadow::update)
            .add_systems(PostUpdate, crate::exhaust::update.before(TransformSystems::Propagate));
    }
}

fn spawn_hud(mut commands: Commands) {
    commands.spawn((
        Hud,
        Text::new(""),
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(8.0),
            left: Val::Px(8.0),
            ..default()
        },
    ));
}

fn read_input(keys: &ButtonInput<KeyCode>, pads: &Query<&Gamepad>, previous_held: u32) -> Input {
    let down = |codes: &[KeyCode]| codes.iter().any(|&k| keys.pressed(k));
    let pad_down = |codes: &[GamepadButton]| pads.iter().any(|p| p.any_pressed(codes.iter().copied()));
    // Analog left stick of the first pad that has it pushed, deadzone already applied.
    let stick = pads.iter().map(|p| p.left_stick()).find(|s| *s != Vec2::ZERO).unwrap_or(Vec2::ZERO);

    let mut held = 0;
    if down(&[KeyCode::KeyW, KeyCode::ArrowUp]) || pad_down(&[GamepadButton::South]) {
        held |= buttons::CROSS;
    }
    // Brake and reverse is only Square (West / S / Down). Pulling the left stick or the D-pad
    // down does nothing: many pads report the D-pad as the left stick, and on retail it does not
    // brake (it must not end a powerslide).
    let reverse = down(&[KeyCode::KeyS, KeyCode::ArrowDown]) || pad_down(&[GamepadButton::West]);
    if reverse {
        held |= buttons::SQUARE;
    }
    if down(&[KeyCode::Space, KeyCode::ShiftLeft, KeyCode::ShiftRight])
        || pad_down(&[GamepadButton::RightTrigger, GamepadButton::RightTrigger2])
    {
        held |= buttons::R1;
    }
    if down(&[KeyCode::KeyQ, KeyCode::ControlLeft]) || pad_down(&[GamepadButton::LeftTrigger, GamepadButton::LeftTrigger2]) {
        held |= buttons::L1;
    }
    let left = down(&[KeyCode::KeyA, KeyCode::ArrowLeft]) || pad_down(&[GamepadButton::DPadLeft]);
    let right = down(&[KeyCode::KeyD, KeyCode::ArrowRight]) || pad_down(&[GamepadButton::DPadRight]);
    // Sticks: 0 is left, 0x80 the middle, 0xff right (and back for LY).
    let axis = |v: f32| (0x80 as f32 + v * 127.0).round().clamp(0.0, 255.0) as u8;
    let stick_lx = match (left, right) {
        (true, false) => 0,
        (false, true) => 0xff,
        _ => axis(stick.x),
    };
    Input {
        buttons_held: held,
        buttons_tapped: held & !previous_held,
        stick_lx,
        stick_ly: 0x80,
        stick_ry: 0x80,
    }
}

/// One game tick of the sim.
fn drive_tick(
    mut particles: ResMut<crate::exhaust::Particles>,
    keys: Res<ButtonInput<KeyCode>>,
    pads: Query<&Gamepad>,
    track: Res<Track>,
    mut drive: ResMut<Drive>,
) {
    let drive = &mut *drive;
    if keys.pressed(KeyCode::KeyR) {
        let (driver, coll) = Drive::birth(&track, drive.character, drive.anim_num_frames);
        (drive.driver, drive.coll, drive.tick) = (driver, coll, 0);
        particles.list.clear();
        drive.camera = camera::CameraDc::default();
    }

    let input = read_input(&keys, &pads, drive.held);
    drive.held = input.buttons_held;
    let env = Env {
        elapsed_event_time: drive.tick as i32 * 32,
        timer: drive.tick,
        ..Env::default()
    };
    let mut effects = Vec::new();
    let result = tick(
        &mut drive.driver,
        &mut drive.coll,
        &track.0,
        &env,
        &input,
        &mut effects,
    );
    drive.tick += 1;
    particles.tick(&vehicle::exhaust::exhaust(&mut drive.driver));

    if result.mask_grab {
        let (driver, coll) = Drive::birth(&track, drive.character, drive.anim_num_frames);
        (drive.driver, drive.coll) = (driver, coll);
        drive.camera = camera::CameraDc::default();
    }

    // A boost pulls the camera back.
    if effects
        .iter()
        .any(|e| matches!(e, Effect::CameraFireSpeedZoom { .. }))
    {
        drive.camera.flags |= camera::flags::FIRE_SPEED_ZOOM;
    }
    let camera_input = camera::Input {
        elapsed_time_ms: env.elapsed_time_ms,
        ..Default::default()
    };
    drive.camera.follow_driver(&drive.driver, &track.0, &camera_input);

    drive.shadow_previous = drive.shadow_current;
    drive.shadow_current = ground_shadow(&drive.driver, &mut drive.coll.gte.clone());
    drive.previous = drive.current;
    drive.current = pose_of(&drive.driver);
    drive.previous_view = drive.current_view;
    drive.current_view = view_of(&drive.camera);
}

/// The kart between the last two ticks.
fn interpolated(drive: &Drive, fixed: &Time<Fixed>) -> Pose {
    let t = fixed.overstep_fraction();
    Pose {
        translation: drive
            .previous
            .translation
            .lerp(drive.current.translation, t),
        rotation: drive.previous.rotation.slerp(drive.current.rotation, t),
    }
}

fn show_kart(
    drive: Res<Drive>,
    fixed: Res<Time<Fixed>>,
    mut karts: Query<&mut Transform, With<Kart>>,
) {
    let pose = interpolated(&drive, &fixed);
    for mut transform in &mut karts {
        transform.translation = pose.translation;
        transform.rotation = pose.rotation;
    }
}

/// The camera of the sim, between the last two ticks.
fn chase(
    drive: Res<Drive>,
    fixed: Res<Time<Fixed>>,
    mut cameras: Query<&mut Transform, With<ChaseCamera>>,
) {
    let t = fixed.overstep_fraction();
    let (a, b) = (drive.previous_view, drive.current_view);
    let eye = a.eye.lerp(b.eye, t);
    let target = a.target.lerp(b.target, t);
    // The roll's sign (which way the horizon tilts) is a guess.
    let roll = (a.roll + (b.roll - a.roll) * t) * std::f32::consts::TAU;
    for mut transform in &mut cameras {
        transform.translation = eye;
        transform.look_at(target, Vec3::Y);
        transform.rotate_local_z(roll);
    }
}

fn hud(drive: Res<Drive>, mut text: Query<&mut Text, With<Hud>>) {
    let d = &drive.driver;
    let grounded = d.actions_flag_set & vehicle::actions::TOUCH_GROUND != 0;
    for mut text in &mut text {
        text.0 = format!(
            "speed {:5}  {:?}{}{}{}{}\nW/S gas/brake, A/D steer, Space/Q hop+drift, R restart",
            d.speed_approx,
            d.phys_state,
            if grounded { "" } else { "  airborne" },
            if d.wall_rub_timer != 0 { "  wall" } else { "" },
            if d.phys_state == vehicle::PhysState::PowerSlide {
                format!("  meter {} boosts {}", d.turbo_meter_room_left, d.num_boosts_success)
            } else if d.reserves != 0 {
                format!("  reserves {}", d.reserves)
            } else {
                String::new()
            },
            match d.last_drift_end {
                Some(why) => format!("  last drift ended: {why:?}"),
                None => String::new(),
            },
        );
    }
}
