//! Collision probe overlay: every frame, sweeps a segment from the camera
//! through `coll` (the port of `COLL_FIXED_*`) and draws what it hits.
//!
//! * `F` cycles the probe: straight down (like the game's player search) or
//!   along the view direction.
//! * `T` toggles which blocks are wanted: the game's ground/collision-surface
//!   mask, or anything.
//!
//! The white line is the probe, the yellow triangle the hit triangle, the
//! cyan line its plane normal and the red dot the hit point.

use bevy::prelude::*;
use coll::{Collision, search_flags};
use loader::lev::{SVec3, quad_flags};

use crate::Track;
use crate::mesh::{SCALE, to_world};

/// Length of the downward probe, in game units.
const DOWN_REACH: f32 = 0x800 as f32;
/// Length of the look-direction probe, in game units.
const LOOK_REACH: f32 = 0x1000 as f32;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Direction {
    Down,
    Look,
}

#[derive(Resource)]
struct Probe {
    collision: Collision,
    direction: Direction,
    /// Anything, instead of the game's ground / collision-surface mask.
    want_all: bool,
}

#[derive(Component)]
struct ProbeText;

pub struct ProbePlugin;

impl Plugin for ProbePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Probe {
            collision: Collision::default(),
            direction: Direction::Down,
            want_all: false,
        })
        .add_systems(Startup, spawn_text)
        .add_systems(Update, (toggles, run_probe).chain());
    }
}

fn spawn_text(mut commands: Commands) {
    commands.spawn((
        ProbeText,
        Text::new(""),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(8.0),
            left: Val::Px(8.0),
            ..default()
        },
    ));
}

fn toggles(keys: Res<ButtonInput<KeyCode>>, mut probe: ResMut<Probe>) {
    if keys.just_pressed(KeyCode::KeyF) {
        probe.direction = match probe.direction {
            Direction::Down => Direction::Look,
            Direction::Look => Direction::Down,
        };
    }
    if keys.just_pressed(KeyCode::KeyT) {
        probe.want_all = !probe.want_all;
    }
}

/// World space back to game units, saturating to the `s16` range.
fn to_game(p: Vec3) -> SVec3 {
    let q = |v: f32| (v / SCALE).round().clamp(i16::MIN as f32, i16::MAX as f32) as i16;
    SVec3 { x: q(p.x), y: q(p.y), z: q(p.z) }
}

fn from_game(p: SVec3) -> Vec3 {
    Vec3::from(to_world(p))
}

fn flag_names(flags: u16) -> String {
    const NAMES: [(u16, &str); 13] = [
        (quad_flags::REFLECT_SPLIT_LINE_1, "reflect1"),
        (quad_flags::LOW_GRAVITY, "low-gravity"),
        (quad_flags::REFLECT_SPLIT_LINE_0, "reflect0"),
        (quad_flags::NO_COLLISION_RESPONSE, "no-response"),
        (quad_flags::TRIGGER, "trigger"),
        (quad_flags::ENGINE_ECHO, "echo"),
        (quad_flags::KILL_PLANE, "kill-plane"),
        (quad_flags::DOOR, "door"),
        (quad_flags::CAMERA_SEARCH, "camera"),
        (quad_flags::GROUND, "ground"),
        (quad_flags::COLLISION_SURFACE, "surface"),
        (quad_flags::NO_CAMERA_RESPAWN_PROBE, "no-respawn-probe"),
        (quad_flags::SKIP_WATER_LIST, "skip-water"),
    ];
    let names: Vec<_> = NAMES.iter().filter(|(bit, _)| flags & bit != 0).map(|(_, n)| *n).collect();
    names.join(" ")
}

fn run_probe(
    track: Res<Track>,
    mut probe: ResMut<Probe>,
    camera: Single<&Transform, With<Camera3d>>,
    mut text: Single<&mut Text, With<ProbeText>>,
    mut gizmos: Gizmos,
) {
    let probe = &mut *probe;
    let level = &track.0;
    let start = camera.translation;
    let reach = match probe.direction {
        Direction::Down => Vec3::NEG_Y * DOWN_REACH * SCALE,
        Direction::Look => *camera.forward() * LOOK_REACH * SCALE,
    };
    let end = start + reach;

    let (wanted, ignored) = if probe.want_all {
        (0xffff, 0)
    } else {
        // `COLL_FIXED_PlayerSearch_SetupSearch`
        ((quad_flags::GROUND | quad_flags::COLLISION_SURFACE) as u32, quad_flags::NO_COLLISION_RESPONSE as u32)
    };

    let c = &mut probe.collision;
    let q = &mut c.sps.quad_block_coll;
    (q.search_flags, q.quad_flags_wanted, q.quad_flags_ignored) = (search_flags::HIGH_LOD, wanted, ignored);
    // The search runs from its second argument to its first.
    c.search_quadblocks(level, to_game(end), to_game(start), 0);

    gizmos.line(start - Vec3::Y * 0.05, end, Color::WHITE);

    let sps = &c.sps;
    let mut info = format!(
        "probe: {}  wanted: {}  (F / T to change)\ntriangles tested: {}",
        match probe.direction {
            Direction::Down => "down",
            Direction::Look => "look",
        },
        if probe.want_all { "anything" } else { "ground | surface" },
        sps.num_triangles_tested,
    );

    if let (true, Some(quad_index)) = (sps.did_touch_quadblock > 0, sps.hit.quad) {
        let quad = &level.quad_blocks[quad_index as usize];
        let hit = from_game(sps.hit.hit_pos);
        let n = sps.hit.plane.normal;
        let normal = Vec3::new(n.x as f32, n.y as f32, n.z as f32) / 4096.0;

        let corners = sps.hit_level_triangle.map(|v| from_game(level.vertices[v.expect("hit triangle") as usize].pos));
        gizmos.linestrip(
            [corners[0], corners[1], corners[2], corners[0]].map(|p| p + normal * 0.02),
            Color::srgb(1.0, 0.9, 0.1),
        );
        gizmos.line(hit, hit + normal * 2.0, Color::srgb(0.1, 0.9, 1.0));
        gizmos.sphere(Isometry3d::from_translation(hit), 0.08, Color::srgb(1.0, 0.1, 0.1));

        info += &format!(
            "\nhit: quad block {quad_index} (id {}), terrain {}, checkpoint {}\nflags {:#06x}: {}\nbarycentrics {} {}, hit {:?}, normal ({:.3}, {:.3}, {:.3}) x4096",
            quad.block_id,
            quad.terrain_type,
            quad.checkpoint_index,
            quad.flags,
            flag_names(quad.flags),
            sps.hit_barycentrics.v1,
            sps.hit_barycentrics.v2,
            (sps.hit.hit_pos.x, sps.hit.hit_pos.y, sps.hit.hit_pos.z),
            normal.x,
            normal.y,
            normal.z,
        );
    } else {
        info += "\nno hit";
    }
    if sps.collision.step_flags != 0 {
        info += &format!("\nstep flags {:#x} (trigger payload / water)", sps.collision.step_flags);
    }
    text.0 = info;
}
