//! The triangle tests: normals, the point test against one triangle, and the moved search's
//! clip classification.

use gte::{ctrl, data};
use loader::lev::{Level, QuadBlock, quad_flags};
use math::mips;

use super::*;

impl Collision {
    /// `COLL_FIXED_QUADBLK_LoadScratchpadVerts`
    pub(super) fn load_scratchpad_verts(&mut self, level: &Level, quad: &QuadBlock) {
        self.sps.quad_second_tri_index_a = quad.vertex_index[2];
        self.sps.quad_second_tri_index_b = quad.vertex_index[3];
        for (bsv, &index) in self.sps.search_vert.iter_mut().zip(&quad.vertex_index) {
            let vertex = &level.vertices[index as usize];
            bsv.level_vertex = Some(index as u32);
            bsv.pos = vertex.pos;
            bsv.normal_axis = vertex.flags as i16;
        }
    }

    /// `COLL_FIXED_QUADBLK_GetNormVecs_LoLOD`: planes for the two big triangles.
    pub(super) fn quadblock_normals_lo_lod(&mut self, level: &Level, quad: &QuadBlock) {
        self.load_scratchpad_verts(level, quad);

        // Always 2 for low poly (big block).
        self.sps.collision.tri_normal_lod_shift = 2;
        self.sps.collision.tri_normal_vec_bit_shift = quad.tri_normal_vec_bit_shift as u8;

        let has_second_triangle =
            self.sps.quad_second_tri_index_a != self.sps.quad_second_tri_index_b;
        if has_second_triangle {
            self.triangle_norm_vec(quad.tri_normal_vec_dividend[dividend::LO_1], 1, 3, 2);
        }
        self.triangle_norm_vec(quad.tri_normal_vec_dividend[dividend::LO_0], 0, 1, 2);
    }

    /// `COLL_FIXED_QUADBLK_GetNormVecs_HiLOD`: planes for the eight small
    /// triangles of the 3x3 vertex patch.
    pub(super) fn quadblock_normals_hi_lod(&mut self, level: &Level, quad: &QuadBlock) {
        self.load_scratchpad_verts(level, quad);

        self.sps.collision.tri_normal_lod_shift = 0;
        self.sps.collision.tri_normal_vec_bit_shift = quad.tri_normal_vec_bit_shift as u8;

        let has_second_triangle =
            self.sps.quad_second_tri_index_a != self.sps.quad_second_tri_index_b;
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
        let (p1, p2, p3) = (
            sps.search_vert[v1].pos,
            sps.search_vert[v2].pos,
            sps.search_vert[v3].pos,
        );

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
    pub(super) fn triangle_test_point_body(
        &mut self,
        level: &Level,
        v1: usize,
        v2: usize,
        v3: usize,
        normal_zw: i32,
    ) {
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

        let (p1, p2, p3) = (
            sps.search_vert[v1].pos,
            sps.search_vert[v2].pos,
            sps.search_vert[v3].pos,
        );
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
        let (second_a_pos, second_b_pos) =
            (sps.search_vert[bary_v1].pos, sps.search_vert[bary_v2].pos);
        let origin = c1[second];
        let second_a = coords(second_a_pos)[second] - origin;
        let second_b = coords(second_b_pos)[second] - origin;
        let second_hit = hit[second] - origin;

        let mut bary_a = FRACTION_INVALID;
        let mut bary_b = FRACTION_INVALID;

        if first_a != 0 {
            let denom = mips::sra(
                mips::sub_lo(
                    mips::mul_lo(second_b, first_a),
                    mips::mul_lo(first_b, second_a),
                ),
                6,
            );
            if denom != 0 {
                bary_b = mips::div(
                    mips::sll(
                        mips::sub_lo(
                            mips::mul_lo(second_hit, first_a),
                            mips::mul_lo(first_hit, second_a),
                        ),
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
    pub(super) fn select_projection(
        &self,
        v1: usize,
        v2: &mut usize,
        v3: &mut usize,
    ) -> Projection {
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
    pub(super) fn reorder_normals(&mut self, v1: usize, v2: usize, v3: usize) -> i32 {
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
                    mips::sub_lo(
                        mips::sll(p.second_hit, 12),
                        mips::mul_lo(bary_b, p.second_b),
                    ),
                    p.second_a,
                );
            }
        } else {
            let denom = mips::sra(
                mips::sub_lo(
                    mips::mul_lo(p.second_b, p.first_a),
                    mips::mul_lo(p.first_b, p.second_a),
                ),
                6,
            );
            if denom != 0 {
                bary_b = mips::div(
                    mips::sll(
                        mips::sub_lo(
                            mips::mul_lo(p.second_hit, p.first_a),
                            mips::mul_lo(p.first_hit, p.second_a),
                        ),
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
            sps.candidate.hit_pos =
                triangle_barycentrics(&mut self.gte, vertex(v1), vertex(bary_v3), push_out);
            return clip::EDGE_V1_V3;
        }

        if bary_b >= 0 {
            if sum <= 0 {
                sps.candidate.hit_pos = push_out;
                return clip::FACE;
            }
            sps.candidate.hit_pos =
                triangle_barycentrics(&mut self.gte, vertex(bary_v2), vertex(bary_v3), push_out);
            return clip::EDGE_V2_V3;
        }

        if sum >= 0 {
            sps.candidate.hit_pos = vertex(bary_v2);
            return clip::V2;
        }
        sps.candidate.hit_pos =
            triangle_barycentrics(&mut self.gte, vertex(v1), vertex(bary_v2), push_out);
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

        if quad.flags & quad_flags::DOOR != 0
            && (quad.terrain_type as i8 as i32 & self.door_access_flags as i32) != 0
        {
            return;
        }

        let (pos, start) = (sps.input1.pos, sps.quad_block_coll.pos);
        gte.ctc2(pack(pos.x as i32, pos.y as i32), ctrl::R_MATRIX);
        gte.ctc2(pack(pos.z as i32, start.x as i32), ctrl::R13R21);
        gte.ctc2(pack(start.y as i32, start.z as i32), ctrl::R22R23);
        gte.mtc2(
            pack(plane.normal.x as i32, plane.normal.y as i32),
            data::VXY0,
        );
        gte.mtc2(normal_zw as u32, data::VZ0);
        gte.cop2(RTV0_SF1);

        let plane_offset = mips::sll(plane.half_distance as i32, 1);
        let mut plane_near = mips::sub_lo(gte.mfc2(data::MAC1) as i32, plane_offset);
        let mut plane_far = mips::sub_lo(gte.mfc2(data::MAC2) as i32, plane_offset);

        // A back-facing plane is flipped for triggers and double-sided quads.
        if plane_far < 0
            && (quad.flags & quad_flags::TRIGGER != 0
                || quad.draw_order_low & DRAW_ORDER_LOW_DOUBLE_SIDED != 0)
        {
            plane_near = mips::neg_lo(plane_near);
            plane_far = mips::neg_lo(plane_far);
            let n = &mut sps.candidate.plane.normal;
            n.x = mips::neg_lo(n.x as i32) as i16;
            n.y = mips::neg_lo(n.y as i32) as i16;
            n.z = mips::neg_lo(n.z as i32) as i16;
            sps.candidate.plane.half_distance =
                mips::neg_lo(sps.candidate.plane.half_distance as i32) as i16;
        }

        sps.num_triangles_tested = sps.num_triangles_tested.wrapping_add(1);
        let radius = sps.input1.hit_radius as i32;

        if mips::sub_lo(plane_near, radius) >= 0 || plane_far < 0 {
            return;
        }
        if quad.flags & quad_flags::TRIGGER == 0 && mips::sub_lo(plane_near, plane_far) > 0 {
            return;
        }

        let used_segment_projection = if plane_near >= 0 {
            gte.mtc2(plane_near as u32, data::IR0);
            let n = sps.candidate.plane.normal;
            load_ir(gte, n.x as i32, n.y as i32, n.z as i32);
            false
        } else {
            load_ir(
                gte,
                mips::sub_lo(pos.x as i32, start.x as i32),
                mips::sub_lo(pos.y as i32, start.y as i32),
                mips::sub_lo(pos.z as i32, start.z as i32),
            );
            let fraction = mips::div(
                mips::sll(mips::neg_lo(plane_near), 12),
                mips::sub_lo(plane_far, plane_near),
            );
            gte.mtc2(fraction as u32, data::IR0);
            true
        };
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
        let (from, hit_pos) = if used_segment_projection {
            (sps.candidate.push_out, sps.candidate.hit_pos)
        } else {
            (pos, sps.candidate.hit_pos)
        };
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
            && (plane_near < 0
                || (mips::sub_lo(plane_near, radius) | mips::sub_lo(plane_far, radius)) < 0)
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
pub(crate) struct Projection {
    first_a: i32,
    first_b: i32,
    first_hit: i32,
    second_a: i32,
    second_b: i32,
    second_hit: i32,
}
