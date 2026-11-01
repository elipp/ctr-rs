//! The mesh of a track: its quad blocks as faces, sorted into parts, with the animated, water
//! and scenery vertices that are patched every game tick.

use std::collections::{HashMap, HashSet};

use loader::dram::DramFile;
use loader::lev::{BspKind, Level};
use loader::texture::{
    AnimTex, FaceMode, TextureLayout, face_back_to_front, face_polygon, face_uv_order,
    resolve_animation, resolve_low_texture, resolve_texture,
};
use loader::vram::Vram;

use super::atlas::{Atlas, AtlasBuilder, TILE, TileKind, passes};
use super::classify::{TURBO_PAD_EXTRA_BIAS, is_collision_wall, is_marker_red, turbo_pad_visuals};
use super::{MeshPart, PARTS, Part, to_world, vertex_channel};

/// `BSP_LEAF_FLAG_WATER`
const BSP_LEAF_FLAG_WATER: u16 = 0x2;

/// `QUADBLOCK_DRAW_ORDER_LOW_DOUBLE_SIDED`
const QUADBLOCK_DRAW_ORDER_LOW_DOUBLE_SIDED: u32 = 0x8000_0000;

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
            for (face, &order) in face_order
                .iter()
                .enumerate()
                .take(if is_quad { 4 } else { 3 })
            {
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
                        order,
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
