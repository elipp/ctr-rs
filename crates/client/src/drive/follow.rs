//! The chase camera: the sim's camera between the last two ticks, orbited by the mouse.

use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;
use render::mesh::SCALE;

use super::{Drive, Kart};
use crate::command::CommandLine;

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
pub(super) struct View {
    eye: Vec3,
    target: Vec3,
    /// Turns, around the view direction.
    roll: f32,
}

impl View {
    /// The camera at this view, as `chase` places it (without the orbit of the debug keys).
    pub(super) fn transform(&self) -> Transform {
        let mut transform = Transform::from_translation(self.eye).looking_at(self.target, Vec3::Y);
        transform.rotate_local_z(self.roll * std::f32::consts::TAU);
        transform
    }
}

pub(super) fn view_of(camera: &camera::CameraDc) -> View {
    let at = |x: i32, y: i32, z: i32| Vec3::new(x as f32, y as f32, z as f32) * SCALE;
    let p = camera.pb_pos;
    // The push buffer looks along its rotation, away from where the camera
    // would sit behind the look-at point (it stays put while the mask holds the kart).
    let turns = |angle: i32| (angle & 0xfff) as f32 / 4096.0 * std::f32::consts::TAU;
    let (yaw, pitch) = (
        turns(camera.pb_rot.y as i32),
        turns(0x800 - camera.pb_rot.x as i32),
    );
    let back = Vec3::new(
        yaw.sin() * pitch.cos(),
        pitch.sin(),
        yaw.cos() * pitch.cos(),
    );
    let eye = at(p.x as i32, p.y as i32, p.z as i32);
    View {
        eye,
        target: eye - back * 256.0 * SCALE,
        roll: camera.pb_rot.z as f32 / 4096.0,
    }
}

/// How far the camera turns per pixel the mouse moves, in radians.
const ORBIT_SPEED: f32 = 0.004;

/// How far up or down the camera may be orbited from where the sim puts it, in radians.
const ORBIT_PITCH_MAX: f32 = 1.2;

/// The longest time between the two clicks of a double click, in seconds.
const DOUBLE_CLICK_SECS: f32 = 0.35;

/// How much the camera's distance from the kart changes per scroll line, as a factor, and the
/// nearest and farthest it can be, as factors of the distance the sim gives it.
const ZOOM_PER_LINE: f32 = 0.9;
const ZOOM_RANGE: (f32, f32) = (0.3, 5.0);

/// How the player has turned and pulled the camera away from the sim's.
pub(super) struct Orbit {
    /// Yaw and pitch, in radians.
    angles: Vec2,
    /// The distance from the kart, as a factor.
    zoom: f32,
    /// When the left button last went down, in seconds of the app's time.
    last_click: f32,
}

impl Default for Orbit {
    fn default() -> Orbit {
        Orbit {
            angles: Vec2::ZERO,
            zoom: 1.0,
            last_click: f32::NEG_INFINITY,
        }
    }
}

/// The camera of the sim, between the last two ticks, turned around the kart as the mouse drags
/// it (left button held and move) and pulled away or closer by the scroll wheel. A double click
/// or Home puts it back. This is the view only, the sim's camera (and the rollback) do not know
/// of it.
pub(super) fn chase(
    drive: Res<Drive>,
    fixed: Res<Time<Fixed>>,
    time: Res<Time<Real>>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    line: Res<CommandLine>,
    mut orbit: Local<Orbit>,
    karts: Query<&Transform, (With<Kart>, Without<ChaseCamera>)>,
    mut cameras: Query<&mut Transform, With<ChaseCamera>>,
) {
    let now = time.elapsed_secs();
    if buttons.just_pressed(MouseButton::Left) {
        if now - orbit.last_click <= DOUBLE_CLICK_SECS {
            *orbit = Orbit::default();
        } else {
            orbit.last_click = now;
        }
    }
    if buttons.pressed(MouseButton::Left) {
        orbit.angles.x -= motion.delta.x * ORBIT_SPEED;
        orbit.angles.y = (orbit.angles.y - motion.delta.y * ORBIT_SPEED)
            .clamp(-ORBIT_PITCH_MAX, ORBIT_PITCH_MAX);
    }
    if keys.just_pressed(KeyCode::Home) {
        *orbit = Orbit::default();
    }
    let lines = match scroll.unit {
        MouseScrollUnit::Line => -scroll.delta.y,
        MouseScrollUnit::Pixel => -scroll.delta.y / 40.0,
    };
    // The wheel scrolls the console while the command line is open.
    if lines != 0.0 && !line.is_open() {
        orbit.zoom = (orbit.zoom * ZOOM_PER_LINE.powf(lines)).clamp(ZOOM_RANGE.0, ZOOM_RANGE.1);
    }
    let t = fixed.overstep_fraction();
    let (a, b) = (drive.previous_view, drive.current_view);
    let mut eye = a.eye.lerp(b.eye, t);
    let mut target = a.target.lerp(b.target, t);
    if (orbit.angles != Vec2::ZERO || orbit.zoom != 1.0)
        && let Ok(kart) = karts.single()
    {
        let pivot = kart.translation;
        let right = (target - eye).cross(Vec3::Y).normalize_or_zero();
        let turn = Quat::from_axis_angle(Vec3::Y, orbit.angles.x)
            * Quat::from_axis_angle(right, orbit.angles.y);
        eye = pivot + turn * (eye - pivot) * orbit.zoom;
        target = pivot + turn * (target - pivot);
    }
    // The roll's sign (which way the horizon tilts) is a guess.
    let roll = (a.roll + (b.roll - a.roll) * t) * std::f32::consts::TAU;
    for mut transform in &mut cameras {
        transform.translation = eye;
        transform.look_at(target, Vec3::Y);
        transform.rotate_local_z(roll);
    }
}
