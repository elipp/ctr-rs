//! `VehPickState_NewState`: what a hit does to the victim. The spin-out, the squish and the burn
//! start `VehPhysProc_SpinFirst_Init` on the next tick. Not ported: the blast (the tumble), the
//! mask grab, the shield and the invincibility, the battle score and the attacker's stats.

use crate::{Driver, Effect, MatrixArray, actions, kart_state};

/// `VEH_PICK_DAMAGE_*`
pub mod damage {
    pub const NONE: u8 = 0;
    pub const SPIN: u8 = 1;
    pub const BLAST: u8 = 2;
    pub const SQUISH: u8 = 3;
    pub const BURN: u8 = 4;
    pub const MASK_GRAB: u8 = 5;
}

const VOICELINE_COMMON_DAMAGE: u8 = 1;
const VOICELINE_VICTIM_LAUGH: u8 = 2;
const VOICELINE_SQUISH: u8 = 4;
const VOICELINE_PRIORITY: u8 = 0x10;

const SPIN_NO_INPUT_TIMER: i16 = 0x3c0;
const SQUISH_NO_INPUT_TIMER: i16 = 0xf0;
const SQUISH_TIMER: i16 = 0xf00;
const BURN_NO_INPUT_TIMER: i16 = 0x780;
const BURN_TIMER: i16 = 0xf00;

const SOUND_SQUISH: u8 = 0x5a;
const SOUND_BURN: u8 = 0x69;

const RUMBLE_FRAMES: u8 = 8;
const RUMBLE_FORCE: u8 = 0x7f;

/// `VehPickState_NewState` for a hit without an attacker. The result is the game's: whether the
/// driver was hurt.
pub fn new_state(d: &mut Driver, damage: u8, effects: &mut Vec<Effect>) -> bool {
    let voice = |id| Effect::Voiceline { id, driver_id: d.driver_id, priority: VOICELINE_PRIORITY };

    if d.kart_state == kart_state::MASK_GRABBED {
        return false;
    }
    // The Aku Aku mask shrugs it off.
    if d.actions_flag_set & actions::MASK_WEAPON != 0 {
        effects.push(voice(VOICELINE_VICTIM_LAUGH));
        return false;
    }

    let spin_out = match damage {
        damage::NONE => return true,
        // The tumble is not ported.
        damage::BLAST | damage::MASK_GRAB => return false,
        damage::SQUISH => {
            if d.kart_state != kart_state::SPINNING {
                effects.push(Effect::Sound { id: SOUND_SQUISH, echo: d.actions_flag_set & actions::ENGINE_ECHO != 0 });
                effects.push(voice(VOICELINE_SQUISH));
            }
            d.squish_timer = SQUISH_TIMER;
            d.no_input_timer = SQUISH_NO_INPUT_TIMER;
            true
        }
        damage::BURN => {
            if d.burn_timer == 0 {
                effects.push(Effect::Sound { id: SOUND_BURN, echo: false });
                effects.push(voice(VOICELINE_COMMON_DAMAGE));
            }
            d.burn_timer = BURN_TIMER;
            d.no_input_timer = BURN_NO_INPUT_TIMER;
            true
        }
        // `VEH_PICK_DAMAGE_SPIN`: a kart that spins already only has its timer reset.
        _ => {
            d.no_input_timer = SPIN_NO_INPUT_TIMER;
            d.kart_state != kart_state::SPINNING
        }
    };
    if spin_out {
        d.pending_spin = true;
        d.init_pending = true;
    }

    d.kart_state = kart_state::NORMAL;
    d.reserves = 0;
    d.turbo_outside_timer = 0;
    d.matrix_array = MatrixArray::None;
    d.matrix_index = 0;

    effects.push(Effect::ShockFreq { driver_id: d.driver_id, a: RUMBLE_FRAMES, b: 0 });
    effects.push(Effect::ShockForce { driver_id: d.driver_id, a: RUMBLE_FRAMES, b: RUMBLE_FORCE });
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn driving() -> Driver {
        Driver { kart_state: kart_state::NORMAL, reserves: 5, ..Driver::default() }
    }

    #[test]
    fn a_spin_starts_the_spin_out_next_tick() {
        let mut d = driving();
        let mut effects = Vec::new();
        assert!(new_state(&mut d, damage::SPIN, &mut effects));
        assert!((d.pending_spin, d.init_pending, d.no_input_timer, d.reserves) == (true, true, SPIN_NO_INPUT_TIMER, 0));
        assert!(effects.iter().any(|e| matches!(e, Effect::ShockForce { .. })));
    }

    #[test]
    fn a_spin_does_not_restart_one_in_progress() {
        let mut d = driving();
        d.kart_state = kart_state::SPINNING;
        assert!(new_state(&mut d, damage::SPIN, &mut Vec::new()));
        assert!(!d.pending_spin);
        assert_eq!((d.no_input_timer, d.kart_state), (SPIN_NO_INPUT_TIMER, kart_state::NORMAL));
    }

    #[test]
    fn a_squish_and_a_burn_set_their_timers() {
        let mut d = driving();
        assert!(new_state(&mut d, damage::SQUISH, &mut Vec::new()));
        assert_eq!((d.squish_timer, d.no_input_timer, d.pending_spin), (SQUISH_TIMER, SQUISH_NO_INPUT_TIMER, true));
        let mut d = driving();
        let mut effects = Vec::new();
        assert!(new_state(&mut d, damage::BURN, &mut effects));
        assert_eq!((d.burn_timer, d.no_input_timer), (BURN_TIMER, BURN_NO_INPUT_TIMER));
        assert!(effects.contains(&Effect::Sound { id: SOUND_BURN, echo: false }));
        // Already burning: no new sound.
        let mut effects = Vec::new();
        new_state(&mut d, damage::BURN, &mut effects);
        assert!(!effects.iter().any(|e| matches!(e, Effect::Sound { .. })));
    }

    #[test]
    fn the_mask_shrugs_a_hit_off() {
        let mut d = driving();
        d.actions_flag_set |= actions::MASK_WEAPON;
        assert!(!new_state(&mut d, damage::SPIN, &mut Vec::new()));
        assert!(!d.pending_spin);
    }
}
