//! Animated scenery vertices (`AnimateQuad`, `struct SCVert`): the swaying of plants and seaweed.
//! A level that has them (`Level::config_flags & 4`) uses them in place of water vertices. Each
//! one circles around its stored position with a phase of its own, and may also darken in step.

use std::io;

use gte::Gte;

use crate::lev::SVec3;

const LEVEL_NUM_SCENERY: usize = 0x174;
const LEVEL_PTR_SCENERY: usize = 0x178;
const SC_VERT_SIZE: usize = 0x10;
const LEV_VERTEX_SIZE: usize = 0x10;

/// `gte_dpcs`
const DPCS: u32 = 0x0780010;

/// The game calls `AnimateQuad` with its timer shifted by this ("emulate 30 fps on 60 fps").
const TIMER_SHIFT: u32 = 7;

/// `AnimateQuadVertex`'s flag bits (the high half of the `z` word).
const FLAG_MOVE: u16 = 0x8000;
const FLAG_COLOR: u16 = 0x4000;
const PHASE_MASK: u16 = 0x3fff;

fn invalid(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

/// One `struct SCVert`.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceneryVertex {
    /// Index into `Level::vertices`.
    pub vertex: usize,
    /// `offset_pos_xy`: x in the low half, y in the high half.
    pub xy: u32,
    /// `offset_pos_zw`: z in the low half, the flags in the high half.
    pub zw: u32,
    /// `offset_color_rgba`: the colour at full brightness.
    pub color: u32,
}

/// What the game writes into a vertex on a tick.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceneryUpdate {
    pub vertex: usize,
    pub pos: Option<SVec3>,
    /// `color_hi` and `color_lo` (the same), `R G B code`.
    pub color: Option<[u8; 4]>,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct Scenery {
    pub vertices: Vec<SceneryVertex>,
}

impl Scenery {
    pub(crate) fn parse(payload: &[u8], verts_at: usize, num_verts: usize) -> io::Result<Self> {
        let u32_at = |at: usize| -> io::Result<u32> {
            payload
                .get(at..at + 4)
                .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
                .ok_or_else(|| invalid(format!("scenery read at {at:#x} is out of range")))
        };
        let count = u32_at(LEVEL_NUM_SCENERY)? as usize;
        let table = u32_at(LEVEL_PTR_SCENERY)? as usize;
        if count == 0 || table == 0 {
            return Ok(Self::default());
        }
        let vertices = (0..count)
            .map(|i| {
                let entry = table + i * SC_VERT_SIZE;
                let vertex_ptr = u32_at(entry)? as usize;
                let offset = vertex_ptr
                    .checked_sub(verts_at)
                    .filter(|o| o % LEV_VERTEX_SIZE == 0 && o / LEV_VERTEX_SIZE < num_verts)
                    .ok_or_else(|| invalid(format!("scenery vertex {i} points at {vertex_ptr:#x}, not into the vertex array")))?;
                Ok(SceneryVertex {
                    vertex: offset / LEV_VERTEX_SIZE,
                    xy: u32_at(entry + 4)?,
                    zw: u32_at(entry + 8)?,
                    color: u32_at(entry + 0xc)?,
                })
            })
            .collect::<io::Result<_>>()?;
        Ok(Self { vertices })
    }

    /// `AnimateQuad` at game tick `tick` (`gGT->timer`, 30 Hz) for the vertices `visible` allows
    /// (the game only touches vertices some camera sees).
    pub fn animate(&self, tick: u32, visible: impl Fn(usize) -> bool) -> Vec<SceneryUpdate> {
        let timer = tick << TIMER_SHIFT;
        let mut gte = Gte::new();
        for far in 21..=23 {
            gte.ctc2(0, far);
        }
        self.vertices
            .iter()
            .enumerate()
            .filter(|(i, _)| visible(*i))
            .map(|(_, v)| {
                let flags = (v.zw >> 16) as u16;
                let (sin, cos) = math::sin_cos(((flags & PHASE_MASK) as u32).wrapping_add(timer));
                let pos = (flags & FLAG_MOVE != 0).then(|| {
                    // The game adds to the whole word, so a negative offset borrows from y,
                    // and ORs the original y back in.
                    let word = v.xy.wrapping_add((sin >> 7) as u32) | (v.xy >> 16 << 16);
                    SVec3 {
                        x: word as i16,
                        y: (word >> 16) as i16,
                        z: v.zw.wrapping_add((cos >> 7) as u32) as i16,
                    }
                });
                let color = (flags & FLAG_COLOR != 0).then(|| {
                    gte.mtc2(((sin + 0x1000) >> 2) as u32, 8);
                    gte.mtc2(v.color, 6);
                    gte.cop2(DPCS);
                    gte.mfc2(22).to_le_bytes()
                });
                SceneryUpdate {
                    vertex: v.vertex,
                    pos,
                    color,
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scenery(xy: u32, zw: u32, color: u32) -> Scenery {
        Scenery {
            vertices: vec![SceneryVertex {
                vertex: 3,
                xy,
                zw,
                color,
            }],
        }
    }

    #[test]
    fn a_moving_vertex_circles_its_stored_position() {
        let s = scenery(100 | 50 << 16, 200 | 0x8000 << 16, 0);
        let at = |tick| s.animate(tick, |_| true)[0].pos.unwrap();
        // Phase 0, tick 0: sin 0, cos 1.0 (0x1000 >> 7 = 32 units).
        assert_eq!(
            at(0),
            SVec3 {
                x: 100,
                y: 50,
                z: 200 + 32
            }
        );
        // A quarter turn is 0x400 angle units, 8 ticks.
        assert_eq!(
            at(8),
            SVec3 {
                x: 100 + 32,
                y: 50,
                z: 200
            }
        );
        // A whole turn is 32 ticks.
        assert_eq!(at(32), at(0));
        assert_eq!(s.animate(0, |_| true)[0].color, None);
    }

    #[test]
    fn a_vertex_without_the_flag_is_left_alone() {
        assert_eq!(
            scenery(0, 0x4000 << 16, 0x00ff_ffff).animate(0, |_| true)[0].pos,
            None
        );
    }

    #[test]
    fn the_colour_darkens_to_half_at_the_top_of_the_swing() {
        let s = scenery(0, 0x4000 << 16, 0x0000_ff80);
        // Phase 0: sin 0, IR0 = 0x400, so a quarter of the way to black.
        let [r, g, b, _] = s.animate(0, |_| true)[0].color.unwrap();
        assert_eq!([r, g, b], [96, 191, 0]);
        // A quarter turn on: sin 1.0, IR0 = 0x800, half way.
        let [r, ..] = s.animate(8, |_| true)[0].color.unwrap();
        assert_eq!(r, 0x40);
    }
}
