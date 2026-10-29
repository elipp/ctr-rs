//! The constants, flag sets and plain data of the collision searches (`COLL_*`, the
//! scratchpad).

use gte::Gte;
use loader::lev::SVec3;

use crate::instance;

/// `COLL_FRACTION_ONE`: the whole segment is clear.
pub const FRACTION_ONE: i32 = math::FP_ONE;
/// `COLL_FRACTION_INVALID`.
pub const FRACTION_INVALID: i32 = -math::FP_ONE;

/// `COLL_SEARCH_*`
pub mod search_flags {
    pub const TEST_INSTANCES: u16 = 0x1;
    pub const HIGH_LOD: u16 = 0x2;
    pub const REUSE_NORMALS: u16 = 0x8;
    pub const WALL_PROJECTION_DONE: u16 = 0x10;
    pub const REPEAT_SCRUB: u16 = 0x20;
    pub const FORCE_INSTANCE_HIT: u16 = 0x40;
}

/// `COLL_STEP_*`
pub mod step_flags {
    pub const FLAG_KILL_PLANE: u32 = 0x4000;
    pub const FLAG_WATER_BSP: u32 = 0x8000;
}

/// `CollNormalAxis`: the dominant axis of a triangle normal.
pub mod normal_axis {
    pub const Z: i16 = 1;
    pub const X: i16 = 2;
    pub const Y: i16 = 3;
}

/// `CollTriangleClipResult`: which part of the triangle the swept point
/// was pushed back against.
pub mod clip {
    pub const MISS: i32 = -1;
    pub const V1: i32 = 0;
    pub const EDGE_V1_V2: i32 = 1;
    pub const V2: i32 = 2;
    pub const EDGE_V2_V3: i32 = 3;
    pub const V3: i32 = 4;
    pub const EDGE_V1_V3: i32 = 5;
    pub const FACE: i32 = 6;
}

/// `CollQuadTriangleId` (high LOD).
pub(crate) mod triangle_id {
    pub const HI_0: u8 = 2;
    pub const HI_1: u8 = 3;
    pub const HI_2: u8 = 4;
    pub const HI_3: u8 = 5;
    pub const HI_4: u8 = 6;
    pub const HI_5: u8 = 7;
    pub const HI_6: u8 = 8;
    pub const HI_7: u8 = 9;
}

/// `Input1.modelID` values: what is searching, which a model's `LInC` looks at.
pub mod model_id {
    /// `DYNAMIC_PLAYER`
    pub const PLAYER: i16 = 0x18;
    /// `DYNAMIC_ROBOT_CAR`
    pub const ROBOT_CAR: i16 = 0x3f;
    /// `PU_EXPLOSIVE_CRATE`: the Nitro crate.
    pub const NITRO_CRATE: i16 = 0x6;
    /// `STATIC_CRATE_TNT`
    pub const TNT_CRATE: i16 = 0x27;
    /// `DYNAMIC_ROCKET`
    pub const ROCKET: i16 = 0x29;
    /// `STATIC_BEAKER_RED`
    pub const RED_BEAKER: i16 = 0x46;
    /// `STATIC_BEAKER_GREEN`
    pub const GREEN_BEAKER: i16 = 0x47;
    /// `DYNAMIC_BOMB`
    pub const BOMB: i16 = 0x3b;
    /// `DYNAMIC_SHIELD` (the blue shield bomb) and `DYNAMIC_SHIELD_GREEN`
    pub const SHIELD: i16 = 0x56;
    pub const SHIELD_GREEN: i16 = 0x5e;
}

/// `COLL_MOVED_PLAYER_HIT_RADIUS`
pub const PLAYER_HIT_RADIUS: i16 = 0x19;
pub(crate) const SCRUB_DEPTH_REPEAT_STEP: i32 = 0x100;
pub(crate) const SCRUB_DEPTH_REPEAT_LIMIT: i32 = 0x401;
/// `ScratchpadStructExtended::bspSearchTriangle` has room for one entry per
/// iteration of the player search.
pub(crate) const MAX_SCRUB_TRIANGLES: usize = 15;
/// `DRAW_ORDER_LOW_DOUBLE_SIDED`
pub(crate) const DRAW_ORDER_LOW_DOUBLE_SIDED: u32 = 0x8000_0000;

/// Indices into `QuadBlock::tri_normal_vec_dividend`.
pub(crate) mod dividend {
    pub const HI_0: usize = 0;
    pub const HI_1: usize = 1;
    pub const HI_2: usize = 2;
    pub const HI_3: usize = 3;
    pub const HI_4: usize = 4;
    pub const HI_5: usize = 5;
    pub const HI_6: usize = 6;
    pub const HI_7: usize = 7;
    pub const LO_0: usize = 8;
    pub const LO_1: usize = 9;
}

/// Room for pending BSP children: the reference keeps them in the scratchpad
/// between 0x70 and `sps` at 0x108, two bytes each.
pub(crate) const BSP_STACK_LEN: usize = (0x108 - 0x70) / 2;

/// `CollPlane`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Plane {
    pub normal: SVec3,
    /// The plane constant shifted right by one.
    pub half_distance: i16,
}

/// `BspSearchVertex`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SearchVertex {
    pub pos: SVec3,
    /// Starts as the level vertex's flags, becomes the dominant normal axis.
    pub normal_axis: i16,
    /// Index into [`Level::vertices`].
    pub level_vertex: Option<u32>,
    pub plane: Plane,
}

/// `BspSearchResult`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SearchResult {
    pub hit_pos: SVec3,
    pub normal_axis: i16,
    pub plane: Plane,
    pub push_out: SVec3,
    pub reorder_result: i8,
    pub triangle_id: u8,
    /// Index into [`Level::quad_blocks`].
    pub quad: Option<u32>,
}

/// `BspSearchTriangle`: a triangle the player has already scraped against
/// during this search, and how hard to ignore it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScrubTriangle {
    /// Index into [`Level::quad_blocks`].
    pub quad: Option<u32>,
    pub triangle_id: i32,
    pub scrub_depth: i32,
}

/// `CollTriangleBarycentrics`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Barycentrics {
    pub v1: i16,
    pub v2: i16,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BoundingBox {
    pub min: SVec3,
    pub max: SVec3,
}

/// `ScratchpadStruct.Input1`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Input1 {
    /// The segment's top.
    pub pos: SVec3,
    pub hit_radius: i16,
    pub hit_radius_squared: i32,
    pub model_id: i16,
    pub scrub_depth: i16,
}

/// `ScratchpadStruct.Union.QuadBlockColl`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QuadBlockColl {
    /// The segment's bottom (the start of the swept test).
    pub pos: SVec3,
    pub hit_radius: i16,
    pub hit_radius_squared: i32,
    /// The segment's end; pulled in to each closer hit.
    pub hit_pos: SVec3,
    pub search_flags: u16,
    pub quad_flags_wanted: u32,
    pub quad_flags_ignored: u32,
}

/// `CollScratchWork` (without the instance hitbox scratch).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScratchWork {
    pub step_flags: u32,
    pub tri_normal_vec_dividend: i16,
    pub tri_normal_vec_bit_shift: u8,
    pub tri_normal_lod_shift: u8,
}

/// The collision scratchpad (`struct ScratchpadStruct`), minus the parts only
/// the left-out searches use. State persists between searches: with
/// [`search_flags::REUSE_NORMALS`] a search reuses the previous one's vertices
/// and planes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scratchpad {
    pub input1: Input1,
    pub quad_block_coll: QuadBlockColl,
    pub bbox: BoundingBox,
    pub num_triangles_tested: i16,
    pub did_touch_quadblock: i16,
    pub reset_only_40: i16,
    pub did_touch_hitbox: i16,
    pub candidate: SearchResult,
    pub hit: SearchResult,
    pub hit_fraction: i32,
    pub num_bsp_hitboxes_hit: i32,
    /// `bspHitbox`: the instance hitbox the search ended on.
    pub bsp_hitbox: Option<instance::HitboxHit>,
    /// `bspHitboxesHit`, the first `num_bsp_hitboxes_hit`: file offsets of the hitboxes already
    /// dealt with, which the search skips.
    pub bsp_hitboxes_hit: [Option<u32>; MAX_SCRUB_TRIANGLES],
    pub hit_barycentrics: Barycentrics,
    /// Level vertices of the hit triangle.
    pub hit_level_triangle: [Option<u32>; 3],
    pub candidate_delta: SVec3,
    /// `hitBspSearchTriangle`: indices into `search_vert` of the last triangle
    /// that got as far as the clip test (moved searches only).
    pub hit_search_triangle: [Option<u8>; 3],
    /// `ScratchpadStructExtended::bspSearchTriangle`, first `num_scrub_triangles`.
    pub scrub_triangles: [ScrubTriangle; MAX_SCRUB_TRIANGLES],
    pub num_scrub_triangles: i32,
    pub quad_second_tri_index_a: u16,
    pub quad_second_tri_index_b: u16,
    pub search_vert: [SearchVertex; 9],
    pub collision: ScratchWork,
}

/// The scratchpad and the GTE the searches run on.
#[derive(Clone, Debug, Default)]
pub struct Collision {
    pub sps: Scratchpad,
    pub gte: Gte,
    /// `sdata->doorAccessFlags`: door quad blocks whose terrain type matches
    /// are passable to moved searches.
    pub door_access_flags: u32,
    /// The `flags` of the level's instances (`Instance::flags`), where the game has changed them;
    /// those past its end have their `InstDef` flags. A hitbox of an instance without
    /// `DRAW_COLLISION_MASK` bits is not tested.
    pub instance_flags: Vec<u32>,
}
