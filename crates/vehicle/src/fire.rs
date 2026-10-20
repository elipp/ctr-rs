//! `VehFire_Increment`: a boost. Adds to the reserves (the turbo's remaining
//! time) and raises the speed cap that goes with them, and lights the exhaust
//! fire (`turbo`) or restarts it.
//!
//! Not modelled: the sound, which only has the audio cooldown for state. The
//! ghost tape is not recorded either.

use math::mips;

use crate::turbo::Turbo;
use crate::{Driver, Effect, actions, kart_state, step_triggers};

/// `TurboType`
pub mod turbo_type {
    pub const START_LINE_BOOST: u32 = 0;
    pub const FREEZE_RESERVES_ON_TURBO_PAD: u32 = 0x1;
    pub const POWER_SLIDE_HANG_TIME: u32 = 0x2;
    pub const TURBO_PAD: u32 = 0x4;
    pub const TURBO_ITEM: u32 = 0x8;
    pub const SUPER_ENGINE: u32 = 0x10;
}

const FIRE_SPEED_CAP_SHIFT: u32 = 8;
const VISIBILITY_COOLDOWN: i16 = 0x60;
const FIRE_SIZE_SHIFT: u32 = 6;
const FIRE_SIZE_BASE: i32 = 5;
const FIRE_SIZE_MAX: i32 = 8;
const RUMBLE_CHANNEL: u8 = 8;
const RUMBLE_FORCE: u8 = 0x7f;

/// `VehFire_Increment(driver, reserves, type, fireLevel)`
pub fn increment(d: &mut Driver, reserves: i32, ty: u32, fire_level: i32, effects: &mut Vec<Effect>) {
    // A turbo pad does nothing while holding the brake.
    if ty & turbo_type::TURBO_PAD != 0 && d.actions_flag_set & actions::ACCEL_PREVENTION != 0 {
        return;
    }

    if matches!(d.kart_state, kart_state::SPINNING | kart_state::BLASTED | kart_state::MASK_GRABBED) {
        return;
    }

    // Clear the turbo input latch and mark an outside turbo.
    d.actions_flag_set = (d.actions_flag_set & !actions::TURBO_INPUT_LATCH) | actions::NEW_BOOST;

    // A fire that burns is reused; its pop (a short hidden spell) marks every boost
    // but a turbo pad's, which is one long boost.
    match &mut d.turbo {
        Some(t) => {
            if ty & turbo_type::TURBO_PAD == 0 {
                t.hidden = true;
                t.fire_visibility_cooldown = VISIBILITY_COOLDOWN;
            }
            t.fire_disappear_countdown = -1;
            t.alpha_scale = 0;
        }
        None => d.turbo = Some(Turbo::new(ty & turbo_type::POWER_SLIDE_HANG_TIME != 0)),
    }

    let single = d.stats.single_turbo_speed as i32;
    let sacred = d.stats.sacred_fire_speed as i32;
    let new_speed_cap = mips::add_lo(mips::sra(mips::mul_lo(fire_level, mips::sub_lo(sacred, single)), FIRE_SPEED_CAP_SHIFT), single);

    // Any gain in boost raises the cap; with the Sacred Fire speed, boosting
    // on something that is not a super turbo pad lowers it.
    if d.reserves == 0
        || (d.fire_speed_cap as i32) < new_speed_cap
        || (sacred < d.fire_speed_cap as i32 && d.step_flag_set & step_triggers::SUPER_TURBO_PAD == 0)
    {
        d.fire_speed_cap = new_speed_cap as i16;
        if let Some(t) = &mut d.turbo {
            t.fire_size = ((fire_level >> FIRE_SIZE_SHIFT) + FIRE_SIZE_BASE).min(FIRE_SIZE_MAX) as i16;
        }
    }

    if ty & turbo_type::TURBO_ITEM != 0 {
        d.actions_flag_set |= actions::TURBO_ITEM;
    }

    if ty & turbo_type::FREEZE_RESERVES_ON_TURBO_PAD != 0 {
        // Adds the reserves on the first frame on the turbo pad, then holds
        // them until the first frame off it.
        let old_outside_timer = d.turbo_outside_timer as i32;
        let delta = mips::sub_lo(reserves, d.turbo_outside_timer as u16 as i32);
        if old_outside_timer < reserves {
            d.reserves = mips::add_lo(d.reserves as u16 as i32, delta) as i16;
            d.turbo_outside_timer = reserves as i16;
        }
    } else if ty & turbo_type::SUPER_ENGINE == 0 {
        // Start line, hang time, powerslide.
        d.reserves = mips::add_lo(d.reserves as u16 as i32, reserves) as i16;
    } else if (d.reserves as i32) < reserves {
        d.reserves = reserves as i16;
    }

    effects.push(Effect::CameraFireSpeedZoom { driver_id: d.driver_id });
    effects.push(Effect::ShockForce { driver_id: d.driver_id, a: RUMBLE_CHANNEL, b: RUMBLE_FORCE });
}
