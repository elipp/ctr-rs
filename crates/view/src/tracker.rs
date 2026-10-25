//! The warning for a missile that chases our kart (`UI_TrackerSelf`): two brackets at the sides of
//! the kart, closing in as the missile does, and the lock-on icon four times around it. It
//! shrinks to the kart when the missile arrives, and flashes white with the beep.
//!
//! The HUD state steps with the game's ticks ([`step`]) and is drawn every frame at the kart's
//! interpolated place ([`show`]), by a camera of its own over the picture, an orthographic one
//! whose world unit is a pixel of the window.
//!
//! Not here: the beep (`UI_TRACKER_BEEP_SFX`), and the electric ball (the warpball is not in the
//! sim), whose brackets are blue.

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::camera::{ClearColorConfig, ScalingMode};
use bevy::mesh::PrimitiveTopology;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use render::tracker::{lock_image, lock_size};
use weapon::Kind;

use crate::drive::{ChaseCamera, Kart};
use crate::leaderboard::{atlas_image, pack_icons};
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

/// Retail blends both with what is under them; here they are solid (the icon's clear texels are
/// cut out).
const ALPHA: f32 = 1.0;

/// The brackets' vertex colours as the game writes them (`0x30BBGGRR`, here as `[r, g, b]`):
/// the outer corners of the border, then of the fill, for a missile.
const BORDER_OUTER: [u8; 3] = [0x9c, 0x69, 0x00];
const BORDER_TIP: [u8; 3] = [0xff, 0xff, 0x00];
const BORDER_CENTER: [u8; 3] = [0xff, 0xff, 0xff];
const FILL_OUTER: [u8; 3] = [0x32, 0x2b, 0x01];
const FILL_CENTER: [u8; 3] = [0x5b, 0x5b, 0x00];
const FILL_TIP: [u8; 3] = [0xff, 0xbb, 0x00];

/// The render layer of the figure, which only its camera draws. (0 to 4 are taken.)
const LAYER: usize = 5;
const DEPTH: f32 = 100.0;

/// What the HUD remembers of a driver between ticks (`trackerTimer`, `trackerDist` and the
/// `ACTION_TRACKER_TARGETED` bit, and the thread that was chasing).
#[derive(Resource, Default, Debug, PartialEq, Eq)]
pub struct Tracker {
    timer: i16,
    /// The distance that the brackets sit at, kept while the missile is gone.
    distance: i16,
    targeted: bool,
    /// `thTrackingMe`: the missile that chased on the last tick.
    chasing: Option<u32>,
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
}

/// `VehCalc_FastSqrt(distanceToTarget)` over `UI_TRACKER_DISTANCE_DISPLAY_DIVISOR`, for the
/// closest missile `(id, squared distance)` that chases us. `tick` is the game's timer.
fn step_state(state: &mut Tracker, closest: Option<(u32, u32)>, tick: u32) {
    // A missile that is fired at us makes itself the one that chases us (`VehPickupItem`'s
    // `thTrackingMe = RB_GetThread_ClosestTracker`), which is what wakes the HUD.
    if state.chasing.is_none() {
        state.chasing = closest.map(|(id, _)| id);
    }
    if state.timer == 0 && state.chasing.is_none() {
        state.targeted = false;
        state.frame = None;
        return;
    }
    let table: &[(i16, i16)] = if state.chasing.is_none() {
        &ANIM_ARRIVED
    } else {
        &ANIM_CHASED
    };
    let (x, y) = table[state.timer as usize];
    let closest_id = closest.map(|(id, _)| id);
    let reset = if state.timer == 0 && closest.is_none() {
        // It hit.
        Some(TIMER_HIT_TARGET)
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
    if state.timer != 0 {
        state.timer -= 1;
    }
    let mut white = false;
    if let Some((_, squared)) = closest {
        let distance = math::fast_sqrt(squared, 0) as u32 / DISTANCE_DIVISOR;
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
    });
}

/// `RB_GetThread_ClosestTracker`: the missile nearest to us of those that chase us, by the
/// squared flat distance of its place to the kart's (the first of equals), and
/// `distanceToTarget` of it, which is to the kart's `posCurr`.
fn closest_missile(sim: &Sim) -> Option<(u32, u32)> {
    let driver = sim.driver();
    let body = weapon::Body::of(driver);
    let mut closest: Option<(i32, (u32, u32))> = None;
    for p in sim.race.weapons.projectiles() {
        if p.kind != Kind::Missile || p.target != Some(sim.own) {
            continue;
        }
        let flat = |a: i32, b: i32| (a as u32).wrapping_sub(b as u32);
        let (dx, dz) = (flat(body.at[0], p.pos[0]), flat(body.at[2], p.pos[2]));
        let nearness = dx.wrapping_mul(dx).wrapping_add(dz.wrapping_mul(dz)) as i32;
        if closest.is_none_or(|(best, _)| nearness < best) {
            let (dx, dz) = (
                flat(body.pos_curr[0] >> 8, p.pos[0]),
                flat(body.pos_curr[2] >> 8, p.pos[2]),
            );
            let squared = dx.wrapping_mul(dx).wrapping_add(dz.wrapping_mul(dz));
            closest = Some((nearness, (p.id, squared)));
        }
    }
    closest.map(|(_, c)| c)
}

/// One game tick of the HUD.
pub fn step(sim: Res<Sim>, mut state: ResMut<Tracker>, mut last: Local<Option<u32>>) {
    let tick = sim.race.tick;
    let previous = last.replace(tick);
    if previous == Some(tick) {
        return;
    }
    // The race began again.
    if previous.is_some_and(|p| tick < p) {
        *state = Tracker::default();
    }
    let closest = closest_missile(&sim);
    step_state(&mut state, closest, tick);
}

/// The two meshes, kept to be rewritten each frame.
#[derive(Resource)]
struct Figure {
    brackets: Handle<Mesh>,
    icon: Handle<Mesh>,
    size: Vec2,
}

#[derive(Component)]
struct Part;

pub struct TrackerPlugin;

impl Plugin for TrackerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Tracker>()
            .add_systems(FixedUpdate, step)
            .add_systems(Update, show);
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
    let (x, y, distance) = (frame.x as i32, frame.y as i32, frame.distance as i32);
    let outer = distance + (x >> 8);
    let height = (y * SIDE_HEIGHT_SCALE) >> SIDE_HEIGHT_SHIFT;
    let mut out = Vec::new();
    for side in [-1, 1] {
        let (outer_x, inner_x) = (side * outer, side * distance);
        let tip = -SIDE_TIP_OFFSET;
        // The border: the outer edge, from the top down to the tip's line.
        out.push(([outer_x, tip - height], gouraud(BORDER_OUTER)));
        out.push(([inner_x, tip], gouraud(BORDER_CENTER)));
        out.push(([outer_x, tip], gouraud(BORDER_TIP)));
        // The fill, under it.
        out.push(([outer_x, tip], gouraud(FILL_OUTER)));
        out.push(([inner_x, tip], gouraud(FILL_CENTER)));
        out.push(([outer_x, tip + height], gouraud(FILL_TIP)));
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
    state: Res<Tracker>,
    textures: Res<crate::Textures>,
    figure: Option<Res<Figure>>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform), With<ChaseCamera>>,
    karts: Query<&Transform, (With<Kart>, Without<ChaseCamera>)>,
    mut parts: Query<&mut Visibility, With<Part>>,
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
        let brackets = meshes.add(mesh(false));
        let icon = meshes.add(mesh(true));
        commands.insert_resource(Figure {
            brackets: brackets.clone(),
            icon: icon.clone(),
            size: Vec2::new(size.0 as f32, size.1 as f32),
        });
        commands.spawn((
            crate::OverlayCamera,
            Camera3d::default(),
            // Over the picture of the cameras before it, with only the depth buffer cleared.
            Camera {
                order: 4,
                clear_color: ClearColorConfig::None,
                ..default()
            },
            Projection::Orthographic(OrthographicProjection {
                scaling_mode: ScalingMode::WindowSize,
                ..OrthographicProjection::default_3d()
            }),
            Transform::default(),
            RenderLayers::layer(LAYER),
        ));
        for (mesh, material) in [
            (brackets, materials.add(unlit(None))),
            (icon, materials.add(unlit(Some(image)))),
        ] {
            commands.spawn((
                Part,
                // The meshes are rewritten every frame; their bounds would go stale.
                NoFrustumCulling,
                Mesh3d(mesh),
                MeshMaterial3d(material),
                Transform::default(),
                Visibility::Hidden,
                RenderLayers::layer(LAYER),
            ));
        }
        return;
    };
    let anchor = (|| {
        let frame = state.frame?;
        let window = windows.single().ok()?;
        let (camera, camera_transform) = cameras.single().ok()?;
        let kart = karts.single().ok()?;
        let at = camera
            .world_to_viewport(camera_transform, kart.translation)
            .ok()?;
        Some((frame, window, at))
    })();
    let Some((frame, window, at)) = anchor else {
        for mut visibility in &mut parts {
            *visibility = Visibility::Hidden;
        }
        return;
    };
    // The picture is 512 by 216 on the PlayStation, stretched over the window.
    let scale = Vec2::new(window.width() / 512.0, window.height() / 216.0);
    let place = |offset: [i32; 2]| {
        let screen = at + Vec2::new(offset[0] as f32, offset[1] as f32) * scale;
        [
            screen.x - window.width() / 2.0,
            window.height() / 2.0 - screen.y,
            -DEPTH,
        ]
    };
    let triangles = brackets(frame);
    if let Some(mut mesh) = meshes.get_mut(&figure.brackets) {
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            triangles.iter().map(|(p, _)| place(*p)).collect::<Vec<_>>(),
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_COLOR,
            triangles.iter().map(|(_, c)| *c).collect::<Vec<_>>(),
        );
    }
    let quads = icon_quads((figure.size.x as usize, figure.size.y as usize), frame);
    if let Some(mut mesh) = meshes.get_mut(&figure.icon) {
        // The icon is red, and white on the beat; the game's colour byte is a multiplier with
        // 0x80 neutral, so both are twice as bright as the picture.
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
    for mut visibility in &mut parts {
        *visibility = Visibility::Inherited;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs ticks with the missile `chasing` for the first `chase` of them.
    fn run(chase: u32, ticks: u32) -> Vec<Option<(i16, i16)>> {
        let mut state = Tracker::default();
        (0..ticks)
            .map(|tick| {
                let missile = (tick < chase).then_some((7, 400 * 400 * 2500));
                step_state(&mut state, missile, tick);
                state.frame.map(|f| (f.x, f.y))
            })
            .collect()
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
        // and the arrival counts down from 12 (its first frame is the off-the-table one).
        assert_eq!(frames[10], Some(ANIM_CHASED[0]));
        assert_eq!(frames[11], Some((0, 1)));
        assert_eq!(frames[12], Some(ANIM_ARRIVED[10]));
        assert_eq!(frames[21], Some(ANIM_ARRIVED[0]));
        assert!(frames[22..].iter().all(Option::is_none));
    }

    #[test]
    fn the_brackets_close_in_with_the_missile_and_beep_faster() {
        let mut state = Tracker::default();
        step_state(&mut state, Some((1, 5000 * 5000)), 0);
        // 5000 / 50 = 100 away: the quick beat, and white on tick 0 and 5.
        assert_eq!(state.frame.unwrap().distance, 100);
        assert!(state.frame.unwrap().white);
        step_state(&mut state, Some((1, 5000 * 5000)), 3);
        assert!(!state.frame.unwrap().white);
        step_state(&mut state, Some((1, 5000 * 5000)), 5);
        assert!(state.frame.unwrap().white);
        step_state(&mut state, Some((1, 20000 * 20000)), 5);
        assert!(!state.frame.unwrap().white);
        step_state(&mut state, Some((1, 20000 * 20000)), 30);
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
            },
        );
        assert_eq!(quads.len(), 24);
        let xs = quads.iter().map(|(p, _)| p[0]);
        // 32 texels each way from -32: centered on the kart at the plain size.
        assert_eq!(xs.clone().min(), Some(-32));
        assert_eq!(xs.max(), Some(31));
    }
}
