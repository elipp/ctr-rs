//! The bot paths (`Level::nav_paths`) drawn, for working on the bots (`race::bot`): always in the
//! viewer (`/debug viewer`), and in the race with `/debug navpaths` (a toggle; handiest with
//! `/debug spectate`).
//!
//! * `N` (in the viewer) cycles what is shown: every path, each one alone, none.
//!
//! Each path is a loop of its own color (0 green, 1 cyan, 2 magenta), with a dot on every frame
//! and a bigger white one on frame 0. Frames with flags get a mark: a yellow tick sideways for
//! `DRIFT_LEFT` / `DRIFT_RIGHT` (on the side the drift turns to), an orange post for `TURBO`, a
//! taller red-orange one for `SUPER_TURBO`, a white post for `JUMP` and a red cross for
//! `KILL_PLANE`. One frame is circled in white and its fields written at the bottom left (in the race, above the place): in the
//! viewer the one nearest the camera, in the race the one the watched bot's tracker is on. In the
//! race every other bot's tracker frame is circled in its path's color.

use bevy::prelude::*;
use loader::lev::{NAV_PATH_COUNT, NavFrame, NavPath};
use render::mesh::{SCALE, to_world};

use crate::Track;
use crate::command::Action;

/// Lifted off the frame's position (in game units), so that the lines are not in the road.
const LIFT: f32 = 24.0;

/// The length of the marks, in game units.
const MARK: f32 = 96.0;

const COLORS: [Color; NAV_PATH_COUNT] = [
    Color::srgb(0.2, 1.0, 0.3),
    Color::srgb(0.2, 0.8, 1.0),
    Color::srgb(1.0, 0.3, 0.9),
];

/// Which paths the viewer draws: all of them, one (by its index), or none.
#[derive(Resource, Clone, Copy, PartialEq, Eq)]
enum Shown {
    All,
    One(usize),
    None,
}

impl Shown {
    fn next(self) -> Self {
        match self {
            Shown::All => Shown::One(0),
            Shown::One(i) if i + 1 < NAV_PATH_COUNT => Shown::One(i + 1),
            Shown::One(_) => Shown::None,
            Shown::None => Shown::All,
        }
    }

    fn shows(self, path: usize) -> bool {
        match self {
            Shown::All => true,
            Shown::One(i) => i == path,
            Shown::None => false,
        }
    }
}

/// `/debug navpaths`: the race draws the paths too.
#[derive(Resource, Default)]
struct InRace(bool);

#[derive(Component)]
struct NavText;

pub struct NavPlugin;

impl Plugin for NavPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Shown::All)
            .init_resource::<InRace>()
            .add_systems(
                Update,
                (
                    toggle_in_race,
                    (toggle, draw_in_viewer)
                        .chain()
                        .run_if(not(crate::drive::driving)),
                    draw_in_race.run_if(crate::drive::driving),
                )
                    .chain(),
            );
    }
}

pub(crate) fn spawn_text(commands: Commands) {
    spawn_text_at(commands, Val::Px(8.0));
}

/// The race's: above the place, which has the bottom left.
pub(crate) fn spawn_race_text(commands: Commands) {
    spawn_text_at(commands, Val::Percent(30.0));
}

fn spawn_text_at(mut commands: Commands, bottom: Val) {
    commands.spawn((
        NavText,
        Text::new(""),
        Node {
            position_type: PositionType::Absolute,
            bottom,
            left: Val::Px(8.0),
            ..default()
        },
    ));
}

fn toggle(keys: Res<ButtonInput<KeyCode>>, mut shown: ResMut<Shown>) {
    if keys.just_pressed(KeyCode::KeyN) {
        *shown = shown.next();
    }
}

fn toggle_in_race(
    mut actions: MessageReader<Action>,
    mut in_race: ResMut<InRace>,
    mut toasts: MessageWriter<crate::toast::Toast>,
) {
    for action in actions.read() {
        if matches!(action, Action::DebugNavPaths) {
            in_race.0 = !in_race.0;
            let said = if in_race.0 { "on" } else { "off" };
            toasts.write(crate::toast::Toast::new(
                format!("(debug) bot paths in the race: {said}"),
                2000,
            ));
        }
    }
}

fn at(f: &NavFrame) -> Vec3 {
    Vec3::from(to_world(f.pos)) + Vec3::Y * LIFT * SCALE
}

/// Sideways on the ground plane, to the right of the way from `a` to `b`.
fn right_of(a: Vec3, b: Vec3) -> Vec3 {
    let d = b - a;
    Vec3::new(d.z, 0.0, -d.x).normalize_or_zero()
}

fn circle(gizmos: &mut Gizmos, f: &NavFrame, size: f32, color: Color) {
    gizmos.circle(
        Isometry3d::new(at(f), Quat::from_rotation_arc(Vec3::Z, Vec3::Y)),
        MARK * SCALE * size,
        color,
    );
}

fn draw_in_viewer(
    track: Res<Track>,
    shown: Res<Shown>,
    camera: Single<
        &Transform,
        (
            With<Camera3d>,
            Without<crate::OverlayCamera>,
            Without<crate::sky::SkyCamera>,
        ),
    >,
    mut text: Single<&mut Text, With<NavText>>,
    mut gizmos: Gizmos,
) {
    let paths = &track.0.nav_paths;
    if paths.iter().all(|p| p.frames.is_empty()) {
        text.0 = "no bot paths on this level".into();
        return;
    }
    if *shown == Shown::None {
        text.0 = "bot paths: hidden (N)".into();
        return;
    }
    let eye = camera.translation;
    // The nearest frame of the shown paths: its path, its index and the square of its distance.
    let mut nearest: Option<(usize, usize, f32)> = None;
    for (p, path) in paths.iter().enumerate() {
        if !shown.shows(p) || path.frames.is_empty() {
            continue;
        }
        draw_path(&mut gizmos, path, COLORS[p]);
        for (i, f) in path.frames.iter().enumerate() {
            let d2 = at(f).distance_squared(eye);
            if nearest.is_none_or(|(_, _, best)| d2 < best) {
                nearest = Some((p, i, d2));
            }
        }
    }
    let Some((p, i, _)) = nearest else {
        text.0 = "bot paths: this one is empty (N)".into();
        return;
    };
    circle(&mut gizmos, &paths[p].frames[i], 1.0, Color::WHITE);
    let shown_label = match *shown {
        Shown::All => "all".to_string(),
        Shown::One(k) => k.to_string(),
        Shown::None => unreachable!(),
    };
    text.0 = format!(
        "bot paths: {shown_label} (N)\n{}",
        describe(p, i, &paths[p])
    );
}

fn draw_in_race(
    track: Res<Track>,
    sim: Res<crate::props::Sim>,
    in_race: Res<InRace>,
    text: Option<Single<&mut Text, With<NavText>>>,
    mut gizmos: Gizmos,
) {
    let mut said = String::new();
    if in_race.0 {
        let paths = &track.0.nav_paths;
        for (p, path) in paths.iter().enumerate() {
            if !path.frames.is_empty() {
                draw_path(&mut gizmos, path, COLORS[p]);
            }
        }
        for (slot, kart) in sim.race.karts.iter().enumerate() {
            let Some(bot) = &kart.bot else {
                continue;
            };
            let path = &paths[bot.path as usize];
            let Some(f) = path.frames.get(bot.tracker.frame as usize) else {
                continue;
            };
            if slot == sim.own {
                circle(&mut gizmos, f, 1.0, Color::WHITE);
                said = format!(
                    "kart {slot}: {}{}\nrelocations {}  stuck {}  reversing {}  speed {}",
                    describe(bot.path as usize, bot.tracker.frame as usize, path),
                    bot.home
                        .map_or(String::new(), |(line, _)| format!("  (own line {line})")),
                    bot.tracker.relocations,
                    bot.stuck,
                    bot.reversing,
                    kart.driver.speed_approx,
                );
            } else {
                circle(&mut gizmos, f, 0.6, COLORS[bot.path as usize]);
            }
        }
    }
    if let Some(mut text) = text
        && text.0 != said
    {
        text.0 = said;
    }
}

/// The fields of frame `i` of path `p`.
fn describe(p: usize, i: usize, path: &NavPath) -> String {
    let f = &path.frames[i];
    format!(
        "path {p} frame {i}/{}  pos ({}, {}, {})  rot {:?}\nflags {:#06x}: {}  terrain {}\ndist {} (xz {})  path change {}  go back {}  special {:#04x}",
        path.frames.len(),
        f.pos.x,
        f.pos.y,
        f.pos.z,
        f.rot,
        f.flags,
        flag_names(f.flags),
        f.terrain(),
        f.dist_to_next_xyz,
        f.dist_to_next_xz,
        f.path_change_opcode,
        f.go_back_count,
        f.special_bits,
    )
}

fn draw_path(gizmos: &mut Gizmos, path: &NavPath, color: Color) {
    let frames = &path.frames;
    let n = frames.len();
    let mark = MARK * SCALE;
    let dim = color.with_alpha(0.6);
    for (i, f) in frames.iter().enumerate() {
        let a = at(f);
        let b = at(&frames[(i + 1) % n]);
        gizmos.line(a, b, color);
        if i == 0 {
            gizmos.sphere(Isometry3d::from_translation(a), mark * 0.6, Color::WHITE);
        } else {
            gizmos.sphere(Isometry3d::from_translation(a), mark * 0.12, dim);
        }
        let right = right_of(a, b);
        let up = Vec3::Y * mark;
        if f.flags & NavFrame::DRIFT_RIGHT != 0 {
            gizmos.line(a, a + right * mark, Color::srgb(1.0, 0.9, 0.1));
        }
        if f.flags & NavFrame::DRIFT_LEFT != 0 {
            gizmos.line(a, a - right * mark, Color::srgb(1.0, 0.9, 0.1));
        }
        if f.flags & NavFrame::SUPER_TURBO != 0 {
            gizmos.line(a, a + up * 2.0, Color::srgb(1.0, 0.3, 0.1));
        } else if f.flags & NavFrame::TURBO != 0 {
            gizmos.line(a, a + up, Color::srgb(1.0, 0.6, 0.1));
        }
        if f.flags & NavFrame::JUMP != 0 {
            gizmos.line(a, a + up * 1.5, Color::WHITE);
        }
        if f.flags & NavFrame::KILL_PLANE != 0 {
            let fwd = (b - a).normalize_or_zero() * mark * 0.5;
            let side = right * mark * 0.5;
            let red = Color::srgb(1.0, 0.1, 0.1);
            gizmos.line(a - fwd - side, a + fwd + side, red);
            gizmos.line(a - fwd + side, a + fwd - side, red);
        }
    }
}

fn flag_names(flags: u16) -> String {
    const NAMES: [(u16, &str); 11] = [
        (NavFrame::TURBO, "turbo"),
        (NavFrame::SKID_BACK, "skid-back"),
        (NavFrame::SKID_FRONT, "skid-front"),
        (NavFrame::SUPER_TURBO, "super-turbo"),
        (NavFrame::KILL_PLANE, "kill-plane"),
        (NavFrame::JUMP, "jump"),
        (NavFrame::DRIFT_LEFT, "drift-left"),
        (NavFrame::DRIFT_RIGHT, "drift-right"),
        (NavFrame::ECHO, "echo"),
        (0x4000, "0x4000"),
        (NavFrame::SPLIT_LINE, "split-line"),
    ];
    let names: Vec<_> = NAMES
        .iter()
        .filter(|(bit, _)| flags & bit != 0)
        .map(|(_, n)| *n)
        .collect();
    names.join(" ")
}
