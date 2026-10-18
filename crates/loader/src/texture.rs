//! Texture layouts of level quad blocks and how they map onto a block's
//! 3x3 vertex patch, plus PS1 pixel decoding (`Vram::texel`).

use std::io;

use crate::dram::DramFile;
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

/// Semi-transparency mode of a face. Only texels with the STP bit blend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blend {
    Opaque,
    /// Half of the face plus half of the background.
    Average,
    Add,
    Subtract,
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

    /// How the face blends with what is behind it, from the page's bits 5-6.
    pub fn blend(&self) -> Blend {
        match self.tpage >> 5 & 3 {
            0 => Blend::Average,
            1 => Blend::Add,
            2 => Blend::Subtract,
            // The level renderer draws these with the opaque primitive code
            // (`DrawLevelOvr1P_SelectRawPrimitiveCode`).
            _ => Blend::Opaque,
        }
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

/// `struct IconGroup4`: the same picture at decreasing distance, as the game
/// picks it by depth (`DrawLevelOvr1P_GetProjectedMidTexture`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextureGroup {
    pub far: TextureLayout,
    pub middle: TextureLayout,
    pub near: TextureLayout,
    /// The fourth slot: a higher-resolution layout of the same picture, or
    /// `None` when the slot holds a pointer to per-sub-quad mosaic data
    /// instead (not supported).
    pub mosaic: Option<TextureLayout>,
}

impl TextureGroup {
    /// The sharpest single layout the group offers.
    pub fn best(&self) -> &TextureLayout {
        self.mosaic.as_ref().unwrap_or(&self.near)
    }
}

pub const TEXTURE_GROUP_SIZE: usize = 4 * TEXTURE_LAYOUT_SIZE;

fn word_at(payload: &[u8], at: usize) -> io::Result<u32> {
    payload
        .get(at..at + 4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        .ok_or_else(|| invalid(format!("texture pointer {at:#x} is outside the payload")))
}

fn group_at(dram: &DramFile, at: usize) -> io::Result<TextureGroup> {
    let bytes = dram
        .data
        .get(at..at + TEXTURE_GROUP_SIZE)
        .ok_or_else(|| invalid(format!("texture group {at:#x} is outside the payload")))?;
    let layout = |i: usize| TextureLayout::parse(&bytes[i * TEXTURE_LAYOUT_SIZE..]);
    // A relocated pointer in the slot is what the game calls a "negative"
    // mosaic word; anything else is a layout.
    let mosaic = if dram.is_pointer((at + 3 * TEXTURE_LAYOUT_SIZE) as u32) { None } else { layout(3).ok() };
    Ok(TextureGroup { far: layout(0)?, middle: layout(1)?, near: layout(2)?, mosaic })
}

/// Follows a quad block texture pointer (a payload offset) to its group.
/// A pointer with bit 0 set is an [`AnimTex`] whose first word is the current
/// group pointer (the word `CTR_CycleTex_LEV` rewrites), so this returns
/// whatever frame the file was saved with; see [`resolve_animation`].
pub fn resolve_texture(dram: &DramFile, ptr: u32) -> io::Result<Option<TextureGroup>> {
    let at = match ptr {
        0 => return Ok(None),
        p if p & 1 == 1 => word_at(&dram.data, p as usize - 1)? as usize,
        p => p as usize,
    };
    if at == 0 {
        return Ok(None);
    }
    group_at(dram, at).map(Some)
}

/// `struct AnimTex`: the frames of one animated level texture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnimTex {
    pub frame_offset: i16,
    /// The tick counter is shifted right by this much (1 = half speed).
    pub frame_skip: u8,
    pub frames: Vec<TextureGroup>,
}

impl AnimTex {
    /// Frame shown at game tick `tick` (`CTR_CycleTex_LEV`; the game ticks at 30 Hz).
    pub fn frame_at(&self, tick: i32) -> usize {
        // The C `%` could go negative for a negative offset and index out of
        // bounds; the euclidean remainder is the same for everything valid.
        ((tick.wrapping_add(self.frame_offset as i32)) >> self.frame_skip).rem_euclid(self.frames.len() as i32) as usize
    }
}

/// The animation behind a texture pointer, if it is the animated kind.
pub fn resolve_animation(dram: &DramFile, ptr: u32) -> io::Result<Option<AnimTex>> {
    if ptr & 1 == 0 {
        return Ok(None);
    }
    let at = ptr as usize - 1;
    let header = dram
        .data
        .get(at..at + 0xc)
        .ok_or_else(|| invalid(format!("animated texture {at:#x} is outside the payload")))?;
    let i16_at = |o: usize| i16::from_le_bytes([header[o], header[o + 1]]);
    let (num_frames, frame_offset, frame_skip) = (i16_at(4), i16_at(6), i16_at(8));
    if num_frames <= 0 || !(0..16).contains(&frame_skip) {
        return Err(invalid(format!("animated texture {at:#x}: {num_frames} frames, skip {frame_skip}")));
    }
    let frames = (0..num_frames as usize)
        .map(|i| group_at(dram, word_at(&dram.data, at + 0xc + i * 4)? as usize))
        .collect::<io::Result<_>>()?;
    Ok(Some(AnimTex { frame_offset, frame_skip: frame_skip as u8, frames }))
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
/// Most of the bits above the low 5 of each byte steer back-face culling; bit 23
/// mirrors the UVs ([`face_uv_order`]).
const VERTEX_TABLE: [u32; 24] = [
    0x18100800, 0x80900818, 0x00081018, 0x98881000, 0x98081000, 0x00881018, 0x80100818, 0x18900800,
    0x10081898, 0x88881810, 0x18180810, 0x90980888, 0x90180888, 0x18980810, 0x88081810, 0x10881898,
    0x08181090, 0x98981008, 0x10101808, 0x88901898, 0x88101898, 0x10901808, 0x98181008, 0x08981090,
];

/// Which of the layout's four (u, v) pairs polygon vertex k takes. Bit 23 of the
/// table word mirrors the texture horizontally (`DrawLevelOvr1P_WriteProjectedUv`
/// swaps u0 with u1 and u2 with u3); that is every odd rotation.
pub fn face_uv_order(mode: FaceMode) -> io::Result<[usize; 4]> {
    let entry = table_entry(mode)?;
    Ok(if entry & 0x0080_0000 != 0 { [1, 0, 3, 2] } else { [0, 1, 2, 3] })
}

fn table_entry(mode: FaceMode) -> io::Result<u32> {
    VERTEX_TABLE
        .get(mode.mode as usize * 8 + mode.rotation as usize)
        .copied()
        .ok_or_else(|| invalid(format!("face mode {} does not exist", mode.mode)))
}

/// Positions in `QuadBlock::vertex_index` of the polygon vertices of `face`
/// (0..4). Vertex k carries `TextureLayout::uv[k]`. The face is drawn as the
/// triangles (0, 1, 2) and (3, 1, 2); in the triangle modes one vertex is
/// repeated, which degenerates one of the two.
pub fn face_polygon(face: usize, mode: FaceMode) -> io::Result<[usize; 4]> {
    let entry = table_entry(mode)?;
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
        Ok(self.texel_stp(texture, u, v)?.map(|(rgba, _)| rgba))
    }

    /// Like [`Vram::texel`], also returning the texel's semi-transparency bit
    /// (bit 15), which decides whether a semi-transparent face blends it.
    pub fn texel_stp(&self, texture: &TextureLayout, u: u8, v: u8) -> io::Result<Option<(Rgba, bool)>> {
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
        Ok(Some(([ch(0), ch(5), ch(10), 255], raw & 0x8000 != 0)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A DRAM file around `payload`, with `pointers` in its pointer map.
    fn dram(payload: Vec<u8>, pointers: &[u32]) -> DramFile {
        let mut file = (payload.len() as i32).to_le_bytes().to_vec();
        file.extend(&payload);
        file.extend(((pointers.len() * 4) as u32).to_le_bytes());
        file.extend(pointers.iter().flat_map(|p| p.to_le_bytes()));
        DramFile::parse(&file).unwrap()
    }

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
    fn animation_frames_and_timing() {
        let mut p = vec![0u8; 0x200];
        // AnimTex at 0x80: active group, 3 frames, offset 1, skip 1, then frame pointers.
        p[0x84..0x86].copy_from_slice(&3i16.to_le_bytes());
        p[0x86..0x88].copy_from_slice(&1i16.to_le_bytes());
        p[0x88..0x8a].copy_from_slice(&1i16.to_le_bytes());
        for (i, g) in [0x100u32, 0x140, 0x180].iter().enumerate() {
            p[0x8c + i * 4..][..4].copy_from_slice(&g.to_le_bytes());
            p[*g as usize + 0x18 + 6] = i as u8 + 1; // near.tpage
        }
        let p = dram(p, &[]);
        let a = resolve_animation(&p, 0x81).unwrap().unwrap();
        assert_eq!(a.frames.iter().map(|g| g.near.tpage).collect::<Vec<_>>(), [1, 2, 3]);
        // (tick + 1) >> 1: ticks 0 -> 0, 1 -> 1, 3 -> 2, 5 -> 0 (wraps), negative stays in range.
        assert_eq!([0, 1, 3, 5].map(|t| a.frame_at(t)), [0, 1, 2, 0]);
        assert!(a.frame_at(-7) < 3);
        assert_eq!(resolve_animation(&p, 0x80).unwrap(), None);
        // Static pointer to a bit-0 slot still resolves to the saved frame.
        assert_eq!(resolve_texture(&p, 0x81).unwrap(), None);
    }

    #[test]
    fn odd_rotations_mirror_the_texture() {
        for mode in 0..3 {
            for rotation in 0..8 {
                let order = face_uv_order(FaceMode { rotation, mode }).unwrap();
                let mirrored = rotation % 2 == 1;
                assert_eq!(order == [1, 0, 3, 2], mirrored, "mode {mode} rotation {rotation}");
            }
        }
    }

    #[test]
    fn mosaic_slot_is_a_layout_unless_it_is_a_pointer() {
        let mut p = vec![0u8; 0x100];
        p[0x20 + 0x24 + 6] = 0x1d; // a plausible tpage in the fourth layout
        let as_layout = dram(p.clone(), &[]);
        let g = resolve_texture(&as_layout, 0x20).unwrap().unwrap();
        assert_eq!(g.mosaic.map(|m| m.tpage), Some(0x1d));
        assert_eq!(g.best().tpage, 0x1d);
        let as_pointer = dram(p, &[0x20 + 0x24]);
        let g = resolve_texture(&as_pointer, 0x20).unwrap().unwrap();
        assert_eq!(g.mosaic, None);
        assert_eq!(g.best(), &g.near);
    }

    #[test]
    fn blend_modes() {
        let t = |mode: u16| TextureLayout { uv: [(0, 0); 4], clut: 0, tpage: mode << 5 };
        assert_eq!([0, 1, 2, 3].map(|m| t(m).blend()), [Blend::Average, Blend::Add, Blend::Subtract, Blend::Opaque]);
    }

    #[test]
    fn tagged_pointer_follows_the_slot() {
        let mut p = vec![0u8; 0x100];
        p[0x80..0x84].copy_from_slice(&0x10u32.to_le_bytes());
        p[0x10 + 0x18 + 6] = 0x1d;
        let p = dram(p, &[]);
        let t = resolve_texture(&p, 0x81).unwrap().unwrap();
        assert_eq!(t.near.tpage, 0x1d);
        assert_eq!(t.far.tpage, 0);
        assert_eq!(resolve_texture(&p, 0x10).unwrap(), Some(t));
        assert_eq!(resolve_texture(&p, 0).unwrap(), None);
        assert!(resolve_texture(&p, 0xf8).is_err());
    }
}
