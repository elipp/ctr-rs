//! Turns a track's quad blocks into plain vertex/index arrays (no Bevy types,
//! so it can be tested without a window).

use loader::lev::{Level, SVec3};

/// World units per game unit.
pub const SCALE: f32 = 0.01;

/// Triangles of a quad block's 3x3 vertex patch, as the collision code uses
/// them (`COLL_FIXED_QUADBLK_TestTriangles`):
///
/// ```text
/// 0 4 1
/// 5 6 7
/// 2 8 3
/// ```
///
/// The first four cover the triangle 0-1-2; the last four add the other half
/// and exist only if the block is a full quad (`index[2] != index[3]`).
const FIRST_TRIANGLES: [[usize; 3]; 4] = [[0, 4, 5], [4, 6, 5], [6, 4, 1], [5, 6, 2]];
const SECOND_TRIANGLES: [[usize; 3]; 4] = [[8, 6, 7], [7, 3, 8], [1, 7, 6], [2, 6, 8]];

pub struct LevelMesh {
    pub positions: Vec<[f32; 3]>,
    pub colors: Vec<[f32; 4]>,
    pub indices: Vec<u32>,
}

/// Game space is Y up with Z pointing away from the viewer of a right-handed
/// frame, so Z is flipped.
pub fn to_world(p: SVec3) -> [f32; 3] {
    [p.x as f32 * SCALE, p.y as f32 * SCALE, -(p.z as f32) * SCALE]
}

/// One vertex per level vertex, so neighbouring quad blocks share them.
/// `low_colors` selects `color_lo` instead of `color_hi`.
pub fn level_mesh(level: &Level, low_colors: bool) -> LevelMesh {
    let positions = level.vertices.iter().map(|v| to_world(v.pos)).collect();
    let colors = level
        .vertices
        .iter()
        .map(|v| {
            let c = if low_colors { v.color_lo } else { v.color_hi };
            [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, 1.0]
        })
        .collect();

    let mut indices = Vec::new();
    for quad in &level.quad_blocks {
        let second = quad.vertex_index[2] != quad.vertex_index[3];
        let triangles = FIRST_TRIANGLES.iter().chain(second.then_some(&SECOND_TRIANGLES).into_iter().flatten());
        for tri in triangles {
            indices.extend(tri.iter().map(|&i| quad.vertex_index[i] as u32));
        }
    }
    LevelMesh { positions, colors, indices }
}
