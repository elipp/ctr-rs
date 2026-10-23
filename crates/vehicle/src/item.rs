//! The weapon a driver holds (`heldItemID`) and the roulette that picks it: a weapon crate starts
//! the roulette, which runs for [`ROLL_TICKS`] and then settles on an item.
//!
//! Only the holding is here. Firing the weapons is not ported, so a held weapon stays held, and
//! the pick is a placeholder: `VehPhysGeneral_SetHeldItem`'s fallback for a driver without a
//! place (`MixRNG_Scramble() % 12`), not the item sets by place that retail rolls from.

use crate::{Driver, actions, rain_cloud};

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
