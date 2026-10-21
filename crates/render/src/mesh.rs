//! Turns a track's quad blocks into plain vertex/index arrays (no Bevy types,
//! so it can be tested without a window).

use std::collections::{HashMap, HashSet};

use loader::dram::DramFile;
use loader::lev::{BspKind, Level, QuadBlock, SVec3, quad_flags};
use loader::model::{Frame, ModelHeader};
use loader::texture::{
    AnimTex, Blend, FaceMode, TextureLayout, face_back_to_front, face_polygon, face_uv_order,
    resolve_animation, resolve_low_texture, resolve_texture,
};
use loader::vram::Vram;

/// `BSP_LEAF_FLAG_WATER`
const BSP_LEAF_FLAG_WATER: u16 = 0x2;

/// World units per game unit.
pub const SCALE: f32 = 0.01;

/// A vertex colour channel (1.0 = the texture as is) for the mesh. The console multiplies the
/// texture by it on the gamma-encoded values, while the renderer multiplies in linear light, so
/// the channel goes through the display gamma to give the same result.
pub fn vertex_channel(c: f32) -> f32 {
    c.powf(2.2)
}

/// Game space is Y up like Bevy's. Flipping any single axis would mirror the
/// tracks (left turns become right turns), so this only scales.
pub fn to_world(p: SVec3) -> [f32; 3] {
    [p.x as f32 * SCALE, p.y as f32 * SCALE, p.z as f32 * SCALE]
}

/// `QUADBLOCK_DRAW_ORDER_LOW_DOUBLE_SIDED`
const QUADBLOCK_DRAW_ORDER_LOW_DOUBLE_SIDED: u32 = 0x8000_0000;

fn is_marker_red(c: [u8; 4]) -> bool {
    c[..3] == [255, 0, 0]
}

/// A block that is only there to be hit or to flag the track, which the game does not show: every
/// `SKIP_WATER_LIST` block (walls, trigger and kill planes, and the `GROUND | CAMERA_SEARCH` planes
/// the camera follows, which lie flush on the visible floor). On all tracks they share one flat
/// marker texture, and the level paints them red, black or magenta; they are hidden whole,
/// whatever their colours are.
pub fn is_collision_wall(quad: &QuadBlock) -> bool {
    quad.flags & quad_flags::SKIP_WATER_LIST != 0
}

/// The quad blocks that are turbo pads: a trigger block whose `terrain_type` holds the
/// `COLL_STEP_TRIGGER_TURBO_PAD` (1) or `..._SUPER_TURBO_PAD` (2) bit that the collision search
/// gives the kart. It is only the collision plane of the pad, which the game does not draw: see
/// [`turbo_pad_visuals`].
pub fn is_turbo_pad(quad: &QuadBlock) -> bool {
    quad.flags & quad_flags::TRIGGER != 0 && quad.terrain_type & 0x3 != 0
}

/// The blocks that show the turbo pads: for each pad's collision plane, the drawn blocks with the
/// same footprint (the same bounds in x and z) that lie within [`PAD_VISUAL_MAX_DROP`] below it.
/// Not every pad has one that way (some are textured on part of a larger road block).
pub fn turbo_pad_visuals(level: &Level) -> HashSet<usize> {
    let footprint = |q: &QuadBlock| [q.bbox.min.x, q.bbox.max.x, q.bbox.min.z, q.bbox.max.z];
    let pads: Vec<&QuadBlock> = level
        .quad_blocks
        .iter()
        .filter(|q| is_turbo_pad(q))
        .collect();
    level
        .quad_blocks
        .iter()
        .enumerate()
        .filter(|(_, q)| !is_collision_wall(q))
        .filter(|(_, q)| {
            pads.iter().any(|pad| {
                footprint(q) == footprint(pad)
                    && (pad.bbox.min.y as i32 - q.bbox.min.y as i32).abs() <= PAD_VISUAL_MAX_DROP
            })
        })
        .map(|(i, _)| i)
        .collect()
}

/// How far (in level units) the lowest point of a pad's visible block is from the lowest point of
/// its collision plane: the plane floats a little above what is drawn (16 or 32 in the tracks).
const PAD_VISUAL_MAX_DROP: i32 = 64;

/// How many steps of draw order bias the faces of a turbo pad get on top of their own (negative is
/// nearer, see [`LevelMesh::biases`]). A pad lies flush on the road, in places a little under it,
/// and the bytes of its block alone (a few steps ahead of the road's) do not win the depth test
/// here, where there is no ordering table. A step is a few `wgpu` constant depth bias units (see
/// the viewer's `DEPTH_BIAS_PER_ORDER`). With the camera's float depth that is roughly `1e-6` of
/// the distance to the camera a step, so 4000 of them pull a pad forward by about 0.4% of that
/// distance: some 8 game units at the distance of the kart. (The exact scale depends on the GPU.)
/// Only the picture changes. Tune by eye.
pub const TURBO_PAD_EXTRA_BIAS: i32 = -4000;

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

#[derive(Default, Clone)]
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

/// One mesh vertex of a water face, patched every game tick from the water
/// vertex it was made from.
pub struct WaterCorner {
    pub part: usize,
    /// Index into the part's vertex arrays (in [`LevelMesh::water`]).
    pub slot: usize,
    /// Top-left of the environment map tile in atlas pixels; the vertex's (u, v) is added.
    pub origin: [f32; 2],
}

/// One mesh vertex of a face on a scenery vertex (a swaying plant), patched every game tick.
pub struct SceneryCorner {
    pub part: usize,
    /// Index into the part's vertex arrays (in [`LevelMesh::parts`]).
    pub slot: usize,
    /// What the vertex colour byte becomes in the mesh: the game doubles it on textured faces.
    pub gain: f32,
}

pub struct LevelMesh {
    /// The solid faces in groups of [`PARTS`]: slot `layer * PARTS + part` holds the faces of
    /// [`LevelMesh::biases`]`[layer]` that reach the screen as `part`.
    pub parts: Vec<MeshPart>,
    /// The draw order bias of each layer of [`LevelMesh::parts`]: the game files a face in its
    /// ordering table `bias` slots farther back (negative: nearer), so that a face lying flush on
    /// another one (a boost pad, a decal) is drawn after it. Layer 0 is bias 0.
    pub biases: Vec<i32>,
    /// Scenery corners by level vertex index.
    pub scenery_corners: HashMap<usize, Vec<SceneryCorner>>,
    /// The water faces, kept apart so that only they are rewritten each tick.
    pub water: [MeshPart; PARTS],
    /// Water corners by level vertex index.
    pub water_corners: HashMap<usize, Vec<WaterCorner>>,
    pub atlas_size: [f32; 2],
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
/// `cull` winds the triangles as the game's `NCLIP` test draws them (front faces counter-clockwise,
/// as Bevy has it) and adds the other winding to the blocks that are double sided, so that the
/// back faces can be culled.
///
/// `hide_unseen` drops blocks without any texture (the game has nothing to draw
/// for them) and, as a viewer heuristic rather than game logic, faces whose
/// corners are all pure (255, 0, 0) vertex colour: the visibility data and quad
/// flags don't hide them, but they look like editor markers.
pub fn level_mesh(
    level: &Level,
    dram: &DramFile,
    vram: &Vram,
    low_colors: bool,
    hide_unseen: bool,
    cull: bool,
) -> LevelMesh {
    let mut atlas = AtlasBuilder {
        vram,
        tiles: Vec::new(),
        by_key: HashMap::new(),
    };
    atlas.tiles.push(vec![255; TILE * TILE * 4]);

    let mut parts: Vec<MeshPart> = (0..PARTS).map(|_| MeshPart::default()).collect();
    let mut biases = vec![0];
    let mut water: [MeshPart; PARTS] = Default::default();
    let mut water_corners: HashMap<usize, Vec<WaterCorner>> = HashMap::new();
    // Blocks of BSP leaves flagged as water are drawn by the water list.
    let mut water_blocks = HashSet::new();
    if level.water.env_map.is_some() {
        for node in level
            .bsp
            .iter()
            .filter(|n| n.flag & BSP_LEAF_FLAG_WATER != 0)
        {
            if let BspKind::Leaf { quad_blocks, .. } = &node.kind {
                water_blocks.extend(quad_blocks.clone());
            }
        }
    }
    let pad_visuals = turbo_pad_visuals(level);
    let scenery: HashSet<usize> = level.scenery.vertices.iter().map(|v| v.vertex).collect();
    let mut scenery_corners: HashMap<usize, Vec<SceneryCorner>> = HashMap::new();
    let mut animations: Vec<Animation> = Vec::new();
    let mut animation_of: HashMap<(u32, usize), usize> = HashMap::new();
    let mut animated_faces = Vec::new();

    for (block_index, quad) in level.quad_blocks.iter().enumerate() {
        let is_water = water_blocks.contains(&block_index);
        if hide_unseen && !is_water && is_collision_wall(quad) {
            continue;
        }
        // Triangle-shaped blocks repeat the third corner in slot 3.
        let is_quad = quad.vertex_index[2] != quad.vertex_index[3];

        // The faces to draw: the four mid-detail faces, or when the block has none,
        // a single quad over its corners with the low-detail texture.
        let no_mid = quad.texture_mid.iter().all(|&t| t == 0);
        let low = if no_mid {
            resolve_low_texture(dram, quad.texture_low).ok().flatten()
        } else {
            None
        };
        // The ordering table slot offset of a face: a byte of the block each (`DrawLevelOvr1P_GetProjectedOtIndex`).
        let [low_order, ..] = quad.draw_order_low.to_le_bytes().map(|b| b as i8);
        let face_order = quad.draw_order_high.to_le_bytes().map(|b| b as i8);
        let mut faces: Vec<(u32, Option<TextureLayout>, [usize; 4], [usize; 4], bool, i8)> =
            Vec::new();
        if is_water {
            // The water list draws the block's faces with the environment map's
            // palette and page; the UVs come from the animated vertices.
            let plain = FaceMode {
                rotation: 0,
                mode: 0,
            };
            for face in 0..if is_quad { 4 } else { 3 } {
                let slots = face_polygon(face, plain).unwrap();
                faces.push((
                    0,
                    level.water.env_map,
                    slots,
                    [0, 1, 2, 3],
                    face_back_to_front(plain).unwrap(),
                    0,
                ));
            }
        } else if let Some(low) = low {
            faces.push((
                0,
                Some(low),
                [0, 1, 2, 3],
                [0, 1, 2, 3],
                face_back_to_front(FaceMode {
                    rotation: 0,
                    mode: 0,
                })
                .unwrap(),
                low_order,
            ));
        } else if no_mid && hide_unseen {
            // No texture at all: the game has nothing to draw for this block.
            continue;
        } else {
            // Face 3 lies entirely in the second half of the patch.
            for face in 0..if is_quad { 4 } else { 3 } {
                let ptr = quad.texture_mid[face];
                // The sharpest layout of the group: nothing here depends on the camera.
                let texture = resolve_texture(dram, ptr).ok().flatten().map(|g| *g.best());
                let mode = match texture {
                    Some(_) => FaceMode::of(quad, face),
                    None => FaceMode {
                        rotation: 0,
                        mode: 0,
                    },
                };
                if let (Ok(slots), Ok(uv_order)) = (face_polygon(face, mode), face_uv_order(mode)) {
                    faces.push((
                        ptr,
                        texture,
                        slots,
                        uv_order,
                        face_back_to_front(mode).unwrap_or(false),
                        face_order[face],
                    ));
                }
            }
        }

        for (ptr, texture, slots, uv_order, back_to_front, order) in faces {
            if hide_unseen
                && !is_water
                && slots
                    .iter()
                    .any(|&s| is_marker_red(level.vertices[quad.vertex_index[s] as usize].color_hi))
            {
                continue;
            }
            let anim = resolve_animation(dram, ptr).ok().flatten();
            let untextured = [(Part::Opaque, TileKind::Full)];
            let face_passes = match &texture {
                Some(t) => passes(t.blend()),
                None => &untextured,
            };

            let order = order as i32
                + if pad_visuals.contains(&block_index) {
                    TURBO_PAD_EXTRA_BIAS
                } else {
                    0
                };
            let layer = biases.iter().position(|&b| b == order).unwrap_or_else(|| {
                biases.push(order);
                parts.extend((0..PARTS).map(|_| MeshPart::default()));
                biases.len() - 1
            });
            for &(part, kind) in face_passes {
                let tile = texture.as_ref().map(|t| atlas.tile(t, kind));
                // Water is drawn by its own list, which has no bias to apply.
                let flat = if is_water {
                    part as usize
                } else {
                    layer * PARTS + part as usize
                };
                let out = &mut if is_water {
                    &mut water[flat]
                } else {
                    &mut parts[flat]
                };
                let base = out.positions.len();
                for (k, &slot) in slots.iter().enumerate() {
                    let v = &level.vertices[quad.vertex_index[slot] as usize];
                    let [x, y, z] = to_world(v.pos);
                    out.positions.push([x, y, z]);
                    let c = if low_colors { v.color_lo } else { v.color_hi };
                    let gain = if texture.is_some() {
                        2.0 / 255.0
                    } else {
                        1.0 / 255.0
                    };
                    // Placeholders for water until its first tick.
                    let [r, g, b] = if is_water {
                        [0.3; 3]
                    } else {
                        [c[0], c[1], c[2]].map(|c| vertex_channel(c as f32 * gain))
                    };
                    out.colors.push([r, g, b, 1.0]);
                    if !is_water && scenery.contains(&(quad.vertex_index[slot] as usize)) {
                        scenery_corners
                            .entry(quad.vertex_index[slot] as usize)
                            .or_default()
                            .push(SceneryCorner {
                                part: flat,
                                slot: out.positions.len() - 1,
                                gain,
                            });
                    }
                    if let (true, Some(tile)) = (is_water, tile) {
                        water_corners
                            .entry(quad.vertex_index[slot] as usize)
                            .or_default()
                            .push(WaterCorner {
                                part: part as usize,
                                slot: out.positions.len() - 1,
                                origin: atlas.uv(tile, 0, 0),
                            });
                    }
                    out.uvs.push(match (&texture, tile) {
                        (Some(t), Some(tile)) => {
                            atlas.uv(tile, t.uv[uv_order[k]].0, t.uv[uv_order[k]].1)
                        }
                        _ => atlas.uv(0, 0, 0),
                    });
                }
                // The game draws (0, 1, 2) and (3, 1, 2); a repeated vertex degenerates one of them.
                // (3, 1, 2) goes in as (1, 3, 2), the same triangle wound like the first.
                let ids: [u16; 4] = slots.map(|s| quad.vertex_index[s]);
                // The game draws a face that is wound clockwise on its screen (y down), or the other way
                // round for the modes in `face_back_to_front`; Bevy's front is counter-clockwise.
                let wound: [[usize; 3]; 2] = if back_to_front {
                    [[0, 1, 2], [1, 3, 2]]
                } else {
                    [[0, 2, 1], [1, 2, 3]]
                };
                let double_sided = quad.draw_order_low & QUADBLOCK_DRAW_ORDER_LOW_DOUBLE_SIDED != 0;
                for (tri, original) in wound.into_iter().zip([[0, 1, 2], [3, 1, 2]]) {
                    let [a, b, c] = original;
                    let degenerate = ids[a] == ids[b] || ids[b] == ids[c] || ids[a] == ids[c];
                    if !degenerate {
                        out.indices.extend(tri.map(|k| (base + k) as u32));
                        if cull && double_sided {
                            out.indices
                                .extend([tri[0], tri[2], tri[1]].map(|k| (base + k) as u32));
                        }
                    }
                }

                if let Some(anim) = &anim {
                    let index = *animation_of.entry((ptr, flat)).or_insert_with(|| {
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
                        part: flat,
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
        .chain(water.iter_mut())
        .flat_map(|p| p.uvs.iter_mut())
        .for_each(norm);
    animations
        .iter_mut()
        .flat_map(|a| a.uvs.iter_mut().flatten())
        .for_each(norm);
    LevelMesh {
        parts,
        biases,
        water,
        water_corners,
        scenery_corners,
        atlas_size: [atlas.width as f32, atlas.height as f32],
        atlas,
        animations,
        animated_faces,
    }
}

/// A textured model in one pose.
pub struct ModelMesh {
    pub parts: [MeshPart; PARTS],
    pub atlas: Atlas,
}

/// Triangles of `header` in `frame`, with the model's vertex colours and
/// textures from `vram`. Model units become game units through the header's
/// scale and the instance's (`inst_scale`, 0x1000 = 1.0), as
/// `RenderBucket_BuildMatrices` does; the result is in world units.
///
/// Corners take the layout's first three UVs in order (`DrawInstPrim_Normal`).
pub fn model_mesh(
    header: &ModelHeader,
    frame: &Frame,
    vram: &Vram,
    inst_scale: [i16; 3],
) -> ModelMesh {
    let mut atlas = AtlasBuilder {
        vram,
        tiles: vec![vec![255; TILE * TILE * 4]],
        by_key: HashMap::new(),
    };
    let k: [f32; 3] = std::array::from_fn(|a| {
        header.scale[a] as f32 / 4096.0 * inst_scale[a] as f32 / 4096.0 * SCALE
    });

    let mut parts: [MeshPart; PARTS] = Default::default();
    for tri in header.triangles(frame) {
        let texture = tri
            .texture
            .checked_sub(1)
            .and_then(|i| header.textures.get(i))
            .copied();
        let untextured = [(Part::Opaque, TileKind::Full)];
        let face_passes = match &texture {
            Some(t) => passes(t.blend()),
            None => &untextured,
        };
        // Textured primitives get their colour doubled, as for the level.
        let gain = if texture.is_some() {
            2.0 / 255.0
        } else {
            1.0 / 255.0
        };
        for &(part, kind) in face_passes {
            let tile = texture.as_ref().map(|t| atlas.tile(t, kind));
            let out = &mut parts[part as usize];
            let base = out.positions.len() as u32;
            for corner in 0..3 {
                let p = tri.positions[corner];
                out.positions
                    .push([p[0] as f32 * k[0], p[1] as f32 * k[1], p[2] as f32 * k[2]]);
                let c = tri.colors[corner];
                let [r, g, b] = [c & 0xff, c >> 8 & 0xff, c >> 16 & 0xff]
                    .map(|c| vertex_channel(c as f32 * gain));
                out.colors.push([r, g, b, 1.0]);
                out.uvs.push(match (&texture, tile) {
                    (Some(t), Some(tile)) => atlas.uv(tile, t.uv[corner].0, t.uv[corner].1),
                    _ => atlas.uv(0, 0, 0),
                });
            }
            out.indices.extend([base, base + 1, base + 2]);
        }
    }

    let atlas = atlas.finish();
    for uv in parts.iter_mut().flat_map(|p| p.uvs.iter_mut()) {
        *uv = [uv[0] / atlas.width as f32, uv[1] / atlas.height as f32];
    }
    ModelMesh { parts, atlas }
}

/// The positions of the parts [`model_mesh`] makes of `header` in `frame`, for a model whose
/// pose changes but not its faces: the triangles come in the same order, so these replace
/// the parts' positions without building the atlas again.
pub fn model_positions(
    header: &ModelHeader,
    frame: &Frame,
    inst_scale: [i16; 3],
) -> [Vec<[f32; 3]>; PARTS] {
    let k: [f32; 3] = std::array::from_fn(|a| {
        header.scale[a] as f32 / 4096.0 * inst_scale[a] as f32 / 4096.0 * SCALE
    });
    let mut parts: [Vec<[f32; 3]>; PARTS] = Default::default();
    for tri in header.triangles(frame) {
        let texture = tri
            .texture
            .checked_sub(1)
            .and_then(|i| header.textures.get(i))
            .copied();
        let face_passes = match &texture {
            Some(t) => passes(t.blend()),
            None => &[(Part::Opaque, TileKind::Full)],
        };
        for &(part, _) in face_passes {
            parts[part as usize].extend(
                tri.positions
                    .map(|p| [p[0] as f32 * k[0], p[1] as f32 * k[1], p[2] as f32 * k[2]]),
            );
        }
    }
    parts
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
    fn racer_models_mesh_with_valid_atlas() {
        use loader::model::Model;

        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/ctr-u.bin");
        let Ok((mut disc, big)) = open_bigfile(&path) else {
            return;
        };
        let mut vram = Vram::default();
        let shared = big.read(&mut disc, BigIndex::SHARED_MPK_VRM.get()).unwrap();
        for img in parse_vram_file(&shared).unwrap() {
            vram.upload(&img).unwrap();
        }
        for character in 0..16 {
            let file = big
                .read(&mut disc, BigIndex::RACER_MODEL_HI.nth(character).get())
                .unwrap();
            let model = Model::parse(&DramFile::parse(&file).unwrap()).unwrap();
            let header = &model.headers[0];
            let frame = &header.animations[0].frames[10];
            let m = model_mesh(header, frame, &vram, [0xccc; 3]);
            let positions = model_positions(header, frame, [0xccc; 3]);
            for (part, p) in m.parts.iter().zip(&positions) {
                assert_eq!(&part.positions, p, "racer {character}");
            }
            // Another pose has the same faces in the same order.
            let other = model_positions(header, &header.animations[0].frames[3], [0xccc; 3]);
            for (a, b) in positions.iter().zip(&other) {
                assert_eq!(a.len(), b.len(), "racer {character}");
            }
            let vertices: usize = m.parts.iter().map(|p| p.positions.len()).sum();
            assert!(
                vertices >= 3 * header.triangles(frame).len(),
                "racer {character}"
            );
            for p in &m.parts {
                assert_eq!(p.positions.len(), p.uvs.len());
                assert_eq!(p.positions.len(), p.colors.len());
                assert!(p.indices.iter().all(|&i| (i as usize) < p.positions.len()));
                assert!(p.uvs.iter().flatten().all(|c| (0.0..=1.0).contains(c)));
            }
            // A kart is a few world units across (the track is hundreds).
            let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
            for p in m.parts.iter().flat_map(|p| &p.positions) {
                for a in 0..3 {
                    lo[a] = lo[a].min(p[a]);
                    hi[a] = hi[a].max(p[a]);
                }
            }
            let size = [0, 1, 2].map(|a| hi[a] - lo[a]);
            println!(
                "racer {character}: size {size:?}, atlas {}x{}",
                m.atlas.width, m.atlas.height
            );
            assert!(
                size.iter().all(|&s| (0.3..3.0).contains(&s)),
                "racer {character}: {size:?}"
            );
        }
    }

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
            let m = level_mesh(
                &level,
                &dram,
                &vram,
                false,
                true,
                true,
            );
            for p in m.parts.iter().chain(&m.water) {
                assert_eq!(p.positions.len(), p.uvs.len());
                assert_eq!(p.positions.len(), p.colors.len());
                assert!(p.indices.iter().all(|&i| (i as usize) < p.positions.len()));
                assert!(p.uvs.iter().flatten().all(|c| (0.0..=1.0).contains(c)));
            }
            for corner in m.water_corners.values().flatten() {
                assert!(corner.slot < m.water[corner.part].positions.len());
            }
            let has_water = level.water.env_map.is_some();
            assert_eq!(
                has_water,
                m.water.iter().any(|p| !p.indices.is_empty()),
                "level {id}"
            );
            assert_eq!(m.parts.len(), m.biases.len() * PARTS);
            for corner in m.scenery_corners.values().flatten() {
                assert!(
                    corner.slot < m.parts[corner.part].positions.len(),
                    "level {id}"
                );
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
            // The faces face one way: of the triangles that lie flat, most point up.
            let (mut up, mut down) = (0, 0);
            for part in &m.parts {
                for t in part.indices.chunks(3) {
                    let [a, b, c] = [0, 1, 2].map(|i| part.positions[t[i] as usize]);
                    let (u, v) = (
                        std::array::from_fn::<f32, 3, _>(|k| b[k] - a[k]),
                        std::array::from_fn::<f32, 3, _>(|k| c[k] - a[k]),
                    );
                    let n = [
                        u[1] * v[2] - u[2] * v[1],
                        u[2] * v[0] - u[0] * v[2],
                        u[0] * v[1] - u[1] * v[0],
                    ];
                    let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                    if n[1].abs() > 0.9 * length {
                        if n[1] > 0.0 { up += 1 } else { down += 1 }
                    }
                }
            }
            assert!(
                up > down,
                "level {id}: flat triangles facing up {up}, down {down}"
            );
            println!(
                "level {id}: verts {:?}, biases {:?}, {} animations on {} faces, atlas {}x{}",
                m.parts
                    .iter()
                    .map(|p| p.positions.len())
                    .collect::<Vec<_>>(),
                m.biases,
                m.animations.len(),
                m.animated_faces.len(),
                m.atlas.width,
                m.atlas.height
            );
        }
    }
}
