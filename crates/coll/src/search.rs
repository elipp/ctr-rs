//! Starting a search and testing the quad blocks it finds: `COLL_SearchBSP_CallbackQUADBLK`
//! and the player search's setup.

use loader::lev::{BspKind, BspNode, Level, SVec3, quad_flags};
use math::mips;

use super::*;

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

        let axis = |top: i16, bottom: i16| {
            if top < bottom {
                (top, bottom)
            } else {
                (bottom, top)
            }
        };
        (sps.bbox.min.x, sps.bbox.max.x) = axis(top.x, bottom.x);
        (sps.bbox.min.y, sps.bbox.max.y) = axis(top.y, bottom.y);
        (sps.bbox.min.z, sps.bbox.max.z) = axis(top.z, bottom.z);

        let bounds = sps.bbox;
        search_bsp(
            level,
            &bounds,
            |this, level, leaf| this.test_leaf_quadblocks(level, leaf, false),
            self,
        );
    }

    /// `COLL_SearchBSP_CallbackPARAM` with `COLL_MOVED_BSPLEAF_TestQuadblocks`:
    /// the player's swept test over `sps.bbox`. The caller sets up the
    /// scratchpad first (see [`Collision::sweep_player`]); hits are left in
    /// `sps` (`hit_fraction`, `hit`, `hit_level_triangle`, ...).
    pub fn search_moved(&mut self, level: &Level) {
        let bounds = self.sps.bbox;
        search_bsp(
            level,
            &bounds,
            |this, level, leaf| this.test_leaf_quadblocks(level, leaf, true),
            self,
        );
    }

    /// `COLL_SearchBSP_CallbackPARAM` with `COLL_FIXED_BSPLEAF_TestQuadblocks`
    /// over `sps.bbox`: the point test of `COLL_FIXED_PlayerSearch`. The
    /// caller sets up the scratchpad (segment, bounds, flags) first.
    pub fn search_fixed(&mut self, level: &Level) {
        let bounds = self.sps.bbox;
        search_bsp(
            level,
            &bounds,
            |this, level, leaf| this.test_leaf_quadblocks(level, leaf, false),
            self,
        );
    }

    /// `COLL_FIXED_QUADBLK_TestTriangles` on a single quad block.
    pub fn test_fixed_quadblock(&mut self, level: &Level, quad_index: u32) {
        self.test_quadblock_triangles(level, quad_index, false);
    }

    /// The setup of `COLL_MOVED_PlayerSearch` before its iterations: hit
    /// radius, the surfaces the player collides with, and an empty scrub list.
    /// Always high LOD (see the module comment). `TEST_INSTANCES` is set as in
    /// the original.
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
        sps.input1.model_id = model_id::PLAYER;
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

        sps.scrub_triangles[count] = ScrubTriangle {
            quad: Some(quad),
            triangle_id,
            scrub_depth: 0,
        };
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
        }
        for i in quad_blocks.clone() {
            self.test_quadblock_triangles(level, i as u32, moved);
        }
        if self.sps.quad_block_coll.search_flags & search_flags::TEST_INSTANCES != 0 {
            self.test_leaf_instances(level, leaf);
        }
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

        if flags & search_flags::HIGH_LOD == 0 {
            // Low LOD (the camera): the quad block as two big triangles.
            assert!(!moved, "low-LOD moved collision is not ported");
            self.quadblock_normals_lo_lod(level, quad);
            self.triangle_test_point(level, 0, 1, 2);
            if has_second_triangle {
                self.triangle_test_point(level, 1, 3, 2);
            }
            return;
        }
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
}
