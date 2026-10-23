//! `game/Vehicle/VehPhysForce.c`: the forces on the kart. Turns the speed into
//! a velocity vector, adds gravity and friction in the kart's own frame, and
//! builds the rotation matrices from the track's normals.
//!
//! Not ported: `CollideDrivers` (kart against kart), the squash and stretch of
//! the model and the baked wheelie matrices (`TranslateMatrix` only builds the
//! facing matrix here; the rest is presentation), and the wake particles.

use coll::Collision;
use gte::{Gte, ctrl, data};
use loader::lev::{Level, SVec3};
use math::{Vec3, mips};

use crate::calc::interp_by_speed;
use crate::terrain::{TERRAINS, terrain_flags, terrain_type};
use crate::fire::{self, turbo_type};
use crate::{Driver, Effect, Env, Matrix, MatrixArray, actions, collision_flags, kart_state, rain_cloud, step_triggers};

/// `gte_mvmva(sf, mx, v, cv, lm)`
pub fn mvmva(sf: u32, mx: u32, v: u32, cv: u32, lm: u32) -> u32 {
    0x0400012 | (sf << 19) | (mx << 17) | (v << 15) | (cv << 13) | (lm << 10)
}
/// `gte_op12`
const OP_SF1: u32 = 0x0178000c;

const QUAD_LOW_GRAVITY: u16 = 0x2;
const MUD_TERMINAL_SPEED: i32 = 0x100;
const SKID_SPEED_THRESHOLD: i32 = 0x300;
const TERRAIN_SIDE_LOCK_TIMER: i32 = -0x140;
/// `FP8(100)`
const MAX_SPEED: i16 = 100 << 8;
/// `FP(-1)`: the height at which mud starts to pull the kart down.
const MAX_MUD_SINK_Y_LEVEL: i32 = -(1 << 12);
const ROLLBACK_WINDOW_TIMER: i16 = 0x280;

fn abs(value: i32) -> i32 {
    if value < 0 { mips::neg_lo(value) } else { value }
}

/// `CTR_GteSetRotMatrix`
pub fn set_rot_matrix(gte: &mut Gte, matrix: &Matrix) {
    for (i, word) in matrix.words().into_iter().enumerate() {
        gte.ctc2(word, ctrl::R_MATRIX + i);
    }
}

/// `VehGteSetColorMatrix`
fn set_color_matrix(gte: &mut Gte, matrix: &Matrix) {
    for (i, word) in matrix.words().into_iter().enumerate() {
        gte.ctc2(word, ctrl::LC_MATRIX + i);
    }
}

fn mac(gte: &mut Gte) -> (i32, i32, i32) {
    (gte.mfc2(data::MAC1) as i32, gte.mfc2(data::MAC2) as i32, gte.mfc2(data::MAC3) as i32)
}

/// `VehPhysForce_ConvertSpeedToVec`: the speed along the axis angles as a
/// velocity.
pub fn convert_speed_to_vec(d: &Driver, velocity: &mut Vec3) {
    let speed = d.speed as i32;
    let (sine, cosine) = math::sin_cos(d.axis_rotation_y as i32 as u32);
    velocity.y = mips::sra(mips::mul_lo(speed, sine), 12);
    let y_component = mips::sra(mips::mul_lo(speed, cosine), 12);

    let (sine, cosine) = math::sin_cos(d.axis_rotation_x as i32 as u32);
    velocity.x = mips::sra(mips::mul_lo(y_component, sine), 12);
    velocity.z = mips::sra(mips::mul_lo(y_component, cosine), 12);
}

/// `VehPhysForce_OnGravity`: the velocity in the kart's frame (forward, side,
/// up), with gravity, the speed limits and the friction applied, and back.
pub fn on_gravity(d: &mut Driver, gte: &mut Gte, env: &Env, level: &Level, velocity: &mut Vec3, effects: &mut Vec<Effect>) {
    // The rotation matrix is matrixMovingDir; the light matrix is its
    // transpose, so that a vector goes from the world into the kart's frame.
    let w = d.matrix_moving_dir.words();
    for (i, word) in w.into_iter().enumerate() {
        gte.ctc2(word, ctrl::R_MATRIX + i);
    }
    let high = 0xffff_0000u32;
    gte.ctc2((w[0] & 0xffff) | (w[1] & high), ctrl::L_MATRIX);
    gte.ctc2((w[3] & 0xffff) | (w[0] & high), ctrl::L_MATRIX + 1);
    gte.ctc2((w[2] & 0xffff) | (w[3] & high), ctrl::L_MATRIX + 2);
    gte.ctc2((w[1] & 0xffff) | (w[2] & high), ctrl::L_MATRIX + 3);
    gte.ctc2(w[4], ctrl::L_MATRIX + 4);

    let world_to_local = mvmva(1, 1, 0, 3, 0);
    gte.mtc2((velocity.x as u32 & 0xffff) | (velocity.y as u32) << 16, data::VXY0);
    gte.mtc2(velocity.z as u32, data::VZ0);
    gte.cop2(world_to_local);
    let (mut local_x, mut local_y, mut local_z) = mac(gte);
    let (original_local_x, mut original_local_y, original_local_z) = (local_x, local_y, local_z);

    let mut gravity_y = mips::neg_lo(d.stats.gravity as i32);
    let quad_flags = d.under_driver.map_or(0, |q| level.quad_blocks[q as usize].flags);
    let elapsed = env.elapsed_time_ms;

    if quad_flags & QUAD_LOW_GRAVITY != 0 {
        let scaled = mips::add_lo(mips::sll(gravity_y, 2), gravity_y);
        gravity_y = mips::add_lo(mips::sll(scaled, 3), gravity_y) / 100;
    }

    let gravity_scaled = mips::sra(mips::mul_lo(gravity_y, elapsed), 5);
    gte.mtc2(mips::sll(gravity_scaled, 16) as u32, data::VXY0);
    gte.mtc2(0, data::VZ0);
    gte.cop2(world_to_local);
    let (mut gravity_x, gravity_local_y, mut gravity_z) = mac(gte);
    let forward_accel_impulse = d.forward_accel_impulse as i32;

    if (gravity_z < 0 && forward_accel_impulse > 0) || (gravity_z > 0 && forward_accel_impulse < 0) {
        gravity_z = 0;
    }

    let mut actions = d.actions_flag_set;
    let speed_approx = d.speed_approx as i32;
    let base_speed = d.base_speed as i32;
    if actions & actions::ACCEL_PREVENTION != 0 || (base_speed > 0 && speed_approx < 0) || (base_speed < 0 && speed_approx > 0) {
        gravity_x = 0;
        gravity_z = 0;
    }

    local_x = mips::add_lo(local_x, gravity_x);
    local_y = mips::add_lo(local_y, gravity_local_y);

    let fire_speed = d.fire_speed as i32;
    let slope_forward_speed_bonus = d.stats.slope_forward_speed_bonus as i32;
    local_z = mips::add_lo(local_z, gravity_z);

    // Limit the speed forwards, backwards, sideways and up/down. A speed that
    // is already over the limit may only come down.
    let mut speed_limit = mips::add_lo(fire_speed, slope_forward_speed_bonus);
    if local_z > speed_limit {
        local_z = original_local_z;
        if original_local_z < speed_limit {
            local_z = speed_limit;
        }
    }
    speed_limit = mips::sub_lo(fire_speed, mips::sra(slope_forward_speed_bonus, 1));
    if local_z < speed_limit {
        local_z = original_local_z;
        if original_local_z > speed_limit {
            local_z = speed_limit;
        }
    }

    let terrain_flags = TERRAINS[d.terrain_meta1 as usize].flags;
    speed_limit = d.stats.side_speed_clamp as i32;
    if speed_limit < local_x {
        local_x = original_local_x;
        if local_x < speed_limit {
            local_x = speed_limit;
        }
    }
    let negative_limit = mips::neg_lo(speed_limit);
    if local_x < negative_limit {
        local_x = original_local_x;
        if original_local_x > negative_limit {
            local_x = negative_limit;
        }
    }

    speed_limit = d.stats.terminal_velocity as i32;
    if local_y < 0 && terrain_flags & terrain_flags::MUD_PHYSICS != 0 {
        speed_limit = MUD_TERMINAL_SPEED;
        if original_local_y < -MUD_TERMINAL_SPEED {
            original_local_y = -MUD_TERMINAL_SPEED;
        }
    }
    if local_y > speed_limit {
        local_y = original_local_y;
        if local_y < speed_limit {
            local_y = speed_limit;
        }
    }
    let negative_limit = mips::neg_lo(speed_limit);
    if local_y < negative_limit {
        local_y = original_local_y;
        if original_local_y > negative_limit {
            local_y = negative_limit;
        }
    }

    let state = d.kart_state;
    if state == kart_state::MASK_GRABBED {
        local_x = 0;
        local_z = 0;
    } else if d.actions_flag_set_prev_frame & actions::TOUCH_GROUND != 0
        || state == kart_state::BLASTED
        || (speed_approx > d.terrain_scaled_base_speed as i32 && TERRAINS[d.terrain_meta2 as usize].speed_multiplier < 0x100)
    {
        let stats = d.stats;
        let (mut perpendicular, mut forward);

        if actions & actions::ACCEL_PREVENTION != 0 {
            if abs(speed_approx) > SKID_SPEED_THRESHOLD {
                actions |= actions::BACK_SKID;
            }
            let brake = stats.brake_friction as i32;
            perpendicular = brake;
            if d.rain_cloud_effect == rain_cloud::HEAVY_FRICTION {
                perpendicular = mips::sll(perpendicular, 4);
            } else if state == kart_state::BLASTED {
                perpendicular = mips::sra(perpendicular * 3, 2);
            } else if state == kart_state::SPINNING {
                perpendicular = mips::sra(brake, 1);
            }
            forward = perpendicular;
        } else if base_speed != 0 {
            let abs_speed = abs(speed_approx);
            if abs_speed > SKID_SPEED_THRESHOLD && ((base_speed > 0 && speed_approx < 0) || (base_speed < 0 && speed_approx > 0)) {
                // Pedal against the direction of travel: braking.
                perpendicular = stats.pedal_friction_perpendicular as i32;
                forward = stats.brake_friction as i32;
                actions |= actions::BACK_SKID;
            } else if state == kart_state::DRIFTING {
                perpendicular = stats.drift_curve as i32;
                forward = stats.drift_friction as i32;
            } else {
                perpendicular = stats.pedal_friction_perpendicular as i32;
                forward = stats.pedal_friction_forward as i32;
                if abs_speed > SKID_SPEED_THRESHOLD && abs_speed < mips::sra(abs(base_speed), 1) {
                    actions |= actions::BACK_SKID;
                }
            }
        } else {
            perpendicular = stats.no_pedal_friction_perpendicular as i32;
            forward = stats.no_pedal_friction_forward as i32;
            if d.rain_cloud_effect == rain_cloud::HEAVY_FRICTION {
                perpendicular = mips::sll(stats.brake_friction as i32, 4);
                forward = perpendicular;
            }
        }

        let terrain = &TERRAINS[d.terrain_meta1 as usize];
        perpendicular = mips::sra(mips::mul_lo(perpendicular, elapsed), 5);
        forward = mips::sra(mips::mul_lo(forward, elapsed), 5);
        if terrain.ground_friction_scale != 0x100 {
            perpendicular = mips::sra(mips::mul_lo(terrain.ground_friction_scale, perpendicular), 8);
            forward = mips::sra(mips::mul_lo(terrain.ground_friction_scale, forward), 8);
        }

        let boost = stats.terrain_friction_boost as i32;
        let mut timer = d.terrain_friction_timer as i32;
        if timer < 0 {
            let abs_local_x = abs(local_x);
            if timer == TERRAIN_SIDE_LOCK_TIMER {
                perpendicular = mips::sra(abs_local_x, 1);
            } else {
                perpendicular = mips::add_lo(perpendicular, mips::sra(mips::mul_lo(perpendicular, boost), 8));
                if perpendicular < 0 {
                    perpendicular = 0;
                }
            }
            if abs_local_x > 0 {
                actions |= actions::BACK_SKID | actions::FRONT_SKID;
                effects.push(Effect::ShockForce { driver_id: d.driver_id, a: 4, b: 0x7f });
                effects.push(Effect::ShockFreq { driver_id: d.driver_id, a: 4, b: 0 });
            }
            timer = mips::add_lo(timer, elapsed);
            if timer > 0 {
                timer = 0;
            }
            d.terrain_friction_timer = timer as i16;
        } else if timer > 0 {
            timer = mips::sub_lo(timer, elapsed);
            if timer < 0 {
                timer = 0;
            }
            perpendicular = mips::add_lo(perpendicular, mips::sra(mips::mul_lo(perpendicular, boost), 8));
            d.terrain_friction_timer = timer as i16;
            if perpendicular < 0 {
                perpendicular = 0;
            }
        }

        // Mud pulls back harder the faster the kart goes sideways, or against its pedal.
        if actions & actions::MASK_WEAPON == 0 && terrain_flags & terrain_flags::MUD_PHYSICS != 0 {
            let side = abs(mips::sra(local_x, 3));
            if perpendicular < side {
                perpendicular = side;
            }
            let min_forward = if local_z != 0 && base_speed != 0 && (local_z ^ base_speed) < 0 {
                Some(abs(mips::sra(local_z, 1)))
            } else if (local_z < base_speed && local_z <= 0) || (base_speed < local_z && local_z >= 0) {
                Some(mips::sra(abs(mips::sub_lo(local_z, base_speed)), 1))
            } else {
                None
            };
            if let Some(min_forward) = min_forward {
                if forward < min_forward {
                    forward = min_forward;
                }
            }
        }

        if terrain_flags & terrain_flags::SIDESLIP_FRICTION != 0 {
            perpendicular = mips::sra(perpendicular * 3, 2);
            if perpendicular < forward {
                perpendicular = forward;
            }
        }

        local_x = interp_by_speed(local_x, perpendicular, 0);
        local_z = interp_by_speed(local_z, forward, 0);
    }

    // Back into the world.
    gte.mtc2((local_x as u32 & 0xffff) | (local_y as u32) << 16, data::VXY0);
    gte.mtc2(local_z as u32, data::VZ0);
    gte.cop2(mvmva(1, 0, 0, 3, 0));
    (velocity.x, velocity.y, velocity.z) = mac(gte);
    d.actions_flag_set = actions;

    if actions & actions::AIRBORNE == 0 {
        if d.bool_first_frame_since_rev_engine != 0 && local_z != 0 {
            d.forward_dir = if local_z < 0 { -1 } else { 1 };
            d.bool_first_frame_since_rev_engine = 0;
        } else {
            if original_local_z < 0 {
                if local_z >= 0 {
                    d.forward_dir = 1;
                }
            } else {
                if local_z < 0 {
                    d.forward_dir = -1;
                }
                if original_local_z == 0 && local_z >= 0 {
                    d.forward_dir = 1;
                }
            }
        }
    }

    // A change of direction along the kart is a "rollback". Many of them in
    // quick succession means the kart is jittering between quad blocks.
    let rolled_back = if original_local_z < 0 { local_z > 0 } else { local_z < 0 || (original_local_z == 0 && local_z > 0) };
    if rolled_back {
        if d.v_shift_window_timer != 0 {
            d.v_shift_count = d.v_shift_count.wrapping_add(1);
        }
        d.v_shift_window_timer = ROLLBACK_WINDOW_TIMER;
    }
}

/// `VehPhysForce_OnApplyForces`: the centre of the kart, the velocity from the
/// speed, mud, gravity, and the acceleration from collisions. Clears the
/// collision results the search that follows sets again.
pub fn on_apply_forces(d: &mut Driver, coll: &mut Collision, env: &Env, level: &Level, effects: &mut Vec<Effect>) {
    if d.speed > MAX_SPEED {
        d.speed = MAX_SPEED;
    }

    // The origin of the driver model is the centre-bottom of the kart; half a
    // radius up (0, 25, 0), turned by the orientation matrix, is its centre.
    set_color_matrix(&mut coll.gte, &d.matrix_facing_dir);
    coll.gte.mtc2(0x190000, data::VXY0);
    coll.gte.mtc2(0, data::VZ0);
    coll.gte.cop2(mvmva(1, 2, 0, 3, 0));
    (d.origin_to_center.x, d.origin_to_center.y, d.origin_to_center.z) = mac(&mut coll.gte);

    let mut velocity = d.velocity;
    convert_speed_to_vec(d, &mut velocity);

    if let Some(under) = d.under_driver {
        if level.quad_blocks[under as usize].terrain_type == terrain_type::MUD && d.pos_curr.y > MAX_MUD_SINK_Y_LEVEL {
            // Sink slower as you approach the mud's bottom.
            let sink_speed = mips::sub_lo(MAX_MUD_SINK_Y_LEVEL, d.pos_curr.y);
            if velocity.y < sink_speed {
                velocity.y = sink_speed;
            }
        }
    }

    on_gravity(d, &mut coll.gte, env, level, &mut velocity, effects);

    d.axis_angle1_normal_vec = SVec3 { x: 0, y: 0x1000, z: 0 };
    d.normal_vec_up = SVec3 { x: 0, y: 0x1000, z: 0 };
    d.curr_block_touching = None;
    d.collision_flags = 0;

    velocity.x = mips::add_lo(velocity.x, d.accel.x as i32);
    velocity.z = mips::add_lo(velocity.z, d.accel.z as i32);
    velocity.y = mips::add_lo(velocity.y, d.accel.y as i32);
    d.velocity = velocity;
}

const TURBO_PAD_RESERVES: i32 = 0x3c0;
const SUPER_TURBO_PAD_RESERVES: i32 = 0x78;
const TURBO_PAD_FIRE_LEVEL: i32 = 0x100;
const SUPER_TURBO_PAD_FIRE_LEVEL: i32 = 0x800;
const SURFACE_PUSHBACK_Y_BIAS: i32 = 4;
const SURFACE_PUSHBACK_SHIFT: u32 = 6;

/// `VehPhysForce_CollideDrivers`: takes the collision acceleration back out of
/// the velocity, requests a mask grab on a kill plane, applies the turbo pads,
/// bumps into the other karts in reach ([`crate::collide`]) and pushes the kart
/// out of a surface it is sinking into. The water split line is not ported.
pub fn collide_drivers(d: &mut Driver, others: &mut crate::collide::Others, env: &Env, effects: &mut Vec<Effect>) {
    let step_flags = d.step_flag_set;

    d.velocity.x = mips::sub_lo(d.velocity.x, d.accel.x as i32);
    d.velocity.y = mips::sub_lo(d.velocity.y, d.accel.y as i32);
    d.velocity.z = mips::sub_lo(d.velocity.z, d.accel.z as i32);

    if step_flags & coll::step_flags::FLAG_KILL_PLANE != 0 {
        d.collision_flags |= collision_flags::MASK_GRAB_REQUEST;
    }

    if step_flags & step_triggers::SUPER_TURBO_PAD != 0 {
        let flags = turbo_type::TURBO_PAD | turbo_type::FREEZE_RESERVES_ON_TURBO_PAD;
        fire::increment(d, SUPER_TURBO_PAD_RESERVES, flags, SUPER_TURBO_PAD_FIRE_LEVEL, effects);
    } else if step_flags & step_triggers::TURBO_PAD != 0 {
        let flags = turbo_type::TURBO_PAD | turbo_type::FREEZE_RESERVES_ON_TURBO_PAD;
        fire::increment(d, TURBO_PAD_RESERVES, flags, TURBO_PAD_FIRE_LEVEL, effects);
    }

    crate::collide::collide(d, others, env, effects);

    if d.collision_flags & collision_flags::SURFACE_PUSHBACK != 0 {
        let diff_x = mips::sub_lo(mips::sra(d.pos_curr.x, 8), d.sps_hit_pos.x as i32);
        let product_x = mips::mul_lo(d.sps_normal_vec.x as i32, diff_x);
        let floor_diff_y =
            mips::add_lo(mips::sub_lo(mips::sra(d.quad_block_height, 8), d.sps_hit_pos.y as i32), SURFACE_PUSHBACK_Y_BIAS);
        let product_y = mips::mul_lo(d.sps_normal_vec.y as i32, floor_diff_y);
        let diff_z = mips::sub_lo(mips::sra(d.pos_curr.z, 8), d.sps_hit_pos.z as i32);
        let product_z = mips::mul_lo(d.sps_normal_vec.z as i32, diff_z);
        let dot = mips::add_lo(mips::add_lo(product_x, product_y), product_z);

        // Behind the surface: push back out, in proportion to how far.
        if dot < 0 {
            let diff_y = mips::sub_lo(mips::sra(d.pos_curr.y, 8), d.sps_hit_pos.y as i32);
            d.velocity.x = mips::add_lo(d.velocity.x, mips::sll(diff_x, SURFACE_PUSHBACK_SHIFT));
            d.velocity.y = mips::add_lo(d.velocity.y, mips::sll(diff_y, SURFACE_PUSHBACK_SHIFT));
            d.velocity.z = mips::add_lo(d.velocity.z, mips::sll(diff_z, SURFACE_PUSHBACK_SHIFT));
        }
    }
}

/// `VehPhysForce_RotAxisAngle`: the matrix that turns `angle` around the
/// axis `normal` (its second column is the axis), the third column is the
/// forward direction on the plane of the normal, the first is their cross
/// product.
pub fn rot_axis_angle(m: &mut Matrix, gte: &mut Gte, normal: SVec3, angle: i32) {
    let (nx, ny, nz) = (normal.x as i32, normal.y as i32, normal.z as i32);
    m.m[0][1] = normal.x;
    m.m[1][1] = normal.y;
    m.m[2][1] = normal.z;

    let (sin, cos) = math::sin_cos(angle as u32);

    let normal_x_sq = mips::mul_lo(nx, nx);
    let normal_z_sq = mips::mul_lo(nz, nz);
    let cross_xz = mips::mul_lo(nx, mips::neg_lo(nz));
    let denominator = mips::add_lo(normal_x_sq, normal_z_sq);
    let scaled_sin_y = mips::sra(mips::mul_lo(sin, ny), 12);
    let scaled_cos_y = mips::sra(mips::mul_lo(cos, ny), 12);

    gte.mtc2(denominator as u32, data::LZCS);
    let mut shift = gte.mfc2(data::LZCR) as i32;

    let (out_x, out_y, out_z);
    if denominator == 0 {
        // The normal points straight up (or down).
        let dot = mips::add_lo(mips::mul_lo(sin, nx), mips::mul_lo(cos, nz));
        out_y = mips::sra(mips::neg_lo(dot), 12);
        out_x = if normal.y < 0 { mips::neg_lo(scaled_sin_y) } else { scaled_sin_y };
        out_z = scaled_cos_y;
    } else {
        let (mut normal_x_sq, mut normal_z_sq, mut cross_xz, mut denominator) = (normal_x_sq, normal_z_sq, cross_xz, denominator);
        shift = mips::sub_lo(0x14, shift);
        if shift > 0 {
            normal_x_sq = mips::sra(normal_x_sq, shift as u32);
            normal_z_sq = mips::sra(normal_z_sq, shift as u32);
            cross_xz = mips::sra(cross_xz, shift as u32);
            denominator = mips::sra(denominator, shift as u32);
        }

        let sin_remainder = mips::sub_lo(sin, scaled_sin_y);
        let cos_remainder = mips::sub_lo(cos, scaled_cos_y);
        let div_x = mips::div(mips::add_lo(mips::mul_lo(sin_remainder, normal_z_sq), mips::mul_lo(cos_remainder, cross_xz)), denominator);
        let div_z = mips::div(mips::add_lo(mips::mul_lo(sin_remainder, cross_xz), mips::mul_lo(cos_remainder, normal_x_sq)), denominator);
        let dot = mips::add_lo(mips::mul_lo(sin, nx), mips::mul_lo(cos, nz));

        out_x = mips::add_lo(scaled_sin_y, div_x);
        out_y = mips::sra(mips::neg_lo(dot), 12);
        out_z = mips::add_lo(scaled_cos_y, div_z);
    }

    let column = [out_x as i16, out_y as i16, out_z as i16];
    m.m[0][2] = column[0];
    m.m[1][2] = column[1];
    m.m[2][2] = column[2];

    // The first column: the GTE's cross product of the normal and the third column.
    gte.ctc2(normal.x as i32 as u32, ctrl::R_MATRIX);
    gte.ctc2(normal.y as i32 as u32, ctrl::R22R23);
    gte.ctc2(normal.z as i32 as u32, ctrl::R33);
    gte.mtc2(column[0] as i32 as u32, data::IR1);
    gte.mtc2(column[1] as i32 as u32, data::IR2);
    gte.mtc2(column[2] as i32 as u32, data::IR3);
    gte.cop2(OP_SF1);
    m.m[0][0] = gte.mfc2(data::MAC1) as i16;
    m.m[1][0] = gte.mfc2(data::MAC2) as i16;
    m.m[2][0] = gte.mfc2(data::MAC3) as i16;
}

/// `VehPhysForce_CounterSteer`: the sideways push that turning the model
/// makes on a slippery surface. Expects `matrixMovingDir` in the GTE's
/// rotation matrix, as `PhysAngular` leaves it.
pub fn counter_steer(d: &mut Driver, gte: &mut Gte) {
    let speed_approx = abs(d.speed_approx as i32);
    d.accel = SVec3::default();

    if speed_approx <= 3 << 8
        || d.kart_state == kart_state::CRASHING
        || d.actions_flag_set & actions::WARP != 0
        || d.wall_rub_timer != 0
        || d.actions_flag_set & actions::TOUCH_GROUND == 0
    {
        return;
    }

    let ratio = TERRAINS[d.terrain_meta1 as usize].counter_steer_ratio;
    if ratio == 0 {
        return;
    }
    let mut value = mips::sra(mips::mul_lo(ratio, -8000), 8);

    let limit = d.stats.model_turn_counter_steer_strength as i32;
    let angle = mips::sub_lo(d.turn_angle_curr as i32, d.turn_angle_prev as i32).clamp(mips::neg_lo(limit), limit);
    let sine = math::sin(angle as u32);

    value = mips::sra(mips::mul_lo(value, sine), 12);
    gte.mtc2(value as u16 as u32, data::VXY0);
    gte.mtc2(0, data::VZ0);
    gte.cop2(mvmva(1, 0, 0, 3, 0));
    let (x, y, z) = mac(gte);
    d.accel = SVec3 { x: x as i16, y: y as i16, z: z as i16 };
}

/// `VEH_PHYS_FORCE_KART_SCALE_BASE`: the kart's scale when it is neither squashed nor stretched.
pub const KART_SCALE_BASE: i32 = 0xccc;
const MASK_GRAB_SCALE_XZ_FACTOR: i32 = 0x28;
const MIN_SQUASH_XZ_SCALE: i32 = 0x400;
const TARGET_SQUISH_DEFAULT: i32 = -800;
const SQUISH_DEADBAND: i32 = 0x960;
const AIR_SQUISH_MIN: i32 = -800;
const LANDING_SQUISH_MIN: i32 = -0x640;
const SQUISH_MAX: i32 = 800;
const HAZARD_BLINK_MASK: i16 = 0x80;
const FALL_STRETCH_HEIGHT_MAX: i32 = 0xa00;
const FALL_STRETCH_MIN: i32 = 0x280;
const FALL_STRETCH_MAX: i32 = 0x320;
const SQUISH_INTERP_SPEED: i32 = 300;
const SQUISH_SCALE_INTERP_SPEED: i32 = 0xa0;
const SQUISH_SCALE_XZ_FACTOR: i32 = 0xa0;
const SQUISH_RESTORE_SFX: u8 = 0x5b;

/// `VehPhysForce_TranslateMatrix_Div256TowardZero`
fn div256_toward_zero(value: i32) -> i32 {
    mips::sra(if value < 0 { mips::add_lo(value, 0xff) } else { value }, 8)
}

/// The squash and stretch of the kart model (the first part of `VehPhysForce_TranslateMatrix`):
/// it stretches in the air, and squashes on landing and when it takes off.
///
/// A squished kart (`squishTimer`) is flat, and springs back when it ends.
///
/// Not ported: the TNT box's wobble.
fn squash_stretch(d: &mut Driver, effects: &mut Vec<Effect>) {
    if d.kart_state == kart_state::WARP_PAD {
        return;
    }
    let squish = d.jump_squish_stretch as i32;

    if d.kart_state == kart_state::MASK_GRABBED && d.actions_flag_set & actions::TOUCH_GROUND == 0 {
        d.inst_scale_y = mips::add_lo(squish, KART_SCALE_BASE) as i16;
        let xz = mips::sub_lo(KART_SCALE_BASE, div256_toward_zero(squish * MASK_GRAB_SCALE_XZ_FACTOR)).max(MIN_SQUASH_XZ_SCALE);
        (d.inst_scale_x, d.inst_scale_z) = (xz as i16, xz as i16);
        return;
    }

    let jump_height = d.jump_height_curr as i32;
    let mut target = TARGET_SQUISH_DEFAULT;

    if d.actions_flag_set & actions::JUMP_STARTED == 0 {
        let current = d.jump_squish_stretch2 as i32;
        let weighted = jump_height * 7;
        target = mips::sll(mips::sub_lo(current, mips::sra(mips::add_lo(current * 9, weighted), 4)), 2);
        if target.abs() < SQUISH_DEADBAND {
            target = 0;
        }
        if (d.actions_flag_set | d.actions_flag_set_prev_frame) & actions::STARTED_TOUCH_GROUND != 0 {
            target = target.max(LANDING_SQUISH_MIN);
        } else {
            target = target.max(AIR_SQUISH_MIN);
        }
        target = target.min(SQUISH_MAX);
    }

    if d.hazard_timer > 0 && d.hazard_timer & HAZARD_BLINK_MASK == 0 && target > TARGET_SQUISH_DEFAULT {
        target = TARGET_SQUISH_DEFAULT;
    }

    // Falling stretches the kart more the farther it falls.
    if d.actions_flag_set & actions::TOUCH_GROUND == 0 && jump_height < 0 {
        let stretch = crate::calc::map_to_range(-jump_height, 0, FALL_STRETCH_HEIGHT_MAX, FALL_STRETCH_MIN, FALL_STRETCH_MAX);
        target = target.max(stretch);
        d.jump_squish_stretch2 = jump_height as i16;
    }

    if (d.jump_squish_stretch as i32).abs() < target.abs() {
        d.jump_squish_stretch = target as i16;
    }
    d.jump_squish_stretch = interp_by_speed(d.jump_squish_stretch as i32, SQUISH_INTERP_SPEED, 0) as i16;
    d.jump_squish_stretch2 = mips::sra(mips::add_lo(d.jump_squish_stretch2 as i32 * 9, jump_height * 7), 4) as i16;

    let squish = d.jump_squish_stretch as i32;
    if d.squish_timer != 0 {
        d.inst_scale_y = 0;
    } else if d.inst_scale_y == 0 {
        effects.push(Effect::Sound { id: SQUISH_RESTORE_SFX, echo: d.actions_flag_set & actions::ENGINE_ECHO != 0 });
        d.inst_scale_y = mips::add_lo(squish, KART_SCALE_BASE) as i16;
        d.matrix_array = MatrixArray::SquishRecover;
        d.matrix_index = 0;
    } else {
        d.inst_scale_y =
            interp_by_speed(d.inst_scale_y as i32, SQUISH_SCALE_INTERP_SPEED, mips::add_lo(squish, KART_SCALE_BASE)) as i16;
    }
    let xz = mips::sub_lo(KART_SCALE_BASE, div256_toward_zero(squish * SQUISH_SCALE_XZ_FACTOR));
    d.inst_scale_x = interp_by_speed(d.inst_scale_x as i32, SQUISH_SCALE_INTERP_SPEED, xz) as i16;
    d.inst_scale_z = interp_by_speed(d.inst_scale_z as i32, SQUISH_SCALE_INTERP_SPEED, xz) as i16;
}

/// `VehPhysForce_TranslateMatrix`: squashes and stretches the model, builds the
/// matrix that faces it along the surface normal, then steps the wheelie.
pub fn translate_matrix(d: &mut Driver, gte: &mut Gte, effects: &mut Vec<Effect>) {
    squash_stretch(d, effects);
    let mut facing = d.matrix_facing_dir;
    rot_axis_angle(&mut facing, gte, d.axis_angle2_normal_vec, d.rot_curr_y as i32);
    d.matrix_facing_dir = facing;
    crate::baked::update_matrix_animation(d);
}

#[cfg(test)]
mod squash_tests {
    use super::*;

    fn kart() -> Driver {
        let mut d = Driver::new(0, 0);
        d.actions_flag_set = actions::TOUCH_GROUND;
        d
    }

    #[test]
    fn a_kart_on_the_ground_keeps_its_size() {
        let mut d = kart();
        for _ in 0..10 {
            squash_stretch(&mut d, &mut Vec::new());
        }
        assert_eq!([d.inst_scale_x, d.inst_scale_y, d.inst_scale_z], [0xccc; 3]);
    }

    #[test]
    fn a_squished_kart_is_flat_and_springs_back() {
        let mut d = kart();
        let mut effects = Vec::new();
        d.squish_timer = 0xf00;
        squash_stretch(&mut d, &mut effects);
        assert_eq!(d.inst_scale_y, 0);
        assert!(effects.is_empty());

        d.squish_timer = 0;
        squash_stretch(&mut d, &mut effects);
        assert_eq!(d.inst_scale_y, 0xccc);
        assert_eq!((d.matrix_array, d.matrix_index), (MatrixArray::SquishRecover, 0));
        assert!(effects.contains(&Effect::Sound { id: SQUISH_RESTORE_SFX, echo: false }));

        // The recovery runs its 15 entries and ends.
        for _ in 0..15 {
            crate::baked::update_matrix_animation(&mut d);
        }
        assert_eq!(d.matrix_array, MatrixArray::None);
    }

    #[test]
    fn a_fall_stretches_the_kart_tall_and_thin_and_it_recovers() {
        let mut d = kart();
        d.actions_flag_set = 0;
        d.jump_height_curr = -0x800;
        squash_stretch(&mut d, &mut Vec::new());
        assert!(d.inst_scale_y > 0xccc && d.inst_scale_x < 0xccc && d.inst_scale_z < 0xccc);
        assert_eq!(d.inst_scale_x, d.inst_scale_z);

        d.actions_flag_set = actions::TOUCH_GROUND;
        d.jump_height_curr = 0;
        for _ in 0..200 {
            squash_stretch(&mut d, &mut Vec::new());
        }
        assert_eq!([d.inst_scale_x, d.inst_scale_y, d.inst_scale_z], [0xccc; 3]);
    }
}
