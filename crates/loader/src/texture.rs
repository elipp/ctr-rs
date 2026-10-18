//! Texture layouts of level quad blocks and how they map onto a block's
//! 3x3 vertex patch, plus PS1 pixel decoding (`Vram::texel`).

use std::io;

use crate::lev::QuadBlock;
use crate::vram::{VRAM_HEIGHT, VRAM_WIDTH, Vram};

fn invalid(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

/// `struct TextureLayout` (0xC bytes): one (u, v) per polygon vertex, plus the
/// CLUT and texture page words the GPU primitive carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextureLayout {
    /// u0v0 = top left, u1v1 = top right, u2v2 = bottom left, u3v3 = bottom right.
    pub uv: [(u8, u8); 4],
    pub clut: u16,
    pub tpage: u16,
}

/// Bit depth of the texture page's pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMode {
    Palette4,
    Palette8,
    Direct15,
}

pub const TEXTURE_LAYOUT_SIZE: usize = 0xc;

impl TextureLayout {
    pub fn parse(b: &[u8]) -> io::Result<Self> {
        let b: &[u8; TEXTURE_LAYOUT_SIZE] = b
            .get(..TEXTURE_LAYOUT_SIZE)
            .and_then(|b| b.try_into().ok())
            .ok_or_else(|| invalid("texture layout truncated"))?;
        let tpage = u16::from_le_bytes([b[6], b[7]]);
        // The game's own plausibility check (`tpage & 0xfe00 == 0`).
        if tpage & 0xfe00 != 0 {
            return Err(invalid(format!("implausible texture page {tpage:#x}")));
        }
        Ok(Self {
            uv: [(b[0], b[1]), (b[4], b[5]), (b[8], b[9]), (b[10], b[11])],
            clut: u16::from_le_bytes([b[2], b[3]]),
            tpage,
        })
    }

    /// Texture page origin in VRAM pixels.
    pub fn tpage_xy(&self) -> (usize, usize) {
        ((self.tpage as usize & 0xf) * 64, (self.tpage as usize >> 4 & 1) * 256)
    }

    pub fn color_mode(&self) -> io::Result<ColorMode> {
        match self.tpage >> 7 & 3 {
            0 => Ok(ColorMode::Palette4),
            1 => Ok(ColorMode::Palette8),
            2 => Ok(ColorMode::Direct15),
            _ => Err(invalid("reserved texture colour mode")),
        }
    }

    /// CLUT origin in VRAM pixels.
    pub fn clut_xy(&self) -> (usize, usize) {
        ((self.clut as usize & 0x3f) * 16, self.clut as usize >> 6)
    }
}

/// Follows a quad block texture pointer (a payload offset) to its layout.
/// A pointer with bit 0 set points at a slot holding the current layout
/// pointer (animated textures; the slot is what `CTR_CycleTex_LEV` rewrites),
/// so this returns whatever frame the file was saved with.
pub fn resolve_texture(payload: &[u8], ptr: u32) -> io::Result<Option<TextureLayout>> {
    let word = |at: usize| -> io::Result<u32> {
        payload
            .get(at..at + 4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
            .ok_or_else(|| invalid(format!("texture pointer {at:#x} is outside the payload")))
    };
    let at = match ptr {
        0 => return Ok(None),
        p if p & 1 == 1 => word(p as usize - 1)? as usize,
        p => p as usize,
    };
    if at == 0 {
        return Ok(None);
    }
    let bytes = payload
        .get(at..at + TEXTURE_LAYOUT_SIZE)
        .ok_or_else(|| invalid(format!("texture layout {at:#x} is outside the payload")))?;
    TextureLayout::parse(bytes).map(Some)
}

/// How one of a quad block's four faces is drawn (a 5-bit field of
/// `draw_order_low`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FaceMode {
    /// 0..8, picks the vertex order the texture's corners are glued to.
    pub rotation: u8,
    /// 0 = quad; 1 and 2 collapse the fourth vertex onto another one, which
    /// leaves a triangle.
    pub mode: u8,
}

impl FaceMode {
    pub fn of(block: &QuadBlock, face: usize) -> Self {
        let f = (block.draw_order_low >> (8 + 5 * face)) & 0x1f;
        Self { rotation: (f & 7) as u8, mode: (f >> 3) as u8 }
    }
}

/// The face's corner slots in the block's 3x3 patch, as `vertex_index`
/// positions in top-left, top-right, bottom-left, bottom-right order
/// (`sDrawLevelOvr1P4x1FaceSelectors` divided by the scratch vertex size).
const FACE_CORNERS: [[usize; 4]; 4] = [[0, 4, 5, 6], [4, 1, 6, 7], [5, 6, 2, 8], [6, 7, 8, 3]];

/// `R226.scratchInitTable`: for each mode * 8 + rotation, the byte of the
/// selector word that supplies polygon vertex 0..3 (most significant first).
/// Bits above the low 5 of each byte steer back-face culling and don't matter here.
const VERTEX_TABLE: [u32; 24] = [
    0x18100800, 0x80900818, 0x00081018, 0x98881000, 0x98081000, 0x00881018, 0x80100818, 0x18900800,
    0x10081898, 0x88881810, 0x18180810, 0x90980888, 0x90180888, 0x18980810, 0x88081810, 0x10881898,
    0x08181090, 0x98981008, 0x10101808, 0x88901898, 0x88101898, 0x10901808, 0x98181008, 0x08981090,
];

/// Positions in `QuadBlock::vertex_index` of the polygon vertices of `face`
/// (0..4). Vertex k carries `TextureLayout::uv[k]`. The face is drawn as the
/// triangles (0, 1, 2) and (3, 1, 2); in the triangle modes one vertex is
/// repeated, which degenerates one of the two.
pub fn face_polygon(face: usize, mode: FaceMode) -> io::Result<[usize; 4]> {
    let entry = *VERTEX_TABLE
        .get(mode.mode as usize * 8 + mode.rotation as usize)
        .ok_or_else(|| invalid(format!("face mode {} does not exist", mode.mode)))?;
    let corners = FACE_CORNERS[face];
    // Selector byte n (n = 0 least significant) holds corner `3 - n`.
    Ok(std::array::from_fn(|k| {
        let shift = entry >> ((3 - k) * 8) & 0x1f;
        corners[3 - (shift / 8) as usize]
    }))
}

/// Decoded texel; `None` is fully transparent (the PS1 treats 0x0000 as such).
pub type Rgba = [u8; 4];

impl Vram {
    /// Reads texel (`u`, `v`) of `texture`'s page through its CLUT, expanding
    /// 5-bit channels to 8 bits.
    pub fn texel(&self, texture: &TextureLayout, u: u8, v: u8) -> io::Result<Option<Rgba>> {
        let (px, py) = texture.tpage_xy();
        let at = |x: usize, y: usize| -> io::Result<u16> {
            if x >= VRAM_WIDTH || y >= VRAM_HEIGHT {
                return Err(invalid(format!("VRAM read at ({x},{y}) is out of range")));
            }
            Ok(self.pixels[y * VRAM_WIDTH + x])
        };
        let (u, y) = (u as usize, py + v as usize);
        let raw = match texture.color_mode()? {
            ColorMode::Direct15 => at(px + u, y)?,
            mode => {
                let (cx, cy) = texture.clut_xy();
                let index = match mode {
                    ColorMode::Palette4 => (at(px + u / 4, y)? >> (u % 4 * 4) & 0xf) as usize,
                    _ => (at(px + u / 2, y)? >> (u % 2 * 8) & 0xff) as usize,
                };
                at(cx + index, cy)?
            }
        };
        if raw == 0 {
            return Ok(None);
        }
        let ch = |shift: u32| {
            let c = (raw >> shift & 0x1f) as u8;
            c << 3 | c >> 2
        };
        Ok(Some([ch(0), ch(5), ch(10), 255]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_quad_is_row_major_corners() {
        let m = FaceMode { rotation: 0, mode: 0 };
        // Corners in top-left, top-right, bottom-left, bottom-right order.
        assert_eq!(face_polygon(0, m).unwrap(), [0, 4, 5, 6]);
        assert_eq!(face_polygon(3, m).unwrap(), [6, 7, 8, 3]);
    }

    #[test]
    fn every_quad_rotation_is_a_permutation_of_the_corners() {
        for face in 0..4 {
            for rotation in 0..8 {
                let mut poly = face_polygon(face, FaceMode { rotation, mode: 0 }).unwrap();
                poly.sort();
                let mut corners = FACE_CORNERS[face];
                corners.sort();
                assert_eq!(poly, corners, "face {face} rotation {rotation}");
            }
        }
    }

    #[test]
    fn triangle_modes_have_three_distinct_vertices() {
        for mode in 1..3 {
            for rotation in 0..8 {
                let mut p = face_polygon(0, FaceMode { rotation, mode }).unwrap().to_vec();
                p.sort();
                p.dedup();
                assert_eq!(p.len(), 3, "mode {mode} rotation {rotation}");
            }
        }
    }

    #[test]
    fn layout_fields() {
        let b = [1, 2, 0x45, 0x01, 3, 4, 0x1d, 0x00, 5, 6, 7, 8];
        let t = TextureLayout::parse(&b).unwrap();
        assert_eq!(t.uv, [(1, 2), (3, 4), (5, 6), (7, 8)]);
        assert_eq!(t.clut_xy(), (5 * 16, 5));
        assert_eq!(t.tpage_xy(), (13 * 64, 256));
        assert_eq!(t.color_mode().unwrap(), ColorMode::Palette4);
        assert!(TextureLayout::parse(&[0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0]).is_err());
    }

    #[test]
    fn four_bit_texel_through_clut() {
        let mut vram = Vram::default();
        // Page 0 at (0,0); pixel u=5 lives in word 1, nibble 1.
        vram.pixels[1] = 0x0030;
        // CLUT at (16, 2): entry 3 is pure red (5551).
        vram.pixels[2 * VRAM_WIDTH + 16 + 3] = 0x001f;
        let t = TextureLayout { uv: [(0, 0); 4], clut: 2 << 6 | 1, tpage: 0 };
        assert_eq!(vram.texel(&t, 5, 0).unwrap(), Some([255, 0, 0, 255]));
        assert_eq!(vram.texel(&t, 0, 0).unwrap(), None);
    }

    #[test]
    fn tagged_pointer_follows_the_slot() {
        let mut p = vec![0u8; 0x40];
        p[0x20..0x24].copy_from_slice(&0x10u32.to_le_bytes());
        p[0x10 + 6] = 0x1d;
        let t = resolve_texture(&p, 0x21).unwrap().unwrap();
        assert_eq!(t.tpage, 0x1d);
        assert_eq!(resolve_texture(&p, 0x10).unwrap(), Some(t));
        assert_eq!(resolve_texture(&p, 0).unwrap(), None);
        assert!(resolve_texture(&p, 0x100).is_err());
    }
}
