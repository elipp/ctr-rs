//! `VehPhysProc_Driving_PhysLinear` and `VehPhysGeneral_GetBaseSpeed`: the
//! start of the driver's tick. Counts the timers down, reads the gamepad and
//! turns it into a target speed (`base_speed`) and a steering command
//! (`simp_turn_state`).
//!
//! Not ported yet, because nothing else of them exists: weapons (the item
//! roulette, firing, bombs and bubbles), the mask weapons (but for having
//! one, which the mask grab gives the driver), the Super Engine,
//! the TNT crate on the head, and the time-spent statistics and tire colour,
//! which do not feed back into the simulation. The driver has no item, so
//! none of that code is reached.
//!
//! In `ctr-native`, the paths that go through `gasClearSteerFlags` drop the
//! reverse-steer bits they clear (the retail registers `actionsFlagSetCopy`
//! and `actionsFlagSetNext` are one register, native has two variables). This
//! follows retail.

use math::mips;

use crate::calc::{interp_by_speed, map_to_range};
use crate::joystick::{return_to_rest, strength_absolute};
use crate::terrain::TERRAINS;
use crate::{Driver, Effect, Env, Input, actions, buttons, kart_state, rain_cloud, step_triggers};

/// `VEH_PHYS_PROC_ACTION_CARRY_MASK`: the actions that survive into the next tick.
const ACTION_CARRY_MASK: u32 = actions::TOUCH_GROUND
    | actions::JUMP_BUTTON_HELD
    | actions::STEER_LEFT
    | actions::HIGH_JUMP
    | actions::TURBO_INPUT_LATCH
    | actions::DRIVING_WRONG_WAY
    | actions::TURBO_ITEM
    | actions::WEAPON_FIRE_REQUEST
    | actions::ENGINE_ECHO
    | actions::REVERSING_ENGINE
    | actions::RACE_TIMER_FROZEN
    | actions::AIRBORNE
    | actions::BOT
    | actions::BEHIND_START_LINE
    | actions::RACE_FINISHED
    | actions::TRACKER_TARGETED
    | actions::CHECKPOINT_BRANCH_PENDING
    | actions::HUMAN_HUMAN_COLLISION
    | actions::REVERSE_STEER_LEFT
    | actions::REVERSE_STEER_RIGHT;

const JUMP_BUTTON_MASK: u32 = buttons::R1 | buttons::L1;
const JUMP_BUFFER_FRAMES: i16 = 10;
const STICK_CENTER: i32 = 0x80;
const REVERSE_STICK_THRESHOLD: i32 = 100;
const BRAKE_HIGH_SPEED_THRESHOLD: i32 = 0x300;
const ACCEL_TAP_WINDOW_MS: i16 = 0x100;
const ACCEL_TAP_STEER_COUNT: i16 = 7;
const STEER_RESET_FRAMES: i16 = 10000;
/// `Driver::accelTapCount` and speed limits for the tapping steer boost.
const STEER_ACCEL_TAP_SPEED_MAX: i32 = 0x2600;
const STEER_ACCEL_TAP_STRENGTH: i32 = 0x5a;
const STEER_WALL_RUB_STRENGTH: i32 = 0x30;
const STEER_BRAKE_STRENGTH: i32 = 0x40;
const WHEEL_ROTATION_STRENGTH: i32 = 0x40;
const WHEEL_ROTATION_INTERP_STEP: i32 = 0x18;

fn abs(value: i32) -> i32 {
    if value < 0 { mips::neg_lo(value) } else { value }
}

/// Counts a timer down by `ms` (given negated), stopping at zero.
fn count_down(timer: &mut i16, neg_ms: i32) {
    if *timer > 0 {
        let value = mips::add_lo(*timer as i32, neg_ms);
        *timer = value.max(0) as i16;
    }
}

/// `value / 2^shift`, rounding towards zero (`(v + bias) >> shift` when negative).
fn shift_toward_zero(value: i32, shift: u32) -> i32 {
    let bias = (1 << shift) - 1;
    if value < 0 { mips::sra(mips::add_lo(value, bias), shift) } else { mips::sra(value, shift) }
}

/// `VehPhysGeneral_GetBaseSpeed`: the speed the driver is aiming for with the
/// gas pedal down: the class speed, plus the wumpa bonus, limited while on
/// reserves, minus damage.
pub fn base_speed(d: &Driver) -> i32 {
    let stats = &d.stats;
    let wumpas = (d.num_wumpas as i32).min(9);
    let turbo = (d.turbo_const as i32).min(5);

    let accel_speed = stats.accel_speed_class_stat as i32;
    let mut stat = stats.speed_class_stat as i32;
    let net_speed_stat = mips::sub_lo(mips::sll(mips::sub_lo(accel_speed, stat), 12) / 5, 1);

    let wumpa_product = mips::mul_lo(wumpas, net_speed_stat);
    let wumpa_bonus = wumpa_product / 10;
    let turbo_bonus = mips::mul_lo(turbo, net_speed_stat);
    let mut bonus = mips::sra(mips::add_lo(wumpa_bonus, turbo_bonus), 12);

    if d.actions_flag_set & actions::MASK_WEAPON != 0 {
        bonus = mips::add_lo(bonus, stats.mask_speed as i32);
    }

    if d.reserves != 0 {
        let single = stats.single_turbo_speed as i32;
        stat = mips::add_lo(stat, d.fire_speed_cap as i32);
        let mut cap = mips::add_lo(single, mips::sll(mips::sub_lo(stats.sacred_fire_speed as i32, single), 1));
        cap = mips::sub_lo(cap, d.fire_speed_cap as i32);
        if cap < 0 {
            cap = 0;
        }
        if cap < bonus {
            bonus = cap;
        }
    }
    stat = mips::add_lo(stat, bonus);

    let mut subtract = 0;
    if d.burn_timer != 0 || d.squish_timer != 0 || d.rain_cloud_effect == rain_cloud::SLOW {
        subtract = stats.damaged_speed as i32;
    }
    if d.clock_receive != 0 {
        let clock_effect = mips::sra(mips::mul_lo(stats.damaged_speed as i32, mips::sub_lo(0x14, d.driver_rank as i32)), 4);
        if subtract < clock_effect {
            subtract = clock_effect;
        }
    }
    stat = mips::sub_lo(stat, subtract);

    stat.min(0x6400)
}

/// The gas and brake section: the target speed, and the actions it sets.
struct Gas {
    target: i32,
    /// `base_speed` before the terrain scales it.
    base: i32,
    actions: u32,
}

fn gas_and_brake(d: &mut Driver, input: &Input, env: &Env, mut a: u32, approx_speed: i32, cross: bool, mut square: bool) -> Gas {
    let stick_ry = if env.end_of_race { STICK_CENTER } else { input.stick_ry as i32 };
    let stick_ly = if env.end_of_race { STICK_CENTER } else { input.stick_ly as i32 };
    let mut cross = cross;

    // Square releases the reserves (unless on a turbo pad).
    if square && d.step_flag_set & step_triggers::TURBO_PAD_MASK == 0 {
        d.reserves = 0;
    }

    if d.reserves != 0 || d.rain_cloud_effect == rain_cloud::RESERVE_RELEASE {
        if !cross {
            let pedal = mips::neg_lo(return_to_rest(stick_ry, STICK_CENTER));
            if pedal < 1 {
                a |= actions::ACCEL_RELEASED_WITH_RESERVES;
            }
        }
        if square && approx_speed > BRAKE_HIGH_SPEED_THRESHOLD {
            a |= actions::BACK_SKID;
        }
        if d.step_flag_set & step_triggers::TURBO_PAD_MASK != 0 {
            square = false;
        }
        // The reserves keep the pedal down.
        cross = true;
    }

    // Reverse steering latches die with the steering (and the speed).
    if (d.simp_turn_state) < 0 {
        a &= !actions::REVERSE_STEER_RIGHT;
    } else {
        a &= !actions::REVERSE_STEER_LEFT;
        if d.simp_turn_state < 1 {
            a &= !actions::REVERSE_STEER_RIGHT;
        }
    }
    if abs(d.speed_approx as i32) < BRAKE_HIGH_SPEED_THRESHOLD {
        a &= !(actions::REVERSE_STEER_LEFT | actions::REVERSE_STEER_RIGHT);
    }

    let target_base = base_speed(d);
    let backward = d.stats.backward_speed as i32;
    let clear_steer = |a: u32| a & !(actions::REVERSE_STEER_LEFT | actions::REVERSE_STEER_RIGHT);
    let target;

    if square {
        let stick = return_to_rest(stick_ly, STICK_CENTER);
        let reverse_brake = stick >= REVERSE_STICK_THRESHOLD || (stick > 0 && a & actions::REVERSING_ENGINE != 0);
        if reverse_brake {
            let t = mips::add_lo(mips::sll(mips::neg_lo(backward), 1), mips::neg_lo(backward));
            target = shift_toward_zero(t, 2);
            a |= actions::REVERSING_ENGINE | actions::BRAKE_WITH_ACCEL;
            a = clear_steer(a);
        } else if cross {
            target = shift_toward_zero(target_base, 1);
            a |= actions::BRAKE_WITH_ACCEL;
            a &= !actions::REVERSING_ENGINE;
            a = clear_steer(a);
        } else {
            let pedal = mips::neg_lo(return_to_rest(stick_ry, STICK_CENTER));
            if pedal > 0 {
                target = shift_toward_zero(mips::mul_lo(target_base, pedal), 8);
                a |= actions::BRAKE_WITH_ACCEL;
            } else if pedal < 0 {
                target = shift_toward_zero(mips::mul_lo(backward, pedal), 8);
                a |= actions::REVERSING_ENGINE | actions::BRAKE_WITH_ACCEL;
            } else {
                target = 0;
                a |= actions::ACCEL_PREVENTION;
                a &= !actions::REVERSING_ENGINE;
            }
            a = clear_steer(a);
        }
    } else if cross {
        target = target_base;
        a &= !actions::REVERSING_ENGINE;
        a = clear_steer(a);
    } else {
        let pedal = mips::neg_lo(return_to_rest(stick_ry, STICK_CENTER));
        if pedal < 0 {
            if d.speed_approx as i32 >= 0x301 || a & (actions::REVERSE_STEER_LEFT | actions::REVERSE_STEER_RIGHT) != 0 {
                // Pulling back while still rolling forwards: brakes, and the
                // kart steers as if reversing.
                target = 0;
                a |= actions::ACCEL_PREVENTION;
                if d.simp_turn_state > 0 {
                    a |= actions::REVERSE_STEER_RIGHT;
                }
                if d.simp_turn_state < 0 {
                    a |= actions::REVERSE_STEER_LEFT;
                }
            } else {
                target = shift_toward_zero(mips::mul_lo(backward, pedal), 7);
                a |= actions::REVERSING_ENGINE;
            }
        } else {
            let mut reverse_from_neutral = false;
            if pedal == 0 {
                let centered = return_to_rest(stick_ly, STICK_CENTER);
                reverse_from_neutral = centered >= REVERSE_STICK_THRESHOLD || (centered > 0 && a & actions::REVERSING_ENGINE != 0);
            }
            if reverse_from_neutral {
                a |= actions::REVERSING_ENGINE;
                target = mips::neg_lo(backward);
            } else {
                target = shift_toward_zero(mips::mul_lo(target_base, pedal), 7);
                a &= !actions::REVERSING_ENGINE;
            }
            a = clear_steer(a);
        }
    }

    Gas { target, base: target_base, actions: a }
}

/// `VehPhysProc_Driving_PhysLinear`.
pub fn phys_linear(d: &mut Driver, env: &Env, input: &Input, effects: &mut Vec<Effect>) {
    let ms = env.elapsed_time_ms;
    let neg_ms = mips::neg_lo(ms);

    // === Count timers ===
    count_down(&mut d.reserves, neg_ms);
    count_down(&mut d.turbo_outside_timer, neg_ms);
    count_down(&mut d.veh_fire_audio_cooldown, neg_ms);
    count_down(&mut d.wall_rub_timer, neg_ms);
    count_down(&mut d.jump_forced_ms, neg_ms);
    count_down(&mut d.jump_cooldown_ms, neg_ms);
    count_down(&mut d.jump_high_jump_timer_ms, neg_ms);
    count_down(&mut d.burn_timer, neg_ms);
    count_down(&mut d.squish_timer, neg_ms);
    count_down(&mut d.v_shift_start_guard_timer, neg_ms);
    count_down(&mut d.v_shift_window_timer, neg_ms);
    count_down(&mut d.super_engine_timer, neg_ms);
    count_down(&mut d.clock_receive, neg_ms);
    count_down(&mut d.accel_tap_window_timer, neg_ms);
    if d.jump_ten_buffer > 0 {
        d.jump_ten_buffer = mips::sub_lo(d.jump_ten_buffer as i32, 1) as i16;
    }

    // === Hazard (blinking after being hit) ===
    // No rain cloud over the head (the item that makes one is not ported).
    let rain = rain_cloud::NONE;
    d.rain_cloud_effect = rain;
    let actions_in = d.actions_flag_set;
    let approx_speed = abs(d.speed_approx as i32);

    // What the original leaves in its scratch variable: the first hazard
    // that applies (clock, squish, or the slowing cloud's event time).
    let hazard = if d.clock_receive != 0 {
        Some(d.clock_receive as i32)
    } else if d.squish_timer != 0 {
        Some(d.squish_timer as i32)
    } else if rain == rain_cloud::SLOW && env.elapsed_event_time != 0 {
        Some(env.elapsed_event_time)
    } else {
        None
    };

    match hazard {
        None => {
            let mut timer = d.hazard_timer as i32;
            // Hazard timer will not go down unless you keep moving.
            if approx_speed >= 0x101 {
                timer = mips::add_lo(timer, neg_ms);
            }
            timer &= -2;
            if timer > -1 {
                timer = -2;
            }
            d.hazard_timer = timer as i16;
        }
        Some(hazard) => {
            let low_speed = approx_speed < 0x101;
            let timer = if low_speed {
                let timer = d.hazard_timer as i32;
                (if timer > 0 { mips::neg_lo(timer) } else { timer }) | 1
            } else if actions_in & actions::TOUCH_GROUND != 0 {
                // Clock waddle: rumble in time with the waddle.
                let clock = d.clock_receive as i32;
                let mut waddle = mips::sra(clock, 6);
                if waddle > 0x40 {
                    waddle = 0x40;
                }
                let trig = math::sin(mips::sll(clock, 4) as u32);
                waddle = mips::add_lo(waddle, mips::sra(trig, 5));
                waddle = mips::add_lo(waddle, mips::sra(approx_speed, 8).min(0x20));
                effects.push(Effect::ShockForce { driver_id: d.driver_id, a: 4, b: mips::add_lo(waddle, 0x18) as u8 });
                abs(hazard) | 1
            } else {
                mips::neg_lo(abs(hazard)) | 1
            };
            d.hazard_timer = timer as u16 as i16;
        }
    }

    // === Normal vector, and last tick's state ===
    d.actions_flag_set_prev_frame = actions_in;
    d.rot_prev[0] = d.rot_curr_x;
    d.rot_prev[1] = d.rot_curr_y;
    d.pos_prev = d.pos_curr;
    d.rot_prev[2] = d.rot_curr_z;
    d.jump_height_prev = d.jump_height_curr;
    d.turn_angle_prev = d.turn_angle_curr;

    let mut a = actions_in & ACTION_CARRY_MASK;

    // The retail ring of four normals has shrunk to one: the slot is always 0.
    d.axis_angle4_normal_vec = if a & actions::TOUCH_GROUND != 0 { d.axis_angle1_normal_vec } else { d.axis_angle2_normal_vec };
    d.normal_vec_id = 0;

    // === Mask weapon ===
    // Having a mask (an Aku Aku or Uka Uka thread under the driver's) is an action.
    if d.mask.is_some() {
        a |= actions::MASK_WEAPON;
    }

    // === Buttons ===
    let held = if env.end_of_race { 0 } else { input.buttons_held };
    let tapped = if env.end_of_race { 0 } else { input.buttons_tapped };
    let cross = held & buttons::CROSS != 0;
    let square = held & buttons::SQUARE != 0;

    // Tapping L1 or R1 starts a jump, and picks the button a drift is held with.
    let jump_taps = tapped & JUMP_BUTTON_MASK;
    if jump_taps != 0 && d.kart_state != kart_state::DRIFTING {
        d.button_used_to_start_drift = (if jump_taps == JUMP_BUTTON_MASK { buttons::R1 } else { jump_taps }) as i16;
        if rain != rain_cloud::JUMP_LOCKOUT {
            d.jump_ten_buffer = JUMP_BUFFER_FRAMES;
            a |= actions::JUMP_BUTTON_HELD;
        }
    } else if held & JUMP_BUTTON_MASK != 0 && rain != rain_cloud::JUMP_LOCKOUT {
        if a & actions::JUMP_BUTTON_HELD == 0 {
            d.jump_ten_buffer = JUMP_BUFFER_FRAMES;
        }
        a |= actions::JUMP_BUTTON_HELD;
    } else {
        a &= !actions::JUMP_BUTTON_HELD;
        if d.jump_ten_buffer > 0 {
            d.jump_ten_buffer = 0;
        }
    }

    // === Gas and brake ===
    let gas = gas_and_brake(d, input, env, a, approx_speed, cross, square);
    let mut a = gas.actions;
    let mut gas_target = gas.target;
    let mut target_base = gas.base;

    if d.accel_tap_window_timer == 0 || (d.kart_state != kart_state::NORMAL && d.kart_state != kart_state::ANTIVSHIFT) {
        d.accel_tap_count = 0;
    }

    // A change of direction of the gas counts as a tap.
    if (d.fire_speed > 0) != (gas_target > 0) {
        if d.accel_tap_window_timer != 0 {
            d.accel_tap_count = (d.accel_tap_count as u16).wrapping_add(1) as i16;
        }
        d.accel_tap_window_timer = ACCEL_TAP_WINDOW_MS;
    }
    d.fire_speed = gas_target as i16;

    // The terrain scales the target speed (unless braking).
    if a & (actions::MASK_WEAPON | actions::BRAKE_WITH_ACCEL) == 0 {
        let multiplier = TERRAINS[d.terrain_meta2 as usize].speed_multiplier;
        if multiplier != 0x100 {
            gas_target = mips::sra(mips::mul_lo(multiplier, gas_target), 8);
            target_base = mips::sra(mips::mul_lo(multiplier, target_base), 8);
        }
    }
    d.terrain_scaled_base_speed = target_base as i16;
    d.base_speed = gas_target as i16;

    // === Steering ===
    let stick_lx = if env.end_of_race { STICK_CENTER } else { input.stick_lx as i32 };
    let default_strength = mips::add_lo(d.stats.turn_rate as i32, mips::sll(d.turn_const as i32, 1) / 5);

    let steer = if d.accel_tap_count >= ACCEL_TAP_STEER_COUNT && approx_speed < STEER_ACCEL_TAP_SPEED_MAX {
        mips::neg_lo(strength_absolute(stick_lx, STEER_ACCEL_TAP_STRENGTH))
    } else if d.wall_rub_timer != 0 {
        mips::neg_lo(strength_absolute(stick_lx, STEER_WALL_RUB_STRENGTH))
    } else if a & (actions::ACCEL_PREVENTION | actions::BRAKE_WITH_ACCEL) == 0 {
        mips::neg_lo(strength_absolute(stick_lx, default_strength))
    } else if !cross {
        mips::neg_lo(strength_absolute(stick_lx, STEER_BRAKE_STRENGTH))
    } else {
        // Braking with the gas down: as speed increases, turn rate decreases.
        let class_speed = mips::sra(mips::sll(d.stats.speed_class_stat as u16 as i32, 16), 17);
        let strength = map_to_range(abs(d.speed as i32), BRAKE_HIGH_SPEED_THRESHOLD, class_speed, STEER_BRAKE_STRENGTH, default_strength);
        mips::neg_lo(strength_absolute(stick_lx, strength))
    };

    if steer == 0 {
        d.num_frames_spent_steering = STEER_RESET_FRAMES;
    } else if steer < 0 {
        if d.simp_turn_state >= 0 {
            a &= !actions::STEER_LEFT;
            d.num_frames_spent_steering = 0;
        }
    } else if d.simp_turn_state <= 0 {
        a |= actions::STEER_LEFT;
        d.num_frames_spent_steering = 0;
    }
    d.simp_turn_state = steer as i8;

    // The front wheels follow the stick.
    let wheel = strength_absolute(stick_lx, WHEEL_ROTATION_STRENGTH);
    d.wheel_rotation = interp_by_speed(d.wheel_rotation as i32, WHEEL_ROTATION_INTERP_STEP, mips::neg_lo(wheel)) as i16;

    d.actions_flag_set = a;
}
