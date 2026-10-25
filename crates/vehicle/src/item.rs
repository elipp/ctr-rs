//! The weapon a driver holds (`heldItemID`) and the roulette that picks it: a weapon crate starts
//! the roulette, which runs for [`ROLL_TICKS`] and then settles on an item.
//!
//! Firing is ported for the turbo, the mask, and the bomb and the missile (which the `weapon` crate
//! flies: see [`Launch`]). The rest stay held until they are ported (see [`fireable`]). The pick is a placeholder: `VehPhysGeneral_SetHeldItem`'s fallback for a driver without a
//! place (`MixRNG_Scramble() % 12`), not the item sets by place that retail rolls from.

use crate::fire::{self, turbo_type};
use crate::{Driver, Effect, actions, kart_state, rain_cloud, stuck};

/// `HELD_ITEM_*`
pub mod held {
    pub const TURBO: u8 = 0;
    pub const BOMB_1X: u8 = 1;
    pub const MISSILE_1X: u8 = 2;
    pub const TNT: u8 = 3;
    pub const POTION: u8 = 4;
    pub const SPRING: u8 = 5;
    pub const SHIELD: u8 = 6;
    pub const MASK: u8 = 7;
    pub const CLOCK: u8 = 8;
    pub const WARPBALL: u8 = 9;
    pub const BOMB_3X: u8 = 0xa;
    pub const MISSILE_3X: u8 = 0xb;
    pub const NONE: u8 = 0xf;
    /// The weapon is being picked.
    pub const ROULETTE: u8 = 0x10;
}

/// `driver->itemRollTimer` after a weapon crate: 3 seconds of the game's 30 ticks.
pub const ROLL_TICKS: i16 = 90;

/// `VEH_PHYS_PROC_ITEM_ROLL_FAST_STOP_FRAMES`: a press of the fire button once this much of the
/// roulette has run (the timer is below it) stops the roulette on the next tick.
const ROLL_FAST_STOP_TICKS: i16 = 70;

/// `VEH_PHYS_PROC_WEAPON_COOLDOWN_STACK_FRAMES`: after a shot of a triple weapon.
const COOLDOWN_STACK_TICKS: i16 = 5;

/// `VEH_PHYS_PROC_WEAPON_COOLDOWN_EMPTY_FRAMES`: after the last shot, when the weapon goes.
const COOLDOWN_EMPTY_TICKS: i16 = 0x1e;

/// `TURBO_ITEM_RESERVES`, and the boost without and with 10 wumpa fruit.
const TURBO_RESERVES: i32 = 0x960;
const TURBO_BOOST_NORMAL: i32 = 0x80;
const TURBO_BOOST_JUICED: i32 = 0x100;
const WUMPA_JUICED_COUNT: i8 = 10;

/// `HELD_ITEM_STACK_COUNT`: what a triple bomb or missile holds.
const STACK_COUNT: u8 = 3;

/// `ITEMSET_FALLBACK_ITEM_COUNT`: items 0 to `MISSILE_3X`.
const FALLBACK_ITEM_COUNT: i32 = held::MISSILE_3X as i32 + 1;

/// A driver with nothing in hand.
pub fn empty(d: &mut Driver) {
    d.held_item_id = held::NONE;
}

/// `RB_CrateWeapon_ThCollide`'s reward for a player that broke a weapon crate: the roulette starts,
/// unless the driver has a weapon already (one that is not on its cooldown), is firing, or is under
/// a cloud that stops the roll. A clock that was thrown at the driver spends the crate for nothing.
pub fn start_roulette(d: &mut Driver) {
    if d.held_item_id != held::NONE && d.no_item_timer == 0
        || d.num_held_items != 0
        || d.actions_flag_set & actions::WEAPON_FIRE_REQUEST != 0
        || d.rain_cloud_effect == rain_cloud::ITEM_ROLL
        || d.clock_receive != 0
    {
        return;
    }
    d.held_item_id = held::ROULETTE;
    d.num_times_hit_weapon_box += 1;
    d.item_roll_timer = ROLL_TICKS;
    d.no_item_timer = 0;
}

/// The roulette, one game tick (`VehPhysProc`'s item roll): it counts down and then picks.
/// `random_number` is `sdata->randomNumber`.
pub fn tick(d: &mut Driver, random_number: &mut i32) {
    if d.held_item_id != held::ROULETTE {
        return;
    }
    if d.item_roll_timer != 0 {
        d.item_roll_timer -= 1;
        return;
    }
    set_held_item(d, random_number);
}

/// The weapons that can be fired so far. A driver that holds another one presses the fire button
/// for nothing, so that it does not lose the weapon to a cooldown that fires nothing.
pub fn fireable(item: u8) -> bool {
    matches!(
        item,
        held::TURBO
            | held::MASK
            | held::BOMB_1X
            | held::BOMB_3X
            | held::MISSILE_1X
            | held::MISSILE_3X
    )
}

/// A weapon that flies, which `VehPickupItem_ShootNow` has the caller launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Launch {
    Bomb,
    Missile,
}

/// `SOUND_BOMB_LAUNCH`, `SOUND_MISSILE_LAUNCH`, `VOICELINE_BOMB_LAUNCH`, `VOICELINE_MISSILE_LAUNCH`,
/// `VOICELINE_WEAPON_PRIORITY`, `WEAPON_GAMEPAD_RUMBLE_FRAMES` and `_FORCE`
const BOMB_SOUND: u8 = 0x47;
const MISSILE_SOUND: u8 = 0x4a;
const BOMB_VOICELINE: u8 = 10;
const MISSILE_VOICELINE: u8 = 11;
const VOICELINE_PRIORITY: u8 = 0x10;
const RUMBLE_FRAMES: u8 = 8;
const RUMBLE_FORCE: u8 = 0x7f;

/// What firing a [`Launch`] makes the driver feel and say, once the weapon is away.
pub fn launch_effects(d: &Driver, launch: Launch, effects: &mut Vec<Effect>) {
    effects.push(Effect::ShockFreq {
        driver_id: d.driver_id,
        a: RUMBLE_FRAMES,
        b: 0,
    });
    effects.push(Effect::ShockForce {
        driver_id: d.driver_id,
        a: RUMBLE_FRAMES,
        b: RUMBLE_FORCE,
    });
    let (id, voice) = match launch {
        Launch::Bomb => (BOMB_SOUND, BOMB_VOICELINE),
        Launch::Missile => (MISSILE_SOUND, MISSILE_VOICELINE),
    };
    effects.push(Effect::Sound {
        id,
        echo: d.actions_flag_set & actions::ENGINE_ECHO != 0,
    });
    effects.push(Effect::Voiceline {
        id: voice,
        driver_id: d.driver_id,
        priority: VOICELINE_PRIORITY,
    });
}

/// `VehPhysProc`'s "Item Used By Player", for the driver whose fire button was tapped: asks to fire
/// the weapon in hand (`WEAPON_FIRE_REQUEST`, which [`shoot_on_circle_press`] answers on the next
/// tick), and starts the cooldown after which the weapon is gone. Returns the actions with the
/// request.
///
/// `tapped_fire` is the fire button's tap, which counts only while the kart drives normally.
/// (The spring, and the bomb and the bubble that are still held after being thrown, are not here.)
pub fn request_fire(d: &mut Driver, tapped_fire: bool, actions_flag_set: u32) -> u32 {
    if !(tapped_fire
        && matches!(
            d.kart_state,
            kart_state::NORMAL | kart_state::DRIFTING | kart_state::ANTIVSHIFT
        ))
    {
        return actions_flag_set;
    }
    // A bomb in flight goes off instead.
    if d.bomb_thrown {
        d.bomb_thrown = false;
        d.detonate_bomb = true;
        return actions_flag_set;
    }
    let can_fire = d.item_roll_timer == 0
        && fireable(d.held_item_id)
        && d.no_item_timer == 0
        && d.rain_cloud_effect != rain_cloud::ITEM_ROLL
        && d.clock_receive == 0;
    if !can_fire {
        // The button also hurries a roulette that has run for long enough.
        if d.item_roll_timer < ROLL_FAST_STOP_TICKS {
            d.item_roll_timer = 0;
        }
        return actions_flag_set;
    }
    if d.num_held_items != 0 {
        d.num_held_items -= 1;
        d.no_item_timer = COOLDOWN_STACK_TICKS;
    } else {
        d.no_item_timer = COOLDOWN_EMPTY_TICKS;
    }
    actions_flag_set | actions::WEAPON_FIRE_REQUEST
}

/// `VehPhysProc`'s count of `noItemTimer`: when it runs out and the last shot is gone, so is the
/// weapon.
pub fn count_cooldown(d: &mut Driver) {
    if d.no_item_timer == 0 {
        return;
    }
    if d.no_item_timer == 1 && d.num_held_items == 0 {
        d.held_item_id = held::NONE;
    }
    d.no_item_timer -= 1;
}

/// `VehPickupItem_ShootOnCirclePress`: the driver that asked to fire fires. A weapon that flies
/// is for the caller to launch.
pub fn shoot_on_circle_press(d: &mut Driver, effects: &mut Vec<Effect>) -> Option<Launch> {
    if d.actions_flag_set & actions::WEAPON_FIRE_REQUEST == 0 {
        return None;
    }
    d.actions_flag_set &= !actions::WEAPON_FIRE_REQUEST;
    shoot_now(d, d.held_item_id, effects)
}

/// `VehPickupItem_ShootNow`, for the weapons that are ported. (Retail turns the triple and the
/// single bomb and missile into one case, then looks at what the driver holds again.)
fn shoot_now(d: &mut Driver, weapon: u8, effects: &mut Vec<Effect>) -> Option<Launch> {
    match weapon {
        held::BOMB_1X | held::BOMB_3X => return Some(Launch::Bomb),
        held::MISSILE_1X | held::MISSILE_3X => return Some(Launch::Missile),
        _ => {}
    }
    match weapon {
        held::TURBO => {
            let boost = if d.num_wumpas >= WUMPA_JUICED_COUNT {
                TURBO_BOOST_JUICED
            } else {
                TURBO_BOOST_NORMAL
            };
            fire::increment(
                d,
                TURBO_RESERVES,
                turbo_type::FREEZE_RESERVES_ON_TURBO_PAD | turbo_type::TURBO_ITEM,
                boost,
                effects,
            );
        }
        held::MASK => stuck::shoot_mask(d, effects),
        _ => {}
    }
    None
}

/// `VehPhysGeneral_SetHeldItem`, as far as a driver without a place goes.
fn set_held_item(d: &mut Driver, random_number: &mut i32) {
    let item = (math::mix_rng_scramble(random_number) % FALLBACK_ITEM_COUNT) as u8;
    // The roulette never shows the spring (it turns into a turbo), so it is not given either.
    d.held_item_id = if item == held::SPRING {
        held::TURBO
    } else {
        item
    };
    if matches!(d.held_item_id, held::BOMB_3X | held::MISSILE_3X) {
        d.num_held_items = STACK_COUNT;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn driver() -> Driver {
        let mut d = Driver::default();
        empty(&mut d);
        d
    }

    #[test]
    fn a_weapon_crate_starts_a_roulette_that_settles_after_three_seconds() {
        let mut d = driver();
        start_roulette(&mut d);
        assert_eq!(d.held_item_id, held::ROULETTE);
        let mut rng = 1;
        for _ in 0..ROLL_TICKS {
            tick(&mut d, &mut rng);
            assert_eq!(d.held_item_id, held::ROULETTE);
        }
        tick(&mut d, &mut rng);
        assert!(d.held_item_id <= held::MISSILE_3X);
        assert_ne!(d.held_item_id, held::SPRING);
    }

    #[test]
    fn a_driver_with_a_weapon_gets_no_roulette() {
        let mut d = driver();
        d.held_item_id = held::TNT;
        start_roulette(&mut d);
        assert_eq!(d.held_item_id, held::TNT);
        let mut d = driver();
        d.clock_receive = 5;
        start_roulette(&mut d);
        assert_eq!(d.held_item_id, held::NONE);
    }

    #[test]
    fn a_turbo_fires_once_and_is_gone_after_the_cooldown() {
        let mut d = driver();
        d.kart_state = kart_state::NORMAL;
        d.held_item_id = held::TURBO;
        d.actions_flag_set = request_fire(&mut d, true, 0);
        assert_ne!(d.actions_flag_set & actions::WEAPON_FIRE_REQUEST, 0);
        assert_eq!(d.no_item_timer, COOLDOWN_EMPTY_TICKS);
        // The cooldown stops a second shot.
        assert_eq!(request_fire(&mut d, true, 0), 0);
        let mut effects = Vec::new();
        shoot_on_circle_press(&mut d, &mut effects);
        assert_eq!(d.actions_flag_set & actions::WEAPON_FIRE_REQUEST, 0);
        assert!(d.reserves > 0);
        for _ in 0..COOLDOWN_EMPTY_TICKS - 1 {
            count_cooldown(&mut d);
            assert_eq!(d.held_item_id, held::TURBO);
        }
        count_cooldown(&mut d);
        assert_eq!(d.held_item_id, held::NONE);
    }

    #[test]
    fn a_stack_keeps_the_weapon_between_shots_and_launches_each() {
        let mut d = driver();
        d.kart_state = kart_state::NORMAL;
        d.held_item_id = held::MISSILE_3X;
        d.num_held_items = 3;
        let mut effects = Vec::new();
        for left in [2, 1, 0] {
            d.actions_flag_set = request_fire(&mut d, true, 0);
            assert_eq!(
                (d.num_held_items, d.no_item_timer),
                (left, COOLDOWN_STACK_TICKS)
            );
            assert_eq!(
                shoot_on_circle_press(&mut d, &mut effects),
                Some(Launch::Missile)
            );
            for _ in 0..COOLDOWN_STACK_TICKS {
                count_cooldown(&mut d);
            }
            // The cooldown of the last shot takes the weapon away.
            let kept = if left == 0 {
                held::NONE
            } else {
                held::MISSILE_3X
            };
            assert_eq!(d.held_item_id, kept);
        }
    }

    #[test]
    fn the_fire_button_sets_off_a_bomb_in_flight_instead_of_firing() {
        let mut d = driver();
        d.kart_state = kart_state::NORMAL;
        d.held_item_id = held::BOMB_3X;
        d.num_held_items = 2;
        d.bomb_thrown = true;
        assert_eq!(request_fire(&mut d, true, 0), 0);
        assert!(d.detonate_bomb && !d.bomb_thrown);
        assert_eq!((d.num_held_items, d.no_item_timer), (2, 0));
    }

    #[test]
    fn the_button_hurries_a_roulette_that_has_run_a_while() {
        let mut d = driver();
        d.kart_state = kart_state::NORMAL;
        start_roulette(&mut d);
        request_fire(&mut d, true, 0);
        assert_eq!(d.item_roll_timer, ROLL_TICKS);
        d.item_roll_timer = ROLL_FAST_STOP_TICKS - 1;
        request_fire(&mut d, true, 0);
        assert_eq!(d.item_roll_timer, 0);
    }

    #[test]
    fn triple_weapons_come_in_threes() {
        for seed in 0..200 {
            let mut d = driver();
            let mut rng = seed;
            set_held_item(&mut d, &mut rng);
            let triple = matches!(d.held_item_id, held::BOMB_3X | held::MISSILE_3X);
            assert_eq!(d.num_held_items, if triple { 3 } else { 0 });
        }
    }
}
