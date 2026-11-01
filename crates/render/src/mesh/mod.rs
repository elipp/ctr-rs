//! Turns a track's quad blocks into plain vertex/index arrays (no Bevy types,
//! so it can be tested without a window).

mod atlas;
mod classify;
mod level;
mod model;
#[cfg(test)]
mod tests;

use loader::lev::SVec3;

pub use atlas::Atlas;
pub use classify::{TURBO_PAD_EXTRA_BIAS, is_collision_wall, is_turbo_pad, turbo_pad_visuals};
pub use level::{AnimatedFace, Animation, LevelMesh, SceneryCorner, WaterCorner, level_mesh};
pub use model::{ModelMesh, model_mesh, model_positions, model_vertices};

/// World units per game unit.
pub const SCALE: f32 = 0.01;

/// A vertex colour channel (1.0 = the texture as is) for the mesh. The console multiplies the
/// texture by it on the gamma-encoded values, while the renderer multiplies in linear light, so
/// the channel goes through the display gamma to give the same result.
pub fn vertex_channel(c: f32) -> f32 {
    c.powf(2.2)
}

/// Game space is Y up like Bevy's. Flipping any single axis would mirror the
/// tracks (left turns become right turns), so this only scales.
pub fn to_world(p: SVec3) -> [f32; 3] {
    [p.x as f32 * SCALE, p.y as f32 * SCALE, p.z as f32 * SCALE]
}
/// Meshes the faces are sorted into, by how they reach the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    /// Alpha-tested; also the non-blended texels of semi-transparent faces.
    Opaque = 0,
    /// 50% background plus 50% face (alpha blended).
    Average = 1,
    Add = 2,
    /// Background minus face.
    Subtract = 3,
}

pub const PARTS: usize = 4;

#[derive(Default, Clone)]
pub struct MeshPart {
    pub positions: Vec<[f32; 3]>,
    pub colors: Vec<[f32; 4]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

/// Winds every triangle of `parts` to face away from the middle of their bounding box, for a
/// convex model (a crate, a roller) to be drawn with back faces culled: the triangles of a model
/// come in no fixed winding (the game decides per frame, with `NCLIP`), so a camera inside the
/// model sees none of it.
pub fn wind_outward(parts: &mut [MeshPart; PARTS]) {
    let mut low = [f32::MAX; 3];
    let mut high = [f32::MIN; 3];
    for p in parts.iter().flat_map(|p| p.positions.iter()) {
        for a in 0..3 {
            low[a] = low[a].min(p[a]);
            high[a] = high[a].max(p[a]);
        }
    }
    let middle: [f32; 3] = std::array::from_fn(|a| (low[a] + high[a]) / 2.0);
    for part in parts {
        for tri in part.indices.as_chunks_mut::<3>().0 {
            let [a, b, c] = [0, 1, 2].map(|i| part.positions[tri[i] as usize]);
            let edge = |p: [f32; 3], q: [f32; 3]| std::array::from_fn::<f32, 3, _>(|i| q[i] - p[i]);
            let (u, v) = (edge(a, b), edge(a, c));
            let normal = [
                u[1] * v[2] - u[2] * v[1],
                u[2] * v[0] - u[0] * v[2],
                u[0] * v[1] - u[1] * v[0],
            ];
            let out: f32 = (0..3)
                .map(|i| normal[i] * ((a[i] + b[i] + c[i]) / 3.0 - middle[i]))
                .sum();
            if out < 0.0 {
                tri.swap(1, 2);
            }
        }
    }
}
