//! The mesh of a model in one pose.

use std::collections::HashMap;

use loader::model::{Frame, ModelHeader};
use loader::vram::Vram;

use super::atlas::{Atlas, AtlasBuilder, TILE, TileKind, passes};
use super::{MeshPart, PARTS, Part, SCALE, vertex_channel};

/// A textured model in one pose.
pub struct ModelMesh {
    pub parts: [MeshPart; PARTS],
    pub atlas: Atlas,
}

/// Triangles of `header` in `frame`, with the model's vertex colours and
/// textures from `vram`. Model units become game units through the header's
/// scale and the instance's (`inst_scale`, 0x1000 = 1.0), as
/// `RenderBucket_BuildMatrices` does; the result is in world units.
///
/// Corners take the layout's first three UVs in order (`DrawInstPrim_Normal`).
pub fn model_mesh(
    header: &ModelHeader,
    frame: &Frame,
    vram: &Vram,
    inst_scale: [i16; 3],
) -> ModelMesh {
    let mut atlas = AtlasBuilder {
        vram,
        tiles: vec![vec![255; TILE * TILE * 4]],
        by_key: HashMap::new(),
    };
    let k: [f32; 3] = std::array::from_fn(|a| {
        header.scale[a] as f32 / 4096.0 * inst_scale[a] as f32 / 4096.0 * SCALE
    });

    let mut parts: [MeshPart; PARTS] = Default::default();
    for tri in header.triangles(frame) {
        let texture = tri
            .texture
            .checked_sub(1)
            .and_then(|i| header.textures.get(i))
            .copied();
        let untextured = [(Part::Opaque, TileKind::Full)];
        let face_passes = match &texture {
            Some(t) => passes(t.blend()),
            None => &untextured,
        };
        // Textured primitives get their colour doubled, as for the level.
        let gain = if texture.is_some() {
            2.0 / 255.0
        } else {
            1.0 / 255.0
        };
        for &(part, kind) in face_passes {
            let tile = texture.as_ref().map(|t| atlas.tile(t, kind));
            let out = &mut parts[part as usize];
            let base = out.positions.len() as u32;
            for corner in 0..3 {
                let p = tri.positions[corner];
                out.positions
                    .push([p[0] as f32 * k[0], p[1] as f32 * k[1], p[2] as f32 * k[2]]);
                let c = tri.colors[corner];
                let [r, g, b] = [c & 0xff, c >> 8 & 0xff, c >> 16 & 0xff]
                    .map(|c| vertex_channel(c as f32 * gain));
                out.colors.push([r, g, b, 1.0]);
                out.uvs.push(match (&texture, tile) {
                    (Some(t), Some(tile)) => atlas.uv(tile, t.uv[corner].0, t.uv[corner].1),
                    _ => atlas.uv(0, 0, 0),
                });
            }
            out.indices.extend([base, base + 1, base + 2]);
        }
    }

    let atlas = atlas.finish();
    for uv in parts.iter_mut().flat_map(|p| p.uvs.iter_mut()) {
        *uv = [uv[0] / atlas.width as f32, uv[1] / atlas.height as f32];
    }
    ModelMesh { parts, atlas }
}

/// The positions of the parts [`model_mesh`] makes of `header` in `frame`, for a model whose
/// pose changes but not its faces: the triangles come in the same order, so these replace
/// the parts' positions without building the atlas again.
pub fn model_positions(
    header: &ModelHeader,
    frame: &Frame,
    inst_scale: [i16; 3],
) -> [Vec<[f32; 3]>; PARTS] {
    let k: [f32; 3] = std::array::from_fn(|a| {
        header.scale[a] as f32 / 4096.0 * inst_scale[a] as f32 / 4096.0 * SCALE
    });
    let mut parts: [Vec<[f32; 3]>; PARTS] = Default::default();
    for tri in header.triangles(frame) {
        let texture = tri
            .texture
            .checked_sub(1)
            .and_then(|i| header.textures.get(i))
            .copied();
        let face_passes = match &texture {
            Some(t) => passes(t.blend()),
            None => &[(Part::Opaque, TileKind::Full)],
        };
        for &(part, _) in face_passes {
            parts[part as usize].extend(
                tri.positions
                    .map(|p| [p[0] as f32 * k[0], p[1] as f32 * k[1], p[2] as f32 * k[2]]),
            );
        }
    }
    parts
}

/// What changes between the poses of a model whose colours change too: the positions and the
/// colours of the parts [`model_mesh`] makes of `header` in `frame`, in the same order.
pub fn model_vertices(
    header: &ModelHeader,
    frame: &Frame,
    inst_scale: [i16; 3],
) -> [(Vec<[f32; 3]>, Vec<[f32; 4]>); PARTS] {
    let k: [f32; 3] = std::array::from_fn(|a| {
        header.scale[a] as f32 / 4096.0 * inst_scale[a] as f32 / 4096.0 * SCALE
    });
    let mut parts: [(Vec<[f32; 3]>, Vec<[f32; 4]>); PARTS] = Default::default();
    for tri in header.triangles(frame) {
        let texture = tri
            .texture
            .checked_sub(1)
            .and_then(|i| header.textures.get(i))
            .copied();
        let face_passes = match &texture {
            Some(t) => passes(t.blend()),
            None => &[(Part::Opaque, TileKind::Full)],
        };
        let gain = if texture.is_some() {
            2.0 / 255.0
        } else {
            1.0 / 255.0
        };
        for &(part, _) in face_passes {
            let (positions, colors) = &mut parts[part as usize];
            for corner in 0..3 {
                let p = tri.positions[corner];
                positions.push([p[0] as f32 * k[0], p[1] as f32 * k[1], p[2] as f32 * k[2]]);
                let c = tri.colors[corner];
                let [r, g, b] = [c & 0xff, c >> 8 & 0xff, c >> 16 & 0xff]
                    .map(|c| vertex_channel(c as f32 * gain));
                colors.push([r, g, b, 1.0]);
            }
        }
    }
    parts
}
