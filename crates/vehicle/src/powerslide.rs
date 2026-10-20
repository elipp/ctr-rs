//! Drifting: `VehPhysProc_PowerSlide_*` and `VehPhysProc_Driving_Update`'s
//! start of a drift.
//!
//! The kart hops, lands while steering, and the drift begins: it turns on its
//! own, steering only widens or tightens the curve, and a turbo meter fills.
//! Tapping a jump button while the meter is in its window turns the meter into
//! reserves. Drifting for too long (or reversing) spins the kart out, see
//! [`crate::spin`]; letting go of the button or slowing down ends the drift.
//!
//! The time spent drifting (`timeSpentDrifting`, for the end-of-race stats) is
//! not kept.

use gte::Gte;
use math::mips;

use crate::calc::{interp_by_speed, map_to_range};
use crate::fire::{self, turbo_type};
use crate::force::{counter_steer, rot_axis_angle, set_rot_matrix};
use crate::spin;
use crate::{Driver, Effect, Env, Input, PhysState, actions, buttons, kart_state, step_triggers};

const JUMP_BUTTON_MASK: u32 = buttons::R1 | buttons::L1;
const HALF_SPEED_SHIFT: u32 = 1;
const STEER_TURN_CONST_SHIFT: u32 = 1;
const STEER_TURN_CONST_DIVISOR: i32 = 5;
const DRIFT_TURN_CONST_SHIFT: u32 = 2;
const FRAME_TIME_SHIFT: u32 = 5;
const DRIFT_ANGLE_LERP_SHIFT: u32 = 3;
const DRIFT_AXIS_STEP_MS_SHIFT: u32 = 6;
const DRIFT_MS_SCALE_SHIFT: u32 = 5;
const DRIFT_SPINOUT_THRESHOLD_SHIFT: u32 = 1;
const ANGLE_INTEGRATION_SHIFT: u32 = 13;
const DRIFT_MAX_BOOSTS: i32 = 3;
const DRIFT_METER_FULL_FX: u8 = 0xf;
const DRIFT_FIRE_LEVEL_SHIFT: u32 = 6;
const DRIFT_FAILED_BOOST_EXHAUST_TIMER: u8 = 8;
const DRIFT_SPINOUT_NO_INPUT_TIMER: i16 = 0x3c0;
const DRIFT_REVERSE_SPINOUT_NO_INPUT_TIMER: i16 = 0x780;
const TURN_WOBBLE_START_ANGLE_MAX: i32 = 10;
const TURN_WOBBLE_START_TIMER: i16 = 8;
const TURN_WOBBLE_START_VELOCITY: i16 = 0x14;
const TURN_WOBBLE_ANGLE_MAX: i32 = 0x32;
const TURN_WOBBLE_RETURN_VELOCITY: i16 = 10;
/// The magic number the compiler divides by 5 with.
const DIVIDE_BY_5_MAGIC: i64 = 0x6666_6667;

/// Why a drift stopped (debugging aid; the sim does not read it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DriftEnd {
    /// Countersteered for as many frames as the spin-out limit.
    SwitchWayLimit,
    /// Slower than half the top speed.
    TooSlow,
    /// `DRIVING_AGAINST_WALL`: a wall scrub the tick before.
    WallScrape,
    /// Brake or accel prevention.
    Brake,
    ButtonReleased,
}

fn abs(value: i32) -> i32 {
    if value < 0 { mips::neg_lo(value) } else { value }
}

/// `sll(turn_const, shift) / 5`: what the turn stat adds to a rate.
fn turn_const_term(d: &Driver, shift: u32) -> i32 {
    mips::div(mips::sll(d.turn_const as i32, shift), STEER_TURN_CONST_DIVISOR)
}

/// The part of `VehPhysProc_Driving_Update` that starts a drift: a kart that
/// touched the ground this tick, steering hard, with the button it hopped with
/// still held, at more than half its top speed.
pub fn start_if_drifting(d: &mut Driver, input: &Input) -> bool {
    if d.actions_flag_set & actions::STARTED_TOUCH_GROUND == 0 {
        return false;
    }

    let turn_const_term = mips::div(mips::mul_lo(d.turn_const as i32, 1 << STEER_TURN_CONST_SHIFT), STEER_TURN_CONST_DIVISOR);
    let steer = abs(d.simp_turn_state as i32);

    if mips::sra(d.stats.turn_rate as i32 + turn_const_term, HALF_SPEED_SHIFT) < steer
        && input.buttons_held & d.button_used_to_start_drift as u32 != 0
        && d.actions_flag_set & actions::ACCEL_PREVENTION == 0
        && (d.stats.speed_class_stat as i32 >> 1) <= d.speed_approx as i32
    {
        init(d);
        return true;
    }
    false
}

/// `VehPhysProc_PowerSlide_Init`
pub fn init(d: &mut Driver) {
    d.kart_state = kart_state::DRIFTING;

    // The character's drift stat, plus the turning stat's share.
    let drift = mips::sll(mips::add_lo(d.stats.drift_turn_base as i32, turn_const_term(d, DRIFT_TURN_CONST_SHIFT)), 8);
    d.mult_drift = drift as i16;
    // Steering right drifts right.
    if d.simp_turn_state < 0 {
        d.mult_drift = mips::neg_lo(drift) as i16;
    }

    d.num_frames_drifting = 0;
    d.drift_total_time_ms = 0;
    d.drift_boost_time_ms = 0;
    d.num_boosts_attempted = 0;
    d.num_boosts_success = 0;

    d.rotation_spin_rate = 0;
    d.turn_angle_lerp_vel = 0;
    d.time_until_drift_spinout = 0;

    // The turbo meter starts empty: the room left to fill is its whole length.
    d.turbo_meter_room_left = mips::sll(d.stats.turbo_max_room as i32, FRAME_TIME_SHIFT) as i16;

    d.phys_state = PhysState::PowerSlide;
    // `PowerSlide_InitSetUpdate` arms the update on the next tick.
    d.init_pending = true;
}

/// `VehPhysProc_PowerSlide_Finalize`: a drift that ends keeps its turn for a
/// while (see `PhysAngular`).
fn finalize(d: &mut Driver) {
    d.previous_frame_mult_drift = d.mult_drift;
    d.time_until_drift_spinout = mips::sll(d.stats.drift_release_turn_assist_frames as i32, FRAME_TIME_SHIFT) as i16;
}

/// `VehPhysProc_PowerSlide_Update`
pub fn update(d: &mut Driver, input: &Input, env: &Env, effects: &mut Vec<Effect>) {
    // Pressing a jump button tries to turn the meter into reserves.
    if input.buttons_tapped & JUMP_BUTTON_MASK != 0 {
        let meter_left = d.turbo_meter_room_left as i32;
        d.num_frames_drifting = 0;

        if meter_left != 0 {
            let high_meter = mips::sll(d.stats.turbo_low_room_warning as i32, FRAME_TIME_SHIFT);

            if meter_left < high_meter {
                let reserves = map_to_range(meter_left, 0, high_meter, mips::sll(d.stats.turbo_full_bar_reserve_gain as i32, FRAME_TIME_SHIFT), 0);

                fire::increment(
                    d,
                    reserves,
                    turbo_type::POWER_SLIDE_HANG_TIME,
                    mips::sll(d.num_boosts_success as i32, DRIFT_FIRE_LEVEL_SHIFT),
                    effects,
                );

                d.num_boosts_success = mips::add_lo(d.num_boosts_success as u8 as i32, 1) as i8;

                if (d.num_boosts_success as i32) < DRIFT_MAX_BOOSTS {
                    d.actions_flag_set |= actions::TURBO_INPUT_LATCH;
                }

                d.drift_boost_time_ms = mips::sll(d.stats.drift_boost_duration_frames as i32, FRAME_TIME_SHIFT) as i16;
            } else {
                d.failed_boost_exhaust_timer = DRIFT_FAILED_BOOST_EXHAUST_TIMER;
            }

            d.turbo_meter_room_left = 0;
            d.num_boosts_attempted = mips::add_lo(d.num_boosts_attempted as u8 as i32, 1) as i8;
        }
    } else if d.turbo_meter_room_left != 0 {
        // The meter runs down by the time passed.
        let meter_left = mips::sub_lo(d.turbo_meter_room_left as u16 as i32, env.elapsed_time_ms as u16 as i32);
        d.turbo_meter_room_left = meter_left as i16;

        if mips::sll(meter_left, 16) < 0 {
            d.turbo_meter_room_left = 0;
        }

        if d.turbo_meter_room_left == 0 {
            effects.push(Effect::Sound { id: DRIFT_METER_FULL_FX, echo: (d.actions_flag_set >> 16) & 1 != 0 });
            d.num_boosts_attempted = mips::add_lo(d.num_boosts_attempted as u8 as i32, DRIFT_MAX_BOOSTS) as i8;
        }
    } else if (d.num_boosts_attempted as i32) < DRIFT_MAX_BOOSTS {
        d.turbo_meter_room_left = mips::sll(d.stats.turbo_max_room as i32, FRAME_TIME_SHIFT) as i16;
    }

    let frames_till_spinout = d.stats.drifting_frames_till_spinout as i32;

    // Drift for too long, or reverse without a turbo pad, and the kart spins out.
    if d.num_frames_drifting as i32 > frames_till_spinout {
        d.no_input_timer = DRIFT_SPINOUT_NO_INPUT_TIMER;
        spin::first_init(d, effects);
    } else if d.speed_approx < 0 && d.step_flag_set & step_triggers::TURBO_PAD_MASK == 0 {
        d.no_input_timer = DRIFT_REVERSE_SPINOUT_NO_INPUT_TIMER;
        spin::first_init(d, effects);
    } else {
        // Which of the ways to stop drifting, if any (the first that holds).
        let reason = if (d.num_frames_drifting as i32) < mips::neg_lo(frames_till_spinout) {
            Some(DriftEnd::SwitchWayLimit)
        } else if (d.speed as i32) < mips::sra(d.stats.speed_class_stat as i32, HALF_SPEED_SHIFT) {
            Some(DriftEnd::TooSlow)
        } else if d.actions_flag_set & actions::DRIVING_AGAINST_WALL != 0 {
            Some(DriftEnd::WallScrape)
        } else if d.actions_flag_set & (actions::ACCEL_PREVENTION | actions::BRAKE_WITH_ACCEL) != 0 {
            Some(DriftEnd::Brake)
        } else if input.buttons_held & d.button_used_to_start_drift as u32 == 0 {
            Some(DriftEnd::ButtonReleased)
        } else {
            None
        };
        if let Some(reason) = reason {
            d.last_drift_end = Some(reason);
            finalize(d);
            crate::sim::driving_init(d);
        }
    }
}

/// `VehPhysProc_PowerSlide_PhysLinear` after the driving one.
pub fn after_phys_linear(d: &mut Driver) {
    d.actions_flag_set |= actions::BACK_SKID | actions::FRONT_SKID;
}

/// `VehPhysProc_PowerSlide_PhysAngular`
pub fn phys_angular(d: &mut Driver, gte: &mut Gte, env: &Env) {
    let elapsed = env.elapsed_time_ms;
    let stats = d.stats;

    // The direction of travel catches up with the axis the lean is around.
    let wrapped_axis_angle = mips::add_lo(mips::sub_lo(d.axis_rotation_x as i32, d.angle as i32), 0x800) & 0xfff;
    let mut axis_angle_delta = wrapped_axis_angle - 0x800;
    let angle = d.angle as u16 as i32;
    if axis_angle_delta != 0 {
        axis_angle_delta = mips::sra(axis_angle_delta, DRIFT_ANGLE_LERP_SHIFT);
        if axis_angle_delta == 0 {
            axis_angle_delta = 1;
        }

        let step_limit = mips::sll(elapsed, DRIFT_AXIS_STEP_MS_SHIFT) >> DRIFT_MS_SCALE_SHIFT;
        if axis_angle_delta > step_limit {
            axis_angle_delta = step_limit;
        }
        if axis_angle_delta < mips::neg_lo(step_limit) {
            axis_angle_delta = mips::neg_lo(step_limit);
        }

        d.angle = mips::add_lo(angle, axis_angle_delta) as i16;
        d.axis_rotation_x = (mips::sub_lo(d.axis_rotation_x as u16 as i32, axis_angle_delta) & 0xfff) as i16;
    }

    // The camera swings round to the side of the drift.
    let camera_spin_rate = if d.mult_drift < 0 { -(stats.drifting_camera_spin_rate as i32) } else { stats.drifting_camera_spin_rate as i32 };
    {
        let remaining_rotation = abs(mips::sub_lo(d.rot_curr_w as i32, camera_spin_rate));
        let mut rotation_step = mips::sra(remaining_rotation, DRIFT_ANGLE_LERP_SHIFT);
        if rotation_step == 0 {
            rotation_step = 1;
        }

        let max_rotation_step = (stats.drift_camera_lerp_step as i32).min(rotation_step);
        d.rot_prev[3] = interp_by_speed(d.rot_prev[3] as i32, 8, max_rotation_step) as i16;
        let rotation_product = mips::mul_lo(d.rot_prev[3] as i32, elapsed);
        d.rot_curr_w = interp_by_speed(d.rot_curr_w as i32, rotation_product >> DRIFT_MS_SCALE_SHIFT, camera_spin_rate) as i16;
    }

    let steer_input_range = mips::add_lo(stats.turn_rate as i32, turn_const_term(d, STEER_TURN_CONST_SHIFT));

    let mut current_spin_rate = d.rotation_spin_rate as i32;
    // The drift direction.
    let mut drift_turn_input = d.mult_drift as i32;
    let mut spin_rate_negated = false;

    let steer_input = d.simp_turn_state as i32;
    let mut desired_spin_rate = steer_input << 8;
    let steer_range = mips::sll(steer_input_range, 8);
    let standard = stats.steer_vel_drift_standard as i32;
    let switch_way = stats.steer_vel_drift_switch_way as i32;

    if drift_turn_input < 0 {
        // Drifting right; steering right.
        if desired_spin_rate < 1 {
            desired_spin_rate = map_to_range(mips::neg_lo(desired_spin_rate), 0, steer_range, 0, mips::sll(mips::neg_lo(standard), 8));
        } else {
            // ... or left.
            desired_spin_rate = map_to_range(desired_spin_rate, 0, steer_range, 0, mips::sll(mips::neg_lo(switch_way), 8));
        }
    } else if desired_spin_rate >= 0 {
        // Drifting left; steering left.
        desired_spin_rate = map_to_range(desired_spin_rate, 0, steer_range, 0, mips::sll(standard, 8));
    } else {
        // ... or right.
        desired_spin_rate = map_to_range(mips::neg_lo(desired_spin_rate), 0, steer_range, 0, mips::sll(switch_way, 8));
    }

    let mut clamp_spin_rate = desired_spin_rate < current_spin_rate;
    if desired_spin_rate < 0 || (desired_spin_rate == 0 && current_spin_rate < 0) {
        spin_rate_negated = true;
        desired_spin_rate = mips::neg_lo(desired_spin_rate);
        current_spin_rate = mips::neg_lo(current_spin_rate);
        drift_turn_input = mips::neg_lo(drift_turn_input);
        clamp_spin_rate = desired_spin_rate < current_spin_rate;
    }

    if clamp_spin_rate {
        current_spin_rate =
            mips::sub_lo(current_spin_rate, mips::sra(mips::mul_lo(stats.drift_spin_rate_decel as i32, elapsed), DRIFT_MS_SCALE_SHIFT));
        clamp_spin_rate = current_spin_rate < desired_spin_rate;
    } else {
        current_spin_rate =
            mips::add_lo(current_spin_rate, mips::sra(mips::mul_lo(stats.drift_spin_rate_accel as i32, elapsed), DRIFT_MS_SCALE_SHIFT));
        clamp_spin_rate = desired_spin_rate < current_spin_rate;
    }
    if clamp_spin_rate {
        current_spin_rate = desired_spin_rate;
    }

    // Frames spent drifting count up on the left and down on the right (the
    // spin-out comes at either limit), and wind back to zero while not steering.
    if desired_spin_rate != 0 && drift_turn_input > 0 {
        d.num_frames_drifting = (d.num_frames_drifting as u16).wrapping_add(1) as i16;
        if d.num_frames_drifting < 0 {
            d.num_frames_drifting = 0;
        }
    } else if desired_spin_rate != 0 && drift_turn_input < 0 {
        d.num_frames_drifting = (d.num_frames_drifting as u16).wrapping_sub(1) as i16;
        if d.num_frames_drifting > 0 {
            d.num_frames_drifting = 0;
        }
    } else {
        d.num_frames_drifting = interp_by_speed(d.num_frames_drifting as i32, 1, 0) as i16;
    }

    if spin_rate_negated {
        current_spin_rate = mips::neg_lo(current_spin_rate);
        drift_turn_input = mips::neg_lo(drift_turn_input);
    }

    // The turn builds up over the first frames of the drift.
    drift_turn_input = map_to_range(
        d.drift_total_time_ms as i32,
        0,
        mips::sll(stats.drift_turn_ramp_frames as i32, FRAME_TIME_SHIFT),
        mips::sra(mips::mul_lo(stats.drift_turn_startup_scale as i32, d.mult_drift as i32), 8),
        drift_turn_input,
    );

    if drift_turn_input >= 0 {
        if current_spin_rate < mips::neg_lo(drift_turn_input) {
            current_spin_rate = mips::neg_lo(drift_turn_input);
        }
    }
    if drift_turn_input <= 0 && mips::neg_lo(drift_turn_input) < current_spin_rate {
        current_spin_rate = mips::neg_lo(drift_turn_input);
    }

    // `turn_const * 4 / 5`, the way the compiler divides by 5.
    let drift_turn_scaled = mips::sll(d.turn_const as i32, DRIFT_TURN_CONST_SHIFT);
    let drift_turn_product = drift_turn_scaled as i64 * DIVIDE_BY_5_MAGIC;
    let drift_turn_quotient = mips::sub_lo(mips::sra((drift_turn_product >> 32) as i32, 1), mips::sra(drift_turn_scaled, 31));

    d.rotation_spin_rate = current_spin_rate as i16;
    let signed_spin_rate = current_spin_rate as i16 as i32;

    let mut drift_turn_angle = map_to_range(
        abs(drift_turn_input),
        0,
        mips::add_lo(stats.drift_turn_base as i32, drift_turn_quotient),
        0,
        stats.drift_turn_angle_scale as i32,
    );
    let same_angle = stats.drift_turn_same_direction_angle as i32;
    let opposite_angle = stats.drift_turn_opposite_direction_angle as i32;
    if drift_turn_input < 0 {
        drift_turn_angle = mips::neg_lo(drift_turn_angle);

        if signed_spin_rate >= 0 {
            drift_turn_angle = mips::add_lo(
                drift_turn_angle,
                map_to_range(signed_spin_rate, 0, mips::sll(switch_way, 8), 0, mips::neg_lo(opposite_angle)),
            );
        } else {
            drift_turn_angle = mips::add_lo(
                drift_turn_angle,
                map_to_range(abs(signed_spin_rate), 0, mips::sll(standard, 8), 0, mips::neg_lo(same_angle)),
            );
        }
    } else if signed_spin_rate < 0 {
        drift_turn_angle =
            mips::add_lo(drift_turn_angle, map_to_range(abs(signed_spin_rate), 0, mips::sll(switch_way, 8), 0, opposite_angle));
    } else {
        drift_turn_angle = mips::add_lo(drift_turn_angle, map_to_range(signed_spin_rate, 0, mips::sll(standard, 8), 0, same_angle));
    }

    // The model's own turn eases towards that angle.
    drift_turn_angle = mips::sub_lo(drift_turn_angle, d.turn_angle_curr as i32);
    if drift_turn_angle != 0 {
        drift_turn_angle = mips::sra(drift_turn_angle, DRIFT_ANGLE_LERP_SHIFT);
        if drift_turn_angle == 0 {
            drift_turn_angle = 1;
        }
        d.turn_angle_curr = mips::add_lo(d.turn_angle_curr as u16 as i32, drift_turn_angle) as i16;
    }

    // Close to the spin-out, the model wobbles.
    let frames_drifting = abs(d.num_frames_drifting as i32);
    if mips::sra(stats.drifting_frames_till_spinout as i32, DRIFT_SPINOUT_THRESHOLD_SHIFT) < frames_drifting {
        if abs(d.turn_wobble_angle as i32) < TURN_WOBBLE_START_ANGLE_MAX {
            d.turn_wobble_timer = TURN_WOBBLE_START_TIMER;
            d.turn_wobble_velocity = TURN_WOBBLE_START_VELOCITY;
            if drift_turn_input < 0 {
                d.turn_wobble_velocity = mips::neg_lo(d.turn_wobble_velocity as i32) as i16;
            }
        }
    } else {
        d.turn_wobble_timer = 0;
    }

    if abs(d.turn_wobble_angle as i32) > TURN_WOBBLE_ANGLE_MAX {
        d.turn_wobble_timer = 0;
    }

    let wobble_angle_next;
    if d.turn_wobble_timer != 0 {
        d.turn_wobble_timer = (d.turn_wobble_timer as u16).wrapping_sub(1) as i16;
        wobble_angle_next = (d.turn_wobble_angle as u16).wrapping_add(d.turn_wobble_velocity as u16);
    } else {
        d.turn_wobble_velocity = TURN_WOBBLE_RETURN_VELOCITY;
        if 0 < d.turn_wobble_angle {
            d.turn_wobble_velocity = mips::neg_lo(d.turn_wobble_velocity as i32) as i16;
        }
        wobble_angle_next = interp_by_speed(d.turn_wobble_angle as i32, abs(d.turn_wobble_velocity as i32), 0) as u16;
    }
    d.turn_wobble_angle = wobble_angle_next as i16;

    let amp_turn_state = mips::add_lo(signed_spin_rate, drift_turn_input);
    d.amp_turn_state = amp_turn_state as i16;

    d.angle = (mips::add_lo(d.angle as u16 as i32, mips::sra(mips::mul_lo(amp_turn_state, elapsed), ANGLE_INTEGRATION_SHIFT)) & 0xfff) as i16;

    // After a successful boost, the lean kicks outwards for a while.
    if d.drift_boost_time_ms != 0 {
        d.drift_boost_time_ms = mips::sub_lo(d.drift_boost_time_ms as u16 as i32, elapsed as u16 as i32) as i16;
        if d.drift_boost_time_ms < 0 {
            d.drift_boost_time_ms = 0;
        }

        let mut axis_kick = mips::sra(mips::mul_lo(stats.drift_boost_axis_kick_rate as i32, elapsed), DRIFT_MS_SCALE_SHIFT);
        if d.turn_angle_curr < 0 {
            axis_kick = mips::neg_lo(axis_kick);
        }
        d.axis_rotation_x = (mips::add_lo(d.axis_rotation_x as u16 as i32, axis_kick) & 0xfff) as i16;
    }

    d.rot_curr_y = mips::add_lo(d.turn_wobble_angle as u16 as i32, mips::add_lo(d.angle as u16 as i32, d.turn_angle_curr as u16 as i32)) as i16;

    // The meter for the turn ramp counts milliseconds.
    d.drift_total_time_ms = mips::add_lo(d.drift_total_time_ms as u16 as i32, elapsed as u16 as i32) as i16;
    let ramp_time = mips::sll(stats.drift_turn_ramp_frames as i32, FRAME_TIME_SHIFT);
    if d.drift_total_time_ms as i32 > ramp_time {
        d.drift_total_time_ms = ramp_time as i16;
    }

    let mut moving = d.matrix_moving_dir;
    rot_axis_angle(&mut moving, gte, d.axis_angle1_normal_vec, d.angle as i32);
    d.matrix_moving_dir = moving;
    set_rot_matrix(gte, &d.matrix_moving_dir);
    counter_steer(d, gte);
}
