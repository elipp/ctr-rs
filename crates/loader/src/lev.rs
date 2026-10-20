//! Track geometry from a `LEV` subfile (the game's `struct Level`).
//!
//! A LEV file is a [`DramFile`](crate::dram::DramFile); every pointer in it is
//! an offset into the payload. This module decodes the parts needed for
//! collision and physics: the quad-block mesh, vertices, the BSP tree, driver
//! spawns and restart points, the skybox and the placed models. Visibility data
//! is not parsed yet.

use std::io;

use crate::dram::DramFile;
use crate::scenery::Scenery;
use crate::water::Water;

/// 16-bit fixed-point position, as stored in the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SVec3 {
    pub x: i16,
    pub y: i16,
    pub z: i16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BoundingBox {
    pub min: SVec3,
    pub max: SVec3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LevVertex {
    pub pos: SVec3,
    pub flags: u16,
    pub color_hi: [u8; 4],
    pub color_lo: [u8; 4],
}

/// `QUADBLOCK_FLAG_*`
pub mod quad_flags {
    pub const REFLECT_SPLIT_LINE_1: u16 = 0x0001;
    pub const LOW_GRAVITY: u16 = 0x0002;
    pub const REFLECT_SPLIT_LINE_0: u16 = 0x0004;
    pub const NO_COLLISION_RESPONSE: u16 = 0x0010;
    pub const TRIGGER: u16 = 0x0040;
    pub const ENGINE_ECHO: u16 = 0x0080;
    pub const KILL_PLANE: u16 = 0x0200;
    pub const DOOR: u16 = 0x0400;
    pub const CAMERA_SEARCH: u16 = 0x0800;
    pub const GROUND: u16 = 0x1000;
    pub const COLLISION_SURFACE: u16 = 0x2000;
    pub const NO_CAMERA_RESPAWN_PROBE: u16 = 0x4000;
    pub const SKIP_WATER_LIST: u16 = 0x8000;
}

/// A 3x3 patch of vertices (9 indices) that forms four quads (or eight triangles).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuadBlock {
    /// Indices into [`Level::vertices`].
    pub vertex_index: [u16; 9],
    pub flags: u16,
    pub draw_order_low: u32,
    pub draw_order_high: u32,
    /// Payload offsets of the mid-detail texture layouts (0 = none).
    pub texture_mid: [u32; 4],
    pub bbox: BoundingBox,
    pub terrain_type: u8,
    pub weather_intensity: u8,
    pub weather_vanish_rate: u8,
    pub mul_norm_vec_y: i8,
    pub block_id: i16,
    pub checkpoint_index: u8,
    pub tri_normal_vec_bit_shift: i8,
    /// Payload offset of the low-detail texture layout (0 = none).
    pub texture_low: u32,
    /// Payload offset of the `PVS` struct (0 = none).
    pub pvs: u32,
    /// Dividends for the per-triangle normals, see `FUN_8001f2dc` in the decomp.
    pub tri_normal_vec_dividend: [i16; 10],
}

/// A reference from a BSP branch to a child node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BspChild {
    None,
    Branch(u16),
    Leaf(u16),
}

impl BspChild {
    fn decode(raw: u16) -> Self {
        const INDEX_MASK: u16 = 0x3fff;
        const LEAF_FLAG: u16 = 0x4000;
        if raw == 0xffff {
            Self::None
        } else if raw & LEAF_FLAG != 0 {
            Self::Leaf(raw & INDEX_MASK)
        } else {
            Self::Branch(raw & INDEX_MASK)
        }
    }
}

/// An instance's hitbox, which the leaves it touches list (`struct BSP` as `data.hitbox`): a sphere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BspHitbox {
    /// Where the entry is in the file: the game tells hitboxes apart by address.
    pub offset: u32,
    /// `BspHitboxFlag` in the low byte, `BspHitboxClass` in the high one.
    pub flag: u16,
    pub id: i16,
    pub bbox: BoundingBox,
    pub center: SVec3,
    pub radius: i16,
    /// The `InstDef` it belongs to: an index into [`Level::instances`].
    pub instance: Option<usize>,
}

impl BspHitbox {
    /// `BSP_HITBOX_COLLIDABLE`
    pub const COLLIDABLE: u16 = 0x80;

    /// `Coll_BspHitboxClass`
    pub fn class(&self) -> u8 {
        (self.flag >> 8) as u8
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BspKind {
    Branch {
        /// Split plane (x, y, z, w) as stored; the exact meaning is still to be worked out.
        axis: [i16; 4],
        children: [BspChild; 4],
    },
    Leaf {
        /// Range into [`Level::quad_blocks`].
        quad_blocks: std::ops::Range<usize>,
        /// The instance hitboxes of the leaf (`bspHitboxArray`, up to its zero flag).
        hitboxes: Vec<BspHitbox>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BspNode {
    pub flag: u16,
    pub id: i16,
    pub bbox: BoundingBox,
    pub kind: BspKind,
}

impl BspNode {
    /// `BSP_LEAF_FLAG_WATER`
    pub fn is_water(&self) -> bool {
        self.flag & 0x2 != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpawnPoint {
    pub pos: SVec3,
    pub rot: SVec3,
}

/// A respawn / progress node (`struct CheckpointNode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckpointNode {
    pub pos: SVec3,
    pub dist_to_finish: u16,
    pub next_forward: u8,
    pub next_left: u8,
    pub next_backward: u8,
    pub next_right: u8,
}

/// A skybox vertex (`struct ShortVertex`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkyVertex {
    pub pos: SVec3,
    pub color: [u8; 3],
}

/// The sky dome (`struct Skybox`): gouraud triangles around the camera, split into
/// 8 segments by heading. The game draws the 4 segments nearest the camera's yaw;
/// the rest are out of view anyway.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skybox {
    pub vertices: Vec<SkyVertex>,
    /// Per segment, triangles as indices into [`Skybox::vertices`].
    pub segments: [Vec<[u16; 3]>; 8],
}

/// A point of a recorded path (`struct SpawnPosRot`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PosRot {
    pub pos: SVec3,
    /// Angles, 0x1000 = a full turn.
    pub rot: SVec3,
}

/// A placed model (`struct InstDef`): a mushroom, a crate, the start banner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstDef {
    pub name: String,
    /// Payload offset of the `struct Model` (0 = none).
    pub model: u32,
    /// The name in that `struct Model`: what picks the object's behaviour (`"armadillo"`).
    pub model_name: String,
    /// 0x1000 = 1.0
    pub scale: SVec3,
    pub color_rgba: u32,
    pub flags: u32,
    pub pos: SVec3,
    /// Angles, 0x1000 = a full turn.
    pub rot: SVec3,
    pub model_id: i32,
}

#[derive(Debug, Clone)]
pub struct Level {
    pub instances: Vec<InstDef>,
    /// How long each armadillo waits before it first moves (`ptrSpawnType1`'s spawn array), in
    /// ticks, indexed by the digit its instance name ends in.
    pub spawn_delays: Vec<u16>,
    /// The position-only paths (`ptrSpawnType2`): the minecarts of Dragon Mines follow the first,
    /// and several other objects (seals, spiders, turtles) use the others.
    pub position_paths: Vec<Vec<SVec3>>,
    /// The recorded paths of the rolling objects (`ptrSpawnType2_PosRot`): the snowball and the
    /// barrels follow one each, picked by the digit their instance name ends in.
    pub paths: Vec<Vec<PosRot>>,
    pub skybox: Option<Skybox>,
    pub quad_blocks: Vec<QuadBlock>,
    pub vertices: Vec<LevVertex>,
    pub bsp: Vec<BspNode>,
    pub driver_spawns: [SpawnPoint; 8],
    pub restart_points: Vec<CheckpointNode>,
    pub water: Water,
    pub scenery: Scenery,
    pub config_flags: u32,
    pub jump_vertical_speed_cap: u8,
}

/// Bounds-checked little-endian reads from the payload.
struct Payload<'a>(&'a [u8]);

fn invalid(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

impl<'a> Payload<'a> {
    fn bytes(&self, at: usize, len: usize) -> io::Result<&'a [u8]> {
        at.checked_add(len)
            .and_then(|end| self.0.get(at..end))
            .ok_or_else(|| {
                invalid(format!(
                    "LEV read of {len} bytes at {at:#x} is out of range"
                ))
            })
    }
    fn u8(&self, at: usize) -> io::Result<u8> {
        Ok(self.bytes(at, 1)?[0])
    }
    fn u16(&self, at: usize) -> io::Result<u16> {
        Ok(u16::from_le_bytes(self.bytes(at, 2)?.try_into().unwrap()))
    }
    fn i16(&self, at: usize) -> io::Result<i16> {
        Ok(self.u16(at)? as i16)
    }
    fn u32(&self, at: usize) -> io::Result<u32> {
        Ok(u32::from_le_bytes(self.bytes(at, 4)?.try_into().unwrap()))
    }
    fn svec3(&self, at: usize) -> io::Result<SVec3> {
        Ok(SVec3 {
            x: self.i16(at)?,
            y: self.i16(at + 2)?,
            z: self.i16(at + 4)?,
        })
    }
    fn bbox(&self, at: usize) -> io::Result<BoundingBox> {
        Ok(BoundingBox {
            min: self.svec3(at)?,
            max: self.svec3(at + 6)?,
        })
    }
    /// A payload pointer that must be non-null.
    fn ptr(&self, at: usize, what: &str) -> io::Result<usize> {
        match self.u32(at)? {
            0 => Err(invalid(format!("LEV {what} pointer is null"))),
            p => Ok(p as usize),
        }
    }
    fn array<T>(
        &self,
        base: usize,
        count: usize,
        stride: usize,
        parse: impl Fn(&Self, usize) -> io::Result<T>,
    ) -> io::Result<Vec<T>> {
        // Fail on the whole range up front so a huge bogus count can't allocate.
        self.bytes(
            base,
            count
                .checked_mul(stride)
                .ok_or_else(|| invalid("LEV array too large"))?,
        )?;
        (0..count).map(|i| parse(self, base + i * stride)).collect()
    }
}

// `struct Level` field offsets
const LEVEL_MESH_INFO: usize = 0x0;
const LEVEL_SKYBOX: usize = 0x4;
const LEVEL_NUM_INSTANCES: usize = 0xc;
const LEVEL_INSTANCES: usize = 0x10;
const INST_DEF_SIZE: usize = 0x40;
const LEVEL_SPAWN_TYPE_1: usize = 0x134;
const LEVEL_NUM_POSITION_PATHS: usize = 0x138;
const LEVEL_POSITION_PATHS: usize = 0x13c;
const LEVEL_NUM_PATHS: usize = 0x140;
const LEVEL_PATHS: usize = 0x144;
const PATH_ENTRY_SIZE: usize = 8;
const POS_ROT_SIZE: usize = 0xc;
const LEVEL_DRIVER_SPAWN: usize = 0x6c;
const LEVEL_CONFIG_FLAGS: usize = 0xdc;
const LEVEL_RESTART_COUNT: usize = 0x148;
const LEVEL_RESTART_PTR: usize = 0x14c;
const LEVEL_JUMP_SPEED_CAP: usize = 0x18c;

const QUAD_BLOCK_SIZE: usize = 0x5c;
const VERTEX_SIZE: usize = 0x10;
const BSP_SIZE: usize = 0x20;
const CHECKPOINT_SIZE: usize = 0xc;

const SKY_VERTEX_SIZE: usize = 12;
const SKY_FACE_SIZE: usize = 8;

impl Level {
    fn parse_skybox(p: &Payload) -> io::Result<Option<Skybox>> {
        let Some(sky) = (match p.u32(LEVEL_SKYBOX)? {
            0 => None,
            at => Some(at as usize),
        }) else {
            return Ok(None);
        };
        let num_vertex = p.u32(sky)? as usize;
        let verts_at = p.ptr(sky + 4, "skybox vertex array")?;
        let vertices = p.array(verts_at, num_vertex, SKY_VERTEX_SIZE, |p, at| {
            let c = p.bytes(at + 8, 3)?;
            Ok(SkyVertex {
                pos: p.svec3(at)?,
                color: [c[0], c[1], c[2]],
            })
        })?;
        let mut segments: [Vec<[u16; 3]>; 8] = Default::default();
        for (i, segment) in segments.iter_mut().enumerate() {
            let count = p.i16(sky + 8 + i * 2)?.max(0) as usize;
            if count == 0 {
                continue;
            }
            let faces_at = p.ptr(sky + 0x18 + i * 4, "skybox faces")?;
            *segment = p.array(faces_at, count, SKY_FACE_SIZE, |p, at| {
                let mut tri = [0u16; 3];
                for (k, v) in tri.iter_mut().enumerate() {
                    let offset = p.u16(at + k * 2)? as usize;
                    let index = offset / SKY_VERTEX_SIZE;
                    if offset % SKY_VERTEX_SIZE != 0 || index >= num_vertex {
                        return Err(invalid(format!("skybox face vertex offset {offset:#x}")));
                    }
                    *v = index as u16;
                }
                Ok(tri)
            })?;
        }
        Ok(Some(Skybox { vertices, segments }))
    }

    /// The delays of `ST1_SPAWN`: ten halfwords at most, one for each digit a name can end in.
    fn parse_spawn_delays(p: &Payload) -> io::Result<Vec<u16>> {
        const ST1_SPAWN: usize = 1;
        const DIGITS: usize = 10;
        let Some(at) = (match p.u32(LEVEL_SPAWN_TYPE_1)? {
            0 => None,
            at => Some(at as usize),
        }) else {
            return Ok(Vec::new());
        };
        if p.u32(at)? as usize <= ST1_SPAWN {
            return Ok(Vec::new());
        }
        // Most tracks have no such table.
        let delays = match p.u32(at + 4 + 4 * ST1_SPAWN)? {
            0 => return Ok(Vec::new()),
            delays => delays as usize,
        };
        (0..DIGITS).map_while(|i| p.u16(delays + i * 2).ok()).map(Ok).collect()
    }

    fn parse_position_paths(p: &Payload) -> io::Result<Vec<Vec<SVec3>>> {
        let count = p.u32(LEVEL_NUM_POSITION_PATHS)? as usize;
        if count == 0 {
            return Ok(Vec::new());
        }
        let at = p.ptr(LEVEL_POSITION_PATHS, "position paths")?;
        p.array(at, count, PATH_ENTRY_SIZE, |p, at| {
            let points = p.u32(at)? as usize;
            if points == 0 {
                return Ok(Vec::new());
            }
            let first = p.ptr(at + 4, "path positions")?;
            p.array(first, points, 6, |p, at| p.svec3(at))
        })
    }

    fn parse_paths(p: &Payload) -> io::Result<Vec<Vec<PosRot>>> {
        let count = p.u32(LEVEL_NUM_PATHS)? as usize;
        if count == 0 {
            return Ok(Vec::new());
        }
        let at = p.ptr(LEVEL_PATHS, "recorded paths")?;
        p.array(at, count, PATH_ENTRY_SIZE, |p, at| {
            let points = p.u32(at)? as usize;
            if points == 0 {
                return Ok(Vec::new());
            }
            let first = p.ptr(at + 4, "path points")?;
            p.array(first, points, POS_ROT_SIZE, |p, at| Ok(PosRot { pos: p.svec3(at)?, rot: p.svec3(at + 6)? }))
        })
    }

    fn parse_instances(p: &Payload) -> io::Result<Vec<InstDef>> {
        let count = p.u32(LEVEL_NUM_INSTANCES)? as usize;
        if count == 0 {
            return Ok(Vec::new());
        }
        let at = p.ptr(LEVEL_INSTANCES, "instance definitions")?;
        p.array(at, count, INST_DEF_SIZE, |p, at| {
            let text = |bytes: &[u8]| {
                let len = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
                String::from_utf8_lossy(&bytes[..len]).into_owned()
            };
            let model = p.u32(at + 0x10)?;
            Ok(InstDef {
                name: text(p.bytes(at, 0x10)?),
                model,
                model_name: if model == 0 { String::new() } else { text(p.bytes(model as usize, 0x10)?) },
                scale: p.svec3(at + 0x14)?,
                color_rgba: p.u32(at + 0x1c)?,
                flags: p.u32(at + 0x20)?,
                pos: p.svec3(at + 0x30)?,
                rot: p.svec3(at + 0x36)?,
                model_id: p.u32(at + 0x3c)? as i32,
            })
        })
    }

    /// The hitboxes at `at` (0 = none), up to the entry with a zero flag.
    fn parse_hitboxes(p: &Payload, at: usize, instances_at: usize, num_instances: usize) -> io::Result<Vec<BspHitbox>> {
        let mut hitboxes = Vec::new();
        if at == 0 {
            return Ok(hitboxes);
        }
        while p.u16(at + hitboxes.len() * BSP_SIZE)? != 0 {
            let at = at + hitboxes.len() * BSP_SIZE;
            let inst_def = p.u32(at + 0x1c)? as usize;
            let instance = inst_def
                .checked_sub(instances_at)
                .filter(|off| inst_def != 0 && off % INST_DEF_SIZE == 0)
                .map(|off| off / INST_DEF_SIZE)
                .filter(|&i| i < num_instances);
            hitboxes.push(BspHitbox {
                offset: at as u32,
                flag: p.u16(at)?,
                id: p.i16(at + 2)?,
                bbox: p.bbox(at + 4)?,
                center: p.svec3(at + 0x10)?,
                radius: p.i16(at + 0x16)?,
                instance,
            });
        }
        Ok(hitboxes)
    }

    pub fn parse(file: &DramFile) -> io::Result<Self> {
        let p = Payload(&file.data);

        let mesh = p.ptr(LEVEL_MESH_INFO, "mesh_info")?;
        let num_quads = p.u32(mesh)? as usize;
        let num_verts = p.u32(mesh + 0x4)? as usize;
        let quads_at = p.ptr(mesh + 0xc, "quad block array")?;
        let verts_at = p.ptr(mesh + 0x10, "vertex array")?;
        let bsp_at = p.ptr(mesh + 0x18, "BSP root")?;
        let num_bsp = p.u32(mesh + 0x1c)? as usize;

        let vertices = p.array(verts_at, num_verts, VERTEX_SIZE, |p, at| {
            Ok(LevVertex {
                pos: p.svec3(at)?,
                flags: p.u16(at + 6)?,
                color_hi: p.bytes(at + 8, 4)?.try_into().unwrap(),
                color_lo: p.bytes(at + 0xc, 4)?.try_into().unwrap(),
            })
        })?;

        let quad_blocks = p.array(quads_at, num_quads, QUAD_BLOCK_SIZE, |p, at| {
            let mut vertex_index = [0u16; 9];
            for (i, v) in vertex_index.iter_mut().enumerate() {
                *v = p.u16(at + i * 2)?;
            }
            let mut texture_mid = [0u32; 4];
            for (i, t) in texture_mid.iter_mut().enumerate() {
                *t = p.u32(at + 0x1c + i * 4)?;
            }
            let mut tri_normal_vec_dividend = [0i16; 10];
            for (i, d) in tri_normal_vec_dividend.iter_mut().enumerate() {
                *d = p.i16(at + 0x48 + i * 2)?;
            }
            Ok(QuadBlock {
                vertex_index,
                flags: p.u16(at + 0x12)?,
                draw_order_low: p.u32(at + 0x14)?,
                draw_order_high: p.u32(at + 0x18)?,
                texture_mid,
                bbox: p.bbox(at + 0x2c)?,
                terrain_type: p.u8(at + 0x38)?,
                weather_intensity: p.u8(at + 0x39)?,
                weather_vanish_rate: p.u8(at + 0x3a)?,
                mul_norm_vec_y: p.u8(at + 0x3b)? as i8,
                block_id: p.i16(at + 0x3c)?,
                checkpoint_index: p.u8(at + 0x3e)?,
                tri_normal_vec_bit_shift: p.u8(at + 0x3f)? as i8,
                texture_low: p.u32(at + 0x40)?,
                pvs: p.u32(at + 0x44)?,
                tri_normal_vec_dividend,
            })
        })?;

        let instances_at = p.u32(LEVEL_INSTANCES)? as usize;
        let num_instances = p.u32(LEVEL_NUM_INSTANCES)? as usize;
        let bsp = p.array(bsp_at, num_bsp, BSP_SIZE, |p, at| {
            let flag = p.u16(at)?;
            let kind = if flag & 1 == 0 {
                let mut axis = [0i16; 4];
                let mut children = [BspChild::None; 4];
                for i in 0..4 {
                    axis[i] = p.i16(at + 0x10 + i * 2)?;
                    children[i] = BspChild::decode(p.u16(at + 0x18 + i * 2)?);
                }
                BspKind::Branch { axis, children }
            } else {
                let num = p.u32(at + 0x18)? as usize;
                let first = match p.u32(at + 0x1c)? as usize {
                    0 => 0,
                    off if off >= quads_at && (off - quads_at) % QUAD_BLOCK_SIZE == 0 => {
                        (off - quads_at) / QUAD_BLOCK_SIZE
                    }
                    off => {
                        return Err(invalid(format!(
                            "BSP leaf quad pointer {off:#x} is not a quad block"
                        )));
                    }
                };
                BspKind::Leaf {
                    quad_blocks: first..first + num,
                    hitboxes: Self::parse_hitboxes(&p, p.u32(at + 0x14)? as usize, instances_at, num_instances)?,
                }
            };
            Ok(BspNode {
                flag,
                id: p.i16(at + 2)?,
                bbox: p.bbox(at + 4)?,
                kind,
            })
        })?;

        let mut driver_spawns = [SpawnPoint {
            pos: SVec3::default(),
            rot: SVec3::default(),
        }; 8];
        for (i, s) in driver_spawns.iter_mut().enumerate() {
            let at = LEVEL_DRIVER_SPAWN + i * 12;
            *s = SpawnPoint {
                pos: p.svec3(at)?,
                rot: p.svec3(at + 6)?,
            };
        }

        let restart_count = p.u32(LEVEL_RESTART_COUNT)? as usize;
        let restart_points = if restart_count == 0 {
            Vec::new()
        } else {
            let at = p.ptr(LEVEL_RESTART_PTR, "restart points")?;
            p.array(at, restart_count, CHECKPOINT_SIZE, |p, at| {
                Ok(CheckpointNode {
                    pos: p.svec3(at)?,
                    dist_to_finish: p.u16(at + 6)?,
                    next_forward: p.u8(at + 8)?,
                    next_left: p.u8(at + 9)?,
                    next_backward: p.u8(at + 10)?,
                    next_right: p.u8(at + 11)?,
                })
            })?
        };

        Ok(Self {
            instances: Self::parse_instances(&p)?,
            spawn_delays: Self::parse_spawn_delays(&p)?,
            position_paths: Self::parse_position_paths(&p)?,
            paths: Self::parse_paths(&p)?,
            skybox: Self::parse_skybox(&p)?,
            quad_blocks,
            vertices,
            bsp,
            driver_spawns,
            restart_points,
            water: Water::parse(&file.data, verts_at, num_verts)?,
            scenery: Scenery::parse(&file.data, verts_at, num_verts)?,
            config_flags: p.u32(LEVEL_CONFIG_FLAGS)?,
            jump_vertical_speed_cap: p.u8(LEVEL_JUMP_SPEED_CAP)?,
        })
    }
}
