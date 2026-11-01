//! The heat shimmer of the flame jets of Tiger Temple (`emSet_fjHeat`, `Torch_Main`). Retail
//! draws it for one player only, as it costs the PS1 a pass over the picture; we do it always, but
//! only on Tiger Temple, whose camera is the only one given a [`HeatWarp`] (`scene.rs`).
//!
//! A heat particle is not a sprite but a lens: three octagons round it (the radii of its rings
//! grow as it ages), and the triangles between them, which `Torch_Main` draws showing the
//! picture from other places than they cover, so that the air wobbles. The lenses go to the
//! picture's post-processing ([`HeatWarp`], `heat.wgsl`) as the points of their octagons on the
//! screen, with the triangles of each sector worked out there; the camera's picture is what the
//! game's framebuffer is to retail. How the particles are born, move and grow is `render`'s and
//! [`crate::fire`]'s.

use bevy::asset::embedded_asset;
use bevy::core_pipeline::fullscreen_material::{FullscreenMaterial, FullscreenMaterialPlugin};
use bevy::prelude::*;
use bevy::render::RenderApp;
use bevy::render::extract_component::ExtractComponent;
use bevy::render::render_resource::ShaderType;
use bevy::shader::ShaderRef;
use render::mesh::SCALE;

use crate::exhaust::SCREEN_PIXEL_ASPECT;
use crate::fire::Fire;

/// How many lenses retail draws a frame: the newest particles, 12 of them (`Torch_Main`).
const LENSES: usize = 12;
/// What a lens takes in the uniform, in `Vec4`s: its box, its centre, and 8 points a ring.
const PER_LENS: usize = 2 + 3 * 8;
/// The height of a ring as a part of its width (`CTC2(0x0a00, 2)`: the picture's pixels are
/// taller than wide).
const RING_HEIGHT: f32 = 0xa00 as f32 / 4096.0;
/// The corners of a ring, as a part of its radius (`0xb50 / 0x1000`).
const DIAGONAL: f32 = 0xb50 as f32 / 4096.0;
/// A lens whose outer ring is wider than this, as a part of half the picture, is not drawn
/// (`sxy0 < 0x80` of 0x100 pixels).
const MAX_WIDTH: f32 = 0.5;

/// The lenses of the frame, for the post-processing of the main camera.
#[derive(Component, ExtractComponent, Clone, Copy, ShaderType)]
#[extract_app(RenderApp)]
pub struct HeatWarp {
    count: u32,
    /// For each lens: the box round it in the picture (min x, min y, max x, max y, as 0 to 1),
    /// its centre, and the points of its rings, outermost first, from the top clockwise.
    points: [Vec4; LENSES * PER_LENS],
}

impl Default for HeatWarp {
    fn default() -> HeatWarp {
        HeatWarp {
            count: 0,
            points: [Vec4::ZERO; LENSES * PER_LENS],
        }
    }
}

impl FullscreenMaterial for HeatWarp {
    fn fragment_shader() -> ShaderRef {
        concat!("embedded://", env!("CARGO_PKG_NAME"), "/heat.wgsl").into()
    }
}

pub struct HeatPlugin;

impl Plugin for HeatPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "heat.wgsl");
        app.add_plugins(FullscreenMaterialPlugin::<HeatWarp>::default())
            // After the camera moved this frame (the lenses go where it sees them now).
            .add_systems(PostUpdate, update.before(TransformSystems::Propagate));
    }
}

/// Puts the lenses where the camera sees them, between the last two ticks.
fn update(
    fire: Option<Res<Fire>>,
    fixed: Res<Time<Fixed>>,
    mut cameras: Query<(&Camera, &Transform, &mut HeatWarp), With<crate::drive::ChaseCamera>>,
) {
    let Ok((camera, transform, mut warp)) = cameras.single_mut() else {
        return;
    };
    let mut lenses: Vec<(f32, [Vec4; PER_LENS])> = Vec::new();
    if let Some(fire) = fire {
        let t = fixed.overstep_fraction();
        let eye = GlobalTransform::from(*transform);
        let (right, up, forward) = (
            transform.rotation * Vec3::X,
            transform.rotation * Vec3::Y,
            transform.rotation * Vec3::NEG_Z,
        );
        // The newest first, as retail's list is, and it looks at no more than 12 (it counts
        // the ones that are not seen too).
        for spark in fire.heat.iter().rev().take(LENSES) {
            let age = spark.lead + t;
            if spark.ticks as f32 + age >= spark.life {
                continue;
            }
            let p = spark.particle.ahead(age);
            let center = Vec3::from_array(p.position().map(|c| c as f32)) / 256.0 * SCALE;
            if let Some(lens) = lens(camera, &eye, center, p.heat_radii(), right, up) {
                lenses.push(((center - transform.translation).dot(forward), lens));
            }
        }
    }
    // Drawn from the farthest to the nearest.
    lenses.sort_by(|a, b| b.0.total_cmp(&a.0));
    warp.count = lenses.len() as u32;
    for (n, (_, lens)) in lenses.iter().enumerate() {
        warp.points[n * PER_LENS..][..PER_LENS].copy_from_slice(lens);
    }
}

/// The points of a lens at `center` with rings of the `radii` (game units) as the picture's
/// coordinates, or none if it is not in view or is too close to be drawn.
fn lens(
    camera: &Camera,
    eye: &GlobalTransform,
    center: Vec3,
    radii: [i32; 3],
    right: Vec3,
    up: Vec3,
) -> Option<[Vec4; PER_LENS]> {
    let to_picture = |at: Vec3| -> Option<Vec2> {
        let ndc = camera.world_to_ndc(eye, at)?;
        (ndc.z > 0.0).then(|| Vec2::new(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5))
    };
    let middle = to_picture(center)?;
    let mut points = Vec::with_capacity(PER_LENS);
    points.push(Vec4::ZERO);
    points.push(middle.extend(0.0).extend(0.0));
    for (ring, radius) in radii.into_iter().enumerate() {
        let across = right * (radius as f32 * SCALE);
        let high = up * (radius as f32 * SCALE * RING_HEIGHT * SCREEN_PIXEL_ASPECT);
        let d = DIAGONAL;
        for offset in [
            high,
            across * d + high * d,
            across,
            across * d - high * d,
            -high,
            -across * d - high * d,
            -across,
            -across * d + high * d,
        ] {
            let point = to_picture(center + offset)?;
            if ring == 0 && offset == across && (point.x - middle.x).abs() >= MAX_WIDTH {
                return None;
            }
            points.push(point.extend(0.0).extend(0.0));
        }
    }
    let (mut low, mut high) = (Vec2::MAX, Vec2::MIN);
    for p in &points[1..] {
        low = low.min(Vec2::new(p.x, p.y));
        high = high.max(Vec2::new(p.x, p.y));
    }
    points[0] = Vec4::new(low.x, low.y, high.x, high.y);
    points.try_into().ok()
}
