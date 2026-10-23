//! `COLL_FIXED_PlayerSearch`: finds the track under the kart. Probes straight
//! down from the kart's position and decides from the quad block it hits
//! whether the kart is on the ground, and which surface it is on; blends the
//! surface normals the model is drawn on, and keeps the "mask grab" (the
//! respawn when falling off the track) request.
//!
//! The scratchpad it works on is the one `COLL_MOVED_PlayerSearch` just used:
//! the hit radius and the hit fraction of that search's last sweep are left in
//! it, and this search relies on them (`Collision` is part of the simulation
//! state for that reason).
//!
//! Not ported: the instance's lighting (`UpdateLighting`: `alphaScale`), the
//! reflection split-lines, and the compressed normal stored in the instance.

use coll::{Collision, search_flags};
use gte::data;
use loader::lev::{Level, SVec3, quad_flags};
use math::mips;

use crate::calc::{fast_sqrt, map_to_range};
use crate::terrain::{TERRAINS, terrain_flags, terrain_type};
use crate::{Driver, Effect, Env, actions, collision_flags, kart_state, rain_cloud, step_triggers};

const TOP_Y_OFFSET: i32 = 0x80;
const BOTTOM_Y_OFFSET: i32 = 0x100;
const AXIS3_OLD_NORMAL_WEIGHT: i32 = 5;
const AXIS3_HIT_NORMAL_WEIGHT: i32 = 3;
const NORMAL_SQRT_SHIFT: u32 = 0x18;
const NORMAL_BLEND_TOTAL_WEIGHT: i32 = 8;
const NORMAL_BLEND_SHIFT: u32 = 3;
const NORMAL_BLEND_GROUND_WEIGHT: i32 = 6;
const NORMAL_BLEND_AIRBORNE_WEIGHT: i32 = 7;
const COYOTE_TIME_MS: i16 = 0xa0;
const HAZARD_WOBBLE_ANGLE_STEP: i32 = 0xc;
const HAZARD_WOBBLE_AMPLITUDE: i32 = 0x19;
const HAZARD_WOBBLE_SHIFT: u32 = 10;
const LANDING_SFX: u8 = 7;
const HAZARD_LAND_SFX: u8 = 0x10;

fn abs(value: i32) -> i32 {
    if value < 0 {
        mips::neg_lo(value)
    } else {
        value
    }
}

/// `COLL_FIXED_PlayerSearch_SetupSearch`: a vertical segment through the
/// kart's position for the point test, and the surfaces it collides with.
fn setup_search(d: &mut Driver, coll: &mut Collision) {
    let pos = [
        mips::sra(d.pos_curr.x, 8),
        mips::sra(d.pos_curr.y, 8),
        mips::sra(d.pos_curr.z, 8),
    ];
    let bottom = SVec3 {
        x: pos[0] as i16,
        y: mips::add_lo(pos[1], -BOTTOM_Y_OFFSET) as i16,
        z: pos[2] as i16,
    };
    let top = SVec3 {
        x: pos[0] as i16,
        y: mips::add_lo(pos[1], TOP_Y_OFFSET) as i16,
        z: pos[2] as i16,
    };

    d.actions_flag_set &= !actions::ENGINE_ECHO;

    let sps = &mut coll.sps;
    sps.quad_block_coll.pos = top;
    sps.input1.pos = bottom;
    sps.quad_block_coll.quad_flags_ignored = quad_flags::NO_COLLISION_RESPONSE as u32;
    sps.quad_block_coll.quad_flags_wanted =
        (quad_flags::GROUND | quad_flags::COLLISION_SURFACE) as u32;
    sps.quad_block_coll.search_flags = search_flags::HIGH_LOD;
    sps.did_touch_quadblock = 0;
    sps.did_touch_hitbox = 0;
    sps.num_triangles_tested = 0;

    sps.bbox.min.x = top.x;
    sps.bbox.max.x = top.x;
    sps.bbox.min.y = bottom.y.min(top.y);
    sps.bbox.max.y = bottom.y.max(top.y);
    sps.bbox.min.z = top.z;
    sps.bbox.max.z = top.z;
    sps.quad_block_coll.hit_pos = bottom;
}

fn normalize(x: i32, y: i32, z: i32) -> SVec3 {
    let sum = mips::add_lo(
        mips::add_lo(mips::mul_lo(x, x), mips::mul_lo(y, y)),
        mips::mul_lo(z, z),
    ) as u32;
    let len = (fast_sqrt(sum, NORMAL_SQRT_SHIFT) >> 12) as i32;
    SVec3 {
        x: mips::div(mips::sll(x, 12), len) as i16,
        y: mips::div(mips::sll(y, 12), len) as i16,
        z: mips::div(mips::sll(z, 12), len) as i16,
    }
}

/// `COLL_FIXED_PlayerSearch_CheckMaskGrabProgress`: whether the quad block
/// the kart is on means it should be put back on the track (a kill plane, or
/// a jump ahead along the track that skips too much of it).
fn check_mask_grab_progress(d: &mut Driver, level: &Level, quad: u32) -> bool {
    let q = &level.quad_blocks[quad as usize];
    if q.flags & quad_flags::KILL_PLANE != 0 {
        return true;
    }
    if d.kart_state == kart_state::MASK_GRABBED
        || d.collision_flags & collision_flags::MASK_GRAB_REQUEST != 0
        || q.flags & quad_flags::GROUND == 0
    {
        return false;
    }
    // No checkpoint: only adventure levels track the last valid quad here.
    if q.checkpoint_index == 0xff {
        return false;
    }

    let points = &level.restart_points;
    let node = &points[q.checkpoint_index as usize];
    let track_length = points[0].dist_to_finish as i32;

    if d.actions_flag_set & actions::BEHIND_START_LINE == 0
        && node.next_forward > 1
        && (mips::sll(track_length >> 2, 3))
            < d.distance_to_finish_checkpoint
                .wrapping_sub(mips::mul_lo(node.dist_to_finish as i32, 8) as u32)
                as i32
    {
        return true;
    }

    let Some(last_valid) = d.last_valid else {
        d.last_valid = d.curr_block_touching;
        return false;
    };
    let last_checkpoint = level.quad_blocks[last_valid as usize].checkpoint_index;
    if (node.dist_to_finish as i32) < (mips::mul_lo(track_length, 0xf) >> 4)
        && last_checkpoint != 0xff
        && mips::add_lo(
            points[last_checkpoint as usize].dist_to_finish as i32,
            track_length >> 2,
        ) < node.dist_to_finish as i32
    {
        return true;
    }

    d.last_valid = d.curr_block_touching;
    false
}

/// `COLL_FIXED_PlayerSearch`. Returns whether the kart should be grabbed by
/// the mask and put back on the track (`VehStuckProc_MaskGrab_Init`, not
/// ported).
pub fn fixed_player_search(
    d: &mut Driver,
    coll: &mut Collision,
    level: &Level,
    env: &Env,
    effects: &mut Vec<Effect>,
) -> bool {
    setup_search(d, coll);

    if let Some(under) = d.under_driver {
        coll.test_fixed_quadblock(level, under);
    }
    if coll.sps.did_touch_quadblock == 0 {
        coll.search_fixed(level);
    }

    if coll.sps.did_touch_quadblock == 0 {
        d.quad_block_height = mips::sub_lo(d.pos_curr.y, mips::sll(BOTTOM_Y_OFFSET, 8));
    } else {
        let sps = &coll.sps;
        let quad = sps.hit.quad.expect("a hit has a quad block");
        let q = &level.quad_blocks[quad as usize];
        d.quad_block_height = mips::sll(sps.quad_block_coll.hit_pos.y as i32, 8);
        d.collision_flags |= collision_flags::TOUCHED_QUADBLOCK;

        // The normal of the surface, smoothed over a few ticks.
        let n = sps.hit.plane.normal;
        d.axis_angle3_normal_vec = normalize(
            mips::add_lo(
                mips::mul_lo(d.axis_angle3_normal_vec.x as i32, AXIS3_OLD_NORMAL_WEIGHT),
                mips::mul_lo(n.x as i32, AXIS3_HIT_NORMAL_WEIGHT),
            ),
            mips::add_lo(
                mips::mul_lo(d.axis_angle3_normal_vec.y as i32, AXIS3_OLD_NORMAL_WEIGHT),
                mips::mul_lo(n.y as i32, AXIS3_HIT_NORMAL_WEIGHT),
            ),
            mips::add_lo(
                mips::mul_lo(d.axis_angle3_normal_vec.z as i32, AXIS3_OLD_NORMAL_WEIGHT),
                mips::mul_lo(n.z as i32, AXIS3_HIT_NORMAL_WEIGHT),
            ),
        );

        if q.flags & quad_flags::ENGINE_ECHO != 0 {
            d.actions_flag_set |= actions::ENGINE_ECHO;
        }

        d.under_driver = Some(quad);

        if d.pos_curr.y <= d.quad_block_height + 0x1000
            || (q.terrain_type == terrain_type::MUD && d.pos_curr.y < 1)
        {
            if q.flags & quad_flags::GROUND != 0 {
                d.normal_vec_up = n;
                d.collision_flags |= collision_flags::GROUNDED;
            }
            if d.curr_block_touching.is_none() {
                d.curr_block_touching = Some(quad);
                d.axis_angle1_normal_vec = n;
            }
        }
    }

    if d.quad_block_height + 0x8000 < d.pos_curr.y {
        d.terrain_meta2 = terrain_type::NONE;
    }

    // Below the lowest point of the track: fell off.
    let lowest = level.bsp.first().map_or(0, |root| root.bbox.min.y as i32);
    if d.pos_curr.y < mips::sll(mips::sub_lo(lowest, 0x40), 8) {
        d.collision_flags |= collision_flags::MASK_GRAB_REQUEST;
    }

    let landing_delta = mips::sub_lo(d.velocity.y, d.y_speed);

    // In the air but touching a wall-like surface: pushed off it.
    if d.curr_block_touching.is_some()
        && d.collision_flags & (collision_flags::MASK_GRAB_REQUEST | collision_flags::GROUNDED) == 0
        && d.kart_state != kart_state::MASK_GRABBED
    {
        d.velocity.x = mips::add_lo(
            d.velocity.x,
            mips::sra(d.axis_angle1_normal_vec.x as i32, 1),
        );
        d.velocity.y = mips::add_lo(
            d.velocity.y,
            mips::sra(d.axis_angle1_normal_vec.y as i32, 1),
        );
        d.velocity.z = mips::add_lo(
            d.velocity.z,
            mips::sra(d.axis_angle1_normal_vec.z as i32, 1),
        );
    }

    d.x_speed = d.velocity.x;
    d.y_speed = d.velocity.y;
    d.z_speed = d.velocity.z;

    let mut grounded = false;
    if let Some(quad) = d.curr_block_touching {
        if check_mask_grab_progress(d, level, quad) {
            d.collision_flags |= collision_flags::MASK_GRAB_REQUEST;
        }
        d.jump_landing_boost = 0;
        d.actions_flag_set &= !(actions::AIRBORNE | actions::HIGH_JUMP);
        grounded = d.collision_flags & collision_flags::GROUNDED != 0;
    }

    let normal_blend_weight;
    if grounded {
        let quad = d.curr_block_touching.expect("grounded on a quad block");
        if d.rain_cloud_effect == rain_cloud::ICE_TERRAIN {
            d.current_terrain = terrain_type::ICE;
        } else {
            let terrain = level.quad_blocks[quad as usize].terrain_type;
            if terrain != terrain_type::ICE && d.current_terrain == terrain_type::ICE {
                d.terrain_friction_timer = 0xfec0u16 as i16;
            }
            d.current_terrain = terrain;
        }
        // Out-of-range terrain types are the first entry (`VehAfterColl_GetTerrain`).
        let terrain = if (d.current_terrain as usize) < TERRAINS.len() {
            d.current_terrain
        } else {
            0
        };
        d.terrain_meta1 = terrain;
        d.terrain_meta2 = terrain;
        d.jump_coyote_timer_ms = COYOTE_TIME_MS;

        let before = d.actions_flag_set;
        d.actions_flag_set = before | actions::TOUCH_GROUND;
        if d.actions_flag_set_prev_frame & actions::TOUCH_GROUND == 0 {
            d.actions_flag_set = before
                | actions::TOUCH_GROUND
                | actions::STARTED_TOUCH_GROUND
                | actions::TURBO_INPUT_LATCH;
            d.terrain_friction_timer = 0x140;

            // The landing sound's volume (`0x78..=0xfa`) is not reported.
            let _volume = map_to_range(abs(landing_delta), 0x100, 0x3c00, 0x78, 0xfa);
            if d.kart_state != kart_state::MASK_GRABBED {
                effects.push(Effect::Sound {
                    id: LANDING_SFX,
                    echo: d.actions_flag_set & actions::ENGINE_ECHO != 0,
                });
            }
        }
        normal_blend_weight = NORMAL_BLEND_GROUND_WEIGHT;
    } else {
        if d.jump_cooldown_ms != 0 {
            d.actions_flag_set |= actions::AIRBORNE;
        }
        if d.jump_high_jump_timer_ms != 0 {
            d.actions_flag_set |= actions::HIGH_JUMP;
        }
        d.terrain_meta1 = terrain_type::NONE;
        d.current_terrain = terrain_type::NONE;
        d.actions_flag_set &= !actions::TOUCH_GROUND;

        d.jump_landing_boost =
            (d.jump_landing_boost as u16).wrapping_add(env.elapsed_time_ms as u16) as i16;
        d.jump_coyote_timer_ms =
            (d.jump_coyote_timer_ms as u16).wrapping_sub(env.elapsed_time_ms as u16) as i16;
        if d.jump_coyote_timer_ms < 0 {
            d.jump_coyote_timer_ms = 0;
        }
        normal_blend_weight = NORMAL_BLEND_AIRBORNE_WEIGHT;
        if d.jump_coyote_timer_ms == 0 {
            d.jump_cooldown_ms = 0;
            d.jump_high_jump_timer_ms = 0;
        }
    }

    // The normal the model is drawn on: a blend of the last one and the surface's.
    let inverse_weight = NORMAL_BLEND_TOTAL_WEIGHT - normal_blend_weight;
    let blend = |old: i16, new: i16| {
        mips::sra(
            mips::add_lo(
                mips::mul_lo(normal_blend_weight, old as i32),
                mips::mul_lo(inverse_weight, new as i32),
            ),
            NORMAL_BLEND_SHIFT,
        )
    };
    let mut normal_x = blend(d.axis_angle2_normal_vec.x, d.normal_vec_up.x);
    let mut normal_y = blend(d.axis_angle2_normal_vec.y, d.normal_vec_up.y);
    let mut normal_z = blend(d.axis_angle2_normal_vec.z, d.normal_vec_up.z);

    // After being hit, the car wobbles.
    if d.hazard_timer > 0 {
        let (sin, cos) =
            math::sin_cos(mips::mul_lo(d.hazard_timer as i32, HAZARD_WOBBLE_ANGLE_STEP) as u32);
        let x = mips::sra(
            mips::mul_lo(sin, HAZARD_WOBBLE_AMPLITUDE),
            HAZARD_WOBBLE_SHIFT,
        ) as i16;
        let z = mips::sra(
            mips::mul_lo(cos, HAZARD_WOBBLE_AMPLITUDE),
            HAZARD_WOBBLE_SHIFT,
        ) as i16;
        let gte = &mut coll.gte;
        gte.mtc2(mips::pack_s16_pair(x as i32, 0), data::VXY0);
        gte.mtc2(mips::pack_s16_pair(z as i32, 0), data::VZ0);
        gte.cop2(0x0486012);
        normal_x = mips::add_lo(normal_x, gte.mfc2(data::MAC1) as i32);
        normal_y = mips::add_lo(normal_y, gte.mfc2(data::MAC2) as i32);
        normal_z = mips::add_lo(normal_z, gte.mfc2(data::MAC3) as i32);
    }
    d.axis_angle2_normal_vec = normalize(normal_x, normal_y, normal_z);

    // The tilt of the model sideways.
    {
        let (sin, cos) = math::sin_cos(d.angle as i32 as u32);
        let n = d.axis_angle2_normal_vec;
        let numerator = mips::add_lo(
            mips::mul_lo(mips::neg_lo(n.x as i32), cos),
            mips::mul_lo(n.z as i32, sin),
        );
        d.rot_curr_z = math::ratan2(mips::sra(numerator, 12), n.y as i32) as i16;
    }

    // The kart bounces slightly on bumpy surfaces (`screen_offset_y`).
    let terrain = &TERRAINS[d.terrain_meta1 as usize];
    let mut raise_offset = false;
    if d.hazard_timer < 1 {
        if d.actions_flag_set & actions::TOUCH_GROUND != 0
            && abs(d.speed as i32) > 0x1000
            && abs(d.screen_offset_y as i32) < 4
            && terrain.flags & terrain_flags::RAISE_GROUND_OFFSET != 0
        {
            d.distance_from_ground = 4;
            raise_offset = true;
        }
        if !raise_offset {
            d.distance_from_ground = 0;
        }
    } else if abs(d.screen_offset_y as i32) < 4 {
        d.distance_from_ground = 4;
        if d.kart_state != kart_state::SPINNING && d.screen_offset_y > 0 {
            effects.push(Effect::Sound {
                id: HAZARD_LAND_SFX,
                echo: false,
            });
        }
    }

    if abs(d.screen_offset_y as i32) > 9 {
        d.distance_from_ground = 0;
    }

    if d.distance_from_ground == 0 {
        let next = (d.screen_offset_y as u8).wrapping_sub(4) as i8;
        if d.screen_offset_y > 0 {
            d.screen_offset_y = next;
            if next < 1 {
                d.screen_offset_y = 0;
                if terrain.flags & terrain_flags::ONESHOT_GROUND_SOUND != 0 {
                    effects.push(Effect::Sound {
                        id: terrain.sound as u8,
                        echo: d.actions_flag_set & actions::ENGINE_ECHO != 0,
                    });
                }
            }
        }
        let next = (d.screen_offset_y as u8).wrapping_sub(4) as i8;
        d.screen_offset_y = next;
        if next < 0 {
            d.screen_offset_y = 0;
        }
    } else {
        d.distance_from_ground -= 1;
        d.screen_offset_y = (d.screen_offset_y as u8).wrapping_add(3) as i8;
    }

    if d.pos_curr.y < -0x8000 && level.config_flags & 2 != 0 {
        d.collision_flags |= collision_flags::MASK_GRAB_REQUEST;
    }

    d.kart_state != kart_state::MASK_GRABBED
        && d.collision_flags & collision_flags::MASK_GRAB_REQUEST != 0
        && d.last_valid.is_some()
        && d.step_flag_set & step_triggers::SUPPRESS_MASK_GRAB == 0
}
