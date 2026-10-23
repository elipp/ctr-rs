//! The hang-time turbo: `UI_JumpMeter_Update` (`game/UI/UI_Meter.c`).
//!
//! Gameplay that retail keeps in the HUD code: it runs once a frame for every
//! human driver, after the drivers' tick. While the kart is in the air the meter
//! follows its hang time (`jump_landing_boost`, counted in milliseconds by the
//! ground search); on landing, a long enough hang time boosts the kart: the
//! longer, the bigger the boost.
//!
//! Not kept: the end-of-race jump statistics (`timeSpentJumping`, `highestJump`).

use math::mips;

use crate::fire::{self, turbo_type};
use crate::{Driver, Effect, Env, actions};

/// Hang time (ms) of a boost of any size.
const HANG_TIME_SMALL: i32 = 640;
const HANG_TIME_MEDIUM: i32 = 960;
const HANG_TIME_LARGE: i32 = 1440;
const HANG_TIME_RESERVES: i32 = 960;
const FIRE_SMALL: i32 = 0;
const FIRE_MEDIUM: i32 = 0x80;
const FIRE_LARGE: i32 = 0x100;
/// Landing after more than this much hang time makes the driver comment.
const LANDING_VOICELINE_MIN: i32 = 0x480;
const LANDING_VOICELINE_CUTOFF: i32 = 0x481;
const LANDING_VOICELINE_ID: u8 = 7;
const LANDING_VOICELINE_PRIORITY: u8 = 0x10;
const METER_MAX: i32 = 0x960;
const METER_DISPLAY_TIMER: i16 = 0x5a0;

/// `UI_JumpMeter_Update`
pub fn update(d: &mut Driver, env: &Env, effects: &mut Vec<Effect>) {
    if d.actions_flag_set & actions::AIRBORNE == 0 {
        if d.actions_flag_set_prev_frame & actions::AIRBORNE == 0 {
            // On the ground: the meter, kept for the HUD, runs out.
            if d.jump_meter_timer == 0 {
                d.jump_meter = 0;
            } else {
                d.jump_meter_timer =
                    mips::sub_lo(d.jump_meter_timer as i32, env.elapsed_time_ms) as i16;
                if d.jump_meter_timer < 0 {
                    d.jump_meter_timer = 0;
                }
            }
        } else if d.jump_meter as i32 >= HANG_TIME_SMALL {
            // Just landed.
            let mut fire_level = FIRE_SMALL;
            if d.jump_meter as i32 >= HANG_TIME_MEDIUM {
                fire_level = FIRE_MEDIUM;
            }
            if d.jump_meter as i32 >= HANG_TIME_LARGE {
                fire_level = FIRE_LARGE;
            }
            fire::increment(
                d,
                HANG_TIME_RESERVES,
                turbo_type::POWER_SLIDE_HANG_TIME,
                fire_level,
                effects,
            );
        }
    } else {
        if LANDING_VOICELINE_MIN < d.jump_landing_boost as i32
            && (d.jump_meter as i32) < LANDING_VOICELINE_CUTOFF
        {
            effects.push(Effect::Voiceline {
                id: LANDING_VOICELINE_ID,
                driver_id: d.driver_id,
                priority: LANDING_VOICELINE_PRIORITY,
            });
        }

        d.jump_meter = (d.jump_landing_boost as i32).min(METER_MAX) as i16;
        d.jump_meter_timer = METER_DISPLAY_TIMER;
    }
}
