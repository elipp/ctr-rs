//! `VehPhysGeneral_PhysAngular` and `VehPhysGeneral_LerpToForwards`: turning.
//! Works out how fast the kart turns from the steering command, the speed and
//! the surface, integrates the direction of travel (`angle`) and the model's
//! own turn (`turn_angle_curr`, the drift-like swing of the car body), and
//! builds `matrixMovingDir`.

use math::Wrap;
use gte::Gte;
use math::mips;

use crate::calc::{interp_by_speed, map_to_range, steer_accel};
use crate::force::{counter_steer, rot_axis_angle, set_rot_matrix};
use crate::terrain::{TERRAINS, terrain_flags};
use crate::{Driver, Env, actions, step_triggers};

/// `DRIVER_WALL_RUB_TIMER_START`
const WALL_RUB_TIMER_START: i16 = 0xf0;
const ACCEL_TAP_STEER_COUNT: i16 = 7;

fn abs(value: i32) -> i32 {
    if value < 0 { mips::neg_lo(value) } else { value }
}

/// `VehPhysGeneral_LerpQuarterStrength`
fn lerp_quarter_strength(current: i32, desired: i32) -> i32 {
    let mut desired = desired;
    if desired != 0 {
        desired = mips::sra(desired, 2);
        if desired == 0 {
            desired = 1;
        }
    }
    if desired <= current { desired } else { current }
}

/// `VehPhysGeneral_LerpToForwards`: the velocity at which the model's turn
/// angle swings back towards `target_angle` (zero while it has not passed it).
fn lerp_to_forwards(d: &mut Driver, current_angle: i32, current_velocity: i32, target_angle: i32) -> i32 {
    let (mut angle, mut velocity, mut target) = (current_angle, current_velocity, target_angle);
    let mut mirrored = false;

    d.turn_angle_lerp_target = 0;
    if target < 0 || (target == 0 && angle < 0) {
        mirrored = true;
        angle = mips::neg_lo(angle);
        velocity = mips::neg_lo(velocity);
        target = mips::neg_lo(target);
    }

    let mut desired = 0;
    if d.wall_rub_timer != WALL_RUB_TIMER_START {
        if target < angle {
            let base = d.stats.model_turn_return_strength as i32;
            let lerp_strength = if (d.stats.model_rot_vel_max as i32) < angle { mips::sub_lo(mips::sll(base, 4), base) } else { base };
            desired = mips::neg_lo(lerp_quarter_strength(lerp_strength, mips::sub_lo(angle, target)));
        } else if angle < target {
            if angle < 0 {
                desired = lerp_quarter_strength(d.stats.model_turn_negative_return_strength as i32, mips::sub_lo(target, angle));
            } else {
                desired = lerp_quarter_strength(d.stats.model_turn_counter_steer_strength as i32, mips::sub_lo(target, angle));
                d.turn_angle_lerp_target = target.wrap_i16();
            }
        }
    }

    velocity = interp_by_speed(velocity, d.stats.model_turn_velocity_lerp as i32, desired);
    if mirrored { mips::neg_lo(velocity) } else { velocity }
}

/// `VehPhysGeneral_PhysAngular`
pub fn phys_angular(d: &mut Driver, gte: &mut Gte, env: &Env) {
    let elapsed = env.elapsed_time_ms;
    let stats = d.stats;

    // The camera-facing spin (`rot_curr_w`) settles back to zero.
    let remaining_rot = d.rot_curr_w as i32;
    let mut lerp_step = mips::sra(abs(remaining_rot), 3);
    if lerp_step == 0 {
        lerp_step = 1;
    }
    lerp_step = lerp_step.min(stats.drift_camera_lerp_step as i32);
    let previous_rot = interp_by_speed(d.rot_prev[3] as i32, 8, lerp_step);
    d.rot_prev[3] = previous_rot.wrap_i16();
    let interpolated_rot = interp_by_speed(remaining_rot, mips::sra(mips::mul_lo(previous_rot, elapsed), 5), 0);
    d.rot_curr_w = interpolated_rot.wrap_i16();

    // Local copy of the actions: the steer-left flip below is not written back.
    let mut actions = d.actions_flag_set;
    let raw_speed = d.speed_approx as i32;
    let base_speed = d.base_speed as i32;
    let mut rotation = mips::sll(d.simp_turn_state as i32, 8);

    // Which way the kart is going decides which way the steering turns it.
    let mut direction = d.forward_dir as i32;
    if raw_speed < 1 && base_speed < 0 {
        direction = -1;
        d.forward_dir = -1;
    }
    if raw_speed >= 0 && base_speed >= 0 {
        direction = 1;
        d.forward_dir = 1;
    }
    if direction < 0 {
        rotation = mips::neg_lo(rotation);
        actions ^= actions::STEER_LEFT;
    }

    let speed_approx = abs(raw_speed);
    // Steering is slower at low speed (except on turbo pads).
    if actions & actions::TOUCH_GROUND != 0 && d.step_flag_set & step_triggers::TURBO_PAD_MASK == 0 {
        rotation = map_to_range(speed_approx, 0x10, 0x300, 0, rotation);
    }

    // The turning rate (`rotation_spin_rate`) builds up towards the steering
    // command, and falls off when there is none.
    let terrain = &TERRAINS[d.terrain_meta1 as usize];
    let turn_const = d.turn_const as i32;
    let rate_for = |scale: i32| {
        mips::sra(
            mips::mul_lo(mips::add_lo(stats.turn_input_delay as i32, turn_const.wrapping_mul(scale)), terrain.turn_response_scale),
            8,
        )
    };
    let mut current_rate = d.rotation_spin_rate as i32;
    if rotation == 0 {
        current_rate = interp_by_speed(current_rate, rate_for(0x32), 0);
    } else {
        let mut mirrored = false;
        if rotation < 0 {
            rotation = mips::neg_lo(rotation);
            current_rate = mips::neg_lo(current_rate);
            mirrored = true;
        }
        if current_rate < rotation {
            current_rate = mips::add_lo(current_rate, rate_for(100));
            if rotation < current_rate {
                current_rate = rotation;
            }
        } else if rotation < current_rate {
            current_rate = mips::sub_lo(current_rate, rate_for(50));
            if current_rate < rotation {
                current_rate = rotation;
            }
        }
        if mirrored {
            current_rate = mips::neg_lo(current_rate);
        }
    }
    rotation = current_rate.sext16();
    d.rotation_spin_rate = current_rate.wrap_i16();

    // After a drift is released, the spin-out timer adds a decaying turn.
    let spinout_time = d.time_until_drift_spinout as i32;
    if spinout_time != 0 {
        let previous_frame_drift = d.previous_frame_mult_drift as i32;
        let spin = map_to_range(spinout_time, 0, 0x140, 0, previous_frame_drift);
        rotation = mips::add_lo(rotation, spin);
        d.time_until_drift_spinout = mips::sub_lo(spinout_time, elapsed).max(0).wrap_i16();
    }

    let class_speed_packed = mips::sll(stats.speed_class_stat.zext16(), 16);
    let class_speed = mips::sra(class_speed_packed, 16);
    let turn_resist_max_product = mips::mul_lo(stats.turn_resist_max as i32, class_speed);
    let turn_resist_min_product = mips::mul_lo(stats.turn_resist_min as i32, class_speed);
    let mut turn_angle_value = d.turn_angle_lerp_vel as i32;
    let mut model_rot_velocity = stats.model_rot_vel_max as i32;
    let mut turn_resist_max = mips::sra(turn_resist_max_product, 8);
    let mut turn_resist_min = mips::sra(turn_resist_min_product, 8);

    // Gas and brake together: the car skids, turns less and shows it more.
    if actions & actions::BRAKE_WITH_ACCEL != 0 {
        turn_resist_max = mips::sra(turn_resist_max_product, 9);
        if speed_approx > 0x300 {
            d.actions_flag_set |= actions::BACK_SKID;
        }
        turn_resist_min = mips::sra(turn_resist_min_product, 9);
        if d.base_speed != 0 {
            let model_speed = abs(d.speed as i32);
            let class_half_speed = mips::sra(class_speed_packed, 17);
            // Rotate the model to exaggerate steering above the steering speed threshold.
            model_rot_velocity = map_to_range(model_speed, 0x300, class_half_speed, stats.model_rot_vel_min as i32, model_rot_velocity);
        } else {
            model_rot_velocity = stats.model_rot_vel_min as i32;
        }
    }

    // This prevents you from steering sharp at low speeds.
    let turn_rate = mips::add_lo(stats.turn_rate as i32, mips::sll(turn_const, 1) / 5);
    let max_turn = mips::sll(turn_rate, 8);
    let turn_resistance = map_to_range(abs(d.speed as i32), turn_resist_min, turn_resist_max, max_turn, 0);

    let mut lerp_target = 0;
    if speed_approx >= turn_resist_min {
        let rotation_abs = abs(rotation);
        if turn_resistance < rotation_abs {
            lerp_target = map_to_range(abs(d.fire_speed as i32), turn_resist_min, turn_resist_max, lerp_target, model_rot_velocity);
            lerp_target = map_to_range(rotation_abs, turn_resistance, max_turn, 0, lerp_target);
            if rotation < 0 {
                lerp_target = mips::neg_lo(lerp_target);
            }
        }
    }

    // Spins the car's body back to face forwards when the drifting ends.
    let mut turn_angle = d.turn_angle_curr as i32;
    let lerp_result = lerp_to_forwards(d, turn_angle, turn_angle_value, lerp_target);
    d.turn_angle_lerp_vel = lerp_result.wrap_i16();
    turn_angle_value = lerp_result.sext16();

    if terrain.turn_angle_scale != 0x100 {
        turn_angle_value = mips::sra(mips::mul_lo(terrain.turn_angle_scale, turn_angle_value), 8);
    }
    turn_angle = mips::add_lo(turn_angle, mips::sra(mips::mul_lo(turn_angle_value, elapsed), 5));
    d.turn_angle_curr = turn_angle.wrap_i16();

    // Above a walking pace, on the ground, the steering accelerates.
    if speed_approx >= 0x300 && actions & actions::TOUCH_GROUND != 0 {
        let frames = d.num_frames_spent_steering as i32;
        let mut acceleration = steer_accel(
            frames,
            stats.steer_accel_stage2_first_frame as i32,
            stats.steer_accel_stage2_frame_length as i32,
            stats.steer_accel_stage4_first_frame as i32,
            stats.steer_accel_stage1_min_steer as i32,
            stats.steer_accel_stage1_max_steer as i32,
        );
        let scaled = mips::sra(mips::mul_lo(stats.steer_accel_turn_vel_scale as i32, abs(rotation)), 8);
        d.num_frames_spent_steering = mips::add_lo(frames, 1).wrap_i16();

        // The higher the value of the acceleration, the more steering is "locked up".
        if scaled < acceleration {
            acceleration = scaled;
        }
        if actions & actions::STEER_LEFT != 0 {
            acceleration = mips::neg_lo(acceleration);
        }

        let limit = stats.steer_accel_turn_vel_limit as i32;
        if 0 < rotation && mips::add_lo(rotation, acceleration) < mips::neg_lo(limit) {
            rotation = mips::neg_lo(limit);
        } else if rotation < 0 && limit < mips::add_lo(rotation, acceleration) {
            rotation = limit;
        } else {
            rotation = mips::add_lo(rotation, acceleration);
        }
    }

    // The wobble of the model after a turn, when the turn angle has swung far.
    let mut wobble_angle = d.turn_wobble_angle as i32;
    let mut wobble_timer = d.turn_wobble_timer as i32;
    let mut wobble_velocity = d.turn_wobble_velocity as i32;
    let mut keep_wobble_timer = false;
    if terrain.flags & terrain_flags::SKIP_TURN_ASSIST == 0 && actions & actions::TOUCH_GROUND != 0 {
        let threshold = mips::sra(mips::add_lo(mips::sll(model_rot_velocity, 1), model_rot_velocity), 2);
        if threshold < abs(turn_angle) {
            if abs(turn_angle_value) < 3 && abs(wobble_angle) < 10 {
                wobble_timer = 8;
                wobble_velocity = 0x14;
                if turn_angle < 0 {
                    wobble_velocity = -0x14;
                }
            }
            keep_wobble_timer = true;
        }
    }
    if !keep_wobble_timer {
        wobble_timer = 0;
    }
    if 0x32 < abs(wobble_angle) {
        wobble_timer = 0;
    }
    if wobble_timer != 0 {
        wobble_timer = mips::sub_lo(wobble_timer, 1);
        wobble_angle = mips::add_lo(wobble_angle, wobble_velocity);
    } else {
        wobble_velocity = if 0 < wobble_angle { -10 } else { 10 };
        wobble_angle = interp_by_speed(wobble_angle, abs(wobble_velocity), 0);
    }
    d.turn_wobble_timer = wobble_timer.wrap_i16();
    d.turn_wobble_angle = wobble_angle.wrap_i16();
    d.turn_wobble_velocity = wobble_velocity.wrap_i16();

    // The direction of travel turns by the turn angle (in the air, by less) ...
    let mut angle = d.angle as i32;
    let mapped_turn = map_to_range(speed_approx, 0, 0x600, turn_angle_value, 0);
    let rotation_delta = mips::sra(mips::mul_lo(mapped_turn, elapsed), 5);
    if 1 < abs(rotation_delta) {
        angle = mips::sub_lo(angle, rotation_delta) & 0xfff;
    }
    // ... and by the steering rotation.
    d.amp_turn_state = rotation.wrap_i16();
    angle = mips::add_lo(angle, mips::sra(mips::mul_lo(rotation, elapsed), 13)) & 0xfff;
    d.angle = angle.wrap_i16();

    // The model is drawn facing the direction of travel plus the swings.
    d.rot_curr_y = mips::add_lo(mips::add_lo(angle, turn_angle), wobble_angle).wrap_i16();

    // Lean of the body.
    if actions & actions::ACCEL_PREVENTION != 0 || d.accel_tap_count >= ACCEL_TAP_STEER_COUNT {
        rotation = mips::sra(rotation * 10, 8);
    } else if terrain.turn_lean_scale != 0x100 {
        rotation = mips::sra(mips::mul_lo(rotation, terrain.turn_lean_scale), 8);
    }

    let axis_delta = mips::sra(mips::mul_lo(rotation, elapsed), 13);
    d.axis_rotation_x = (mips::add_lo(d.axis_rotation_x.zext16(), axis_delta) & 0xfff).wrap_i16();

    let mut moving = d.matrix_moving_dir;
    rot_axis_angle(&mut moving, gte, d.axis_angle1_normal_vec, angle);
    d.matrix_moving_dir = moving;
    set_rot_matrix(gte, &d.matrix_moving_dir);
    counter_steer(d, gte);
}
