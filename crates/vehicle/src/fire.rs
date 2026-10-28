//! `VehFire_Increment`: a boost. Adds to the reserves (the turbo's remaining
//! time) and raises the speed cap that goes with them, and lights the exhaust
//! fire (`turbo`) or restarts it.
//!
//! The sound is reported ([`Effect::FireAudio`]) where the reference plays it; its cooldown is
//! the listener's, as the reference's C oracle never runs it. The ghost tape is not recorded.

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

/// `VEH_FIRE_AUDIO_SFX`: the sound of a boost, which does not play again within its cooldown.
pub const AUDIO_SFX: u8 = 0xd;
pub const AUDIO_COOLDOWN_MS: i16 = 0xf0;
/// `VEH_FIRE_VOICELINE_HIGH_BOOST_ID` and its priority: a big boost makes the driver cheer.
pub const BIG_BOOST_VOICELINE: u8 = 0x10;
pub const VOICELINE_PRIORITY: u8 = 0x10;

/// `VehFire_Audio`, without its cooldown: the volume and distortion of the sound of a boost of
/// `fire_level`, louder and lower the bigger, and whether the driver cheers.
pub fn audio(fire_level: i32) -> (u8, u8, bool) {
    match fire_level {
        ..0x40 => (0x80, 0x94, false),
        0x40..0x80 => (0xc0, 0x80, false),
        _ => (0xff, 0x6c, true),
    }
}

/// `VehFire_Increment(driver, reserves, type, fireLevel)`
pub fn increment(
    d: &mut Driver,
    reserves: i32,
    ty: u32,
    fire_level: i32,
    effects: &mut Vec<Effect>,
) {
    // A turbo pad does nothing while holding the brake.
    if ty & turbo_type::TURBO_PAD != 0 && d.actions_flag_set & actions::ACCEL_PREVENTION != 0 {
        return;
    }

    if matches!(
        d.kart_state,
        kart_state::SPINNING | kart_state::BLASTED | kart_state::MASK_GRABBED
    ) {
        return;
    }

    // Clear the turbo input latch and mark an outside turbo.
    d.actions_flag_set = (d.actions_flag_set & !actions::TURBO_INPUT_LATCH) | actions::NEW_BOOST;

    // A fire that burns is reused; its pop (a short hidden spell) marks every boost
    // but a turbo pad's, which is one long boost.
    let player = d.actions_flag_set & actions::BOT == 0;
    match &mut d.turbo {
        Some(t) => {
            if ty & turbo_type::TURBO_PAD == 0 {
                t.hidden = true;
                t.fire_visibility_cooldown = VISIBILITY_COOLDOWN;
            }
            t.fire_disappear_countdown = -1;
            t.alpha_scale = 0;
            // One sound for a boost that goes on from the last tick (a turbo pad).
            if player && d.actions_flag_set_prev_frame & actions::NEW_BOOST == 0 {
                t.fire_audio_distort = 0;
                effects.push(Effect::FireAudio { fire_level });
            }
        }
        None => {
            d.turbo = Some(Turbo::new(ty & turbo_type::POWER_SLIDE_HANG_TIME != 0));
            if player && d.kart_state != kart_state::CRASHING {
                effects.push(Effect::FireAudio { fire_level });
            }
        }
    }

    let single = d.stats.single_turbo_speed as i32;
    let sacred = d.stats.sacred_fire_speed as i32;
    let new_speed_cap = mips::add_lo(
        mips::sra(
            mips::mul_lo(fire_level, mips::sub_lo(sacred, single)),
            FIRE_SPEED_CAP_SHIFT,
        ),
        single,
    );

    // Any gain in boost raises the cap; with the Sacred Fire speed, boosting
    // on something that is not a super turbo pad lowers it.
    if d.reserves == 0
        || (d.fire_speed_cap as i32) < new_speed_cap
        || (sacred < d.fire_speed_cap as i32
            && d.step_flag_set & step_triggers::SUPER_TURBO_PAD == 0)
    {
        d.fire_speed_cap = new_speed_cap as i16;
        if let Some(t) = &mut d.turbo {
            t.fire_size =
                ((fire_level >> FIRE_SIZE_SHIFT) + FIRE_SIZE_BASE).min(FIRE_SIZE_MAX) as i16;
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

    effects.push(Effect::CameraFireSpeedZoom {
        driver_id: d.driver_id,
    });
    effects.push(Effect::ShockForce {
        driver_id: d.driver_id,
        a: RUMBLE_CHANNEL,
        b: RUMBLE_FORCE,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boosts(effects: &[Effect]) -> Vec<i32> {
        effects
            .iter()
            .filter_map(|e| match *e {
                Effect::FireAudio { fire_level } => Some(fire_level),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_boost_sounds_once_however_long_a_turbo_pad_holds_it() {
        let mut d = Driver::new(0, 0);
        let mut effects = vec![];
        increment(&mut d, 0x100, turbo_type::TURBO_PAD, 0x80, &mut effects);
        assert_eq!(boosts(&effects), [0x80]);
        // The next tick on the pad: the last one boosted already.
        d.actions_flag_set_prev_frame = d.actions_flag_set;
        effects.clear();
        increment(&mut d, 0x100, turbo_type::TURBO_PAD, 0x80, &mut effects);
        assert!(boosts(&effects).is_empty());
        // Off the pad and on again, it sounds again and the fire's roar starts low again.
        d.actions_flag_set_prev_frame = 0;
        d.turbo.as_mut().unwrap().fire_audio_distort = 9;
        increment(&mut d, 0x100, turbo_type::TURBO_PAD, 0x80, &mut effects);
        assert_eq!(boosts(&effects), [0x80]);
        assert_eq!(d.turbo.unwrap().fire_audio_distort, 0);
    }

    #[test]
    fn a_crashing_kart_lights_its_fire_quietly() {
        let mut d = Driver::new(0, 0);
        d.kart_state = kart_state::CRASHING;
        let mut effects = vec![];
        increment(
            &mut d,
            0x100,
            turbo_type::START_LINE_BOOST,
            0x20,
            &mut effects,
        );
        assert!(d.turbo.is_some());
        assert!(boosts(&effects).is_empty());
    }

    #[test]
    fn a_bigger_boost_is_louder_and_lower_and_the_biggest_cheers() {
        assert_eq!(audio(0x20), (0x80, 0x94, false));
        assert_eq!(audio(0x40), (0xc0, 0x80, false));
        assert_eq!(audio(0x100), (0xff, 0x6c, true));
    }
}
