//! Spinning out: `VehPhysProc_SpinFirst_*`, `SpinLast_*` and `SpinStop_*`.
//! A drift that goes on for too long (or a reversing one) throws the kart
//! into a spin: it spins until it has slowed down (or the time is up), the
//! spin winds down, the model straightens up, and driving resumes.

use math::Wrap;
use gte::Gte;
use math::mips;

use crate::calc::interp_by_speed;
use crate::force::rot_axis_angle;
use crate::frame::{num_anim_frames, start_frame};
use crate::{Driver, Effect, Env, PhysState, actions, kart_state};

const STEER_RESET_FRAMES: i16 = 10000;
const ANGLE_INTEGRATION_SHIFT: u32 = 13;
const SPIN_SLOW_SPEED_THRESHOLD: i32 = 0x2ff;
const SPIN_INITIAL_RATE: i16 = 300;
const SPIN_VOICELINE_ID: u8 = 3;
const SPIN_VOICELINE_PRIORITY: u8 = 0x10;
const SPIN_LEFT_FEEDBACK: u8 = 0x19;
const SPIN_RIGHT_FEEDBACK: u8 = 0x29;
const SPIN_FEEDBACK_TIMER: u8 = 0x60;
const SPIN_STOP_ANGLE_THRESHOLD: i32 = 16;
const SPIN_RECENTER_ANGLE_THRESHOLD: i32 = 400;
const SPIN_RECENTER_RATE_NUM_SHIFT: u32 = 2;
const SPIN_RECENTER_RATE_DEN_SHIFT: u32 = 3;
const SPIN_MIN_RECENTER_RATE: i16 = 0x20;
const SPIN_STOP_ANIM_SWEEP_STEP: u16 = 5;
const SPIN_STOP_ANIM_RETURN_STEP: i32 = 2;
const ANG_PI: i32 = 0x800;
const WUMPA_MAX_COUNT: i32 = 10;

fn abs(value: i32) -> i32 {
    if value < 0 { mips::neg_lo(value) } else { value }
}

/// `RB_Player_ModifyWumpa` for a loss of wumpa fruit: the Aku Aku mask protects them.
fn lose_wumpa(d: &mut Driver, amount: i32) {
    if d.actions_flag_set & actions::MASK_WEAPON != 0 {
        return;
    }
    d.num_wumpas = ((d.num_wumpas as i32 - amount).clamp(0, WUMPA_MAX_COUNT)).wrap_i8();
}

/// `VehPhysProc_SpinFirst_Init`
pub fn first_init(d: &mut Driver, effects: &mut Vec<Effect>) {
    d.kart_state = kart_state::SPINNING;

    d.turn_angle_lerp_vel = 0;
    d.turbo_meter_room_left = 0;

    lose_wumpa(d, 1);

    effects.push(Effect::Voiceline { id: SPIN_VOICELINE_ID, driver_id: d.driver_id, priority: SPIN_VOICELINE_PRIORITY });

    // Spinning left, unless the drift's turn was to the right.
    d.set_spin_dir(1);
    d.set_spin_rate(SPIN_INITIAL_RATE);
    if d.amp_turn_state < 0 {
        d.set_spin_rate(-SPIN_INITIAL_RATE);
        d.set_spin_dir(-1);
    }

    d.phys_state = PhysState::SpinFirst;
    // `SpinFirst_InitSetUpdate` arms the update on the next tick.
    d.init_pending = true;

    let pattern = if d.simp_turn_state > 0 { SPIN_RIGHT_FEEDBACK } else { SPIN_LEFT_FEEDBACK };
    effects.push(Effect::JogCon { driver_id: d.driver_id, pattern, strength: SPIN_FEEDBACK_TIMER });
}

/// `VehPhysProc_SpinFirst_Update`
pub fn first_update(d: &mut Driver) {
    // While the no-input timer runs, the spin goes on as long as the kart is fast.
    if d.no_input_timer != 0 && abs(d.speed_approx as i32) > SPIN_SLOW_SPEED_THRESHOLD {
        return;
    }
    last_init(d);
}

/// `VehPhysProc_SpinFirst_PhysLinear` after the driving one.
pub fn first_after_phys_linear(d: &mut Driver) {
    d.base_speed = 0;
    d.fire_speed = 0;
    d.actions_flag_set |= actions::WARP | actions::FRONT_SKID | actions::BACK_SKID | actions::ACCEL_PREVENTION;
}

/// `VehPhysProc_SpinFirst_PhysLinear` before the driving one: the timer.
pub fn first_before_phys_linear(d: &mut Driver, env: &Env) {
    d.no_input_timer = mips::sub_lo(d.no_input_timer.zext16(), env.elapsed_time_ms.zext16()).wrap_i16();
    if d.no_input_timer < 0 {
        d.no_input_timer = 0;
    }
}

/// `VehPhysProc_SpinFirst_PhysAngular`
pub fn first_phys_angular(d: &mut Driver, gte: &mut Gte, env: &Env) {
    let elapsed = env.elapsed_time_ms;

    d.num_frames_spent_steering = STEER_RESET_FRAMES;

    // The turning rate and the wobble die away.
    let spin_rate = d.rotation_spin_rate;
    d.rotation_spin_rate = spin_rate.wrapping_sub(spin_rate >> 3);

    d.turn_angle_curr = d.turn_angle_curr.wrapping_add(d.spin_rate());
    d.turn_angle_curr = (d.turn_angle_curr.wrapping_add(ANG_PI.wrap_i16()) & 0xfff).wrapping_sub(ANG_PI.wrap_i16());

    d.amp_turn_state = d.rotation_spin_rate;
    d.turn_wobble_angle = d.turn_wobble_angle.wrapping_sub(d.turn_wobble_angle >> 3);

    let step = mips::sra(mips::mul_lo(d.amp_turn_state as i32, elapsed), ANGLE_INTEGRATION_SHIFT).wrap_i16();
    d.angle = d.angle.wrapping_add(step) & 0xfff;

    d.rot_curr_y = d.angle.wrapping_add(d.turn_angle_curr).wrapping_add(d.turn_wobble_angle);
    d.rot_curr_w = interp_by_speed(d.rot_curr_w as i32, mips::sra(mips::sll(elapsed, 5), 5), 0).wrap_i16();

    let mut moving = d.matrix_moving_dir;
    rot_axis_angle(&mut moving, gte, d.axis_angle1_normal_vec, d.angle as i32);
    d.matrix_moving_dir = moving;
}

/// `VehPhysProc_SpinLast_Init`
fn last_init(d: &mut Driver) {
    d.phys_state = PhysState::SpinLast;
    d.init_pending = false;
}

/// `VehPhysProc_SpinLast_Update`
pub fn last_update(d: &mut Driver) {
    // Almost facing forward: stop the spin.
    if abs(d.turn_angle_curr as i32) < SPIN_STOP_ANGLE_THRESHOLD {
        stop_init(d);
    }
}

/// `VehPhysProc_SpinLast_PhysLinear` after the driving one.
pub fn last_after_phys_linear(d: &mut Driver) {
    d.base_speed = 0;
    d.fire_speed = 0;
    d.actions_flag_set |= actions::WARP | actions::ACCEL_PREVENTION;
}

/// `VehPhysProc_SpinLast_PhysAngular`
pub fn last_phys_angular(d: &mut Driver, gte: &mut Gte, env: &Env) {
    let elapsed = env.elapsed_time_ms;
    let drift_angle = d.turn_angle_curr as i32;

    d.num_frames_spent_steering = STEER_RESET_FRAMES;

    d.rotation_spin_rate = d.rotation_spin_rate.wrapping_sub(d.rotation_spin_rate >> 3);
    d.turn_wobble_angle = d.turn_wobble_angle.wrapping_sub(d.turn_wobble_angle >> 3);
    d.amp_turn_state = d.rotation_spin_rate;

    // The model's spin turns back to facing forwards: within a range of the
    // front it slows to arrive there.
    let recenter_rate = |angle: i32| mips::sra(mips::neg_lo(mips::sll(angle, SPIN_RECENTER_RATE_NUM_SHIFT)), SPIN_RECENTER_RATE_DEN_SHIFT).wrap_i16();
    let spin_step = |d: &Driver| {
        (mips::add_lo(mips::add_lo(d.turn_angle_curr.zext16(), d.spin_rate().zext16()), ANG_PI) & 0xfff).wrapping_sub(ANG_PI).wrap_i16()
    };

    if drift_angle > 0 {
        if d.spin_rate() < 0 && drift_angle < SPIN_RECENTER_ANGLE_THRESHOLD {
            d.set_spin_rate(recenter_rate(drift_angle));
            if d.spin_rate() > -SPIN_MIN_RECENTER_RATE {
                d.set_spin_rate(-SPIN_MIN_RECENTER_RATE);
            }
        }

        d.turn_angle_curr = spin_step(d);

        if d.spin_rate() < 0 && d.turn_angle_curr < 0 {
            d.turn_angle_curr = 0;
        }
    } else if drift_angle < 0 {
        if d.spin_rate() > 0 && drift_angle > -SPIN_RECENTER_ANGLE_THRESHOLD {
            d.set_spin_rate(recenter_rate(drift_angle));
            if d.spin_rate() < SPIN_MIN_RECENTER_RATE {
                d.set_spin_rate(SPIN_MIN_RECENTER_RATE);
            }
        }

        d.turn_angle_curr = spin_step(d);

        if d.spin_rate() > 0 && d.turn_angle_curr > 0 {
            d.turn_angle_curr = 0;
        }
    }

    d.angle = (mips::add_lo(d.angle.zext16(), mips::sra(mips::mul_lo(d.amp_turn_state as i32, elapsed), ANGLE_INTEGRATION_SHIFT)) & 0xfff).wrap_i16();

    d.rot_curr_y = mips::add_lo(d.turn_wobble_angle.zext16(), mips::add_lo(d.angle.zext16(), d.turn_angle_curr.zext16())).wrap_i16();
    d.rot_curr_w = interp_by_speed(d.rot_curr_w as i32, mips::sra(mips::sll(elapsed, 5), 5), 0).wrap_i16();

    let mut moving = d.matrix_moving_dir;
    rot_axis_angle(&mut moving, gte, d.axis_angle1_normal_vec, d.angle as i32);
    d.matrix_moving_dir = moving;
}

/// `VehPhysProc_SpinStop_Init`
fn stop_init(d: &mut Driver) {
    d.phys_state = PhysState::SpinStop;
    d.init_pending = false;
}

/// `VehPhysProc_SpinStop_PhysLinear` after the driving one.
pub fn stop_after_phys_linear(d: &mut Driver) {
    d.base_speed = 0;
    d.fire_speed = 0;
}

/// `VehPhysProc_SpinStop_PhysAngular`
pub fn stop_phys_angular(d: &mut Driver, gte: &mut Gte, env: &Env) {
    let elapsed = env.elapsed_time_ms;

    d.angle = (mips::add_lo(d.angle.zext16(), mips::sra(mips::mul_lo(d.amp_turn_state as i32, elapsed), ANGLE_INTEGRATION_SHIFT)) & 0xfff).wrap_i16();
    d.rot_curr_y = mips::add_lo(d.turn_wobble_angle.zext16(), mips::add_lo(d.angle.zext16(), d.turn_angle_curr.zext16())).wrap_i16();

    d.rot_curr_w = interp_by_speed(d.rot_curr_w as i32, mips::sra(mips::sll(elapsed, 5), 5), 0).wrap_i16();
    d.turn_angle_curr = interp_by_speed(d.turn_angle_curr as i32, mips::sra(mips::sll(elapsed, 7), 5), 0).wrap_i16();

    let mut moving = d.matrix_moving_dir;
    rot_axis_angle(&mut moving, gte, d.axis_angle1_normal_vec, d.angle as i32);
    d.matrix_moving_dir = moving;
}

/// `VehPhysProc_SpinStop_Animate`: the model steers one way, then the other
/// ("to exaggerate the force when steering stops abruptly"), then back to
/// the middle; then driving resumes.
pub fn stop_animate(d: &mut Driver) {
    let num_frames = num_anim_frames(d, d.anim_index);
    let mut resume = num_frames <= 0;

    if !resume {
        match d.spin_dir() {
            1 => {
                // From right to left.
                d.anim_frame = (d.anim_frame.wrap_u16()).wrapping_sub(SPIN_STOP_ANIM_SWEEP_STEP).wrap_i16();
                if d.anim_frame < 0 {
                    d.anim_frame = 0;
                    d.set_spin_dir(0);
                }
            }
            -1 => {
                // From left to right.
                d.anim_frame = (d.anim_frame.wrap_u16()).wrapping_add(SPIN_STOP_ANIM_SWEEP_STEP).wrap_i16();
                if d.anim_frame as i32 >= num_frames {
                    d.anim_frame = (num_frames - 1).wrap_i16();
                    d.set_spin_dir(0);
                }
            }
            _ => {
                let target = start_frame(0, num_frames);
                d.anim_frame = interp_by_speed(d.anim_frame as i32, SPIN_STOP_ANIM_RETURN_STEP, target).wrap_i16();
                resume = d.anim_frame as i32 == target;
            }
        }
    }

    if resume {
        d.init_pending = true;
    }
}
