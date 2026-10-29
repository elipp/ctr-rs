//! The texture atlas of a mesh: the tiles of the pages a face uses, laid out in a grid.

use std::collections::HashMap;

use loader::texture::{Blend, TextureLayout};
use loader::vram::Vram;

use super::Part;

/// Textures are decoded into 256x256 tiles (a whole texture page seen through
/// one CLUT) and laid out in a grid. Tile 0 is solid white for untextured faces.
pub(super) const TILE: usize = 256;
pub(super) const TILE_COLUMNS: usize = 16;

pub struct Atlas {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

/// Which texels of a page a tile keeps. A semi-transparent face is drawn
/// twice: its non-STP texels opaque, its STP texels blended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum TileKind {
    Full,
    OpaqueOnly,
    /// STP texels only, with this alpha.
    StpOnly(u8),
}
pub(super) struct AtlasBuilder<'a> {
    pub(super) vram: &'a Vram,
    pub(super) tiles: Vec<Vec<u8>>,
    pub(super) by_key: HashMap<(u16, u16, TileKind), usize>,
}

impl AtlasBuilder<'_> {
    pub(super) fn tile(&mut self, t: &TextureLayout, kind: TileKind) -> usize {
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

    pub(super) fn finish(self) -> Atlas {
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
    pub(super) fn uv(&self, tile: usize, u: u8, v: u8) -> [f32; 2] {
        let (tx, ty) = (tile % TILE_COLUMNS, tile / TILE_COLUMNS);
        // Atlas pixels; `level_mesh` normalises them once the atlas size is known.
        [(tx * TILE) as f32 + u as f32, (ty * TILE) as f32 + v as f32]
    }
}

/// The (part, texels) passes a face of this blend mode is drawn in.
pub(super) fn passes(blend: Blend) -> &'static [(Part, TileKind)] {
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
