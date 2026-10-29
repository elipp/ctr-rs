//! Which quad blocks the game draws, and which are turbo pads.

use std::collections::HashSet;

use loader::lev::{Level, QuadBlock, quad_flags};

pub(super) fn is_marker_red(c: [u8; 4]) -> bool {
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
pub(super) const PAD_VISUAL_MAX_DROP: i32 = 64;

/// How many steps of draw order bias the faces of a turbo pad get on top of their own (negative is
/// nearer, see [`LevelMesh::biases`]). A pad lies flush on the road, in places a little under it,
/// and the bytes of its block alone (a few steps ahead of the road's) do not win the depth test
/// here, where there is no ordering table. A step is a few `wgpu` constant depth bias units (see
/// the viewer's `DEPTH_BIAS_PER_ORDER`). With the camera's float depth that is roughly `1e-6` of
/// the distance to the camera a step, so 4000 of them pull a pad forward by about 0.4% of that
/// distance: some 8 game units at the distance of the kart. (The exact scale depends on the GPU.)
/// Only the picture changes. Tune by eye.
pub const TURBO_PAD_EXTRA_BIAS: i32 = -4000;
