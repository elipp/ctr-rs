//! The rain under the red beaker's clouds (`RedBeaker_RenderRain`): short streaks that fade in
//! as they fall. Where they are is `render::rain`'s; the clouds are `vehicle::cloud`'s. Drawn as
//! gizmo lines, each from nothing to a pale blue (retail's lines run from black to a colour, and
//! are added to the picture).

use bevy::prelude::*;
use render::mesh::SCALE;

/// How much of the box's width the rain is drawn in (x and z, not the height). The game's box is
/// 256 units wide, by its numbers; retail's rain looks tighter under the cloud than that, and
/// this is chosen by eye.
const NARROWING: f32 = 0.5;

/// The colour at the lower end of a streak.
const COLOR: Color = Color::srgba(0.8, 0.8, 1.0, 0.8);

pub fn draw(sim: Res<crate::props::Sim>, mut gizmos: Gizmos) {
    let clouds = sim
        .race
        .karts
        .iter()
        .flat_map(|k| k.driver.cloud.iter().chain(&k.driver.fading_clouds));
    for cloud in clouds {
        let at = |p: [i32; 3]| {
            Vec3::new(
                cloud.pos[0] as f32 + p[0] as f32 * NARROWING,
                (cloud.pos[1] + p[1]) as f32,
                cloud.pos[2] as f32 + p[2] as f32 * NARROWING,
            ) * SCALE
        };
        for [top, bottom] in render::rain::streaks(cloud.rain_frames, cloud.age) {
            gizmos.line_gradient(at(top), at(bottom), COLOR.with_alpha(0.0), COLOR);
        }
    }
}
