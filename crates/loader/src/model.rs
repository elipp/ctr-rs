//! CTR models (`struct Model`, `ModelHeader`, `ModelAnim`, `ModelFrame`): the
//! racer models in `BigIndex::RACER_MODEL_*` and everything else drawn by
//! `RenderBucket_DrawFunc_Normal`.
//!
//! A model is a [`DramFile`]. Each LOD ([`ModelHeader`]) has a command list
//! that builds triangles out of triangle strips of vertices, and a table of
//! 8-bit vertex positions per animation frame (raw, or delta-compressed with a
//! bit stream). Positions are kept as the renderer's GTE vectors: the vertex
//! bytes plus the frame origin, in "model units" that the header's `scale`
//! (and the instance's) turn into world units: `world = v * scale / 0x1000`
//! per axis, with Y up as in the level.

use std::io;

use crate::dram::DramFile;
use crate::texture::{TEXTURE_LAYOUT_SIZE, TextureLayout};

fn invalid(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

fn word(dram: &DramFile, at: u32) -> io::Result<u32> {
    let at = at as usize;
    dram.data
        .get(at..at + 4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        .ok_or_else(|| invalid(format!("model word {at:#x} is outside the payload")))
}

fn half(dram: &DramFile, at: u32) -> io::Result<i16> {
    let at = at as usize;
    dram.data
        .get(at..at + 2)
        .map(|b| i16::from_le_bytes(b.try_into().unwrap()))
        .ok_or_else(|| invalid(format!("model half {at:#x} is outside the payload")))
}

fn name(dram: &DramFile, at: u32) -> io::Result<String> {
    let at = at as usize;
    let bytes = dram.data.get(at..at + 16).ok_or_else(|| invalid("model name is outside the payload"))?;
    let len = bytes.iter().position(|&b| b == 0).unwrap_or(16);
    Ok(String::from_utf8_lossy(&bytes[..len]).into_owned())
}

/// `struct Model`: a named group of LODs.
#[derive(Debug, Clone)]
pub struct Model {
    pub name: String,
    pub id: i16,
    pub headers: Vec<ModelHeader>,
}

/// `struct ModelHeader`: one LOD.
#[derive(Debug, Clone)]
pub struct ModelHeader {
    pub name: String,
    pub max_distance_lod: i16,
    pub flags: u16,
    pub scale: [i16; 3],
    /// The commands after the leading count, without the terminator.
    pub commands: Vec<u32>,
    /// Vertex colours, indexed by the commands; `R | G << 8 | B << 16`.
    pub colors: Vec<u32>,
    /// Texture layouts, indexed by `command & 0x1ff` minus one.
    pub textures: Vec<TextureLayout>,
    /// The single frame of a model without animations.
    pub static_frame: Option<Frame>,
    pub animations: Vec<Animation>,
}

/// `struct ModelAnim`, decompressed.
#[derive(Debug, Clone)]
pub struct Animation {
    pub name: String,
    /// Playback runs at twice `frames`' rate, blending in between.
    pub interpolated: bool,
    pub frames: Vec<Frame>,
}

/// One pose: a position for every vertex the commands read, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// `ModelFrame::pos`
    pub origin: [i16; 3],
    /// Model-space positions: the frame's vertex bytes plus its origin.
    pub vertices: Vec<[i32; 3]>,
}

/// A triangle of the model, in model units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Triangle {
    pub positions: [[i32; 3]; 3],
    /// `R | G << 8 | B << 16` of each corner; 0x80 is neutral.
    pub colors: [u32; 3],
    /// 1-based index into [`ModelHeader::textures`], 0 for none.
    pub texture: usize,
}

const MODEL_HEADER_SIZE: u32 = 0x40;
const MODEL_ANIM_SIZE: u32 = 0x18;
const END_OF_COMMANDS: u32 = 0xffff_ffff;

impl Model {
    pub fn parse(dram: &DramFile) -> io::Result<Self> {
        let count = half(dram, 0x12)?;
        let headers = word(dram, 0x14)?;
        if !(0..=16).contains(&count) {
            return Err(invalid(format!("implausible LOD count {count}")));
        }
        Ok(Self {
            name: name(dram, 0)?,
            id: half(dram, 0x10)?,
            headers: (0..count as u32)
                .map(|i| ModelHeader::parse(dram, headers + i * MODEL_HEADER_SIZE))
                .collect::<io::Result<_>>()?,
        })
    }
}

impl ModelHeader {
    fn parse(dram: &DramFile, at: u32) -> io::Result<Self> {
        let command_list = word(dram, at + 0x20)?;
        let frame_data = word(dram, at + 0x24)?;
        let tex_table = word(dram, at + 0x28)?;
        let colors = word(dram, at + 0x2c)?;
        let num_animations = word(dram, at + 0x34)?;
        let animations = word(dram, at + 0x38)?;

        let color_count = word(dram, command_list)?;
        let mut commands = Vec::new();
        for i in 1.. {
            match word(dram, command_list + i * 4)? {
                END_OF_COMMANDS => break,
                c => commands.push(c),
            }
        }

        let max_texture = commands.iter().filter(|&&c| c >> 16 != 0).map(|c| c & 0x1ff).max().unwrap_or(0);
        let textures = (1..=max_texture)
            .map(|i| {
                let ptr = word(dram, tex_table + (i - 1) * 4)?;
                let bytes = dram.data.get(ptr as usize..ptr as usize + TEXTURE_LAYOUT_SIZE);
                TextureLayout::parse(bytes.ok_or_else(|| invalid("texture layout is outside the payload"))?)
            })
            .collect::<io::Result<Vec<_>>>()?;

        let vertex_count = commands.iter().filter(|&&c| c >> 16 != 0 && c >> 24 & 4 == 0).count();

        let static_frame = if frame_data != 0 { Some(read_frame(dram, frame_data, vertex_count, None)?) } else { None };
        let animations = (0..num_animations)
            .map(|i| {
                let anim = word(dram, animations + i * 4)?;
                read_animation(dram, anim, vertex_count)
            })
            .collect::<io::Result<Vec<_>>>()?;

        Ok(Self {
            name: name(dram, at)?,
            max_distance_lod: half(dram, at + 0x14)?,
            flags: half(dram, at + 0x16)? as u16,
            scale: [half(dram, at + 0x18)?, half(dram, at + 0x1a)?, half(dram, at + 0x1c)?],
            commands,
            colors: (0..color_count).map(|i| word(dram, colors + i * 4)).collect::<io::Result<_>>()?,
            textures,
            static_frame,
            animations,
        })
    }

    /// `RenderBucket_DrawFunc_Normal`: assembles the triangles of a pose.
    /// Strips are expanded as the original feeds them to the GPU; no culling
    /// is applied, so the winding of a triangle is not meaningful.
    pub fn triangles(&self, frame: &Frame) -> Vec<Triangle> {
        #[derive(Clone, Copy, Default)]
        struct Corner {
            position: [i32; 3],
            color: u32,
        }

        let mut out = Vec::new();
        let mut stack = [Corner::default(); 256];
        let mut temp = [Corner::default(); 4];
        let mut strip_length = 0;
        let mut prim_command = 0;
        let mut vertex_index = 0;

        for &command in &self.commands {
            let flags = (command >> 24) as u8;
            let stack_index = (command >> 16) as u8 as usize;

            if command >> 16 == 0 {
                // Colour only: replaces the colours the next corners take.
                let color_a = self.color(command >> 9 & 0x7f);
                let color_b = self.color(command >> 2 & 0x7f);
                temp[1].color = color_a;
                temp[2].color = color_a;
                temp[3].color = color_b;
                continue;
            }

            if flags & 4 == 0 {
                stack[stack_index].position = frame.vertices.get(vertex_index).copied().unwrap_or_default();
                vertex_index += 1;
            }
            stack[stack_index].color = self.color(command >> 9 & 0x7f);

            temp.copy_within(1.., 0);
            temp[3] = stack[stack_index];

            let starts_new_strip = flags & 0x80 != 0;
            if starts_new_strip || strip_length == 0 {
                prim_command = command;
            }
            if starts_new_strip {
                strip_length = 0;
            }

            if strip_length > 2 && flags & 0x40 != 0 {
                temp[1] = temp[0];
            }

            if strip_length < 2 {
                strip_length += 1;
                continue;
            }

            let draw_command = if strip_length == 2 { prim_command } else { command };
            out.push(Triangle {
                positions: [temp[1].position, temp[2].position, temp[3].position],
                colors: [temp[1].color, temp[2].color, temp[3].color],
                texture: (draw_command & 0x1ff) as usize,
            });
            strip_length += 1;
        }
        out
    }

    fn color(&self, index: u32) -> u32 {
        self.colors.get(index as usize).copied().unwrap_or(0x80_8080)
    }
}

fn read_animation(dram: &DramFile, at: u32, vertex_count: usize) -> io::Result<Animation> {
    // With bit 15 set, only every other frame is stored and the odd ones are
    // blends of their neighbours (the instance's `animFrame` counts both).
    let count_word = word(dram, at + 0x10)?;
    let interpolated = count_word & 0x8000 != 0;
    let count = count_word & 0x7fff;
    let frames = if interpolated && count > 0 { ((count - 1) >> 1) + 1 } else { count };
    let frame_size = half(dram, at + 0x12)? as u16 as u32;
    let delta = word(dram, at + 0x14)?;
    let delta = (delta != 0).then_some(delta);
    Ok(Animation {
        name: name(dram, at)?,
        interpolated,
        frames: (0..frames)
            .map(|i| read_frame(dram, at + MODEL_ANIM_SIZE + i * frame_size, vertex_count, delta))
            .collect::<io::Result<_>>()?,
    })
}

/// `RenderBucket_UncompressAnimationFrame` for every vertex of a frame.
fn read_frame(dram: &DramFile, at: u32, vertex_count: usize, delta: Option<u32>) -> io::Result<Frame> {
    let origin = [half(dram, at)?, half(dram, at + 2)?, half(dram, at + 4)?];
    let vertex_offset = word(dram, at + 0x18)?;
    let data = at + vertex_offset;
    let o = origin.map(i32::from);

    let mut vertices = Vec::with_capacity(vertex_count);
    match delta {
        None => {
            let bytes = dram
                .data
                .get(data as usize..data as usize + vertex_count * 3)
                .ok_or_else(|| invalid("frame vertices are outside the payload"))?;
            for v in bytes.chunks_exact(3) {
                // Stored x, "y", "z"; the GTE vector is x, z, y.
                vertices.push([v[0] as i32 + o[0], v[2] as i32 + o[1], v[1] as i32 + o[2]]);
            }
        }
        Some(delta) => {
            let stream = dram.data.get(data as usize..).ok_or_else(|| invalid(format!("frame stream {data:#x} is outside the payload")))?;
            let (mut bit_index, mut accum) = (0usize, [0i32; 3]);
            for i in 0..vertex_count as u32 {
                let temporal = word(dram, delta + i * 4)?;
                let bits = [(temporal >> 6 & 7), (temporal >> 3 & 7), (temporal & 7)];
                let base = [sign_extend(temporal >> 25, 7) << 1, sign_extend(temporal >> 17, 8), sign_extend(temporal >> 9, 8)];
                for c in 0..3 {
                    let value = read_signed_bits(stream, &mut bit_index, bits[c] + 1);
                    accum[c] = if bits[c] == 7 { value as i8 as i32 } else { (accum[c] + value + base[c]) as i8 as i32 };
                }
                // Read order: x, then the GTE's Y, then its Z.
                vertices.push([accum[0] as u8 as i32 + o[0], accum[1] as u8 as i32 + o[1], accum[2] as u8 as i32 + o[2]]);
            }
        }
    }
    Ok(Frame { origin, vertices })
}

fn sign_extend(value: u32, bits: u32) -> i32 {
    let shift = 32 - bits;
    (value << shift) as i32 >> shift
}

/// `RenderBucket_GetSignedBits`: an MSB-first bit stream of little-endian words.
fn read_signed_bits(stream: &[u8], bit_index: &mut usize, bits: u32) -> i32 {
    let word_at = |i: usize| {
        stream.get(i * 4..i * 4 + 4).map_or(0, |b| u32::from_le_bytes(b.try_into().unwrap()))
    };
    let b = *bit_index >> 5;
    let shift = (32 - bits as i32) - (*bit_index & 31) as i32;
    let value = if shift < 0 {
        (word_at(b) << -shift) | (word_at(b + 1) >> (shift & 31))
    } else {
        word_at(b) >> shift
    };
    *bit_index += bits as usize;
    sign_extend(value, bits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_bit_fields_cross_word_boundaries() {
        // 0xf0000000 then 0x0000000f, little-endian words, MSB first: the
        // stream reads 1111 0000 ... 0000 1111.
        let stream = [0x00, 0x00, 0x00, 0xf0, 0x0f, 0x00, 0x00, 0x00];
        let mut at = 0;
        assert_eq!(read_signed_bits(&stream, &mut at, 4), -1);
        assert_eq!(read_signed_bits(&stream, &mut at, 4), 0);
        at = 28;
        // Four bits from the first word's end and four from the second's start.
        assert_eq!(read_signed_bits(&stream, &mut at, 8), 0);
        assert_eq!(at, 36);
        // The last byte of the second word: 0000 1111.
        at = 56;
        assert_eq!(read_signed_bits(&stream, &mut at, 8), 15);
        // Reading past the end gives zero bits.
        assert_eq!(read_signed_bits(&stream, &mut at, 8), 0);
    }

    #[test]
    fn sign_extension() {
        assert_eq!(sign_extend(0x7f, 7), -1);
        assert_eq!(sign_extend(0x3f, 7), 63);
        assert_eq!(sign_extend(0x80, 8), -128);
    }
}
