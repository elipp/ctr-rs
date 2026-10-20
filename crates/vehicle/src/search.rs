//! `COLL_MOVED_PlayerSearch`: moves the driver along its velocity against the
//! track, scraping along whatever it hits.

use math::Wrap;
use coll::instance::{CLASS_TOUCH, DRAW_COLLISION_MASK, LINC_USES_INSTDEF};
use coll::{Collision, FRACTION_ONE, Scratchpad, search_flags};
use loader::lev::BspHitbox;
use loader::lev::{Level, SVec3, quad_flags};
use math::mips;

use crate::scrub::{self, Impact, LightMatrix, scrub_id};
use crate::{Driver, Effect, collision_flags};

/// `MetaDataMODEL::LInC` of the level's models: what a hitbox the player touches does.
pub trait Instances {
    /// `LInC` of the model of `instance` (an index into `Level::instances`), when the player
    /// touches its hitbox. `sps` has the hit. The result is the game's: 2 means it was dealt
    /// with and the driver goes on as if nothing was there; 0 or 1 that the driver collides with
    /// it like a surface (1 is also what a model without an `LInC` gives).
    fn touch(&mut self, instance: usize, driver: &mut Driver, sps: &Scratchpad) -> i32;
}

/// No model has an `LInC`: every hitbox a driver reaches is a surface.
pub struct NoInstances;

impl Instances for NoInstances {
    fn touch(&mut self, _: usize, _: &mut Driver, _: &Scratchpad) -> i32 {
        1
    }
}

/// `CollMoved_PlayerSearch_RunHitboxLInC`, for the hitbox the search ended on.
fn run_hitbox_linc(d: &mut Driver, coll: &Collision, level: &Level, instances: &mut dyn Instances) -> i32 {
    let Some(hit) = coll.sps.bsp_hitbox else { return 1 };
    let Some(instance) = hit.instance else { return 1 };
    if hit.flag & BspHitbox::COLLIDABLE != 0 {
        // Only an instance that is drawn can be hit.
        if coll.instance_flags(level, instance) & DRAW_COLLISION_MASK == 0 {
            return 1;
        }
    } else if hit.flag & LINC_USES_INSTDEF == 0 {
        return 1;
    }
    instances.touch(instance, d, &coll.sps)
}

/// `COLL_HITBOX_SCRUB_DEPTH_BONUS`
const HITBOX_SCRUB_DEPTH_BONUS: i32 = 0x200;
/// The most sweeps a single step makes before giving up.
const MAX_SWEEPS: u32 = 15;
/// Stop once less than this much of the step's velocity is left (4.12).
const MIN_MULTIPLIER: i32 = 100;
/// A quad block flag (unnamed in the reference) that releases `under_driver`.
const QUADBLOCK_FLAG_UNDER_DRIVER_RELEASE: u16 = 0x8;

/// `CollMoved_PlayerSearch_StepVelocity`: velocity over `elapsed_ms`, scaled
/// by the 4.12 `multiplier` (what is left of the step).
fn step_velocity(velocity: i32, elapsed_ms: i32, multiplier: i32) -> i32 {
    mips::sra(mips::mul_lo(mips::sra(mips::mul_lo(velocity, elapsed_ms), 5), multiplier), 12)
}

/// The sphere's centre in the scratchpad's units: `pos_curr` is 24.8, the
/// scratchpad 1/256 of that, offset by `origin_to_center`.
pub fn sphere_center(d: &Driver, pos: math::Vec3) -> SVec3 {
    let axis = |origin_to_center: i32, pos: i32| mips::add_lo(origin_to_center.zext16(), mips::sra(pos, 8)).wrap_i16();
    SVec3 {
        x: axis(d.origin_to_center.x, pos.x),
        y: axis(d.origin_to_center.y, pos.y),
        z: axis(d.origin_to_center.z, pos.z),
    }
}

/// `COLL_MOVED_PlayerSearch`. `elapsed_ms` is `gGT->elapsedTimeMS`; `coll`
/// carries the scratchpad and the GTE. `light_matrix` is what impacts are
/// transformed by, normally [`scrub::IDENTITY_LIGHT_MATRIX`]. `instances` is what touching the
/// hitboxes of the level's instances does.
pub fn moved_player_search(
    d: &mut Driver,
    coll: &mut Collision,
    level: &Level,
    light_matrix: &LightMatrix,
    elapsed_ms: i32,
    instances: &mut dyn Instances,
    effects: &mut Vec<Effect>,
) {
    let mut multiplier = FRACTION_ONE;

    coll.begin_player_search();

    for _ in 0..MAX_SWEEPS {
        let velocity = math::Vec3 {
            x: step_velocity(d.velocity.x, elapsed_ms, multiplier),
            y: step_velocity(d.velocity.y, elapsed_ms, multiplier),
            z: step_velocity(d.velocity.z, elapsed_ms, multiplier),
        };

        coll.sps.quad_block_coll.search_flags |= search_flags::TEST_INSTANCES;

        let current = sphere_center(d, d.pos_curr);
        let next = sphere_center(
            d,
            math::Vec3 {
                x: mips::add_lo(d.pos_curr.x, velocity.x),
                y: mips::add_lo(d.pos_curr.y, velocity.y),
                z: mips::add_lo(d.pos_curr.z, velocity.z),
            },
        );

        if !coll.prepare_sweep(current, next) {
            break;
        }
        coll.sps.quad_block_coll.search_flags &= !search_flags::REUSE_NORMALS;
        coll.search_moved(level);

        let sps = &coll.sps;
        if sps.did_touch_quadblock != 0 {
            d.collision_flags |= collision_flags::TOUCHED_QUADBLOCK;
        }

        let hit_fraction = sps.hit_fraction;
        if hit_fraction > 0 {
            d.pos_curr.x = mips::add_lo(d.pos_curr.x, mips::sra(mips::mul_lo(velocity.x, hit_fraction), 12));
            d.pos_curr.y = mips::add_lo(d.pos_curr.y, mips::sra(mips::mul_lo(velocity.y, hit_fraction), 12));
            d.pos_curr.z = mips::add_lo(d.pos_curr.z, mips::sra(mips::mul_lo(velocity.z, hit_fraction), 12));
        }

        if sps.did_touch_hitbox != 0 {
            let Some(hit) = sps.bsp_hitbox else { break };
            coll.sps.quad_block_coll.search_flags &= !search_flags::REUSE_NORMALS;
            d.collision_flags &= !collision_flags::SURFACE_PUSHBACK;

            let result = run_hitbox_linc(d, coll, level, instances);
            if result == 2 || hit.class == CLASS_TOUCH {
                coll.store_hitbox();
            } else {
                coll.find_scrub(Some(hit.scrub_key()), 0);
                let depth = coll.sps.input1.scrub_depth.zext16();
                coll.sps.input1.scrub_depth = mips::add_lo(depth, HITBOX_SCRUB_DEPTH_BONUS).wrap_i16();

                let impact = scrub::scrub_impact(d, coll, level, light_matrix, scrub::surface(hit.class as usize), effects);
                if impact == Impact::None {
                    coll.store_hitbox();
                }
                if impact == Impact::Slammed {
                    return;
                }
            }
            continue;
        }
        let Some(quad_index) = sps.hit.quad.filter(|_| sps.did_touch_quadblock != 0) else {
            break;
        };
        let quad_flags = level.quad_blocks[quad_index as usize].flags;
        let triangle_id = sps.hit.triangle_id as i32;
        let normal = sps.hit.plane.normal;
        let hit_pos = sps.hit.hit_pos;

        if quad_flags & quad_flags::KILL_PLANE != 0 {
            d.collision_flags |= collision_flags::MASK_GRAB_REQUEST;
        }

        coll.find_scrub(Some(quad_index), triangle_id);

        let id = if quad_flags & quad_flags::GROUND == 0 {
            if quad_flags & quad_flags::REFLECT_SPLIT_LINE_1 == 0 { scrub_id::SOLID } else { scrub_id::NO_SCRUB }
        } else {
            if d.under_driver != Some(quad_index) && quad_flags & QUADBLOCK_FLAG_UNDER_DRIVER_RELEASE != 0 {
                d.under_driver = None;
            }
            d.curr_block_touching = Some(quad_index);
            d.normal_vec_up = normal;
            d.axis_angle1_normal_vec = normal;
            d.collision_flags |= collision_flags::GROUNDED;
            scrub_id::ROAD
        };

        d.collision_flags |= collision_flags::SURFACE_PUSHBACK;
        d.sps_hit_pos = hit_pos;
        d.sps_normal_vec = normal;

        if scrub::scrub_impact(d, coll, level, light_matrix, scrub::surface(id), effects) == Impact::Slammed {
            return;
        }

        let hit_fraction = coll.sps.hit_fraction;
        if hit_fraction > 0 {
            multiplier = mips::sub_lo(multiplier, mips::sra(mips::mul_lo(multiplier, hit_fraction), 12));
            if multiplier < MIN_MULTIPLIER {
                break;
            }
        }

        coll.sps.quad_block_coll.search_flags |= search_flags::REUSE_NORMALS;
    }

    d.step_flag_set = coll.sps.collision.step_flags;
}
