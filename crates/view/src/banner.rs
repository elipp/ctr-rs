//! The start banner waving (`RB_Banner_*`, `STATIC_STARTBANNERWAVE`): its colours are a ring of 64
//! greys that is two cycles of a sine, turned one step every tick. Each vertex takes the grey of
//! its place along the banner (its x) for its shade, and for how far it sticks out (its y byte),
//! less so near the two ends, so the wave runs along the cloth.
//!
//! The model is shown as the level has it until [`wave`] first poses it.

use bevy::prelude::*;
use loader::model::{Frame, ModelHeader};
use render::mesh::{self, PARTS};

use crate::GameTick;

/// The greys of the ring (the first of the model's colours).
const RING: usize = 64;

/// A banner that waves.
pub struct Banner {
    /// The model with its commands made to take the colour of their vertex's place
    /// (`RB_Banner_Animate_Init`), and its ring set to the sine.
    pub header: ModelHeader,
    /// The pose the vertices were read from: only their heights change.
    frame: Frame,
    /// How many vertices the frame has that the commands read.
    count: usize,
    ring: [u32; RING],
    pub inst_scale: [i16; 3],
    pub parts: [Option<Handle<Mesh>>; PARTS],
    shown: Option<u32>,
}

impl Banner {
    /// `RB_Banner_Animate_Init` and the ring `RB_Banner_LInB` sets, for the model in `frame`; none
    /// if the model does not carry a ring.
    pub fn new(header: &ModelHeader, frame: &Frame, inst_scale: [i16; 3]) -> Option<Banner> {
        if header.colors.len() < RING {
            return None;
        }
        let mut header = header.clone();
        let x_of = |i: usize| (frame.vertices[i][0] - frame.origin[0] as i32) as u8 as u32;

        // Each command takes the colour at a quarter of its vertex's x. A command that reuses a
        // vertex from the stack takes the x it was pushed with.
        let mut stack = [0u32; 256];
        let mut count = 0;
        for command in &mut header.commands {
            if *command >> 16 == 0 {
                continue;
            }
            let slot = (*command >> 16) as u8 as usize;
            if *command & 0x0400_0000 == 0 {
                if count >= frame.vertices.len() {
                    return None;
                }
                stack[slot] = x_of(count);
                count += 1;
                *command &= 0xffff_01ff;
            } else {
                *command &= 0xf7ff_01ff;
            }
            *command |= (stack[slot] >> 2) << 9;
        }
        if count == 0 {
            return None;
        }

        // Two cycles of a sine over the ring, as grey.
        let mut ring = [0; RING];
        for (i, grey) in ring.iter_mut().enumerate() {
            let sine = math::sin((i * 128) as u32);
            let value = ((sine >> 6) + 0x80) as u32 & 0xff;
            *grey = header.colors[i] & 0xff00_0000 | (value * 0x01_0101);
        }
        header.colors[..RING].copy_from_slice(&ring);
        Some(Banner {
            header,
            frame: frame.clone(),
            count,
            ring,
            inst_scale,
            parts: Default::default(),
            shown: None,
        })
    }

    /// How many vertices wave.
    pub fn vertex_count(&self) -> usize {
        self.count
    }

    /// `RB_Banner_Animate_Play` after `ticks` ticks: the ring has turned that many steps, and the
    /// vertices stick out by the grey at their place.
    fn pose(&mut self, ticks: u32) -> [(Vec<[f32; 3]>, Vec<[f32; 4]>); PARTS] {
        let turn = (ticks as usize + 1) % RING;
        let mut ring = self.ring;
        ring.rotate_left(turn);
        self.header.colors[..RING].copy_from_slice(&ring);

        let origin = self.frame.origin.map(i32::from);
        for vertex in self.frame.vertices.iter_mut().take(self.count) {
            let x = (vertex[0] - origin[0]) as u8 as i32;
            let grey = (ring[((x >> 2) + 10) as usize & 0x3f] & 0xff) as i32;
            let wave = grey - 0x80;
            // The outer quarters taper toward the ends; the middle keeps all of it.
            let scaled = if x < 0x40 {
                (wave * (x * 4)) >> 8
            } else if x > 0xc0 {
                (wave * ((0x100 - x) * 4)) >> 8
            } else {
                wave
            };
            vertex[2] = (scaled - 0x80) as u8 as i32 + origin[2];
        }
        mesh::model_vertices(&self.header, &self.frame, self.inst_scale)
    }
}

#[derive(Resource, Default)]
pub struct Banners(pub Vec<Banner>);

/// Waves the banners, once per game tick.
pub fn wave(tick: Res<GameTick>, mut banners: ResMut<Banners>, mut meshes: ResMut<Assets<Mesh>>) {
    for banner in &mut banners.0 {
        if banner.shown == Some(tick.0) {
            continue;
        }
        banner.shown = Some(tick.0);
        let poses = banner.pose(tick.0);
        for (handle, (positions, colors)) in banner.parts.iter().zip(poses) {
            let Some(mut mesh) = handle.as_ref().and_then(|h| meshes.get_mut(h)) else {
                continue;
            };
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
            mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
        }
    }
}
