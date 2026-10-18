//! Turns a track's quad blocks into plain vertex/index arrays (no Bevy types,
//! so it can be tested without a window).

use std::collections::HashMap;

use loader::lev::{Level, SVec3};
use loader::texture::{FaceMode, TextureLayout, face_polygon, resolve_texture};
use loader::vram::Vram;

/// World units per game unit.
pub const SCALE: f32 = 0.01;

/// Game space is Y up like Bevy's. Flipping any single axis would mirror the
/// tracks (left turns become right turns), so this only scales.
pub fn to_world(p: SVec3) -> [f32; 3] {
    [p.x as f32 * SCALE, p.y as f32 * SCALE, p.z as f32 * SCALE]
}

fn is_marker_red(c: [u8; 4]) -> bool {
    c[..3] == [255, 0, 0]
}

/// Textures are decoded into 256x256 tiles (a whole texture page seen through
/// one CLUT) and laid out in a grid. Tile 0 is solid white for untextured faces.
const TILE: usize = 256;
const TILE_COLUMNS: usize = 16;

pub struct Atlas {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

pub struct LevelMesh {
    pub positions: Vec<[f32; 3]>,
    pub colors: Vec<[f32; 4]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
    pub atlas: Atlas,
}

struct AtlasBuilder<'a> {
    vram: &'a Vram,
    tiles: Vec<Vec<u8>>,
    by_key: HashMap<(u16, u16), usize>,
}

impl AtlasBuilder<'_> {
    fn tile(&mut self, t: &TextureLayout) -> usize {
        let key = (t.tpage, t.clut);
        if let Some(&i) = self.by_key.get(&key) {
            return i;
        }
        let mut rgba = vec![0u8; TILE * TILE * 4];
        for v in 0..TILE {
            for u in 0..TILE {
                // Texels beyond VRAM or in a reserved mode stay transparent.
                if let Ok(Some(c)) = self.vram.texel(t, u as u8, v as u8) {
                    rgba[(v * TILE + u) * 4..][..4].copy_from_slice(&c);
                }
            }
        }
        self.tiles.push(rgba);
        self.by_key.insert(key, self.tiles.len() - 1);
        self.tiles.len() - 1
    }

    fn finish(self) -> Atlas {
        let rows = self.tiles.len().div_ceil(TILE_COLUMNS);
        let (width, height) = (TILE_COLUMNS * TILE, rows * TILE);
        let mut rgba = vec![0u8; width * height * 4];
        for (n, tile) in self.tiles.iter().enumerate() {
            let (ox, oy) = (n % TILE_COLUMNS * TILE, n / TILE_COLUMNS * TILE);
            for row in 0..TILE {
                rgba[((oy + row) * width + ox) * 4..][..TILE * 4].copy_from_slice(&tile[row * TILE * 4..][..TILE * 4]);
            }
        }
        Atlas { width, height, rgba }
    }

    fn uv(&self, tile: usize, u: u8, v: u8) -> [f32; 2] {
        let (tx, ty) = (tile % TILE_COLUMNS, tile / TILE_COLUMNS);
        // Atlas pixels; `level_mesh` normalises them once the atlas size is known.
        [(tx * TILE) as f32 + u as f32, (ty * TILE) as f32 + v as f32]
    }
}

/// One mesh vertex per polygon corner of every face, so each face can carry
/// its own texture coordinates. `payload` is the LEV payload the quad blocks'
/// texture pointers refer to. `low_colors` selects `color_lo` instead of `color_hi`.
///
/// Textured faces get their vertex colour doubled (128 is neutral on the
/// PS1), untextured faces sample the white tile and keep it as is.
///
/// `hide_red` drops faces whose corners are all pure (255, 0, 0) vertex colour.
/// That is a viewer heuristic, not game logic: these faces are never hidden by
/// the visibility data or the quad flags, but they look like editor markers.
pub fn level_mesh(level: &Level, payload: &[u8], vram: &Vram, low_colors: bool, hide_red: bool) -> LevelMesh {
    let mut atlas = AtlasBuilder { vram, tiles: Vec::new(), by_key: HashMap::new() };
    atlas.tiles.push(vec![255; TILE * TILE * 4]);

    let (mut positions, mut colors, mut uvs, mut indices) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for quad in &level.quad_blocks {
        // Triangle-shaped blocks repeat the third corner in slot 3.
        let is_quad = quad.vertex_index[2] != quad.vertex_index[3];
        // Face 3 lies entirely in the second half of the patch.
        for face in 0..if is_quad { 4 } else { 3 } {
            // The closest-range layout of the group: nothing here depends on the camera.
            let texture = resolve_texture(payload, quad.texture_mid[face]).ok().flatten().map(|g| g.near);
            let slots = match &texture {
                Some(_) => match face_polygon(face, FaceMode::of(quad, face)) {
                    Ok(slots) => slots,
                    Err(_) => continue,
                },
                None => face_polygon(face, FaceMode { rotation: 0, mode: 0 }).unwrap(),
            };
            if hide_red && slots.iter().all(|&s| is_marker_red(level.vertices[quad.vertex_index[s] as usize].color_hi)) {
                continue;
            }
            let tile = texture.as_ref().map(|t| atlas.tile(t));
            let base = positions.len() as u32;
            for (k, &slot) in slots.iter().enumerate() {
                let v = &level.vertices[quad.vertex_index[slot] as usize];
                positions.push(to_world(v.pos));
                let c = if low_colors { v.color_lo } else { v.color_hi };
                let gain = if texture.is_some() { 2.0 / 255.0 } else { 1.0 / 255.0 };
                let [r, g, b] = [c[0], c[1], c[2]].map(|c| (c as f32 * gain).min(1.0));
                colors.push([r, g, b, 1.0]);
                uvs.push(match (&texture, tile) {
                    (Some(t), Some(tile)) => atlas.uv(tile, t.uv[k].0, t.uv[k].1),
                    _ => atlas.uv(0, 0, 0),
                });
            }
            // The game draws (0, 1, 2) and (3, 1, 2); a repeated vertex degenerates one of them.
            let ids: [u16; 4] = slots.map(|s| quad.vertex_index[s]);
            for tri in [[0, 1, 2], [3, 1, 2]] {
                let [a, b, c] = tri;
                let degenerate = ids[a] == ids[b] || ids[b] == ids[c] || ids[a] == ids[c];
                if !degenerate {
                    indices.extend(tri.map(|k| base + k as u32));
                }
            }
        }
    }
    let atlas = atlas.finish();
    for uv in &mut uvs {
        *uv = [uv[0] / atlas.width as f32, uv[1] / atlas.height as f32];
    }
    LevelMesh { positions, colors, uvs, indices, atlas }
}

#[cfg(test)]
mod tests {
    use super::*;
    use loader::bigindex::{BigIndex, LevFile, Lod};
    use loader::disc::open_bigfile;
    use loader::dram::DramFile;
    use loader::level::LevelId;
    use loader::vram::parse_vram_file;

    #[test]
    fn real_tracks_mesh_with_valid_atlas() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/ctr-u.bin");
        let Ok((mut disc, big)) = open_bigfile(&path) else {
            return;
        };
        for id in 0..=17 {
            let mut read = |f| big.read(&mut disc, BigIndex::level(LevelId(id), Lod::OnePlayer, f).get()).unwrap();
            let dram = DramFile::parse(&read(LevFile::Lev)).unwrap();
            let level = Level::parse(&dram).unwrap();
            let mut vram = Vram::default();
            for img in parse_vram_file(&read(LevFile::Vram)).unwrap() {
                vram.upload(&img).unwrap();
            }
            let m = level_mesh(&level, &dram.data, &vram, false, true);
            assert_eq!(m.positions.len(), m.uvs.len());
            assert_eq!(m.positions.len(), m.colors.len());
            assert!(m.indices.iter().all(|&i| (i as usize) < m.positions.len()));
            assert!(m.atlas.height <= 8192, "level {id}: atlas {}x{}", m.atlas.width, m.atlas.height);
            assert!(m.atlas.height > TILE, "level {id}: no textures were decoded");
            assert!(m.uvs.iter().all(|uv| uv.iter().all(|c| (0.0..=1.0).contains(c))));
            println!("level {id}: {} verts, atlas {}x{}", m.positions.len(), m.atlas.width, m.atlas.height);
        }
    }
}
