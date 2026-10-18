//! Turns a track's quad blocks into plain vertex/index arrays (no Bevy types,
//! so it can be tested without a window).

use std::collections::HashMap;

use loader::dram::DramFile;
use loader::lev::{Level, SVec3};
use loader::texture::{
    AnimTex, Blend, FaceMode, TextureLayout, face_polygon, face_uv_order, resolve_animation,
    resolve_texture,
};
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

/// Which texels of a page a tile keeps. A semi-transparent face is drawn
/// twice: its non-STP texels opaque, its STP texels blended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum TileKind {
    Full,
    OpaqueOnly,
    /// STP texels only, with this alpha.
    StpOnly(u8),
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

#[derive(Default)]
pub struct MeshPart {
    pub positions: Vec<[f32; 3]>,
    pub colors: Vec<[f32; 4]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

/// An animated texture and, for every frame, the layout's four UVs (in layout
/// order: the face's own [`AnimatedFace::uv_order`] says which goes where).
pub struct Animation {
    pub texture: AnimTex,
    pub uvs: Vec<[[f32; 2]; 4]>,
}

/// Where an animated face's four vertices start in its part.
pub struct AnimatedFace {
    pub part: usize,
    pub first_vertex: usize,
    pub animation: usize,
    /// Which of the frame's four UVs each of the face's vertices takes.
    pub uv_order: [usize; 4],
}

pub struct LevelMesh {
    pub parts: [MeshPart; PARTS],
    pub atlas: Atlas,
    pub animations: Vec<Animation>,
    pub animated_faces: Vec<AnimatedFace>,
}

struct AtlasBuilder<'a> {
    vram: &'a Vram,
    tiles: Vec<Vec<u8>>,
    by_key: HashMap<(u16, u16, TileKind), usize>,
}

impl AtlasBuilder<'_> {
    fn tile(&mut self, t: &TextureLayout, kind: TileKind) -> usize {
        let key = (t.tpage, t.clut, kind);
        if let Some(&i) = self.by_key.get(&key) {
            return i;
        }
        let mut rgba = vec![0u8; TILE * TILE * 4];
        for v in 0..TILE {
            for u in 0..TILE {
                // Texels beyond VRAM or in a reserved mode stay transparent.
                let Ok(Some((mut c, stp))) = self.vram.texel_stp(t, u as u8, v as u8) else {
                    continue;
                };
                match kind {
                    TileKind::Full => {}
                    TileKind::OpaqueOnly if !stp => {}
                    TileKind::StpOnly(alpha) if stp => c[3] = alpha,
                    _ => continue,
                }
                rgba[(v * TILE + u) * 4..][..4].copy_from_slice(&c);
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
                rgba[((oy + row) * width + ox) * 4..][..TILE * 4]
                    .copy_from_slice(&tile[row * TILE * 4..][..TILE * 4]);
            }
        }
        Atlas {
            width,
            height,
            rgba,
        }
    }

    // TODO: a face only reaches its last UV at its far edge, so the last texel row/column
    // is a sliver and tiled faces (e.g. the boost pad, v 14..29 repeated per face) lose a row
    // at the seams. Unknown whether the original does too; compare against it before touching this.
    fn uv(&self, tile: usize, u: u8, v: u8) -> [f32; 2] {
        let (tx, ty) = (tile % TILE_COLUMNS, tile / TILE_COLUMNS);
        // Atlas pixels; `level_mesh` normalises them once the atlas size is known.
        [(tx * TILE) as f32 + u as f32, (ty * TILE) as f32 + v as f32]
    }
}

/// The (part, texels) passes a face of this blend mode is drawn in.
fn passes(blend: Blend) -> &'static [(Part, TileKind)] {
    match blend {
        Blend::Opaque => &[(Part::Opaque, TileKind::Full)],
        Blend::Average => &[
            (Part::Opaque, TileKind::OpaqueOnly),
            (Part::Average, TileKind::StpOnly(128)),
        ],
        Blend::Add => &[
            (Part::Opaque, TileKind::OpaqueOnly),
            (Part::Add, TileKind::StpOnly(255)),
        ],
        Blend::Subtract => &[
            (Part::Opaque, TileKind::OpaqueOnly),
            (Part::Subtract, TileKind::StpOnly(255)),
        ],
    }
}

/// One mesh vertex per polygon corner of every face, so each face can carry
/// its own texture coordinates. `dram` is the LEV file the quad blocks'
/// texture pointers refer to. `low_colors` selects `color_lo` instead of `color_hi`.
///
/// Textured faces get their vertex colour doubled (128 is neutral on the PS1,
/// `bright = 2` in the reference renderer) and left unclamped so that the
/// product with the texel clamps, as there. Untextured faces sample the white
/// tile and keep their colour as is.
///
/// `hide_red` drops faces whose corners are all pure (255, 0, 0) vertex colour.
/// That is a viewer heuristic, not game logic: these faces are never hidden by
/// the visibility data or the quad flags, but they look like editor markers.
pub fn level_mesh(
    level: &Level,
    dram: &DramFile,
    vram: &Vram,
    low_colors: bool,
    hide_red: bool,
) -> LevelMesh {
    let mut atlas = AtlasBuilder {
        vram,
        tiles: Vec::new(),
        by_key: HashMap::new(),
    };
    atlas.tiles.push(vec![255; TILE * TILE * 4]);

    let mut parts: [MeshPart; PARTS] = Default::default();
    let mut animations: Vec<Animation> = Vec::new();
    let mut animation_of: HashMap<(u32, usize), usize> = HashMap::new();
    let mut animated_faces = Vec::new();

    for quad in &level.quad_blocks {
        // Triangle-shaped blocks repeat the third corner in slot 3.
        let is_quad = quad.vertex_index[2] != quad.vertex_index[3];
        // Face 3 lies entirely in the second half of the patch.
        for face in 0..if is_quad { 4 } else { 3 } {
            let ptr = quad.texture_mid[face];
            // The sharpest layout of the group: nothing here depends on the camera.
            let texture = resolve_texture(dram, ptr).ok().flatten().map(|g| *g.best());
            let mode = if texture.is_some() {
                FaceMode::of(quad, face)
            } else {
                FaceMode {
                    rotation: 0,
                    mode: 0,
                }
            };
            let (Ok(slots), Ok(uv_order)) = (face_polygon(face, mode), face_uv_order(mode)) else {
                continue;
            };
            if hide_red
                && slots
                    .iter()
                    .all(|&s| is_marker_red(level.vertices[quad.vertex_index[s] as usize].color_hi))
            {
                continue;
            }
            let anim = resolve_animation(dram, ptr).ok().flatten();
            let untextured = [(Part::Opaque, TileKind::Full)];
            let face_passes = match &texture {
                Some(t) => passes(t.blend()),
                None => &untextured,
            };

            for &(part, kind) in face_passes {
                let tile = texture.as_ref().map(|t| atlas.tile(t, kind));
                let out = &mut parts[part as usize];
                let base = out.positions.len();
                for (k, &slot) in slots.iter().enumerate() {
                    let v = &level.vertices[quad.vertex_index[slot] as usize];
                    out.positions.push(to_world(v.pos));
                    let c = if low_colors { v.color_lo } else { v.color_hi };
                    let gain = if texture.is_some() {
                        2.0 / 255.0
                    } else {
                        1.0 / 255.0
                    };
                    let [r, g, b] = [c[0], c[1], c[2]].map(|c| c as f32 * gain);
                    out.colors.push([r, g, b, 1.0]);
                    out.uvs.push(match (&texture, tile) {
                        (Some(t), Some(tile)) => {
                            atlas.uv(tile, t.uv[uv_order[k]].0, t.uv[uv_order[k]].1)
                        }
                        _ => atlas.uv(0, 0, 0),
                    });
                }
                // The game draws (0, 1, 2) and (3, 1, 2); a repeated vertex degenerates one of them.
                let ids: [u16; 4] = slots.map(|s| quad.vertex_index[s]);
                for tri in [[0, 1, 2], [3, 1, 2]] {
                    let [a, b, c] = tri;
                    let degenerate = ids[a] == ids[b] || ids[b] == ids[c] || ids[a] == ids[c];
                    if !degenerate {
                        out.indices.extend(tri.map(|k| (base + k) as u32));
                    }
                }

                if let Some(anim) = &anim {
                    let index = *animation_of.entry((ptr, part as usize)).or_insert_with(|| {
                        let uvs = anim
                            .frames
                            .iter()
                            .map(|g| {
                                let tile = atlas.tile(g.best(), kind);
                                g.best().uv.map(|(u, v)| atlas.uv(tile, u, v))
                            })
                            .collect();
                        animations.push(Animation {
                            texture: anim.clone(),
                            uvs,
                        });
                        animations.len() - 1
                    });
                    animated_faces.push(AnimatedFace {
                        part: part as usize,
                        first_vertex: base,
                        animation: index,
                        uv_order,
                    });
                }
            }
        }
    }

    let atlas = atlas.finish();
    let norm = |uv: &mut [f32; 2]| *uv = [uv[0] / atlas.width as f32, uv[1] / atlas.height as f32];
    parts
        .iter_mut()
        .flat_map(|p| p.uvs.iter_mut())
        .for_each(norm);
    animations
        .iter_mut()
        .flat_map(|a| a.uvs.iter_mut().flatten())
        .for_each(norm);
    LevelMesh {
        parts,
        atlas,
        animations,
        animated_faces,
    }
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
            let mut read = |f| {
                big.read(
                    &mut disc,
                    BigIndex::level(LevelId(id), Lod::OnePlayer, f).get(),
                )
                .unwrap()
            };
            let dram = DramFile::parse(&read(LevFile::Lev)).unwrap();
            let level = Level::parse(&dram).unwrap();
            let mut vram = Vram::default();
            for img in parse_vram_file(&read(LevFile::Vram)).unwrap() {
                vram.upload(&img).unwrap();
            }
            let m = level_mesh(&level, &dram, &vram, false, true);
            for p in &m.parts {
                assert_eq!(p.positions.len(), p.uvs.len());
                assert_eq!(p.positions.len(), p.colors.len());
                assert!(p.indices.iter().all(|&i| (i as usize) < p.positions.len()));
                assert!(p.uvs.iter().flatten().all(|c| (0.0..=1.0).contains(c)));
            }
            for f in &m.animated_faces {
                assert!(f.first_vertex + 4 <= m.parts[f.part].positions.len());
                assert_eq!(
                    m.animations[f.animation].uvs.len(),
                    m.animations[f.animation].texture.frames.len()
                );
            }
            assert!(
                m.animations
                    .iter()
                    .flat_map(|a| a.uvs.iter().flatten().flatten())
                    .all(|c| (0.0..=1.0).contains(c))
            );
            assert!(
                m.atlas.height <= 8192,
                "level {id}: atlas {}x{}",
                m.atlas.width,
                m.atlas.height
            );
            assert!(
                m.atlas.height > TILE,
                "level {id}: no textures were decoded"
            );
            println!(
                "level {id}: verts {:?}, {} animations on {} faces, atlas {}x{}",
                m.parts.each_ref().map(|p| p.positions.len()),
                m.animations.len(),
                m.animated_faces.len(),
                m.atlas.width,
                m.atlas.height
            );
        }
    }
}
