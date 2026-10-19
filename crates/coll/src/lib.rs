//! Track collision, ported from the reference `ctr-native` (`game/COLL.c`,
//! `include/namespace_Coll.h`).
//!
//! This is the `COLL_FIXED_*` family: a swept point test of a segment against
//! the track's quad blocks, found through the BSP tree. It is plain integer
//! code with no allocation; all of its working state lives in [`Collision`]
//! (the reference's scratchpad plus the GTE it uses), which is cheap to clone.
//!
//! The `COLL_MOVED_*` family (the player's swept test, [`Collision::search_moved`])
//! shares the BSP walk, scratchpad and normals; it adds the clip classification
//! of each hit, the scrub-depth tracking and the step-flag triggers.
//!
//! Left out for now:
//! * the low-LOD triangle path (`COLL_FIXED_QUADBLK_GetNormVecs_LoLOD` and the
//!   two-triangle test): every racer, bot and camera search sets
//!   [`search_flags::HIGH_LOD`], so it is unused by the simulation;
//! * instance hitboxes (`COLL_FIXED_BSPLEAF_TestInstance`, `INSTANC_TestPoint`),
//!   which need the instance and thread model;
//! * the `ThBuckColl` view of the scratchpad union, which overlays
//!   `QuadBlockColl` and is only used by those searches;
//! * `COLL_MOVED_PlayerSearch` and `COLL_MOVED_ScrubImpact` themselves, which
//!   drive and mutate a `Driver`; only their search setup is here
//!   ([`Collision::begin_player_search`], [`Collision::prepare_sweep`]).
//!
//! Pointers in the reference scratchpad (quad blocks, level vertices) are
//! indices into [`Level`] here. `tests/reference.rs` checks this port against
//! the C code on real tracks.

use gte::{Gte, cmd, ctrl, data};
use loader::lev::{BspChild, BspKind, BspNode, Level, QuadBlock, SVec3, quad_flags};
use math::mips;

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
mod triangle_id {
    pub const HI_0: u8 = 2;
    pub const HI_1: u8 = 3;
    pub const HI_2: u8 = 4;
    pub const HI_3: u8 = 5;
    pub const HI_4: u8 = 6;
    pub const HI_5: u8 = 7;
    pub const HI_6: u8 = 8;
    pub const HI_7: u8 = 9;
}

/// `COLL_MOVED_PLAYER_HIT_RADIUS`
pub const PLAYER_HIT_RADIUS: i16 = 0x19;
const SCRUB_DEPTH_REPEAT_STEP: i32 = 0x100;
const SCRUB_DEPTH_REPEAT_LIMIT: i32 = 0x401;
/// `ScratchpadStructExtended::bspSearchTriangle` has room for one entry per
/// iteration of the player search.
const MAX_SCRUB_TRIANGLES: usize = 15;
/// `DRAW_ORDER_LOW_DOUBLE_SIDED`
const DRAW_ORDER_LOW_DOUBLE_SIDED: u32 = 0x8000_0000;

/// Indices into `QuadBlock::tri_normal_vec_dividend`.
mod dividend {
    pub const HI_0: usize = 0;
    pub const HI_1: usize = 1;
    pub const HI_2: usize = 2;
    pub const HI_3: usize = 3;
    pub const HI_4: usize = 4;
    pub const HI_5: usize = 5;
    pub const HI_6: usize = 6;
    pub const HI_7: usize = 7;
}

/// Room for pending BSP children: the reference keeps them in the scratchpad
/// between 0x70 and `sps` at 0x108, two bytes each.
const BSP_STACK_LEN: usize = (0x108 - 0x70) / 2;

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
}

fn pack(lo: i32, hi: i32) -> u32 {
    mips::pack_s16_pair(lo, hi)
}

fn abs(value: i32) -> i32 {
    if value < 0 { mips::neg_lo(value) } else { value }
}

/// The two coordinates (x=0, y=1, z=2) a triangle is projected onto, for its
/// dominant normal axis.
fn projection_axes(axis: i16) -> (usize, usize) {
    match axis {
        normal_axis::Y => (2, 0),
        normal_axis::Z => (0, 1),
        _ => (1, 2),
    }
}

fn coords(p: SVec3) -> [i32; 3] {
    [p.x as i32, p.y as i32, p.z as i32]
}

fn sv(x: i32, y: i32, z: i32) -> SVec3 {
    SVec3 { x: x as i16, y: y as i16, z: z as i16 }
}

/// `COLL_FIXED_TRIANGL_Barycentrics`: the point on edge `v1`-`v2` closest to
/// `point`, as a 4.12 fraction of the edge.
pub fn triangle_barycentrics(gte: &mut Gte, v1: SVec3, v2: SVec3, point: SVec3) -> SVec3 {
    let (v1x, v1y, v1z) = (v1.x as i32, v1.y as i32, v1.z as i32);
    let edge = (v2.x as i32 - v1x, v2.y as i32 - v1y, v2.z as i32 - v1z);
    let p = (point.x as i32 - v1x, point.y as i32 - v1y, point.z as i32 - v1z);

    gte.ctc2(pack(edge.0, edge.1), ctrl::R_MATRIX);
    gte.ctc2(pack(edge.2, p.0), ctrl::R13R21);
    gte.ctc2(pack(p.1, p.2), ctrl::R22R23);
    gte.mtc2(pack(edge.0, edge.1), data::VXY0);
    gte.mtc2(edge.2 as u32, data::VZ0);
    gte.cop2(MVMVA_R_V0);
    let mut point_dot = gte.mfc2(data::MAC2) as i32;
    let mut edge_dot = gte.mfc2(data::MAC1) as i32;

    gte.mtc2(point_dot as u32, data::LZCS);
    let shift = (gte.mfc2(data::LZCR) as i32 - 2).clamp(0, 12) as u32;
    point_dot = mips::sll(point_dot, shift);
    if shift < 12 {
        edge_dot = mips::sra(edge_dot, 12 - shift);
    }

    let mut factor = 0;
    if edge_dot != 0 {
        factor = mips::div(point_dot, edge_dot).clamp(0, FRACTION_ONE);
    }

    gte.mtc2(factor as u32, data::IR0);
    load_mac(gte, v1x, v1y, v1z);
    load_ir(gte, edge.0, edge.1, edge.2);
    gte.cop2(GPL_SF1);
    SVec3 {
        x: gte.mfc2(data::MAC1) as i16,
        y: gte.mfc2(data::MAC2) as i16,
        z: gte.mfc2(data::MAC3) as i16,
    }
}

// Command words the reference issues (`CollFixed_Gte*`).
/// `mvmva` with the rotation matrix, V0, no translation.
const MVMVA_R_V0: u32 = 0x0406012;
const GPL_SF1: u32 = 0x01a8003e;
/// `mvmva` with sf=1, the rotation matrix, V0, no translation.
const RTV0_SF1: u32 = 0x0486012;
const GPF_SF1: u32 = 0x0198003d;
const OP_SF0: u32 = 0x0170000c;
/// `mvmva` with sf=1, the rotation matrix, IR as the vector, no translation.
const RTIR: u32 = 0x049e012;
const _: () = assert!(MVMVA_R_V0 & cmd::FUNCT_MASK == cmd::MVMVA);

fn load_ir(gte: &mut Gte, x: i32, y: i32, z: i32) {
    gte.mtc2(x as u32, data::IR1);
    gte.mtc2(y as u32, data::IR2);
    gte.mtc2(z as u32, data::IR3);
}

fn load_mac(gte: &mut Gte, x: i32, y: i32, z: i32) {
    gte.mtc2(x as u32, data::MAC1);
    gte.mtc2(y as u32, data::MAC2);
    gte.mtc2(z as u32, data::MAC3);
}

/// Do two bounding boxes touch (the BSP search's test; inclusive).
fn overlaps(node: &BspNode, bounds: &BoundingBox) -> bool {
    let b = &node.bbox;
    b.min.y <= bounds.max.y
        && b.min.x <= bounds.max.x
        && bounds.min.x <= b.max.x
        && b.min.z <= bounds.max.z
        && bounds.min.z <= b.max.z
        && bounds.min.y <= b.max.y
}

impl Collision {
    /// `COLL_SearchBSP_CallbackQUADBLK`: sweep from `bottom` up to `top` with
    /// the given radius. Set `sps.quad_block_coll.{search_flags,
    /// quad_flags_wanted, quad_flags_ignored}` first. Results are left in
    /// `sps` (`hit_fraction`, `hit`, `hit_barycentrics`, ...).
    pub fn search_quadblocks(&mut self, level: &Level, top: SVec3, bottom: SVec3, hit_radius: i32) {
        let sps = &mut self.sps;
        let radius_squared = mips::mul_lo(hit_radius, hit_radius);

        sps.input1.hit_radius = hit_radius as i16;
        sps.quad_block_coll.hit_radius = hit_radius as i16;
        sps.did_touch_hitbox = 0;
        sps.num_triangles_tested = 0;
        sps.did_touch_quadblock = 0;
        sps.collision.step_flags = 0;
        sps.num_bsp_hitboxes_hit = 0;

        sps.input1.pos = top;
        sps.quad_block_coll.hit_pos = top;
        sps.quad_block_coll.pos = bottom;

        sps.hit_fraction = FRACTION_ONE;
        sps.input1.hit_radius_squared = radius_squared;
        sps.quad_block_coll.hit_radius_squared = radius_squared;

        let axis = |top: i16, bottom: i16| if top < bottom { (top, bottom) } else { (bottom, top) };
        (sps.bbox.min.x, sps.bbox.max.x) = axis(top.x, bottom.x);
        (sps.bbox.min.y, sps.bbox.max.y) = axis(top.y, bottom.y);
        (sps.bbox.min.z, sps.bbox.max.z) = axis(top.z, bottom.z);

        let bounds = sps.bbox;
        search_bsp(level, &bounds, |this, level, leaf| this.test_leaf_quadblocks(level, leaf, false), self);
    }

    /// `COLL_SearchBSP_CallbackPARAM` with `COLL_MOVED_BSPLEAF_TestQuadblocks`:
    /// the player's swept test over `sps.bbox`. The caller sets up the
    /// scratchpad first (see [`Collision::sweep_player`]); hits are left in
    /// `sps` (`hit_fraction`, `hit`, `hit_level_triangle`, ...).
    pub fn search_moved(&mut self, level: &Level) {
        let bounds = self.sps.bbox;
        search_bsp(level, &bounds, |this, level, leaf| this.test_leaf_quadblocks(level, leaf, true), self);
    }

    /// `COLL_SearchBSP_CallbackPARAM` with `COLL_FIXED_BSPLEAF_TestQuadblocks`
    /// over `sps.bbox`: the point test of `COLL_FIXED_PlayerSearch`. The
    /// caller sets up the scratchpad (segment, bounds, flags) first.
    pub fn search_fixed(&mut self, level: &Level) {
        let bounds = self.sps.bbox;
        search_bsp(level, &bounds, |this, level, leaf| this.test_leaf_quadblocks(level, leaf, false), self);
    }

    /// `COLL_FIXED_QUADBLK_TestTriangles` on a single quad block.
    pub fn test_fixed_quadblock(&mut self, level: &Level, quad_index: u32) {
        self.test_quadblock_triangles(level, quad_index, false);
    }

    /// The setup of `COLL_MOVED_PlayerSearch` before its iterations: hit
    /// radius, the surfaces the player collides with, and an empty scrub list.
    /// Always high LOD (see the module comment). `TEST_INSTANCES` is set as in
    /// the original, but there are no instance hitboxes to test yet.
    pub fn begin_player_search(&mut self) {
        let radius = PLAYER_HIT_RADIUS;
        let sps = &mut self.sps;
        sps.input1.hit_radius = radius;
        sps.input1.hit_radius_squared = radius as i32 * radius as i32;
        let q = &mut sps.quad_block_coll;
        q.hit_radius = radius;
        q.hit_radius_squared = radius as i32 * radius as i32;
        q.quad_flags_wanted = (quad_flags::GROUND | quad_flags::COLLISION_SURFACE) as u32;
        q.quad_flags_ignored = 0;
        q.search_flags = search_flags::TEST_INSTANCES | search_flags::HIGH_LOD;
        sps.num_bsp_hitboxes_hit = 0;
        sps.collision.step_flags = 0;
        self.find_scrub(None, 0);
    }

    /// The start of one iteration of `COLL_MOVED_PlayerSearch`'s sweep from
    /// `current` to `next` (both in the scratchpad's 1/256 units): resets the
    /// per-sweep results and sets the swept bounding box. Returns false when
    /// the player does not move, which ends the search; otherwise
    /// [`Collision::search_moved`] follows.
    /// `sps.input1.hit_radius` and the search flags are the caller's.
    pub fn prepare_sweep(&mut self, current: SVec3, next: SVec3) -> bool {
        let sps = &mut self.sps;
        sps.did_touch_quadblock = 0;
        sps.num_triangles_tested = 0;
        sps.did_touch_hitbox = 0;
        sps.reset_only_40 = 0;
        sps.hit_fraction = FRACTION_ONE;
        sps.quad_block_coll.pos = current;
        sps.input1.pos = next;
        if next == current {
            return false;
        }

        let radius = sps.input1.hit_radius as i32;
        let axis = |current: i16, next: i16| {
            let (current, next) = (current as i32, next as i32);
            let min = mips::sub_lo(next, radius).min(mips::sub_lo(current, radius));
            let max = mips::add_lo(current, radius).max(mips::add_lo(next, radius));
            (min as i16, max as i16)
        };
        (sps.bbox.min.x, sps.bbox.max.x) = axis(current.x, next.x);
        (sps.bbox.min.y, sps.bbox.max.y) = axis(current.y, next.y);
        (sps.bbox.min.z, sps.bbox.max.z) = axis(current.z, next.z);
        sps.quad_block_coll.hit_pos = next;
        true
    }

    /// `COLL_MOVED_FindScrub`: looks the hit triangle up in the scrub list.
    /// A repeat hit raises its depth (which `ScrubImpact` subtracts from the
    /// impact, so repeated scraping softens) and sets `REPEAT_SCRUB`; a new
    /// one is added at depth 0. `None` clears the list.
    pub fn find_scrub(&mut self, quad: Option<u32>, triangle_id: i32) {
        let sps = &mut self.sps;
        let flags = sps.quad_block_coll.search_flags;
        let Some(quad) = quad else {
            sps.quad_block_coll.search_flags = flags & !search_flags::REPEAT_SCRUB;
            sps.input1.scrub_depth = 0;
            sps.num_scrub_triangles = 0;
            return;
        };

        let count = sps.num_scrub_triangles as usize;
        for tri in sps.scrub_triangles[..count].iter_mut().rev() {
            if tri.quad == Some(quad) && tri.triangle_id == triangle_id {
                if tri.scrub_depth < SCRUB_DEPTH_REPEAT_LIMIT {
                    tri.scrub_depth = mips::add_lo(tri.scrub_depth, SCRUB_DEPTH_REPEAT_STEP);
                }
                sps.quad_block_coll.search_flags = flags | search_flags::REPEAT_SCRUB;
                sps.input1.scrub_depth = tri.scrub_depth as i16;
                return;
            }
        }

        sps.scrub_triangles[count] = ScrubTriangle { quad: Some(quad), triangle_id, scrub_depth: 0 };
        sps.quad_block_coll.search_flags = flags & !search_flags::REPEAT_SCRUB;
        sps.input1.scrub_depth = 0;
        sps.num_scrub_triangles += 1;
    }

    /// `COLL_FIXED_BSPLEAF_TestQuadblocks`, or with `moved` its
    /// `COLL_MOVED_BSPLEAF_TestQuadblocks` twin.
    fn test_leaf_quadblocks(&mut self, level: &Level, leaf: &BspNode, moved: bool) {
        if leaf.is_water() {
            self.sps.collision.step_flags |= step_flags::FLAG_WATER_BSP;
        }
        let BspKind::Leaf { quad_blocks, .. } = &leaf.kind else {
            panic!("BSP leaf reference points at a branch node");
        };
        // The reference loops do-while, so an empty leaf still tests the quad
        // at its (null) pointer. Dingo Canyon has one; whatever that reads is
        // not meaningful, so it is modelled as a quad that never matches.
        if quad_blocks.is_empty() {
            self.sps.candidate.quad = None;
            return;
        }
        for i in quad_blocks.clone() {
            self.test_quadblock_triangles(level, i as u32, moved);
        }
        // Instance hitboxes (`COLL_SEARCH_TEST_INSTANCES`) are not ported yet.
    }

    /// `COLL_FIXED_QUADBLK_TestTriangles` (high LOD only), or with `moved`
    /// `COLL_MOVED_QUADBLK_TestTriangles`, which also records the triangle id.
    fn test_quadblock_triangles(&mut self, level: &Level, quad_index: u32, moved: bool) {
        let quad = &level.quad_blocks[quad_index as usize];
        let has_second_triangle = quad.vertex_index[2] != quad.vertex_index[3];
        let flags = self.sps.quad_block_coll.search_flags;
        let sps = &mut self.sps;

        sps.candidate.quad = Some(quad_index);

        let (b, q) = (&sps.bbox, &quad.bbox);
        if sps.quad_block_coll.quad_flags_wanted & quad.flags as u32 == 0
            || sps.quad_block_coll.quad_flags_ignored & quad.flags as u32 != 0
            || q.min.x > b.max.x
            || q.min.y > b.max.y
            || q.min.z > b.max.z
            || b.min.x > q.max.x
            || b.min.y > q.max.y
            || b.min.z > q.max.z
        {
            return;
        }

        assert!(flags & search_flags::HIGH_LOD != 0, "low-LOD collision is not ported");
        if flags & search_flags::REUSE_NORMALS == 0 {
            self.quadblock_normals_hi_lod(level, quad);
        }

        let test = |this: &mut Self, id: u8, v1: usize, v2: usize, v3: usize| {
            if moved {
                this.sps.candidate.triangle_id = id;
                this.moved_triangle_test_point(level, v1, v2, v3);
            } else {
                this.triangle_test_point(level, v1, v2, v3);
            }
        };
        test(self, triangle_id::HI_0, 0, 4, 5);
        test(self, triangle_id::HI_1, 4, 6, 5);
        test(self, triangle_id::HI_2, 6, 4, 1);
        test(self, triangle_id::HI_3, 5, 6, 2);
        if moved {
            self.sps.candidate.triangle_id = triangle_id::HI_4;
        }
        if has_second_triangle {
            test(self, triangle_id::HI_4, 8, 6, 7);
            test(self, triangle_id::HI_5, 7, 3, 8);
            test(self, triangle_id::HI_6, 1, 7, 6);
            test(self, triangle_id::HI_7, 2, 6, 8);
        }
    }

    /// `COLL_FIXED_QUADBLK_LoadScratchpadVerts`
    fn load_scratchpad_verts(&mut self, level: &Level, quad: &QuadBlock) {
        self.sps.quad_second_tri_index_a = quad.vertex_index[2];
        self.sps.quad_second_tri_index_b = quad.vertex_index[3];
        for (bsv, &index) in self.sps.search_vert.iter_mut().zip(&quad.vertex_index) {
            let vertex = &level.vertices[index as usize];
            bsv.level_vertex = Some(index as u32);
            bsv.pos = vertex.pos;
            bsv.normal_axis = vertex.flags as i16;
        }
    }

    /// `COLL_FIXED_QUADBLK_GetNormVecs_HiLOD`: planes for the eight small
    /// triangles of the 3x3 vertex patch.
    fn quadblock_normals_hi_lod(&mut self, level: &Level, quad: &QuadBlock) {
        self.load_scratchpad_verts(level, quad);

        self.sps.collision.tri_normal_lod_shift = 0;
        self.sps.collision.tri_normal_vec_bit_shift = quad.tri_normal_vec_bit_shift as u8;

        let has_second_triangle = self.sps.quad_second_tri_index_a != self.sps.quad_second_tri_index_b;
        if has_second_triangle {
            self.triangle_norm_vec(quad.tri_normal_vec_dividend[dividend::HI_4], 8, 6, 7);
            self.triangle_norm_vec(quad.tri_normal_vec_dividend[dividend::HI_5], 7, 3, 8);
            self.triangle_norm_vec(quad.tri_normal_vec_dividend[dividend::HI_6], 1, 7, 6);
            self.triangle_norm_vec(quad.tri_normal_vec_dividend[dividend::HI_7], 2, 6, 8);
        }
        self.triangle_norm_vec(quad.tri_normal_vec_dividend[dividend::HI_0], 0, 4, 5);
        self.triangle_norm_vec(quad.tri_normal_vec_dividend[dividend::HI_1], 4, 6, 5);
        self.triangle_norm_vec(quad.tri_normal_vec_dividend[dividend::HI_2], 6, 4, 1);
        self.triangle_norm_vec(quad.tri_normal_vec_dividend[dividend::HI_3], 5, 6, 2);
    }

    /// `COLL_FIXED_TRIANGL_GetNormVec`: computes the plane of triangle
    /// (`v1`, `v2`, `v3`) into `search_vert[v1]`. The dividend is set in the
    /// scratchpad first, as the reference does.
    pub fn triangle_norm_vec(&mut self, dividend: i16, v1: usize, v2: usize, v3: usize) {
        self.sps.collision.tri_normal_vec_dividend = dividend;
        let sps = &mut self.sps;
        let gte = &mut self.gte;
        let (p1, p2, p3) = (sps.search_vert[v1].pos, sps.search_vert[v2].pos, sps.search_vert[v3].pos);

        let (v1x, v1y, v1z) = (p1.x as i32, p1.y as i32, p1.z as i32);
        let edge_a = (p3.x as i32 - v1x, p3.y as i32 - v1y, p3.z as i32 - v1z);
        let edge_b = (p2.x as i32 - v1x, p2.y as i32 - v1y, p2.z as i32 - v1z);

        gte.ctc2(edge_a.0 as u16 as u32, ctrl::R_MATRIX);
        gte.ctc2(edge_a.1 as u16 as u32, ctrl::R22R23);
        gte.ctc2(edge_a.2 as u16 as u32, ctrl::R33);
        load_ir(gte, edge_b.0, edge_b.1, edge_b.2);

        let lod_shift = sps.collision.tri_normal_lod_shift as u32;
        let normal_shift = sps.collision.tri_normal_vec_bit_shift as u32;
        let scale = sps.collision.tri_normal_vec_dividend as i32;

        gte.cop2(OP_SF0);
        let mut n = [
            gte.mfc2(data::MAC1) as i32,
            gte.mfc2(data::MAC2) as i32,
            gte.mfc2(data::MAC3) as i32,
        ];

        gte.ctc2(pack(v1x, v1y), ctrl::R_MATRIX);
        gte.ctc2(v1z as u16 as u32, ctrl::R13R21);

        for c in &mut n {
            *c = mips::sra(mips::mul_lo(mips::sra(*c, lod_shift), scale), normal_shift);
        }

        load_ir(gte, n[0], n[1], n[2]);
        let bsv = &mut sps.search_vert[v1];
        bsv.plane.normal.x = n[0] as i16;
        bsv.plane.normal.y = n[1] as i16;

        gte.cop2(RTIR);
        let plane = gte.mfc2(data::MAC1) as i32;

        bsv.plane.normal.z = n[2] as i16;
        bsv.plane.half_distance = mips::sra(plane, 1) as i16;

        let (ax, ay, az) = (abs(n[0]), abs(n[1]), abs(n[2]));
        let mut dominant = normal_axis::Z;
        if ax - ay < 0 {
            if ay - az >= 0 {
                dominant = normal_axis::Y;
            }
        } else if ax - az >= 0 {
            dominant = normal_axis::X;
        }
        bsv.normal_axis = dominant;
    }

    /// `COLL_FIXED_TRIANGL_TestPoint`: tests the segment against triangle
    /// (`v1`, `v2`, `v3`), whose plane must already be in `search_vert[v1]`.
    pub fn triangle_test_point(&mut self, level: &Level, v1: usize, v2: usize, v3: usize) {
        let sps = &mut self.sps;
        let plane = sps.search_vert[v1].plane;
        let normal_zw = pack(plane.normal.z as i32, plane.half_distance as i32) as i32;

        sps.num_triangles_tested = sps.num_triangles_tested.wrapping_add(1);
        sps.candidate.normal_axis = sps.search_vert[v1].normal_axis;
        sps.candidate.plane = plane;

        self.triangle_test_point_body(level, v1, v2, v3, normal_zw);
    }

    /// `COLL_FIXED_TRIANGL_TestPoint_Body`
    fn triangle_test_point_body(&mut self, level: &Level, v1: usize, v2: usize, v3: usize, normal_zw: i32) {
        let sps = &mut self.sps;
        let gte = &mut self.gte;
        let q = sps.quad_block_coll;

        let (start_x, start_y, start_z) = (q.pos.x as i32, q.pos.y as i32, q.pos.z as i32);
        let delta_x = q.hit_pos.x as i32 - start_x;
        let delta_y = q.hit_pos.y as i32 - start_y;
        let delta_z = q.hit_pos.z as i32 - start_z;

        let normal = sps.candidate.plane.normal;
        gte.ctc2(pack(start_x, start_y), ctrl::R_MATRIX);
        gte.ctc2(pack(start_z, delta_x), ctrl::R13R21);
        gte.ctc2(pack(delta_y, delta_z), ctrl::R22R23);
        gte.mtc2(pack(normal.x as i32, normal.y as i32), data::VXY0);
        gte.mtc2(normal_zw as u32, data::VZ0);

        let normal_zw = mips::sll(mips::sra(normal_zw, 16), 13);

        gte.cop2(MVMVA_R_V0);
        let line_dot = gte.mfc2(data::MAC2) as i32;
        let plane_dot = mips::sub_lo(gte.mfc2(data::MAC1) as i32, normal_zw);

        if line_dot >= 0 {
            return;
        }

        let factor = mips::div(mips::neg_lo(plane_dot), mips::sra(line_dot, 12));

        load_mac(gte, start_x, start_y, start_z);
        load_ir(gte, delta_x, delta_y, delta_z);
        gte.mtc2(factor as u32, data::IR0);

        if !(0..=FRACTION_ONE).contains(&factor) {
            return;
        }

        gte.cop2(GPL_SF1);
        let hit = [
            gte.mfc2(data::MAC1) as i32,
            gte.mfc2(data::MAC2) as i32,
            gte.mfc2(data::MAC3) as i32,
        ];

        sps.candidate.hit_pos = sv(hit[0], hit[1], hit[2]);

        let (p1, p2, p3) = (sps.search_vert[v1].pos, sps.search_vert[v2].pos, sps.search_vert[v3].pos);
        let (c1, c2, c3) = (coords(p1), coords(p2), coords(p3));

        // Project onto the plane's two least-significant axes: (first, second).
        let (first, second) = projection_axes(sps.candidate.normal_axis);

        let (mut bary_v1, mut bary_v2) = (v2, v3);
        let origin = c1[first];
        let mut first_a = c2[first] - origin;
        let mut first_b = c3[first] - origin;
        let first_hit = hit[first] - origin;
        if abs(first_a) < abs(first_b) {
            std::mem::swap(&mut first_a, &mut first_b);
            (bary_v1, bary_v2) = (v3, v2);
        }
        let (second_a_pos, second_b_pos) = (
            sps.search_vert[bary_v1].pos,
            sps.search_vert[bary_v2].pos,
        );
        let origin = c1[second];
        let second_a = coords(second_a_pos)[second] - origin;
        let second_b = coords(second_b_pos)[second] - origin;
        let second_hit = hit[second] - origin;

        let mut bary_a = FRACTION_INVALID;
        let mut bary_b = FRACTION_INVALID;

        if first_a != 0 {
            let denom = mips::sra(
                mips::sub_lo(mips::mul_lo(second_b, first_a), mips::mul_lo(first_b, second_a)),
                6,
            );
            if denom != 0 {
                bary_b = mips::div(
                    mips::sll(
                        mips::sub_lo(mips::mul_lo(second_hit, first_a), mips::mul_lo(first_hit, second_a)),
                        6,
                    ),
                    denom,
                );
                if (0..=FRACTION_ONE).contains(&bary_b) {
                    bary_a = mips::div(
                        mips::sub_lo(mips::sll(first_hit, 12), mips::mul_lo(bary_b, first_b)),
                        first_a,
                    );
                }
            }
        } else {
            if first_b == 0 {
                return;
            }
            bary_b = mips::div(mips::sll(first_hit, 12), first_b);
            if (0..=FRACTION_ONE).contains(&bary_b) {
                bary_a = mips::div(
                    mips::sub_lo(mips::sll(second_hit, 12), mips::mul_lo(bary_b, second_b)),
                    second_a,
                );
            }
        }

        let quad_index = sps.candidate.quad.expect("candidate quad block is set");
        let quad = &level.quad_blocks[quad_index as usize];

        if bary_a < 0 || mips::sub_lo(mips::add_lo(bary_a, bary_b), FRACTION_ONE) > 0 {
            return;
        }

        if quad.flags & quad_flags::TRIGGER != 0 {
            sps.collision.step_flags |= quad.terrain_type as u32;
            return;
        }

        sps.hit.quad = Some(quad_index);
        sps.hit_barycentrics.v1 = bary_a as i16;
        sps.hit_barycentrics.v2 = bary_b as i16;
        sps.hit_level_triangle = [
            sps.search_vert[v1].level_vertex,
            sps.search_vert[bary_v1].level_vertex,
            sps.search_vert[bary_v2].level_vertex,
        ];
        sps.did_touch_quadblock = sps.did_touch_quadblock.wrapping_add(1);
        sps.hit.hit_pos = sps.candidate.hit_pos;
        sps.quad_block_coll.hit_pos = sps.candidate.hit_pos;
        sps.hit.plane = sps.candidate.plane;
    }

    /// `CollMoved_SelectProjection`: the edge vectors and hit point of the
    /// clip test, projected onto two axes. Swaps `v2`/`v3` so that `first_a` is
    /// the longer edge.
    fn select_projection(&self, v1: usize, v2: &mut usize, v3: &mut usize) -> Projection {
        let sps = &self.sps;
        let pos = |i: usize| coords(sps.search_vert[i].pos);
        let push_out = coords(sps.candidate.push_out);
        let (first, second) = projection_axes(sps.candidate.normal_axis);

        let origin = pos(v1)[first];
        let mut first_a = pos(*v2)[first] - origin;
        let mut first_b = pos(*v3)[first] - origin;
        let first_hit = push_out[first] - origin;
        if abs(first_a) < abs(first_b) {
            std::mem::swap(&mut first_a, &mut first_b);
            std::mem::swap(v2, v3);
        }

        let origin = pos(v1)[second];
        Projection {
            first_a,
            first_b,
            first_hit,
            second_a: pos(*v2)[second] - origin,
            second_b: pos(*v3)[second] - origin,
            second_hit: push_out[second] - origin,
        }
    }

    /// `COLL_MOVED_TRIANGL_ReorderNormals`: classifies where the pushed-out
    /// point lies relative to the triangle ([`clip`]) and sets
    /// `candidate.hit_pos` to the closest point on it.
    fn reorder_normals(&mut self, v1: usize, v2: usize, v3: usize) -> i32 {
        let (mut bary_v2, mut bary_v3) = (v2, v3);
        let p = self.select_projection(v1, &mut bary_v2, &mut bary_v3);

        let mut bary_a = FRACTION_INVALID;
        let mut bary_b = FRACTION_INVALID;

        if p.first_a == 0 {
            if p.first_b == 0 {
                return clip::MISS;
            }
            bary_b = mips::div(mips::sll(p.first_hit, 12), p.first_b);
            if p.second_a != 0 {
                bary_a = mips::div(
                    mips::sub_lo(mips::sll(p.second_hit, 12), mips::mul_lo(bary_b, p.second_b)),
                    p.second_a,
                );
            }
        } else {
            let denom = mips::sra(
                mips::sub_lo(mips::mul_lo(p.second_b, p.first_a), mips::mul_lo(p.first_b, p.second_a)),
                6,
            );
            if denom != 0 {
                bary_b = mips::div(
                    mips::sll(
                        mips::sub_lo(mips::mul_lo(p.second_hit, p.first_a), mips::mul_lo(p.first_hit, p.second_a)),
                        6,
                    ),
                    denom,
                );
                bary_a = mips::div(
                    mips::sub_lo(mips::sll(p.first_hit, 12), mips::mul_lo(bary_b, p.first_b)),
                    p.first_a,
                );
            }
        }

        if bary_a == FRACTION_INVALID {
            return clip::MISS;
        }

        let sps = &mut self.sps;
        let vertex = |i: usize| sps.search_vert[i].pos;
        let sum = mips::sub_lo(mips::add_lo(bary_a, bary_b), FRACTION_ONE);
        let push_out = sps.candidate.push_out;

        if bary_a < 0 {
            if bary_b < 0 {
                sps.candidate.hit_pos = vertex(v1);
                return clip::V1;
            }
            if sum >= 0 {
                sps.candidate.hit_pos = vertex(bary_v3);
                return clip::V3;
            }
            sps.candidate.hit_pos = triangle_barycentrics(&mut self.gte, vertex(v1), vertex(bary_v3), push_out);
            return clip::EDGE_V1_V3;
        }

        if bary_b >= 0 {
            if sum <= 0 {
                sps.candidate.hit_pos = push_out;
                return clip::FACE;
            }
            sps.candidate.hit_pos = triangle_barycentrics(&mut self.gte, vertex(bary_v2), vertex(bary_v3), push_out);
            return clip::EDGE_V2_V3;
        }

        if sum >= 0 {
            sps.candidate.hit_pos = vertex(bary_v2);
            return clip::V2;
        }
        sps.candidate.hit_pos = triangle_barycentrics(&mut self.gte, vertex(v1), vertex(bary_v2), push_out);
        clip::EDGE_V1_V2
    }

    /// `COLL_MOVED_TRIANGL_TestPoint`: sweeps the player's sphere from
    /// `quad_block_coll.pos` to `input1.pos` against triangle (`v1`, `v2`,
    /// `v3`), whose plane must already be in `search_vert[v1]`, and keeps the
    /// hit if it is the earliest so far (`hit_fraction`).
    pub fn moved_triangle_test_point(&mut self, level: &Level, v1: usize, v2: usize, v3: usize) {
        let sps = &mut self.sps;
        let gte = &mut self.gte;

        sps.num_triangles_tested = sps.num_triangles_tested.wrapping_add(1);
        sps.candidate.normal_axis = sps.search_vert[v1].normal_axis;
        sps.candidate.plane = sps.search_vert[v1].plane;

        let quad_index = sps.candidate.quad.expect("candidate quad block is set");
        let quad = &level.quad_blocks[quad_index as usize];
        let plane = sps.candidate.plane;
        let normal_zw = pack(plane.normal.z as i32, plane.half_distance as i32) as i32;

        if quad.flags & quad_flags::DOOR != 0 && (quad.terrain_type as i8 as i32 & self.door_access_flags as i32) != 0 {
            return;
        }

        let (pos, start) = (sps.input1.pos, sps.quad_block_coll.pos);
        gte.ctc2(pack(pos.x as i32, pos.y as i32), ctrl::R_MATRIX);
        gte.ctc2(pack(pos.z as i32, start.x as i32), ctrl::R13R21);
        gte.ctc2(pack(start.y as i32, start.z as i32), ctrl::R22R23);
        gte.mtc2(pack(plane.normal.x as i32, plane.normal.y as i32), data::VXY0);
        gte.mtc2(normal_zw as u32, data::VZ0);
        gte.cop2(RTV0_SF1);

        let plane_offset = mips::sll(plane.half_distance as i32, 1);
        let mut plane_near = mips::sub_lo(gte.mfc2(data::MAC1) as i32, plane_offset);
        let mut plane_far = mips::sub_lo(gte.mfc2(data::MAC2) as i32, plane_offset);

        // A back-facing plane is flipped for triggers and double-sided quads.
        if plane_far < 0 && (quad.flags & quad_flags::TRIGGER != 0 || quad.draw_order_low & DRAW_ORDER_LOW_DOUBLE_SIDED != 0) {
            plane_near = mips::neg_lo(plane_near);
            plane_far = mips::neg_lo(plane_far);
            let n = &mut sps.candidate.plane.normal;
            n.x = mips::neg_lo(n.x as i32) as i16;
            n.y = mips::neg_lo(n.y as i32) as i16;
            n.z = mips::neg_lo(n.z as i32) as i16;
            sps.candidate.plane.half_distance = mips::neg_lo(sps.candidate.plane.half_distance as i32) as i16;
        }

        sps.num_triangles_tested = sps.num_triangles_tested.wrapping_add(1);
        let radius = sps.input1.hit_radius as i32;

        if mips::sub_lo(plane_near, radius) >= 0 || plane_far < 0 {
            return;
        }
        if quad.flags & quad_flags::TRIGGER == 0 && mips::sub_lo(plane_near, plane_far) > 0 {
            return;
        }

        let used_segment_projection;
        if plane_near >= 0 {
            gte.mtc2(plane_near as u32, data::IR0);
            let n = sps.candidate.plane.normal;
            load_ir(gte, n.x as i32, n.y as i32, n.z as i32);
            used_segment_projection = false;
        } else {
            load_ir(
                gte,
                mips::sub_lo(pos.x as i32, start.x as i32),
                mips::sub_lo(pos.y as i32, start.y as i32),
                mips::sub_lo(pos.z as i32, start.z as i32),
            );
            let fraction = mips::div(mips::sll(mips::neg_lo(plane_near), 12), mips::sub_lo(plane_far, plane_near));
            gte.mtc2(fraction as u32, data::IR0);
            used_segment_projection = true;
        }
        gte.cop2(GPF_SF1);
        let hit = [
            gte.mfc2(data::MAC1) as i32,
            gte.mfc2(data::MAC2) as i32,
            gte.mfc2(data::MAC3) as i32,
        ];

        sps.candidate.push_out = sv(
            mips::sub_lo(pos.x as i32, hit[0]),
            mips::sub_lo(pos.y as i32, hit[1]),
            mips::sub_lo(pos.z as i32, hit[2]),
        );
        sps.hit_search_triangle = [Some(v1 as u8), Some(v2 as u8), Some(v3 as u8)];

        let reorder_result = self.reorder_normals(v1, v2, v3);
        if reorder_result < 0 {
            return;
        }

        let sps = &mut self.sps;
        let gte = &mut self.gte;
        let (from, hit_pos) = if used_segment_projection { (sps.candidate.push_out, sps.candidate.hit_pos) } else { (pos, sps.candidate.hit_pos) };
        sps.candidate_delta = sv(
            mips::sub_lo(from.x as i32, hit_pos.x as i32),
            mips::sub_lo(from.y as i32, hit_pos.y as i32),
            mips::sub_lo(from.z as i32, hit_pos.z as i32),
        );

        let d = sps.candidate_delta;
        gte.ctc2(pack(d.x as i32, d.y as i32), ctrl::R_MATRIX);
        gte.ctc2(d.z as i32 as u32, ctrl::R13R21);
        gte.mtc2(pack(d.x as i32, d.y as i32), data::VXY0);
        gte.mtc2(d.z as i32 as u32, data::VZ0);
        gte.cop2(MVMVA_R_V0);
        let distance_squared = gte.mfc2(data::MAC1) as i32;

        if mips::sub_lo(distance_squared, sps.input1.hit_radius_squared) > 0 {
            return;
        }

        if quad.flags & quad_flags::TRIGGER != 0
            && (plane_near < 0 || (mips::sub_lo(plane_near, radius) | mips::sub_lo(plane_far, radius)) < 0)
        {
            sps.collision.step_flags |= quad.terrain_type as u32;
            return;
        }

        let mut distance = mips::sub_lo(plane_far, plane_near);
        if distance != 0 {
            distance = mips::sub_lo(
                FRACTION_ONE,
                mips::div(mips::sll(mips::sub_lo(radius, plane_near), 12), distance),
            );
        }

        if mips::sub_lo(distance, sps.hit_fraction) >= 0 {
            return;
        }

        if quad.flags & quad_flags::NO_COLLISION_RESPONSE != 0 {
            if quad.flags & quad_flags::KILL_PLANE != 0 {
                sps.collision.step_flags |= step_flags::FLAG_KILL_PLANE;
            }
            return;
        }

        sps.hit_fraction = distance;
        let level_vertex = |i: usize| sps.search_vert[i].level_vertex;
        sps.hit_level_triangle = [level_vertex(v1), level_vertex(v2), level_vertex(v3)];

        sps.hit.hit_pos = sps.candidate.hit_pos;
        sps.hit.normal_axis = sps.candidate.normal_axis;
        sps.hit.plane = sps.candidate.plane;
        sps.hit.push_out = sps.candidate.push_out;
        sps.hit.quad = Some(quad_index);
        sps.hit.triangle_id = sps.candidate.triangle_id;
        sps.hit.reorder_result = reorder_result as i8;

        if distance <= 0 {
            sps.quad_block_coll.hit_pos = start;
        } else {
            gte.mtc2(distance as u32, data::IR0);
            load_ir(
                gte,
                mips::sub_lo(pos.x as i32, start.x as i32),
                mips::sub_lo(pos.y as i32, start.y as i32),
                mips::sub_lo(pos.z as i32, start.z as i32),
            );
            gte.cop2(GPF_SF1);
            sps.quad_block_coll.hit_pos = sv(
                mips::add_lo(start.x as i32, gte.mfc2(data::MAC1) as i32),
                mips::add_lo(start.y as i32, gte.mfc2(data::MAC2) as i32),
                mips::add_lo(start.z as i32, gte.mfc2(data::MAC3) as i32),
            );
        }

        sps.did_touch_quadblock = sps.did_touch_quadblock.wrapping_add(1);
    }
}

/// `CollTriangleProjection`
struct Projection {
    first_a: i32,
    first_b: i32,
    first_hit: i32,
    second_a: i32,
    second_b: i32,
    second_hit: i32,
}

/// `COLL_SearchBSP_CallbackPARAM`: calls `callback` on every BSP leaf whose
/// bounds touch `bounds`, in the reference's order (the second child of a
/// branch is visited before the first).
fn search_bsp<T>(level: &Level, bounds: &BoundingBox, callback: impl Fn(&mut T, &Level, &BspNode), state: &mut T) {
    let Some(root) = level.bsp.first() else { return };
    let mut stack = [BspChild::None; BSP_STACK_LEN];
    let mut top = 0;

    let push_children = |node: &BspNode, stack: &mut [BspChild; BSP_STACK_LEN], top: &mut usize| {
        let BspKind::Branch { children, .. } = &node.kind else { return };
        for &child in &children[..2] {
            let index = match child {
                BspChild::None => continue,
                BspChild::Branch(i) | BspChild::Leaf(i) => i,
            };
            if overlaps(&level.bsp[index as usize], bounds) {
                stack[*top] = child;
                *top += 1;
            }
        }
    };

    push_children(root, &mut stack, &mut top);
    while top > 0 {
        top -= 1;
        match stack[top] {
            BspChild::None => unreachable!(),
            BspChild::Leaf(i) => callback(state, level, &level.bsp[i as usize]),
            BspChild::Branch(i) => push_children(&level.bsp[i as usize], &mut stack, &mut top),
        }
    }
}
