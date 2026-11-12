//! Moving bot path frames in the viewer (`/debug viewer`), into `loader::nav_amend`'s file.
//!
//! * `E` turns editing on and off. While it is on, the left mouse button is the editor's (the
//!   right one still turns the camera).
//! * The frame under the cursor is circled; dragging it with the left button moves it along the
//!   ground, as the cursor's ray through the collision (`coll`) hits it.
//! * `-` / `=`: how many frames on either side go along, less the further they are (a cosine),
//!   so that a corner is taken wider in one drag and the line stays smooth.
//! * `1` jump, `2` drift left, `3` drift right, `4` turbo: toggles the flag of the frame under
//!   the cursor; with `Shift`, the frames that go along get it too (or lose it). A drift one way
//!   takes away the other.
//! * `Backspace`: the frame under the cursor and those that go along back where the level has
//!   them. `Z`: undoes the last drag or reset.
//! * `Ctrl+S` (or `Cmd+S`): saves every track's moves to `nav_amendments.ron` (compiled in, see
//!   `loader::nav_amend`).
//!
//! Each frame keeps how high it was above the ground. Where the level has a frame is drawn in
//! grey under the moved line.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use coll::{Collision, search_flags};
use loader::lev::{NAV_PATH_COUNT, NavFrame, NavPath, SVec3, quad_flags};
use loader::nav_amend::{Amendments, Flags, Move, SOURCE_PATH};
use render::mesh::{SCALE, to_world};

use crate::Track;
use crate::command::CommandLine;
use crate::nav::{Shown, at};

/// How far from the cursor a frame can be grabbed, in pixels.
const GRAB_PIXELS: f32 = 16.0;

/// How far the cursor's ray reaches, in game units.
const RAY_REACH: f32 = 0x3000 as f32;

/// Above and below a frame, how far the ground is looked for, in game units.
const GROUND_ABOVE: f32 = 0x100 as f32;
const GROUND_BELOW: f32 = 0x400 as f32;

/// The flags the number keys toggle, and the one that each takes away.
const FLAG_KEYS: [(KeyCode, u16, u16); 4] = [
    (KeyCode::Digit1, NavFrame::JUMP, 0),
    (KeyCode::Digit2, NavFrame::DRIFT_LEFT, NavFrame::DRIFT_RIGHT),
    (KeyCode::Digit3, NavFrame::DRIFT_RIGHT, NavFrame::DRIFT_LEFT),
    (KeyCode::Digit4, NavFrame::TURBO, 0),
];

/// The most frames on either side that go along.
const MAX_RADIUS: usize = 32;

#[derive(Resource)]
pub(crate) struct NavEdit {
    pub(crate) on: bool,
    /// Every track's moves, as they are edited: from the file when it is there (it may be newer
    /// than what was compiled in), else the compiled ones.
    amendments: Amendments,
    /// The level's own paths of the track that is loaded, and its number.
    original: Option<(u32, [NavPath; NAV_PATH_COUNT])>,
    /// Frames on either side that go along with the one dragged.
    radius: usize,
    /// The frame under the cursor: its path and index.
    pub(crate) hover: Option<(usize, usize)>,
    drag: Option<Drag>,
    undo: Vec<Amendments>,
    /// Moves not saved yet.
    dirty: bool,
    /// What the editor last said (a save, a refusal).
    note: String,
    collision: Collision,
}

struct Drag {
    path: usize,
    frame: usize,
    /// The moves before the drag, for [`NavEdit::undo`].
    before: Amendments,
    /// The frames that go along, with how far from the dragged one they are, where they were
    /// when the drag started and how high above the ground.
    frames: Vec<(usize, usize, SVec3, i32)>,
}

impl Default for NavEdit {
    fn default() -> Self {
        let amendments = match std::fs::read_to_string(SOURCE_PATH) {
            Ok(text) => Amendments::parse(&text).unwrap_or_else(|e| {
                warn!("{SOURCE_PATH}: {e}: the compiled-in bot path amendments are edited");
                Amendments::builtin().clone()
            }),
            Err(_) => Amendments::builtin().clone(),
        };
        NavEdit {
            on: false,
            amendments,
            original: None,
            radius: 4,
            hover: None,
            drag: None,
            undo: Vec::new(),
            dirty: false,
            note: String::new(),
            collision: Collision::default(),
        }
    }
}

impl NavEdit {
    /// Whether the left mouse button is the editor's, not the fly camera's.
    pub(crate) fn takes_left_button(&self) -> bool {
        self.on
    }

    /// The editor's line under the bot paths' text.
    pub(crate) fn status(&self) -> String {
        if !self.on {
            return "editing: off (E)".into();
        }
        let track = self.original.as_ref().map_or(0, |(id, _)| *id);
        let moved = self.amendments.of(track).count();
        let flagged = self.amendments.flags_of(track).count();
        format!(
            "editing (E): drag a frame  radius {} (-/=)  reset (Backspace)  undo (Z)  save (Ctrl+S)\n\
             flags: jump (1) drift left (2) right (3) turbo (4), Shift: the radius too\n\
             {moved} frames moved, {flagged} flagged here{}{}",
            self.radius,
            if self.dirty { ", not saved" } else { "" },
            if self.note.is_empty() {
                String::new()
            } else {
                format!("\n{}", self.note)
            },
        )
    }

    /// The track's paths: the level's own with the moves on top.
    fn rebuild(&mut self, track: &mut Track) {
        let Some((id, original)) = &self.original else {
            return;
        };
        let mut paths = original.clone();
        let refused = self.amendments.apply(*id, &mut paths);
        if !refused.is_empty() {
            self.note = format!("left out: {}", refused.join("; "));
        }
        track.0.nav_paths = paths;
    }

    /// Toggles `bit` on frame `i` of path `p` (taking `other` away when it is set), and on those
    /// around it when `spread`.
    fn toggle_flag(&mut self, track: &Track, (p, i): (usize, usize), bit: u16, other: u16, spread: bool) {
        let Some((id, original)) = &self.original else {
            return;
        };
        let (id, own) = (*id, &original[p].frames);
        let frames = &track.0.nav_paths[p].frames;
        if i >= frames.len() || own.len() != frames.len() {
            return;
        }
        let on = frames[i].flags & bit == 0;
        let targets = if spread {
            self.around(frames.len(), i)
        } else {
            vec![(i, 0)]
        };
        let changes: Vec<Flags> = targets
            .into_iter()
            .map(|(j, _)| {
                let now = frames[j].flags;
                let to = if on { (now | bit) & !other } else { now & !bit };
                Flags {
                    track: id,
                    path: p,
                    frame: j,
                    was: own[j].flags,
                    to,
                }
            })
            .collect();
        for c in changes {
            self.amendments.set_flags(c);
        }
    }

    /// The level's own position of a frame.
    fn original_pos(&self, path: usize, frame: usize) -> Option<SVec3> {
        Some(self.original.as_ref()?.1.get(path)?.frames.get(frame)?.pos)
    }

    /// The frames `radius` on either side of `frame`, the path going round, with how far they are.
    fn around(&self, len: usize, frame: usize) -> Vec<(usize, usize)> {
        let r = self.radius.min(len.saturating_sub(1) / 2);
        let mut out = vec![(frame, 0)];
        for k in 1..=r {
            out.push(((frame + k) % len, k));
            out.push(((frame + len - k) % len, k));
        }
        out
    }

    /// Where a segment (world space) first meets the ground, in game units.
    fn hit(&mut self, track: &Track, from: Vec3, to: Vec3) -> Option<SVec3> {
        let c = &mut self.collision;
        let q = &mut c.sps.quad_block_coll;
        // `COLL_FIXED_PlayerSearch_SetupSearch`, as the probe's "ground | surface".
        (q.search_flags, q.quad_flags_wanted, q.quad_flags_ignored) = (
            search_flags::HIGH_LOD,
            (quad_flags::GROUND | quad_flags::COLLISION_SURFACE) as u32,
            quad_flags::NO_COLLISION_RESPONSE as u32,
        );
        // The search runs from its second argument to its first.
        c.search_quadblocks(&track.0, to_game(to), to_game(from), 0);
        let sps = &c.sps;
        (sps.did_touch_quadblock > 0 && sps.hit.quad.is_some()).then_some(sps.hit.hit_pos)
    }

    /// The ground under (or a little above) `p`.
    fn ground(&mut self, track: &Track, p: SVec3) -> Option<i32> {
        let w = from_game(p);
        let hit = self.hit(
            track,
            w + Vec3::Y * GROUND_ABOVE * SCALE,
            w - Vec3::Y * GROUND_BELOW * SCALE,
        )?;
        Some(hit.y as i32)
    }
}

pub struct NavEditPlugin;

impl Plugin for NavEditPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<NavEdit>().add_systems(
            Update,
            (sync, keys, mouse, draw)
                .chain()
                .before(crate::nav::draw_in_viewer)
                .run_if(not(crate::drive::driving)),
        );
    }
}

/// A track that was loaded (or loaded again, by a race) gets the edited moves, not the compiled
/// ones. `Track` is changed without telling: what rebuilds on a new track has nothing to do.
fn sync(mut track: ResMut<Track>, mut edit: ResMut<NavEdit>) {
    let id = track.1.0;
    if !track.is_changed() && edit.original.as_ref().is_some_and(|(o, _)| *o == id) {
        return;
    }
    let Some(map) = crate::maps::get(id) else {
        return;
    };
    edit.original = Some((id, map.level.nav_paths.clone()));
    edit.hover = None;
    edit.drag = None;
    edit.note.clear();
    edit.rebuild(track.bypass_change_detection());
}

fn keys(
    keys: Res<ButtonInput<KeyCode>>,
    line: Option<Res<CommandLine>>,
    mut track: ResMut<Track>,
    mut edit: ResMut<NavEdit>,
    mut toasts: MessageWriter<crate::toast::Toast>,
) {
    if line.is_some_and(|l| l.is_open()) {
        return;
    }
    if keys.just_pressed(KeyCode::KeyE) {
        edit.on = !edit.on;
        edit.drag = None;
    }
    if !edit.on {
        return;
    }
    if keys.just_pressed(KeyCode::Minus) {
        edit.radius = edit.radius.saturating_sub(1);
    }
    if keys.just_pressed(KeyCode::Equal) {
        edit.radius = (edit.radius + 1).min(MAX_RADIUS);
    }
    let ctrl = keys.any_pressed([
        KeyCode::ControlLeft,
        KeyCode::ControlRight,
        KeyCode::SuperLeft,
        KeyCode::SuperRight,
    ]);
    if keys.just_pressed(KeyCode::KeyS) && ctrl {
        edit.note = match std::fs::write(SOURCE_PATH, edit.amendments.to_text()) {
            Ok(()) => {
                edit.dirty = false;
                format!("saved to {SOURCE_PATH}")
            }
            Err(e) => format!("not saved: {SOURCE_PATH}: {e}"),
        };
        toasts.write(crate::toast::Toast::new(edit.note.clone(), 3000));
    }
    if keys.just_pressed(KeyCode::KeyZ)
        && let Some(before) = edit.undo.pop()
    {
        edit.amendments = before;
        edit.dirty = true;
        edit.rebuild(track.bypass_change_detection());
    }
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    for (key, bit, other) in FLAG_KEYS {
        if keys.just_pressed(key)
            && let Some(hover) = edit.hover
        {
            let before = edit.amendments.clone();
            edit.toggle_flag(&track, hover, bit, other, shift);
            if edit.amendments != before {
                edit.undo.push(before);
                edit.dirty = true;
                edit.rebuild(track.bypass_change_detection());
            }
        }
    }
    if keys.just_pressed(KeyCode::Backspace)
        && let (Some((p, i)), Some((id, original))) = (edit.hover, &edit.original)
    {
        let (id, len) = (*id, original[p].frames.len());
        let before = edit.amendments.clone();
        for (j, _) in edit.around(len, i) {
            edit.amendments.clear(id, p, j);
        }
        if edit.amendments != before {
            edit.undo.push(before);
            edit.dirty = true;
            edit.rebuild(track.bypass_change_detection());
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn mouse(
    buttons: Res<ButtonInput<MouseButton>>,
    window: Option<Single<&Window, With<PrimaryWindow>>>,
    camera: Option<Single<(&Camera, &GlobalTransform), With<crate::FlyCamera>>>,
    shown: Res<Shown>,
    mut track: ResMut<Track>,
    mut edit: ResMut<NavEdit>,
) {
    let edit = &mut *edit;
    if !edit.on {
        edit.hover = None;
        return;
    }
    let (Some(window), Some(camera)) = (window, camera) else {
        return;
    };
    let (camera, eye) = *camera;
    let Some(cursor) = window.cursor_position() else {
        edit.hover = None;
        return;
    };

    if edit.drag.is_none() {
        // The frame nearest the cursor on the screen.
        let mut best: Option<((usize, usize), f32)> = None;
        for (p, path) in track.0.nav_paths.iter().enumerate() {
            if !shown.shows(p) {
                continue;
            }
            for (i, f) in path.frames.iter().enumerate() {
                let w = at(f);
                let Ok(screen) = camera.world_to_viewport(eye, w) else {
                    continue;
                };
                let px = screen.distance(cursor);
                if px <= GRAB_PIXELS && best.is_none_or(|(_, b)| px < b) {
                    best = Some(((p, i), px));
                }
            }
        }
        edit.hover = best.map(|(h, _)| h);
        if buttons.just_pressed(MouseButton::Left)
            && let Some((p, i)) = edit.hover
        {
            let len = track.0.nav_paths[p].frames.len();
            let frames = edit
                .around(len, i)
                .into_iter()
                .map(|(j, k)| {
                    let pos = track.0.nav_paths[p].frames[j].pos;
                    let clearance = edit.ground(&track, pos).map_or(0, |g| pos.y as i32 - g);
                    (j, k, pos, clearance)
                })
                .collect();
            edit.drag = Some(Drag {
                path: p,
                frame: i,
                before: edit.amendments.clone(),
                frames,
            });
        }
        return;
    }

    if !buttons.pressed(MouseButton::Left) {
        let drag = edit.drag.take().expect("dragging");
        if edit.amendments != drag.before {
            edit.undo.push(drag.before);
            edit.dirty = true;
        }
        return;
    }
    let Ok(ray) = camera.viewport_to_world(eye, cursor) else {
        return;
    };
    let from = ray.origin;
    let Some(hit) = edit.hit(&track, from, from + *ray.direction * RAY_REACH * SCALE) else {
        return;
    };
    let drag = edit.drag.as_ref().expect("dragging");
    let (p, grabbed) = (drag.path, drag.frame);
    let Some(&(_, _, start, _)) = drag.frames.iter().find(|(j, ..)| *j == grabbed) else {
        return;
    };
    let (dx, dz) = (hit.x as i32 - start.x as i32, hit.z as i32 - start.z as i32);
    let r = drag.frames.iter().map(|(_, k, ..)| *k).max().unwrap_or(0);
    let id = edit.original.as_ref().map_or(track.1.0, |(id, _)| *id);
    let frames = drag.frames.clone();
    for (j, k, start, clearance) in frames {
        let w = 0.5 * (1.0 + (std::f32::consts::PI * k as f32 / (r + 1) as f32).cos());
        let mut to = SVec3 {
            x: clamp16(start.x as i32 + (dx as f32 * w).round() as i32),
            y: start.y,
            z: clamp16(start.z as i32 + (dz as f32 * w).round() as i32),
        };
        let ground = if j == grabbed {
            Some(hit.y as i32)
        } else {
            edit.ground(&track, to)
        };
        if let Some(g) = ground {
            to.y = clamp16(g + clearance);
        }
        let Some(was) = edit.original_pos(p, j) else {
            continue;
        };
        edit.amendments.set(Move {
            track: id,
            path: p,
            frame: j,
            was,
            to,
        });
    }
    edit.rebuild(track.bypass_change_detection());
}

/// Where the level has the moved frames, in grey, and the frames that would go along.
fn draw(track: Res<Track>, shown: Res<Shown>, edit: Res<NavEdit>, mut gizmos: Gizmos) {
    let Some((_, original)) = &edit.original else {
        return;
    };
    if !edit.on {
        return;
    }
    let grey = Color::srgba(0.7, 0.7, 0.7, 0.7);
    for (p, (path, own)) in track.0.nav_paths.iter().zip(original).enumerate() {
        let n = path.frames.len();
        if !shown.shows(p) || n != own.frames.len() {
            continue;
        }
        for i in 0..n {
            let next = (i + 1) % n;
            let moved = |j: usize| path.frames[j].pos != own.frames[j].pos;
            if moved(i) || moved(next) {
                gizmos.line(at(&own.frames[i]), at(&own.frames[next]), grey);
            }
            if moved(i) {
                gizmos.line(at(&own.frames[i]), at(&path.frames[i]), grey.with_alpha(0.4));
            }
        }
    }
    let target = edit
        .drag
        .as_ref()
        .map(|d| (d.path, d.frame))
        .or(edit.hover);
    if let Some((p, i)) = target {
        let frames = &track.0.nav_paths[p].frames;
        for (j, k) in edit.around(frames.len(), i) {
            let size = if k == 0 { 0.5 } else { 0.25 };
            gizmos.sphere(
                Isometry3d::from_translation(at(&frames[j])),
                size * 96.0 * SCALE,
                Color::srgb(1.0, 0.9, 0.2),
            );
        }
    }
}

fn clamp16(v: i32) -> i16 {
    v.clamp(i16::MIN as i32, i16::MAX as i32) as i16
}

/// World space back to game units, saturating to the `s16` range.
fn to_game(p: Vec3) -> SVec3 {
    let q = |v: f32| (v / SCALE).round().clamp(i16::MIN as f32, i16::MAX as f32) as i16;
    SVec3 {
        x: q(p.x),
        y: q(p.y),
        z: q(p.z),
    }
}

fn from_game(p: SVec3) -> Vec3 {
    Vec3::from(to_world(p))
}
