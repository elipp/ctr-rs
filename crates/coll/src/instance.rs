//! Instance hitboxes: `COLL_FIXED_BSPLEAF_TestInstance` and `COLL_FIXED_INSTANC_TestPoint`.
//!
//! Every instance the player can touch (crates, fruit, turtles, ...) has a hitbox in the BSP
//! leaves it overlaps: a sphere (a cylinder without `BSP_HITBOX_USE_Y_AXIS`, which ignores
//! height) in the level units. A moved search tests the swept player sphere against them as it
//! visits the leaves, after the quad blocks, when it has [`search_flags::TEST_INSTANCES`].
//! What touching one does is up to the model (`MetaDataMODEL::LInC`), which is the caller's.
//!
//! The reference's `CollInstanceHitboxScratch` (the intermediate dots and deltas) is not kept:
//! nothing reads it after the test.

use math::Wrap;
use gte::{ctrl, data};
use loader::lev::{BspHitbox, BspKind, BspNode, Level, SVec3};
use math::mips;

use crate::{Collision, FRACTION_ONE, MVMVA_R_V0, clip, pack, search_flags};

/// `BSP_HITBOX_CLASS_TOUCH`: touching it only records the hitbox, with no collision response.
pub const CLASS_TOUCH: u8 = 4;
/// `BSP_HITBOX_CHECK_Y_RANGE`
const CHECK_Y_RANGE: u16 = 0x20;
/// `BSP_HITBOX_USE_Y_AXIS`
const USE_Y_AXIS: u16 = 0x40;
/// `BSP_HITBOX_LINC_USES_INSTDEF`: a hitbox that is not `COLLIDABLE` still has an `LInC` when set.
pub const LINC_USES_INSTDEF: u16 = 0x10;
/// `DRAW_COLLISION_MASK`: the bits of an instance's `flags` that say it can be hit.
pub const DRAW_COLLISION_MASK: u32 = 0xf;

/// The hitbox a search ended on (`sps->bspHitbox`): what the caller needs to tell what it hit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HitboxHit {
    /// Where the hitbox is in the level file: the reference tells them apart by address.
    pub offset: u32,
    /// `BspHitboxClass`
    pub class: u8,
    pub flag: u16,
    /// The instance it belongs to: an index into [`Level::instances`].
    pub instance: Option<usize>,
}

impl HitboxHit {
    fn of(hitbox: &BspHitbox) -> HitboxHit {
        HitboxHit { offset: hitbox.offset, class: hitbox.class(), flag: hitbox.flag, instance: hitbox.instance }
    }
}

impl HitboxHit {
    /// What the reference passes to `COLL_MOVED_FindScrub` for a hitbox: the hitbox itself, cast to
    /// a quad block, only to tell it from the others. It is a key that no quad block index is.
    pub fn scrub_key(&self) -> u32 {
        0x8000_0000 | self.offset
    }
}

/// Do two bounding boxes touch.
fn overlaps(a: &crate::BoundingBox, b: &loader::lev::BoundingBox) -> bool {
    a.min.x <= b.max.x
        && b.min.x <= a.max.x
        && a.min.y <= b.max.y
        && b.min.y <= a.max.y
        && a.min.z <= b.max.z
        && b.min.z <= a.max.z
}

impl Collision {
    /// The `flags` of an instance, which the game changes as it hides it (a crate that was hit).
    pub fn instance_flags(&self, level: &Level, instance: usize) -> u32 {
        self.instance_flags.get(instance).copied().unwrap_or(level.instances[instance].flags)
    }

    /// `CollMoved_PlayerSearch_StoreHitbox`: the hitbox the search ended on is dealt with, and
    /// the rest of the step skips it.
    pub fn store_hitbox(&mut self) {
        let sps = &mut self.sps;
        if let (Some(hit), Some(slot)) = (sps.bsp_hitbox, sps.bsp_hitboxes_hit.get_mut(sps.num_bsp_hitboxes_hit as usize)) {
            *slot = Some(hit.offset);
            sps.num_bsp_hitboxes_hit += 1;
        }
    }

    /// `COLL_FIXED_BSPLEAF_TestInstance`: tests the hitboxes of a leaf the sweep reaches.
    pub(crate) fn test_leaf_instances(&mut self, level: &Level, leaf: &BspNode) {
        let BspKind::Leaf { hitboxes, .. } = &leaf.kind else { return };
        for hitbox in hitboxes {
            let done = self.sps.num_bsp_hitboxes_hit.max(0) as usize;
            if self.sps.bsp_hitboxes_hit[..done].contains(&Some(hitbox.offset)) {
                continue;
            }
            // A hitbox that is not tied to a live instance is always tested; one that is needs
            // the instance to be drawn.
            let tied = hitbox.flag & BspHitbox::COLLIDABLE != 0;
            let live = match hitbox.instance {
                Some(instance) if tied => self.instance_flags(level, instance) & DRAW_COLLISION_MASK != 0,
                _ => true,
            };
            if live && overlaps(&self.sps.bbox, &hitbox.bbox) {
                self.instance_test_point(hitbox);
            }
        }
    }

    /// `COLL_FIXED_INSTANC_TestPoint`: the swept sphere from `quad_block_coll.pos` to
    /// `input1.pos` against one hitbox. A hit that is nearer than the best so far pulls
    /// `hit_fraction` in and leaves the normal, `hit_pos` and push-out of the sphere's surface
    /// in the scratchpad.
    pub(crate) fn instance_test_point(&mut self, hitbox: &BspHitbox) -> u32 {
        let hit = HitboxHit::of(hitbox);
        let gte = &mut self.gte;
        let sps = &mut self.sps;

        if hit.class == CLASS_TOUCH {
            sps.bsp_hitbox = Some(hit);
            sps.did_touch_hitbox = sps.did_touch_hitbox.wrapping_add(1);
        }

        if sps.quad_block_coll.search_flags & search_flags::FORCE_INSTANCE_HIT != 0 {
            sps.hit.plane.normal = SVec3 { x: 0, y: FRACTION_ONE.wrap_i16(), z: 0 };
            sps.hit.reorder_result = clip::FACE.wrap_i8();
            sps.hit_fraction = 0;
            sps.bsp_hitbox = Some(hit);
            sps.quad_block_coll.hit_pos = sps.quad_block_coll.pos;
            sps.did_touch_hitbox = sps.did_touch_hitbox.wrapping_add(1);
            return 6;
        }

        let (from, to, center) = (sps.quad_block_coll.pos, sps.input1.pos, hitbox.center);
        let use_y = hitbox.flag & USE_Y_AXIS != 0;
        let diff_x = to.x as i32 - from.x as i32;
        let diff_z = to.z as i32 - from.z as i32;
        let center_diff_x = center.x as i32 - from.x as i32;
        let center_diff_z = center.z as i32 - from.z as i32;
        let (diff_y, center_diff_y) =
            if use_y { (to.y as i32 - from.y as i32, center.y as i32 - from.y as i32) } else { (0, 0) };

        gte.ctc2(pack(diff_x, diff_y), ctrl::R_MATRIX);
        gte.ctc2(pack(diff_z, center_diff_x), ctrl::R13R21);
        gte.ctc2(pack(center_diff_y, center_diff_z), ctrl::R22R23);
        gte.mtc2(pack(diff_x, diff_y), data::VXY0);
        gte.mtc2(diff_z as u32, data::VZ0);
        gte.cop2(MVMVA_R_V0);
        let dot_segment = gte.mfc2(data::MAC1) as i32;
        let mut dot_center = gte.mfc2(data::MAC2) as i32;

        if dot_center <= 0 {
            return 0;
        }

        gte.mtc2(dot_center as u32, data::LZCS);
        let shift = (gte.mfc2(data::LZCR) as i32 - 2).clamp(0, 12) as u32;
        let divisor = mips::sra(dot_segment, 12 - shift);
        dot_center = mips::sll(dot_center, shift);
        if divisor < 0 {
            return 0;
        }

        // How far along the segment the point nearest the center is.
        let mut factor = if divisor != 0 { mips::div(dot_center, divisor) } else { 0 };

        let proj_x = mips::sra(mips::mul_lo(factor, diff_x), 12);
        let proj_y = if use_y { mips::sra(mips::mul_lo(factor, diff_y), 12) } else { 0 };
        let proj_z = mips::sra(mips::mul_lo(factor, diff_z), 12);

        let rel_x = mips::sub_lo(proj_x, center_diff_x);
        let rel_y = mips::sub_lo(proj_y, center_diff_y);
        let rel_z = mips::sub_lo(proj_z, center_diff_z);
        gte.ctc2(pack(rel_x, rel_y), ctrl::R_MATRIX);
        gte.ctc2(rel_z as u32, ctrl::R13R21);
        gte.mtc2(pack(rel_x, rel_y), data::VXY0);
        gte.mtc2(rel_z as u32, data::VZ0);
        gte.cop2(MVMVA_R_V0);

        let radius = hitbox.radius as i32;
        let reach = sps.input1.hit_radius as i32 + radius;
        let radius_squared = mips::mul_lo(reach, reach);
        let dist_squared = gte.mfc2(data::MAC1) as i32;

        let remaining = mips::sub_lo(radius_squared, dist_squared);
        if remaining < 0 {
            return 0;
        }

        // Back off from the nearest point to where the sphere is first touched.
        if remaining != 0 && dot_segment != 0 {
            factor = mips::sub_lo(factor, mips::div(mips::sll(remaining, 12), dot_segment));
        }

        if sps.hit_fraction < factor {
            return 0;
        }

        let (mut hit_x, mut hit_y, mut hit_z) = (0, 0, 0);
        if factor > 0 {
            hit_x = mips::sra(mips::mul_lo(diff_x, factor), 12);
            hit_y = mips::sra(mips::mul_lo(diff_y, factor), 12);
            hit_z = mips::sra(mips::mul_lo(diff_z, factor), 12);
        }

        if hitbox.flag & CHECK_Y_RANGE != 0 {
            let center_y = center.y as i32;
            if hit_y < center_y && mips::add_lo(center_y, hitbox.id as i32) < hit_y {
                return 0;
            }
        }

        sps.bsp_hitbox = Some(hit);
        sps.hit_fraction = factor;
        sps.did_touch_hitbox = sps.did_touch_hitbox.wrapping_add(1);

        let normal_x = mips::sub_lo(hit_x, center_diff_x);
        let normal_y = if use_y { mips::sub_lo(hit_y, center_diff_y) } else { 0 };
        let normal_z = mips::sub_lo(hit_z, center_diff_z);

        gte.ctc2(pack(normal_x, normal_y), ctrl::R_MATRIX);
        gte.ctc2(normal_z as u32, ctrl::R13R21);
        gte.mtc2(pack(normal_x, normal_y), data::VXY0);
        gte.mtc2(normal_z as u32, data::VZ0);
        gte.cop2(MVMVA_R_V0);

        // The reference traps on a zero length (a hit dead on the center, never seen).
        let length = math::square_root0(gte.mfc2(data::MAC1) as i32);
        let inv_length = if length != 0 { mips::div(0x100_0000, length) } else { 0 };
        let normal = [normal_x, normal_y, normal_z].map(|n| mips::sra(mips::mul_lo(n, inv_length), 12));

        let at = |origin: i16, delta: i32| mips::add_lo(origin.zext16(), delta).wrap_i16();
        sps.quad_block_coll.hit_pos = SVec3 {
            x: at(sps.quad_block_coll.pos.x, hit_x),
            y: at(sps.quad_block_coll.pos.y, hit_y),
            z: at(sps.quad_block_coll.pos.z, hit_z),
        };
        sps.hit.plane.normal = SVec3 { x: normal[0].wrap_i16(), y: normal[1].wrap_i16(), z: normal[2].wrap_i16() };
        sps.hit.reorder_result = clip::FACE.wrap_i8();

        let scaled = normal.map(|n| mips::sra(mips::mul_lo(n, radius), 12));
        sps.hit.push_out =
            SVec3 { x: at(center.x, scaled[0]), y: at(center.y, scaled[1]), z: at(center.z, scaled[2]) };
        sps.hit.hit_pos = sps.hit.push_out;
        0
    }
}
