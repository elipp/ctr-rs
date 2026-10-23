//! `VehPhysGeneral_JumpAndFriction`: accelerates the kart along its heading,
//! starts jumps, and turns the new velocity back into the driver's speeds.
//!
//! Not ported: the Spring item's jump (the driver holds no item).

use gte::{Gte, data};
use loader::lev::Level;
use math::{Vec3, mips};

use crate::calc::{fast_sqrt, map_to_range};
use crate::force::{mvmva, set_rot_matrix};
use crate::terrain::{TERRAINS, terrain_flags};
use crate::{Driver, Effect, Env, actions, crash, forced_jump, kart_state, step_triggers};

const NORMAL_Y_MIN: i32 = 0x15;
const TURBO_PAD_ACCEL: i32 = 8000;
const REVERSE_SLIDE_SPEED_COMPARE: i32 = 0x2ff;
const JUMP_HIGH_TIMER_MS: i16 = 0x180;
const JUMP_FORCED_MS: i16 = 0xa0;
const JUMP_COOLDOWN_MS: i16 = 0x180;
const RUMBLE_CHANNEL: u8 = 8;
const RUMBLE_FORCE: u8 = 0x7f;
const JUMP_NORMAL_SFX: u8 = 8;
const JUMP_FORCED_SFX: u8 = 0x7e;
const VERTICAL_SPEED_DEFAULT: i32 = 0x3700;
const VERTICAL_SPEED_MAX: i32 = 0x5000;
const SPEEDOMETER_REVERSE_THRESHOLD: i32 = 0x100;

fn abs(value: i32) -> i32 {
    if value < 0 {
        mips::neg_lo(value)
    } else {
        value
    }
}

fn div2_toward_zero(value: i32) -> i32 {
    mips::sra(mips::add_lo(value, (value as u32 >> 31) as i32), 1)
}

/// `VehPhysGeneral_JumpGetVelY`: the vertical speed (in the plane of the
/// surface) the horizontal velocity has, for the surface with `normal`.
fn jump_get_vel_y(normal: loader::lev::SVec3, speed: &Vec3) -> i32 {
    let normal_y = normal.y as i32;
    if abs(normal_y) < NORMAL_Y_MIN {
        return 0;
    }
    let dot = mips::add_lo(
        mips::mul_lo(speed.x, normal.x as i32),
        mips::mul_lo(speed.z, normal.z as i32),
    );
    mips::div(dot, normal_y)
}

fn rotated(gte: &mut Gte) -> Vec3 {
    Vec3 {
        x: gte.mfc2(data::MAC1) as i32,
        y: gte.mfc2(data::MAC2) as i32,
        z: gte.mfc2(data::MAC3) as i32,
    }
}

/// `VehPhysGeneral_JumpAndFriction`
pub fn jump_and_friction(
    d: &mut Driver,
    gte: &mut Gte,
    env: &Env,
    level: &Level,
    effects: &mut Vec<Effect>,
) {
    set_rot_matrix(gte, &d.matrix_moving_dir);

    // Turning scrubs speed off the base speed.
    if d.kart_state != kart_state::DRIFTING
        && d.actions_flag_set & actions::MASK_WEAPON == 0
        && d.reserves == 0
    {
        let amp_turn = abs(mips::sra(d.amp_turn_state as i32, 8));
        let mapped = map_to_range(
            amp_turn,
            0,
            d.stats.backward_turn_rate as i32,
            0,
            d.stats.turn_decrease_rate as i32,
        );
        let base_speed = d.base_speed as i32;
        let mut turn_decrease = mapped.min(abs(base_speed));
        if base_speed < 0 {
            turn_decrease = mips::neg_lo(turn_decrease);
        }
        d.base_speed = mips::sub_lo(base_speed, turn_decrease) as i16;
    }

    if d.wall_rub_timer != 0 {
        if d.wall_rub_speed_limit < d.base_speed {
            d.base_speed = d.wall_rub_speed_limit;
        }
        if (d.base_speed as i32) < mips::neg_lo(d.wall_rub_speed_limit as i32) {
            d.base_speed = mips::neg_lo(d.wall_rub_speed_limit as u16 as i32) as i16;
        }
    }

    let mut movement = d.velocity;
    let mut speed_loss = 0;
    let mut jump = false;

    if d.actions_flag_set & actions::TOUCH_GROUND != 0 {
        let terrain = &TERRAINS[d.terrain_meta1 as usize];
        let base_speed = d.base_speed as i32;
        let speed_approx = d.speed_approx as i32;
        let on_turbo_pad = d.step_flag_set & step_triggers::TURBO_PAD_MASK != 0;

        // The acceleration along the heading.
        let mut acceleration = 0;
        if on_turbo_pad && base_speed > 0 {
            acceleration = TURBO_PAD_ACCEL;
        } else if base_speed != 0 {
            let sliding_back = (terrain.flags & terrain_flags::ACCEL_WHILE_REVERSE_SLIDING == 0
                || base_speed < 1
                || speed_approx >= 0)
                && abs(speed_approx) > REVERSE_SLIDE_SPEED_COMPARE
                && (base_speed < 1 || speed_approx < 1)
                && (base_speed >= 0 || speed_approx >= 0);
            // No acceleration while the pedal and the motion disagree at speed.
            if !sliding_back {
                acceleration = mips::add_lo(
                    d.stats.accel_class_stat as i32,
                    mips::sll(d.accel_const as i32, 5) / 5,
                );
                if !on_turbo_pad {
                    if d.reserves != 0 && base_speed > 0 {
                        acceleration = d.stats.accel_reserves as i32;
                    }
                    let slow_until_speed = terrain.slow_until_speed;
                    if slow_until_speed != 0x100 && d.actions_flag_set & actions::MASK_WEAPON == 0 {
                        acceleration = mips::sra(mips::mul_lo(slow_until_speed, acceleration), 8);
                    }
                } else if base_speed > 0 {
                    acceleration = TURBO_PAD_ACCEL;
                }
            }
        }

        acceleration = mips::sra(mips::mul_lo(acceleration, env.elapsed_time_ms), 5);
        gte.mtc2(0, data::VXY0);
        gte.mtc2(acceleration as u16 as u32, data::VZ0);
        gte.cop2(mvmva(1, 0, 0, 3, 0));
        let push = rotated(gte);

        if d.base_speed >= 0 {
            d.forward_accel_impulse = acceleration as i16;
            movement.x = mips::add_lo(movement.x, push.x);
            movement.y = mips::add_lo(movement.y, push.y);
            movement.z = mips::add_lo(movement.z, push.z);
            d.forward_accel_vector = loader::lev::SVec3 {
                x: push.x as i16,
                y: push.y as i16,
                z: push.z as i16,
            };
        } else {
            d.forward_accel_impulse = mips::neg_lo(acceleration) as i16;
            movement.x = mips::sub_lo(movement.x, push.x);
            movement.z = mips::sub_lo(movement.z, push.z);
            movement.y = mips::sub_lo(movement.y, push.y);
            d.forward_accel_vector = loader::lev::SVec3 {
                x: mips::neg_lo(push.x) as i16,
                y: mips::neg_lo(push.y) as i16,
                z: mips::neg_lo(push.z) as i16,
            };
        }

        // The speed lost is what the acceleration added beyond the base speed.
        let length_sq = mips::add_lo(
            mips::add_lo(
                mips::mul_lo(movement.x, movement.x),
                mips::mul_lo(movement.y, movement.y),
            ),
            mips::mul_lo(movement.z, movement.z),
        ) as u32;
        let speed_magnitude = fast_sqrt(length_sq, 0x10) as i32;
        speed_loss = mips::sub_lo(
            (speed_magnitude as u32 >> 8) as i32,
            abs(d.base_speed as i32),
        );
        if speed_loss < 0 {
            speed_loss = 0;
        }
        if acceleration < speed_loss {
            speed_loss = acceleration;
        }

        if d.jump_forced_ms != 0 {
            if d.jump_high_jump_timer_ms != 0 {
                d.jump_high_jump_timer_ms = JUMP_HIGH_TIMER_MS;
            }
            if d.kart_state == kart_state::BLASTED {
                effects.push(Effect::ShockFreq {
                    driver_id: d.driver_id,
                    a: RUMBLE_CHANNEL,
                    b: 0,
                });
                effects.push(Effect::ShockForce {
                    driver_id: d.driver_id,
                    a: RUMBLE_CHANNEL,
                    b: RUMBLE_FORCE,
                });
            }
            jump = true;
        }
    }

    if !jump {
        let jump_force = d.stats.jump_force as i32;
        let echo = d.actions_flag_set & actions::ENGINE_ECHO != 0;
        if d.forced_jump_type != forced_jump::NONE {
            if d.jump_forced_ms == 0 || d.jump_initial_vel_y as i32 == jump_force {
                effects.push(Effect::Sound {
                    id: JUMP_FORCED_SFX,
                    echo: false,
                });
            }
            d.jump_forced_ms = JUMP_FORCED_MS;
            if d.forced_jump_type == forced_jump::HIGH {
                d.jump_high_jump_timer_ms = JUMP_HIGH_TIMER_MS;
                d.jump_initial_vel_y = mips::add_lo(mips::sll(jump_force, 1), jump_force) as i16;
            } else {
                d.jump_initial_vel_y =
                    div2_toward_zero(mips::add_lo(mips::sll(jump_force, 1), jump_force)) as i16;
            }
            d.forced_jump_type = forced_jump::NONE;
            jump = true;
        } else if d.jump_coyote_timer_ms != 0 && d.jump_ten_buffer != 0 && d.jump_cooldown_ms == 0 {
            d.jump_forced_ms = JUMP_FORCED_MS;
            d.number_of_jumps = (d.number_of_jumps as u16).wrapping_add(1) as i16;
            d.jump_initial_vel_y = jump_force as i16;
            effects.push(Effect::Sound {
                id: JUMP_NORMAL_SFX,
                echo,
            });
            jump = true;
        }
    }

    if jump {
        d.jump_cooldown_ms = JUMP_COOLDOWN_MS;
        d.jump_ten_buffer = 0;
        d.actions_flag_set |= actions::JUMP_STARTED | actions::TURBO_INPUT_LATCH;

        // The strongest vertical speed over the surface normals (the last
        // tick's, then this tick's).
        let mut best = 0;
        let mut consider = |vel_y: i32| {
            if abs(best) < abs(vel_y) {
                best = vel_y;
            }
        };
        consider(jump_get_vel_y(d.axis_angle4_normal_vec, &movement));
        let normal = if d.actions_flag_set & actions::TOUCH_GROUND != 0 {
            d.axis_angle1_normal_vec
        } else {
            d.axis_angle2_normal_vec
        };
        consider(jump_get_vel_y(normal, &movement));

        let initial = d.jump_initial_vel_y as i32;
        let vertical_speed = fast_sqrt(
            mips::sra(
                mips::add_lo(mips::mul_lo(best, best), mips::mul_lo(initial, initial)),
                8,
            ) as u32,
            8,
        ) as i32;

        let mut max_vertical = mips::sll(level.jump_vertical_speed_cap as i32, 8);
        let mut kick = mips::sub_lo(vertical_speed, best);
        if max_vertical == 0 {
            max_vertical = VERTICAL_SPEED_DEFAULT;
        } else if max_vertical > VERTICAL_SPEED_MAX {
            max_vertical = VERTICAL_SPEED_MAX;
        }
        if max_vertical < kick {
            kick = max_vertical;
        }
        if movement.y < kick {
            movement.y = kick;
        }
    } else if d.actions_flag_set & actions::TOUCH_GROUND != 0 {
        // On a surface with anti-gravity (`mulNormVecY`), speed pushes the kart up.
        if let Some(under) = d.under_driver {
            let anti_gravity = level.quad_blocks[under as usize].mul_norm_vec_y as i32;
            if anti_gravity != 0 {
                let velocity = mips::sra(mips::mul_lo(anti_gravity, abs(d.speed_approx as i32)), 8);
                gte.mtc2(mips::sll(velocity, 16) as u32, data::VXY0);
                gte.mtc2(0, data::VZ0);
                gte.cop2(mvmva(1, 0, 0, 3, 0));
                let push = rotated(gte);
                movement.x = mips::add_lo(movement.x, push.x);
                movement.y = mips::add_lo(movement.y, push.y);
                movement.z = mips::add_lo(movement.z, push.z);
            }
        }
    }

    crash::convert_vec_to_speed(d, &movement);

    d.speed = mips::sub_lo(d.speed as u16 as i32, speed_loss) as i16;
    if d.speed < 0 {
        d.speed = 0;
    }

    // The speedometer eases towards the speed.
    let speed_approx = d.speed_approx as i32;
    let needle = d.speedometer_needle_value as i32;
    d.speedometer_needle_value = if speed_approx < 0 {
        if abs(speed_approx) < SPEEDOMETER_REVERSE_THRESHOLD {
            mips::sub_lo(needle, mips::sra(needle, 3)) as i16
        } else {
            // Shake the needle when going backwards.
            let timer = (env.timer & 7) as i32;
            let shake = mips::add_lo(mips::sll(timer, 9), mips::sll(timer, 8));
            (mips::add_lo(mips::mul_lo(needle, 13), shake) as u32 >> 4) as i16
        }
    } else {
        mips::sra(
            mips::add_lo(mips::mul_lo(needle, 13), mips::mul_lo(speed_approx, 3)),
            4,
        ) as i16
    };
}
