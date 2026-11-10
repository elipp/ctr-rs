//! The weapon a driver holds (`heldItemID`) and the roulette that picks it: a weapon crate starts
//! the roulette, which runs for [`ROLL_TICKS`] and then settles on an item.
//!
//! Firing is ported for the turbo, the mask, the shield, the bomb, the missile and the warpball
//! (which the `weapon` crate flies), TNT and the potion (which it drops, or throws), and the clock
//! (which hurts every other driver, see [`clock`]): see [`Launch`]. The rest stay held until they
//! are ported (see [`fireable`]). The pick is a placeholder: `VehPhysGeneral_SetHeldItem`'s fallback for a driver without a
//! place (`MixRNG_Scramble() % 12`), not the item sets by place that retail rolls from.

use crate::fire::{self, turbo_type};
use crate::{Driver, Effect, Sound3D, actions, kart_state, rain_cloud, stuck};

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

/// The items that can be had: all but the spring, which the roulette never gives.
pub const ENABLED: [u8; 11] = [
    held::TURBO,
    held::BOMB_1X,
    held::MISSILE_1X,
    held::TNT,
    held::POTION,
    held::SHIELD,
    held::MASK,
    held::CLOCK,
    held::WARPBALL,
    held::BOMB_3X,
    held::MISSILE_3X,
];

/// `data.RNG_itemSetRace1`: by 20 entries, the more an item has, the likelier it is. 8/20 potion,
/// 8/20 TNT, 2/20 bomb, 1/20 turbo, 1/20 shield.
const RACE1: [u8; 20] = [4, 4, 4, 4, 4, 4, 4, 4, 3, 3, 3, 3, 3, 3, 3, 3, 1, 1, 0, 6];

/// `data.RNG_itemSetRace2`: 52 entries.
const RACE2: [u8; 52] = [
    4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 3, 3, 3, 3, 3, 3, 3, 3, 1, 1, 1, 1, 1, 10, 10, 10, 0, 0, 0, 0, 0,
    6, 6, 6, 2, 2, 2, 11, 11, 9, 9, 9, 9, 9, 7, 7, 7, 7, 7, 7, 7, 8,
];

/// `data.RNG_itemSetRace3`: 20 entries.
const RACE3: [u8; 20] = [
    4, 3, 1, 10, 10, 0, 0, 6, 2, 2, 11, 9, 9, 9, 7, 7, 7, 7, 7, 8,
];

/// `data.RNG_itemSetRace4`: 20 stored, but the game draws from the first 19 (the last entry, a
/// turbo, is never given).
const RACE4: [u8; 19] = [0, 6, 2, 11, 9, 9, 9, 9, 9, 7, 7, 7, 7, 7, 7, 7, 7, 8, 8];

/// `ITEMSET_RNG_BUCKET_COUNT` and `ITEMSET_RNG_RANDOM_SHIFT`
const RNG_BUCKETS: u32 = 200;
const RNG_SHIFT: u32 = 3;

/// The item set of a driver in `place` (0 for the leader) of `racers` on `lap`, as
/// `VehPhysGeneral_SetHeldItem` picks it for a race: 0 to 3 for `RACE1` to `RACE4`.
///
/// DEVIATION: retail has no set for 7 racers or for 1, which leaves the roulette without an
/// item. 7 or more go by the table for 8 here, and 2 or fewer give the leader `RACE1` and the
/// rest `RACE2`. (Retail's 2 is a boss race, which is not here.)
fn item_set(racers: usize, place: usize, lap: u8, random_number: &mut i32) -> usize {
    let mut set = match racers {
        0..=2 => usize::from(place != 0),
        3 => match place {
            0 => 0,
            // The 2nd place of three has the 2nd or the 3rd place's items, by the next number.
            1 if math::mix_rng_scramble(random_number) & 1 != 0 => 1,
            1 => 2,
            _ => 3,
        },
        4 => place.min(3),
        5 => place.min(3),
        6 => match place {
            0 => 0,
            5 => 3,
            p => (p - 1) / 2 + 1,
        },
        _ => match place {
            1 => 1,
            p => (p / 2).min(3),
        },
    };
    // The 4th place's set is the 3rd place's on the first lap.
    if set == 3 && lap == 0 {
        set = 2;
    }
    set
}

/// Draws an item from `set`'s table: the entry that `rng` lands on, or if that is not enabled the
/// next one that is, wrapping round.
fn draw(set: usize, rng: u32) -> u8 {
    let table: &[u8] = match set {
        0 => &RACE1,
        1 => &RACE2,
        2 => &RACE3,
        _ => &RACE4,
    };
    let start = (rng * table.len() as u32 / RNG_BUCKETS) as usize;
    (0..table.len())
        .map(|n| table[(start + n) % table.len()])
        .find(|item| ENABLED.contains(item))
        .unwrap_or(held::TURBO)
}

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
/// `random_number` is `sdata->randomNumber`, `warpball_held` the race's `WARPBALL_HELD`.
pub fn tick(d: &mut Driver, racers: usize, random_number: &mut i32, warpball_held: &mut bool) {
    if d.held_item_id != held::ROULETTE {
        return;
    }
    if d.item_roll_timer != 0 {
        d.item_roll_timer -= 1;
        return;
    }
    set_held_item(d, racers, random_number, warpball_held);
}

/// The weapons that can be fired so far. A driver that holds another one throws it away with the
/// fire button (see [`request_fire`]) instead of losing it to a cooldown that fires nothing.
pub fn fireable(item: u8) -> bool {
    matches!(
        item,
        held::TURBO
            | held::MASK
            | held::BOMB_1X
            | held::BOMB_3X
            | held::MISSILE_1X
            | held::MISSILE_3X
            | held::TNT
            | held::POTION
            | held::SHIELD
            | held::CLOCK
            | held::WARPBALL
    )
}

/// What firing left for the caller, which `VehPickupItem_ShootNow` does with more than the
/// driver that fired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fired {
    /// A weapon to fly or drop (the `weapon` crate's).
    Weapon(Launch),
    /// The clock, which reaches every driver at once ([`clock`]).
    Clock,
}

/// A weapon that flies or is dropped, which `VehPickupItem_ShootNow` has the caller launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Launch {
    Bomb,
    Missile,
    /// TNT, or Nitro for a driver with 10 wumpa fruit: dropped behind the kart.
    Mine,
    /// A green beaker, or a red one for a driver with 10 wumpa fruit: dropped, or thrown with
    /// Up or Down held.
    Potion,
    /// The warpball, which flies along the track to the leader (or, with 10 wumpa fruit, to each
    /// kart ahead in turn).
    Warpball,
}

/// `SOUND_BOMB_LAUNCH`, `SOUND_MISSILE_LAUNCH`, `VOICELINE_BOMB_LAUNCH`, `VOICELINE_MISSILE_LAUNCH`,
/// `VOICELINE_WEAPON_PRIORITY`, `WEAPON_GAMEPAD_RUMBLE_FRAMES` and `_FORCE`
const BOMB_SOUND: u8 = 0x47;
const MISSILE_SOUND: u8 = 0x4a;
/// `SOUND_MINE_DROP`, `VOICELINE_MINE_DROP`
const MINE_SOUND: u8 = 0x52;
/// `SOUND_WARPBALL`, `VOICELINE_WARPBALL`
const WARPBALL_SOUND: u8 = 0x4d;
const WARPBALL_VOICELINE: u8 = 0xc;
const BOMB_VOICELINE: u8 = 10;
const MISSILE_VOICELINE: u8 = 11;
const MINE_VOICELINE: u8 = 0xf;
const VOICELINE_PRIORITY: u8 = 0x10;
const RUMBLE_FRAMES: u8 = 8;
const RUMBLE_FORCE: u8 = 0x7f;

/// What firing a [`Launch`] makes the driver feel and say, once the weapon is away, and the
/// sound of the weapon, heard from where it is (`at`, in level units).
pub fn launch_effects(d: &Driver, launch: Launch, at: [i32; 3], effects: &mut Vec<Effect>) {
    // Dropping a mine does not shake the pad.
    if !matches!(launch, Launch::Mine | Launch::Potion) {
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
    }
    let (id, voice) = match launch {
        Launch::Bomb => (BOMB_SOUND, BOMB_VOICELINE),
        Launch::Missile => (MISSILE_SOUND, MISSILE_VOICELINE),
        Launch::Mine | Launch::Potion => (MINE_SOUND, MINE_VOICELINE),
        Launch::Warpball => (WARPBALL_SOUND, WARPBALL_VOICELINE),
    };
    effects.push(Effect::Sound3D(Sound3D { id, pos: at }));
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
/// (The spring is not here.)
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
    // A shield round the kart is shot.
    if crate::shield::shoot(d) {
        return actions_flag_set;
    }
    // DEVIATION (for testing): a weapon that is not ported yet is thrown away by the button, to
    // make room for another one. Retail has no such thing.
    if d.item_roll_timer == 0
        && !fireable(d.held_item_id)
        && !matches!(d.held_item_id, held::NONE | held::ROULETTE)
    {
        d.held_item_id = held::NONE;
        d.num_held_items = 0;
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
/// and the clock are for the caller ([`Fired`]).
pub fn shoot_on_circle_press(d: &mut Driver, effects: &mut Vec<Effect>) -> Option<Fired> {
    if d.actions_flag_set & actions::WEAPON_FIRE_REQUEST == 0 {
        return None;
    }
    d.actions_flag_set &= !actions::WEAPON_FIRE_REQUEST;
    shoot_now(d, d.held_item_id, effects)
}

/// `VehPickupItem_ShootNow`, for the weapons that are ported. (Retail turns the triple and the
/// single bomb and missile into one case, then looks at what the driver holds again.)
fn shoot_now(d: &mut Driver, weapon: u8, effects: &mut Vec<Effect>) -> Option<Fired> {
    let launch = match weapon {
        held::BOMB_1X | held::BOMB_3X => Some(Launch::Bomb),
        held::MISSILE_1X | held::MISSILE_3X => Some(Launch::Missile),
        held::TNT => Some(Launch::Mine),
        held::POTION => Some(Launch::Potion),
        held::WARPBALL => Some(Launch::Warpball),
        held::CLOCK => return Some(Fired::Clock),
        _ => None,
    };
    if let Some(launch) = launch {
        return Some(Fired::Weapon(launch));
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
        held::SHIELD => crate::shield::shoot_shield(d, effects),
        _ => {}
    }
    None
}

/// `CLOCK_HURT_DURATION_NORMAL` and `_JUICED`: the milliseconds of `clockReceive` a clock gives
/// the drivers it hurt, by whether its user has 10 wumpa fruit.
const CLOCK_HURT_NORMAL: i16 = 0x1e00;
const CLOCK_HURT_JUICED: i16 = 0x2d00;
/// `CLOCK_FLASH_FRAMES` and `CLOCK_SELF_SEND_FRAMES`
const CLOCK_FLASH_TICKS: u8 = 4;
const CLOCK_SEND_TICKS: u8 = 0x1e;
/// `SOUND_CLOCK`, `VOICELINE_CLOCK`
const CLOCK_SOUND: u8 = 0x44;
const CLOCK_VOICELINE: u8 = 0xe;

/// `VehPickupItem_ShootNow`'s clock, used by the driver in `shooter` of `drivers` (by
/// `driver_id`, as `gGT->drivers` is). Every screen flashes, and every other driver that it hurts
/// (`RB_Hazard_HurtDriver`, a spin-out) is slowed for 7.5 seconds, or 11.25 if the user has 10
/// wumpa fruit. `effects` are by the same index as `drivers`.
pub fn clock(shooter: usize, drivers: &mut [&mut Driver], effects: &mut [Vec<Effect>]) {
    let user = &mut *drivers[shooter];
    user.num_times_clock_weapon_used = user.num_times_clock_weapon_used.wrapping_add(1);
    let juiced = user.num_wumpas >= WUMPA_JUICED_COUNT;
    effects[shooter].push(Effect::SoundForAll { id: CLOCK_SOUND });
    if user.actions_flag_set & actions::BOT == 0 {
        effects[shooter].push(Effect::Voiceline {
            id: CLOCK_VOICELINE,
            driver_id: user.driver_id,
            priority: VOICELINE_PRIORITY,
        });
    }
    for (i, (d, effects)) in drivers.iter_mut().zip(effects.iter_mut()).enumerate() {
        d.clock_flash = CLOCK_FLASH_TICKS;
        if i == shooter {
            d.clock_send = CLOCK_SEND_TICKS;
        } else if crate::pick::new_state(d, crate::pick::damage::SPIN, effects) {
            d.clock_receive = if juiced {
                CLOCK_HURT_JUICED
            } else {
                CLOCK_HURT_NORMAL
            };
        }
    }
}

/// `MainFrame_GameLogic`'s pass over the players before a tick: the clock's timers on their
/// screens run down. The result is the strength `DISPLAY_Blur_Main` draws the driver's screen
/// with this tick: the flash's ticks as a negative (white), a clock on the driver (the
/// milliseconds it has left) or one it used (`clockSend`'s ticks) as a positive (a trail that
/// zooms in, the more the stronger up to `0x1000`), and 0 for none. A clock on a driver that has
/// finished is gone.
pub fn clock_frame(d: &mut Driver) -> i32 {
    d.clock_send = d.clock_send.saturating_sub(1);
    if d.clock_flash != 0 {
        let flash = -i32::from(d.clock_flash);
        d.clock_flash -= 1;
        return flash;
    }
    if d.clock_receive != 0 {
        if d.actions_flag_set & actions::RACE_FINISHED != 0 {
            d.clock_receive = 0;
        }
        return i32::from(d.clock_receive);
    }
    i32::from(d.clock_send)
}

/// (Debug) Throws away what the driver holds, roulette and cooldown included, and has the
/// roulette settle on `item` right away.
pub fn debug_roll(d: &mut Driver, item: u8) {
    d.item_roll_timer = 0;
    d.no_item_timer = 0;
    d.num_held_items = 0;
    d.actions_flag_set &= !actions::WEAPON_FIRE_REQUEST;
    give(d, item);
}

/// `VehPhysGeneral_SetHeldItem` for the rain cloud, which picks the weapon when it ends.
pub fn pick(d: &mut Driver, racers: usize, random_number: &mut i32, warpball_held: &mut bool) {
    set_held_item(d, racers, random_number, warpball_held);
}

/// `VehPhysGeneral_SetHeldItem` for a race of `racers`. (Battle and the crystal challenge are not
/// here.) Only one warpball is had at a time: while `warpball_held` (`WARPBALL_HELD`), another
/// is three missiles instead, and the one given sets it (until it fades away, see
/// `weapon::warpball`).
fn set_held_item(d: &mut Driver, racers: usize, random_number: &mut i32, warpball_held: &mut bool) {
    let set = item_set(
        racers,
        d.driver_rank.max(0) as usize,
        d.lap_index,
        random_number,
    );
    // (`MixRNG_Scramble() >> 3) % 200`; the shift is of an unsigned word.)
    let rng = (math::mix_rng_scramble(random_number) as u32 >> RNG_SHIFT) % RNG_BUCKETS;
    let mut item = draw(set, rng);
    if item == held::WARPBALL {
        if *warpball_held {
            item = held::MISSILE_3X;
        } else {
            *warpball_held = true;
        }
    }
    give(d, item);
}

/// What the roulette does with the item it picked.
fn give(d: &mut Driver, item: u8) {
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
            tick(&mut d, 8, &mut rng, &mut false);
            assert_eq!(d.held_item_id, held::ROULETTE);
        }
        tick(&mut d, 8, &mut rng, &mut false);
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
    fn a_clock_slows_the_others_and_flashes_every_screen() {
        let mut karts: Vec<Driver> = (0..3)
            .map(|id| Driver {
                driver_id: id,
                kart_state: kart_state::NORMAL,
                ..driver()
            })
            .collect();
        karts[0].held_item_id = held::CLOCK;
        // Driver 2 is invincible (just hurt), and laughs it off.
        karts[2].invincible_timer = 1;
        karts[0].actions_flag_set = request_fire(&mut karts[0], true, 0);
        let mut effects = vec![Vec::new(); 3];
        let launch = shoot_on_circle_press(&mut karts[0], &mut effects[0]);
        assert_eq!(launch, Some(Fired::Clock));
        let mut drivers: Vec<&mut Driver> = karts.iter_mut().collect();
        clock(0, &mut drivers, &mut effects);

        assert_eq!(karts[0].num_times_clock_weapon_used, 1);
        assert_eq!(
            karts.iter().map(|d| d.clock_flash).collect::<Vec<_>>(),
            [CLOCK_FLASH_TICKS; 3]
        );
        assert_eq!(
            karts.iter().map(|d| d.clock_receive).collect::<Vec<_>>(),
            [0, CLOCK_HURT_NORMAL, 0]
        );
        assert_eq!(karts[0].clock_send, CLOCK_SEND_TICKS);
        assert!(karts[1].pending_spin);
        assert!(effects[0].contains(&Effect::SoundForAll { id: CLOCK_SOUND }));
    }

    #[test]
    fn a_clock_with_ten_wumpa_lasts_longer() {
        let mut karts = [driver(), driver()];
        karts[0].num_wumpas = WUMPA_JUICED_COUNT;
        let mut effects = vec![Vec::new(); 2];
        let [a, b] = &mut karts;
        clock(0, &mut [a, b], &mut effects);
        assert_eq!(karts[1].clock_receive, CLOCK_HURT_JUICED);
    }

    #[test]
    fn the_clock_flashes_white_then_trails() {
        let mut d = driver();
        d.clock_flash = CLOCK_FLASH_TICKS;
        d.clock_receive = CLOCK_HURT_NORMAL;
        let strengths: Vec<i32> = (0..5).map(|_| clock_frame(&mut d)).collect();
        assert_eq!(strengths, [-4, -3, -2, -1, CLOCK_HURT_NORMAL as i32]);
        // The user's own trail is faint, and runs out.
        let mut d = driver();
        d.clock_flash = CLOCK_FLASH_TICKS;
        d.clock_send = CLOCK_SEND_TICKS;
        let strengths: Vec<i32> = (0..CLOCK_SEND_TICKS).map(|_| clock_frame(&mut d)).collect();
        assert_eq!(strengths[4], CLOCK_SEND_TICKS as i32 - 5);
        assert_eq!(*strengths.last().unwrap(), 0);
        // A driver that finished is free of it.
        let mut d = driver();
        d.clock_receive = CLOCK_HURT_NORMAL;
        d.actions_flag_set = actions::RACE_FINISHED;
        assert_eq!(clock_frame(&mut d), 0);
        assert_eq!(d.clock_receive, 0);
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
                Some(Fired::Weapon(Launch::Missile))
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
            set_held_item(&mut d, 8, &mut rng, &mut false);
            let triple = matches!(d.held_item_id, held::BOMB_3X | held::MISSILE_3X);
            assert_eq!(d.num_held_items, if triple { 3 } else { 0 });
        }
    }

    #[test]
    fn the_tables_are_drawn_from_as_the_game_does() {
        // Race 1: entry 0 of 20 is a potion, the last is a shield.
        assert_eq!(draw(0, 0), held::POTION);
        assert_eq!(draw(0, 199), held::SHIELD);
        // Race 4 samples 19 entries: the first is a turbo.
        assert_eq!(draw(3, 0), held::TURBO);
        // Race 3 ends 9, 9, 9, 7, 7, 7, 7, 7, 8: the warpballs, the masks and the clock.
        assert_eq!(draw(2, 11 * 200 / 20), held::WARPBALL);
        assert_eq!(draw(2, 14 * 200 / 20), held::MASK);
        assert_eq!(draw(2, 199), held::CLOCK);
        for set in 0..4 {
            for rng in 0..RNG_BUCKETS {
                assert!(ENABLED.contains(&draw(set, rng)));
            }
        }
    }

    #[test]
    fn a_second_warpball_is_three_missiles() {
        // Race 4 is mostly warpballs: entry 4 of 19 is one.
        let rng = (0..)
            .find(|&seed| {
                let mut r = seed;
                (math::mix_rng_scramble(&mut r) as u32 >> RNG_SHIFT) % RNG_BUCKETS * 19 / RNG_BUCKETS
                    == 4
            })
            .unwrap();
        let roll = |held: &mut bool| {
            let mut d = driver();
            d.driver_rank = 7;
            d.lap_index = 1;
            let mut r = rng;
            set_held_item(&mut d, 8, &mut r, held);
            (d.held_item_id, d.num_held_items)
        };
        let mut held = false;
        assert_eq!(roll(&mut held), (held::WARPBALL, 0));
        assert!(held);
        assert_eq!(roll(&mut held), (held::MISSILE_3X, STACK_COUNT));
        // It is fired like the other weapons that fly.
        let mut d = driver();
        d.kart_state = kart_state::NORMAL;
        d.held_item_id = held::WARPBALL;
        d.actions_flag_set = request_fire(&mut d, true, 0);
        assert_eq!(
            shoot_on_circle_press(&mut d, &mut Vec::new()),
            Some(Fired::Weapon(Launch::Warpball))
        );
    }

    #[test]
    fn the_place_decides_the_item_set() {
        let mut rng = 0;
        // Eight racers, from the lead on a later lap.
        let sets: Vec<_> = (0..8).map(|p| item_set(8, p, 1, &mut rng)).collect();
        assert_eq!(sets, [0, 1, 1, 1, 2, 2, 3, 3]);
        // The 4th place's set is the 3rd place's on the first lap.
        assert_eq!(item_set(8, 7, 0, &mut rng), 2);
        let sets: Vec<_> = (0..6).map(|p| item_set(6, p, 1, &mut rng)).collect();
        assert_eq!(sets, [0, 1, 1, 2, 2, 3]);
        let sets: Vec<_> = (0..5).map(|p| item_set(5, p, 1, &mut rng)).collect();
        assert_eq!(sets, [0, 1, 2, 3, 3]);
        assert_eq!(item_set(4, 3, 1, &mut rng), 3);
        assert_eq!(rng, 0, "only 3 racers draw a number for the set");
        assert_eq!(item_set(3, 0, 1, &mut rng), 0);
        assert_eq!(item_set(3, 2, 1, &mut rng), 3);
        assert!(matches!(item_set(3, 1, 1, &mut rng), 1 | 2));
        assert_ne!(rng, 0);
    }
}
