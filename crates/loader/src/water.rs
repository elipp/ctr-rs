//! Animated water vertices (`AnimateWater1P` and friends). The file stores
//! placeholder colours for these vertices; every game tick the game rewrites
//! their `color_hi` (a gray) and `color_lo` (the UV the water list draws them
//! with) from a 28-frame colour table per vertex.

use std::io;

use gte::Gte;

use crate::texture::{TEXTURE_LAYOUT_SIZE, TextureLayout};

/// `struct mesh_info`-adjacent `struct Level` fields.
const LEVEL_NUM_WATER_VERTICES: usize = 0x34;
const LEVEL_PTR_WATER: usize = 0x38;
const LEVEL_PTR_WATER_ENV_MAP: usize = 0x44;

const WATER_VERT_SIZE: usize = 8;
const LEV_VERTEX_SIZE: usize = 0x10;
/// Frames in an `OVert`.
pub const WATER_FRAMES: usize = 28;
/// Game ticks each table frame lasts; the game blends between neighbours.
const TICKS_PER_FRAME: u32 = 8;

/// `gte_dpcs`
const DPCS: u32 = 0x0780010;

fn invalid(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

/// One entry of `struct WaterVert`: a level vertex and its colour table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaterVertex {
    /// Index into `Level::vertices`.
    pub vertex: usize,
    pub frames: [u16; WATER_FRAMES],
}

/// What the game writes into a water vertex on a tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaterColor {
    pub vertex: usize,
    /// Gray vertex colour (`color_hi`, the same value in all three channels).
    pub gray: u8,
    /// `color_lo[0..2]`: UV of the vertex, already offset by the environment map.
    pub uv: [u8; 2],
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Water {
    pub vertices: Vec<WaterVertex>,
    /// First word of the environment map layout: its (u0, v0), added to the vertex UVs.
    pub env_map_offset: u16,
    /// The environment map layout. The water list draws every water quad with
    /// its palette and texture page (`SeedWaterListState`), taking the UVs from
    /// the animated vertices. `None` on levels without water.
    pub env_map: Option<TextureLayout>,
}

impl Water {
    pub(crate) fn parse(payload: &[u8], verts_at: usize, num_verts: usize) -> io::Result<Self> {
        let u32_at = |at: usize| -> io::Result<u32> {
            payload
                .get(at..at + 4)
                .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
                .ok_or_else(|| invalid(format!("water read at {at:#x} is out of range")))
        };
        let count = u32_at(LEVEL_NUM_WATER_VERTICES)? as usize;
        if count == 0 {
            return Ok(Self::default());
        }
        let table = u32_at(LEVEL_PTR_WATER)? as usize;
        let env = u32_at(LEVEL_PTR_WATER_ENV_MAP)? as usize;
        let env_bytes = payload
            .get(env..env + TEXTURE_LAYOUT_SIZE)
            .ok_or_else(|| invalid("water environment map is outside the payload"))?;
        let env_map_offset = u16::from_le_bytes([env_bytes[0], env_bytes[1]]);
        let env_map = TextureLayout::parse(env_bytes)?;

        let vertices = (0..count)
            .map(|i| {
                let entry = table + i * WATER_VERT_SIZE;
                let vertex_ptr = u32_at(entry)? as usize;
                let colors = u32_at(entry + 4)? as usize;
                let offset = vertex_ptr
                    .checked_sub(verts_at)
                    .filter(|o| o % LEV_VERTEX_SIZE == 0 && o / LEV_VERTEX_SIZE < num_verts)
                    .ok_or_else(|| {
                        invalid(format!(
                            "water vertex {i} points at {vertex_ptr:#x}, not into the vertex array"
                        ))
                    })?;
                let bytes = payload
                    .get(colors..colors + WATER_FRAMES * 2)
                    .ok_or_else(|| {
                        invalid(format!("water colour table {i} is outside the payload"))
                    })?;
                let mut frames = [0u16; WATER_FRAMES];
                for (f, b) in frames.iter_mut().zip(bytes.as_chunks::<2>().0) {
                    *f = u16::from_le_bytes([b[0], b[1]]);
                }
                Ok(WaterVertex {
                    vertex: offset / LEV_VERTEX_SIZE,
                    frames,
                })
            })
            .collect::<io::Result<_>>()?;
        Ok(Self {
            vertices,
            env_map_offset,
            env_map: Some(env_map),
        })
    }

    /// `AnimateWater_Common` at game tick `tick` (`gGT->timer`, 30 Hz) for the
    /// vertices `visible` allows (the game only touches vertices some camera sees).
    pub fn animate(&self, tick: u32, visible: impl Fn(usize) -> bool) -> Vec<WaterColor> {
        let first_frame = (tick / TICKS_PER_FRAME) as usize % WATER_FRAMES;
        let second_frame = (first_frame + 1) % WATER_FRAMES;

        let mut gte = Gte::new();
        gte.mtc2((tick & (TICKS_PER_FRAME - 1)) << 9, 8);
        self.vertices
            .iter()
            .enumerate()
            .filter(|(i, _)| visible(*i))
            .map(|(_, v)| {
                let (first, second) = (v.frames[first_frame] as u32, v.frames[second_frame] as u32);

                let mut rgb = first & 0x3f;
                let far_r = (second & 0x3f) << 4;
                let far_g = (second & 0xfc0) >> 2;
                let mut far_b = (second & 0xf000) >> 4;

                rgb |= (first & 0xfc0) << 2;
                rgb |= (first & 0xf000) << 8;
                rgb |= (first & 0xf000) << 4;
                far_b |= (second & 0xf000) >> 8;

                gte.mtc2(rgb, 6);
                gte.ctc2(far_r, 21);
                gte.ctc2(far_g, 22);
                gte.ctc2(far_b, 23);
                gte.cop2(DPCS);
                let color = gte.mfc2(22);

                let uv = (color as u16)
                    .wrapping_add(self.env_map_offset)
                    .to_le_bytes();
                WaterColor {
                    vertex: v.vertex,
                    gray: (color >> 16) as u8,
                    uv,
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn water(frames: [u16; WATER_FRAMES]) -> Water {
        Water {
            vertices: vec![WaterVertex { vertex: 7, frames }],
            env_map_offset: 0x1840,
            env_map: None,
        }
    }

    #[test]
    fn first_frame_boundary_returns_its_own_colour() {
        // Tick 0: IR0 = 0, so DPCS returns the near colour untouched.
        // first = 0xa000 | (g=5 << 6) | r=3: R = 3, G = 5, B nibble 0xa -> 0xaa.
        let mut frames = [0u16; WATER_FRAMES];
        frames[0] = 0xa000 | 5 << 6 | 3;
        let c = water(frames).animate(0, |_| true);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].vertex, 7);
        assert_eq!(c[0].gray, 0xaa);
        assert_eq!(c[0].uv, ((5u16 << 8 | 3) + 0x1840).to_le_bytes());
    }

    #[test]
    fn frames_advance_every_eight_ticks_and_wrap() {
        let mut frames = [0u16; WATER_FRAMES];
        frames[0] = 0x1000;
        frames[1] = 0x2000;
        frames[27] = 0x3000;
        let w = water(frames);
        let gray = |tick| w.animate(tick, |_| true)[0].gray;
        assert_eq!(gray(0), 0x11);
        assert_eq!(gray(8), 0x22);
        // 28 frames of 8 ticks, then back to frame 0.
        assert_eq!(gray(28 * 8), 0x11);
        assert_eq!(gray(27 * 8), 0x33);
    }

    #[test]
    fn invisible_vertices_are_skipped() {
        assert!(water([0; WATER_FRAMES]).animate(0, |_| false).is_empty());
    }
}
