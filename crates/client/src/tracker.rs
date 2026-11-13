//! The warning for a missile that chases our kart (`UI_TrackerSelf`): two brackets at the sides of
//! the kart, closing in as the missile does, and the lock-on icon four times around it. It
//! shrinks to the kart when the missile arrives, and flashes white with the beep.
//!
//! The HUD state steps with the game's ticks ([`step`]) and is drawn every frame at the kart's
//! interpolated place ([`show`]), by a camera of its own over the picture, an orthographic one
//! whose world unit is a pixel of the window.
//!
//! A warpball that chases us gets the same warning with blue brackets (`UI_TRACKER_TYPE_WARPBALL`),
//! and the brackets are left out while it is far along the track from us.
//!
//! Not here: the beep (`UI_TRACKER_BEEP_SFX`).

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::mesh::PrimitiveTopology;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use render::tracker::{lock_image, lock_size};
use loader::lev::CheckpointNode;
use weapon::Kind;
use weapon::warpball::flags::MASK_REPATH;

use crate::hud_view::{self, HudView};
use crate::kart::KartSlot;
use crate::hud::{atlas_image, pack_icons};
use crate::Track;
use crate::props::Sim;

/// `data.trackerAnim1`: the size of the figure (x, y; `0x1000` is 1) for each frame of the timer
/// while a missile chases the kart. It starts at 8 and counts down.
const ANIM_CHASED: [(i16, i16); 8] = [
    (0x1000, 0x1000),
    (0xe66, 0x10cc),
    (0xccc, 0x1199),
    (0x10d3, 0xe66),
    (0x14d9, 0xb33),
    (0x13b1, 0x12b5),
    (0x1000, 0x1a38),
    (0x8bb, 0x1063),
];

/// `data.trackerAnim2`: the same once the missile is gone, from 12 down. (Retail's 12 is one over
/// the table, a bug: its first frame reads the two numbers after it, which are the first of
/// `rankIconsDesired`, 0 and 1 unless the ranks have changed. That frame is drawn as nothing.)
const ANIM_ARRIVED: [(i16, i16); 12] = [
    (0xd09, 0x199),
    (0x1a12, 0x199),
    (0x2000, 0x199),
    (0x2000, 0x199),
    (0x2000, 0x199),
    (0x2000, 0x555),
    (0x2000, 0xc44),
    (0x2000, 0x1000),
    (0x2000, 0x1000),
    (0x1692, 0x13f8),
    (0xccc, 0x1666),
    (0, 1),
];

const TIMER_HIT_TARGET: i16 = 12;
const TIMER_NEW_TARGET: i16 = 8;
const DISTANCE_DIVISOR: u32 = 0x32;
const BEEP_RATE_NEAR: u32 = 5;
const BEEP_RATE_MID: u32 = 10;
const BEEP_RATE_FAR: u32 = 30;
const BEEP_DIST_NEAR_MAX: u32 = 100;
const BEEP_DIST_MID_MAX: u32 = 200;
const SIDE_HEIGHT_SCALE: i32 = 7;
const SIDE_HEIGHT_SHIFT: i32 = 12;
const SIDE_TIP_OFFSET: i32 = 12;
const BG_X_SHIFT: i32 = 7;
const BG_Y_SCALE: i32 = 0xf;
const BG_Y_SHIFT: i32 = 0xb;
const WARPBALL_DISTANCE_SCALE: i32 = 8;
const WARPBALL_DRAW_DIST_MAX: i32 = 16000;

/// Retail blends both with what is under them; here they are solid (the icon's clear texels are
/// cut out).
const ALPHA: f32 = 1.0;

/// The brackets' vertex colours as the game writes them (`0x30BBGGRR`, here as `[r, g, b]`):
/// the outer corners of the border, then of the fill.
struct Palette {
    border_outer: [u8; 3],
    border_tip: [u8; 3],
    fill_outer: [u8; 3],
    fill_center: [u8; 3],
    fill_tip: [u8; 3],
}

/// Orange.
const MISSILE: Palette = Palette {
    border_outer: [0x9c, 0x69, 0x00],
    border_tip: [0xff, 0xff, 0x00],
    fill_outer: [0x32, 0x2b, 0x01],
    fill_center: [0x5b, 0x5b, 0x00],
    fill_tip: [0xff, 0xbb, 0x00],
};

/// Blue.
const WARPBALL: Palette = Palette {
    border_outer: [0x00, 0x69, 0x9c],
    border_tip: [0x00, 0xff, 0xff],
    fill_outer: [0x01, 0x2b, 0x32],
    fill_center: [0x00, 0x5b, 0x5b],
    fill_tip: [0x00, 0xbb, 0xff],
};

const BORDER_CENTER: [u8; 3] = [0xff, 0xff, 0xff];

/// The render layer of the figure, which only the HUD camera (`scene.rs`) draws. (0 to 4 are taken.)
pub(crate) const LAYER: usize = 5;
const DEPTH: f32 = 100.0;

/// How far above the kart's place the figure is drawn, in pixels of the PlayStation's screen
/// (retail's sits nearer the middle of the screen).
const LIFT: f32 = 30.0;

/// What the HUD remembers of a driver between ticks (`trackerTimer`, `trackerDist` and the
/// `ACTION_TRACKER_TARGETED` bit, and the thread that was chasing).
#[derive(Resource, Default, Debug, PartialEq, Eq)]
pub struct Tracker {
    timer: i16,
    /// The distance that the brackets sit at, kept while the missile is gone.
    distance: i16,
    targeted: bool,
    /// `thTrackingMe`: the missile or warpball that chased on the last tick.
    chasing: Option<u32>,
    /// Its `flags`, as they are now if it is still in play, else as they were last seen (retail
    /// reads them from the thread's memory, which is not cleared).
    chasing_flags: u16,
    /// `trackerType` is `UI_TRACKER_TYPE_WARPBALL`: what last chased us was a warpball.
    warpball: bool,
    /// What to draw: None when nothing is going on.
    frame: Option<Frame>,
}

/// One tick's picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Frame {
    x: i16,
    y: i16,
    distance: i16,
    /// The icon flashes white instead of red, on the beat.
    white: bool,
    /// The brackets are blue.
    warpball: bool,
    /// The brackets are drawn (a warpball far along the track has only the icon).
    brackets: bool,
}

/// What chases us (`RB_GetThread_ClosestTracker`'s thread).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Chaser {
    id: u32,
    /// `distanceToTarget`
    squared: u32,
    flags: u16,
    /// A warpball: how far it is from us along the track, eight times the nodes' `distToFinish`.
    along: Option<i32>,
}

/// `VehCalc_FastSqrt(distanceToTarget)` over `UI_TRACKER_DISTANCE_DISPLAY_DIVISOR`, for the
/// closest missile or warpball that chases us. `tick` is the game's timer.
fn step_state(state: &mut Tracker, closest: Option<Chaser>, tick: u32) {
    // A missile that is fired at us makes itself the one that chases us (`VehPickupItem`'s
    // `thTrackingMe = RB_GetThread_ClosestTracker`), which is what wakes the HUD.
    if state.chasing.is_none()
        && let Some(c) = closest
    {
        state.chasing = Some(c.id);
        state.chasing_flags = c.flags;
    }
    if state.timer == 0 && state.chasing.is_none() {
        state.targeted = false;
        state.warpball = false;
        state.frame = None;
        return;
    }
    let table: &[(i16, i16)] = if state.chasing.is_none() {
        &ANIM_ARRIVED
    } else {
        &ANIM_CHASED
    };
    let (x, y) = table[state.timer as usize];
    let closest_id = closest.map(|c| c.id);
    let reset = if state.timer == 0 && closest.is_none() {
        // It hit; or it is a warpball that went back to the path when the mask grabbed its
        // target, and the warning stays.
        (state.chasing_flags & MASK_REPATH == 0).then_some(TIMER_HIT_TARGET)
    } else if (state.targeted && closest_id == state.chasing) || state.timer != 0 {
        None
    } else {
        state.targeted = true;
        Some(TIMER_NEW_TARGET)
    };
    if let Some(timer) = reset {
        state.timer = timer;
    }
    state.chasing = closest_id;
    if let Some(c) = closest {
        state.chasing_flags = c.flags;
    }
    if state.timer != 0 {
        state.timer -= 1;
    }
    let mut white = false;
    let mut along = 0;
    if let Some(c) = closest {
        state.warpball = c.along.is_some();
        along = c.along.unwrap_or(0);
        let distance = math::fast_sqrt(c.squared, 0) as u32 / DISTANCE_DIVISOR;
        state.distance = distance as i16;
        let rate = if distance > BEEP_DIST_MID_MAX {
            BEEP_RATE_FAR
        } else if distance > BEEP_DIST_NEAR_MAX {
            BEEP_RATE_MID
        } else {
            BEEP_RATE_NEAR
        };
        white = tick % rate == 0;
    }
    state.frame = Some(Frame {
        x,
        y,
        distance: state.distance,
        white,
        warpball: state.warpball,
        brackets: along < WARPBALL_DRAW_DIST_MAX,
    });
}

/// How far the warpball's node is behind ours along the track (what it has left to go to us),
/// around the lap if it is further in it than we are.
fn warpball_along(nodes: &[CheckpointNode], node_curr: usize, ours: usize) -> i32 {
    let dist = |i: usize| nodes.get(i).map_or(0, |n| n.dist_to_finish as i32);
    let along = (dist(node_curr) - dist(ours)) * WARPBALL_DISTANCE_SCALE;
    if along < 0 {
        along + dist(0) * WARPBALL_DISTANCE_SCALE
    } else {
        along
    }
}

/// `RB_GetThread_ClosestTracker`: the missile or warpball nearest to us of those that chase us,
/// by the squared flat distance of its place to the kart's (the first of equals; the missiles
/// come before the warpballs, as the sim ticks them).
fn closest_tracker(sim: &Sim, slot: usize, nodes: &[CheckpointNode]) -> Option<Chaser> {
    let body = weapon::Body::of(&sim.race.karts[slot].driver);
    let flat = |a: i32, b: i32| (a as u32).wrapping_sub(b as u32);
    let squared = |at: [i32; 3], pos: [i32; 3]| {
        let (dx, dz) = (flat(at[0], pos[0]), flat(at[2], pos[2]));
        dx.wrapping_mul(dx).wrapping_add(dz.wrapping_mul(dz))
    };
    let pos_curr = body.pos_curr.map(|v| v >> 8);
    let weapons = &sim.race.weapons;
    // The missile's `distanceToTarget` is to the kart's `posCurr`; the warpball keeps its own.
    let missiles = weapons
        .projectiles()
        .iter()
        .filter(|p| p.kind == Kind::Missile && p.target == Some(slot))
        .map(|p| {
            let chaser = Chaser {
                id: p.id,
                squared: squared(pos_curr, p.pos),
                flags: p.flags,
                along: None,
            };
            (p.pos, chaser)
        });
    let warpballs = weapons
        .warpballs()
        .iter()
        .filter(|w| w.target == Some(slot))
        .map(|w| {
            let chaser = Chaser {
                id: w.id,
                squared: w.distance_to_target,
                flags: w.flags,
                along: Some(warpball_along(nodes, w.node_curr(), body.checkpoint as usize)),
            };
            (w.pos, chaser)
        });
    let mut closest: Option<(i32, Chaser)> = None;
    for (pos, chaser) in missiles.chain(warpballs) {
        let nearness = squared(body.at, pos) as i32;
        if closest.is_none_or(|(best, _)| nearness < best) {
            closest = Some((nearness, chaser));
        }
    }
    closest.map(|(_, c)| c)
}

/// The `flags` of what chases us now, if it is still in play.
fn flags_of(sim: &Sim, id: u32) -> Option<u16> {
    let weapons = &sim.race.weapons;
    let missile = weapons.projectiles().iter().find(|p| p.id == id);
    let warpball = weapons.warpballs().iter().find(|w| w.id == id);
    missile.map(|p| p.flags).or(warpball.map(|w| w.flags))
}

/// What the HUD remembers for each local player, by view.
#[derive(Resource, Default)]
pub struct Trackers(Vec<Tracker>);

/// One game tick of the HUD.
pub fn step(
    sim: Res<Sim>,
    drive: Res<crate::drive::Drive>,
    track: Res<Track>,
    mut states: ResMut<Trackers>,
    mut last: Local<Option<u32>>,
) {
    let tick = sim.race.tick;
    let previous = last.replace(tick);
    if previous == Some(tick) {
        return;
    }
    // The race began again, or the players changed.
    let players = hud_view::views(&sim, &drive);
    if previous.is_some_and(|p| tick < p) || states.0.len() != players {
        states.0.clear();
        states.0.resize_with(players, Tracker::default);
    }
    for (view, state) in states.0.iter_mut().enumerate() {
        if let Some(flags) = state.chasing.and_then(|id| flags_of(&sim, id)) {
            state.chasing_flags = flags;
        }
        let slot = crate::hud_view::slot_of(&sim, &drive, view);
        let closest = closest_tracker(&sim, slot, &track.0.restart_points);
        step_state(state, closest, tick);
    }
}

/// What the figures of each view share: the materials, and the picture's size.
#[derive(Resource)]
struct Figure {
    brackets: Handle<StandardMaterial>,
    icon: Handle<StandardMaterial>,
    size: Vec2,
}

/// A figure of the view `HudView` says, in its own mesh (rewritten each frame): the brackets or
/// the lock-on icon.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum Part {
    Brackets,
    Icon,
}

pub struct TrackerPlugin;

impl Plugin for TrackerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Trackers>()
            .add_systems(FixedUpdate, step.run_if(crate::drive::driving))
            .add_systems(Update, show.run_if(crate::drive::driving));
    }
}

/// A mesh with the attributes the figure uses, as one triangle with no area: a mesh with no
/// vertices gets no render data, and the attributes that are set later would not change that.
fn mesh(textured: bool) -> Mesh {
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0f32; 3]; 3]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[0.0f32; 4]; 3]);
    if textured {
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0f32; 2]; 3]);
    }
    mesh
}

/// A colour as the PlayStation's GPU takes an untextured polygon's: the byte is the brightness.
fn gouraud(rgb: [u8; 3]) -> [f32; 4] {
    let c = Color::srgba_u8(rgb[0], rgb[1], rgb[2], 255).to_linear();
    [c.red, c.green, c.blue, ALPHA]
}

/// The brackets (`UI_TrackerSelf`'s four triangles), in pixels on the PlayStation's screen from
/// the kart's.
fn brackets(frame: Frame) -> Vec<([i32; 2], [f32; 4])> {
    if !frame.brackets {
        // One triangle with no area: a mesh with no vertices would lose its render data.
        return vec![([0, 0], [0.0; 4]); 3];
    }
    let p = if frame.warpball { &WARPBALL } else { &MISSILE };
    let (x, y, distance) = (frame.x as i32, frame.y as i32, frame.distance as i32);
    let outer = distance + (x >> 8);
    let height = (y * SIDE_HEIGHT_SCALE) >> SIDE_HEIGHT_SHIFT;
    let mut out = Vec::new();
    for side in [-1, 1] {
        let (outer_x, inner_x) = (side * outer, side * distance);
        let tip = -SIDE_TIP_OFFSET;
        // The border: the outer edge, from the top down to the tip's line.
        out.push(([outer_x, tip - height], gouraud(p.border_outer)));
        out.push(([inner_x, tip], gouraud(BORDER_CENTER)));
        out.push(([outer_x, tip], gouraud(p.border_tip)));
        // The fill, under it.
        out.push(([outer_x, tip], gouraud(p.fill_outer)));
        out.push(([inner_x, tip], gouraud(p.fill_center)));
        out.push(([outer_x, tip + height], gouraud(p.fill_tip)));
    }
    out
}

/// The four mirrored quads of the icon (`UI_TrackerBG`): `(offset, uv)` of a triangle's corners.
fn icon_quads(size: (usize, usize), frame: Frame) -> Vec<([i32; 2], [f32; 2])> {
    let (x, y) = (frame.x as i32, frame.y as i32);
    let (width, height) = ((size.0 as i32 * x) >> 12, (size.1 as i32 * y) >> 12);
    let (center_x, center_y) = (-(x >> BG_X_SHIFT), -((y * BG_Y_SCALE) >> BG_Y_SHIFT));
    let right = center_x + width;
    let left = right - (x >> 12);
    let bottom = center_y + height;
    let top = bottom - (y >> 12);
    let alt_x = center_x + width * 2 - (x >> 12);
    let alt_y = center_y + height * 2 - (y >> 12);
    let mut out = Vec::new();
    for quad in 0..4 {
        let (x0, x1) = match quad {
            1 | 3 => (alt_x, left),
            _ => (center_x, right),
        };
        let (y0, y1) = match quad {
            2 | 3 => (alt_y, top),
            _ => (center_y, bottom),
        };
        let corner = |cx: i32, cy: i32, u: f32, v: f32| ([cx, cy], [u, v]);
        let (a, b, c, d) = (
            corner(x0, y0, 0.0, 0.0),
            corner(x1, y0, 1.0, 0.0),
            corner(x0, y1, 0.0, 1.0),
            corner(x1, y1, 1.0, 1.0),
        );
        out.extend([a, b, c, b, d, c]);
    }
    out
}

fn show(
    mut commands: Commands,
    states: Res<Trackers>,
    drive: Res<crate::drive::Drive>,
    textures: Res<crate::Textures>,
    figure: Option<Res<Figure>>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    screen: Res<crate::pixel::RenderSize>,
    cameras: Query<(&crate::drive::MainView, &Camera, &GlobalTransform)>,
    karts: Query<(&KartSlot, &Transform)>,
    mut parts: Query<(&HudView, &Part, &Mesh3d, &mut Visibility)>,
    sim: Res<Sim>,
) {
    let Some(figure) = figure else {
        let icons = pack_icons();
        let Some(size) = lock_size(&icons) else {
            return;
        };
        let image = atlas_image(lock_image(&icons, &textures.vram), &mut images);
        let unlit = |texture| StandardMaterial {
            base_color_texture: texture,
            unlit: true,
            cull_mode: None,
            double_sided: true,
            alpha_mode: AlphaMode::Mask(0.5),
            ..default()
        };
        commands.insert_resource(Figure {
            brackets: materials.add(unlit(None)),
            icon: materials.add(unlit(Some(image))),
            size: Vec2::new(size.0 as f32, size.1 as f32),
        });
        return;
    };
    // The figures of each view, made when there are none: one pair, in the same layer as the
    // others', which the one camera over the whole window draws.
    let missing = hud_view::missing(
        &sim,
        &drive,
        parts
            .iter()
            .filter(|(_, part, ..)| **part == Part::Brackets)
            .map(|(view, ..)| *view),
    );
    for view in missing {
        for (part, mesh_, material) in [
            (Part::Brackets, mesh(false), &figure.brackets),
            (Part::Icon, mesh(true), &figure.icon),
        ] {
            commands.spawn((
                part,
                HudView(view),
                // The meshes are rewritten every frame; their bounds would go stale.
                NoFrustumCulling,
                Mesh3d(meshes.add(mesh_)),
                MeshMaterial3d(material.clone()),
                Transform::default(),
                Visibility::Hidden,
                RenderLayers::layer(LAYER),
            ));
        }
    }
    let Ok(window) = windows.single() else {
        return;
    };
    let players = drive.views().len();
    for (&HudView(view), part, mesh_handle, mut visibility) in &mut parts {
        let frame = states.0.get(view).and_then(|state| state.frame);
        let anchor = (|| {
            let frame = frame?;
            let slot = hud_view::slot_of(&sim, &drive, view);
            let (_, camera, camera_transform) = cameras.iter().find(|(m, ..)| m.0 == view)?;
            let (_, kart) = karts.iter().find(|(k, _)| k.0 == slot)?;
            // (The camera draws into a part of the picture of the scene, which may be smaller
            // than the window: the point is in the part, which is then in the picture.)
            let origin = camera
                .viewport
                .as_ref()
                .map_or(Vec2::ZERO, |v| v.physical_position.as_vec2() / screen.scale_factor);
            let at = screen.to_window(
                origin
                    + camera
                        .world_to_viewport(camera_transform, kart.translation)
                        .ok()?,
            );
            Some((frame, at))
        })();
        let Some((frame, at)) = anchor else {
            visibility.set_if_neq(Visibility::Hidden);
            continue;
        };
        visibility.set_if_neq(Visibility::Inherited);
        // The picture is 512 by 216 on the PlayStation, stretched over the player's part of the
        // window.
        let rect = crate::drive::window_rect(view, players, window);
        let scale = Vec2::new(rect.width() / 512.0, rect.height() / 216.0);
        let place = |offset: [i32; 2]| {
            let screen = at + Vec2::new(offset[0] as f32, offset[1] as f32 - LIFT) * scale;
            [
                screen.x - window.width() / 2.0,
                window.height() / 2.0 - screen.y,
                -DEPTH,
            ]
        };
        let Some(mut mesh) = meshes.get_mut(&mesh_handle.0) else {
            continue;
        };
        match part {
            Part::Brackets => {
                let triangles = brackets(frame);
                mesh.insert_attribute(
                    Mesh::ATTRIBUTE_POSITION,
                    triangles.iter().map(|(p, _)| place(*p)).collect::<Vec<_>>(),
                );
                mesh.insert_attribute(
                    Mesh::ATTRIBUTE_COLOR,
                    triangles.iter().map(|(_, c)| *c).collect::<Vec<_>>(),
                );
            }
            Part::Icon => {
                let quads = icon_quads((figure.size.x as usize, figure.size.y as usize), frame);
                // The icon is red, and white on the beat; the game's colour byte is a multiplier
                // with 0x80 neutral, so both are twice as bright as the picture.
                let tint = if frame.white {
                    [2.0, 2.0, 2.0, ALPHA]
                } else {
                    [2.0, 0.0, 0.0, ALPHA]
                };
                mesh.insert_attribute(
                    Mesh::ATTRIBUTE_POSITION,
                    quads.iter().map(|(p, _)| place(*p)).collect::<Vec<_>>(),
                );
                mesh.insert_attribute(
                    Mesh::ATTRIBUTE_UV_0,
                    quads.iter().map(|(_, uv)| *uv).collect::<Vec<_>>(),
                );
                mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![tint; quads.len()]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn missile(id: u32, squared: u32) -> Option<Chaser> {
        Some(Chaser {
            id,
            squared,
            flags: 0,
            along: None,
        })
    }

    fn warpball(flags: u16, along: i32) -> Option<Chaser> {
        Some(Chaser {
            id: 9,
            squared: 400 * 400 * 2500,
            flags,
            along: Some(along),
        })
    }

    /// Runs ticks with `chaser` chasing for the first `chase` of them.
    fn run_with(chaser: Option<Chaser>, chase: u32, ticks: u32) -> Vec<Option<(i16, i16)>> {
        let mut state = Tracker::default();
        (0..ticks)
            .map(|tick| {
                step_state(&mut state, chaser.filter(|_| tick < chase), tick);
                state.frame.map(|f| (f.x, f.y))
            })
            .collect()
    }

    fn run(chase: u32, ticks: u32) -> Vec<Option<(i16, i16)>> {
        run_with(missile(7, 400 * 400 * 2500), chase, ticks)
    }

    #[test]
    fn nothing_is_drawn_without_a_missile() {
        assert!(run(0, 5).iter().all(Option::is_none));
    }

    #[test]
    fn a_new_missile_plays_the_chased_animation_and_then_holds_its_first_frame() {
        let frames = run(30, 12);
        // The first tick reads timer 0, then 7 down to 0.
        assert_eq!(frames[0], Some(ANIM_CHASED[0]));
        assert_eq!(frames[1], Some(ANIM_CHASED[7]));
        assert_eq!(frames[8], Some(ANIM_CHASED[0]));
        assert_eq!(frames[11], Some(ANIM_CHASED[0]));
    }

    #[test]
    fn the_arrival_plays_to_the_end_and_goes() {
        let frames = run(10, 40);
        // The missile is gone at tick 10; the first frame reads timer 0 of the chased table,
        // and the arrival counts down from 12 (its first frame is the off-the-table one). Timer 0
        // draws nothing, so the last frame is the table's second.
        assert_eq!(frames[10], Some(ANIM_CHASED[0]));
        assert_eq!(frames[11], Some((0, 1)));
        assert_eq!(frames[12], Some(ANIM_ARRIVED[10]));
        assert_eq!(frames[21], Some(ANIM_ARRIVED[1]));
        assert!(frames[22..].iter().all(Option::is_none));
    }

    #[test]
    fn the_brackets_close_in_with_the_missile_and_beep_faster() {
        let mut state = Tracker::default();
        step_state(&mut state, missile(1, 5000 * 5000), 0);
        // 5000 / 50 = 100 away: the quick beat, and white on tick 0 and 5.
        assert_eq!(state.frame.unwrap().distance, 100);
        assert!(state.frame.unwrap().white);
        step_state(&mut state, missile(1, 5000 * 5000), 3);
        assert!(!state.frame.unwrap().white);
        step_state(&mut state, missile(1, 5000 * 5000), 5);
        assert!(state.frame.unwrap().white);
        step_state(&mut state, missile(1, 20000 * 20000), 5);
        assert!(!state.frame.unwrap().white);
        step_state(&mut state, missile(1, 20000 * 20000), 30);
        assert!(state.frame.unwrap().white);
    }

    #[test]
    fn the_icon_is_four_mirrored_quads() {
        let quads = icon_quads(
            (32, 30),
            Frame {
                x: 0x1000,
                y: 0x1000,
                distance: 0,
                white: false,
                warpball: false,
                brackets: true,
            },
        );
        assert_eq!(quads.len(), 24);
        let xs = quads.iter().map(|(p, _)| p[0]);
        // 32 texels each way from -32: centered on the kart at the plain size.
        assert_eq!(xs.clone().min(), Some(-32));
        assert_eq!(xs.max(), Some(31));
    }

    #[test]
    fn a_warpball_has_blue_brackets_and_none_when_far_along_the_track() {
        let mut state = Tracker::default();
        step_state(&mut state, warpball(0, 100), 0);
        let frame = state.frame.unwrap();
        assert!(frame.warpball && frame.brackets);
        step_state(&mut state, warpball(0, WARPBALL_DRAW_DIST_MAX), 1);
        assert!(!state.frame.unwrap().brackets);
        // Blue through the arrival, too.
        step_state(&mut state, None, 2);
        assert!(state.frame.unwrap().warpball);
    }

    #[test]
    fn a_warpball_that_lost_its_target_to_the_mask_skips_the_arrival() {
        let frames = run_with(warpball(MASK_REPATH, 100), 10, 14);
        // The tick it stops chasing us still draws, and then the warning is gone.
        assert_eq!(frames[10], Some(ANIM_CHASED[0]));
        assert!(frames[11..].iter().all(Option::is_none));
    }

    #[test]
    fn the_warpball_counts_around_the_lap_when_it_is_further_in_it() {
        let node = |dist_to_finish| CheckpointNode {
            pos: loader::lev::SVec3 { x: 0, y: 0, z: 0 },
            dist_to_finish,
            next_forward: 0,
            next_left: 0xff,
            next_backward: 0,
            next_right: 0xff,
        };
        let nodes = [node(1000), node(750), node(500), node(250)];
        // Behind us: from 750 to 500.
        assert_eq!(warpball_along(&nodes, 1, 2), 250 * 8);
        // Further in the lap than us: it comes around the lap.
        assert_eq!(warpball_along(&nodes, 3, 2), (250 - 500 + 1000) * 8);
    }
}
