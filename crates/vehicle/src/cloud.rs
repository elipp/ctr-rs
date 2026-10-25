//! The red beaker's rain cloud (`RB_RainCloud_Init`, `RB_RainCloud_ThTick`,
//! `RB_RainCloud_FadeAway`). A kart that a red potion spun out gets a cloud over its head for
//! 7.68 seconds, with one effect (`rain_cloud`): it slows the kart, makes the weapon in hand roll
//! again, makes the ground icy, forbids jumping, and so on. Another red potion refreshes the
//! cloud and draws the effect again.
//!
//! Like the mask and the shield, the cloud is a thread under the driver's: its state is [`Cloud`],
//! in `Driver::cloud` while it hangs over the kart and in `Driver::fading_clouds` while it
//! shrinks away. What it looks like (where it is, how large, how far its animation has run) is
//! kept there for the outside to draw, and so is the rain.
//!
//! Not ported: the rain's emitter (`RainLocal`) beyond its frame count, and the cloud being
//! cleared when the kart is eaten by a plant (`VehStuckProc_PlantEaten_Init`).

use crate::item::{self, held};
use crate::{Driver, actions, rain_cloud};

/// `RainCloud::timeMS` of a new or refreshed cloud: 7.68 seconds.
const DURATION: i16 = 0x1e00;
/// `RainLocal::frameCount` of a new cloud's rain, and what it loses each tick of the fade.
const RAIN_FRAMES: i16 = 0x1e;
const RAIN_FADE_STEP: i16 = 2;
/// How far above the kart the cloud hangs, and its scale (0x1000 is the model's size).
const LIFT: i32 = 0x80;
const SCALE: i16 = 0x1000;
/// What the scale shrinks by each tick of the fade.
const FADE_STEP: i16 = 0x100;
/// `RB_RainCloud_ThTick` keeps the roll five ticks from its end while the cloud lasts.
const ROLL_TICKS: i16 = 5;

/// `struct RainCloud` and its thread's instance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cloud {
    /// `timeMS`: what is left of the cloud, in milliseconds.
    pub time_ms: i16,
    /// `effect`: a `rain_cloud` value.
    pub effect: u8,
    /// `inst->matrix.t`: it follows the kart, a little behind.
    pub pos: [i32; 3],
    /// `inst->scale`: the kart's size, once it has caught up; the model's before.
    pub scale: [i16; 3],
    /// The tick it began, which tells it from the other clouds of the driver.
    pub born: u32,
    /// How many ticks the model's animation has run (`inst->animFrame`, before it wraps).
    pub age: u32,
    /// `RainLocal::frameCount`: the rain's. It falls off while the cloud fades.
    pub rain_frames: i16,
}

/// `RB_RainCloud_Init`: a red potion hurt the driver. The first one makes a cloud; another one
/// restarts it and draws its effect again (`rng` is `MixRNG_Scramble`'s).
pub fn init(d: &mut Driver, tick: u32, random_number: &mut i32) {
    if let Some(cloud) = d.cloud.as_mut() {
        cloud.time_ms = DURATION;
        let rng = math::mix_rng_scramble(random_number);
        cloud.effect = ((rng % 400) / 100) as u8;
        return;
    }
    let (_, at) = crate::baked::inst_matrix(d);
    let effect = if d.held_item_id == held::NONE || d.no_item_timer != 0 {
        rain_cloud::SLOW
    } else {
        rain_cloud::ITEM_ROLL
    };
    d.cloud = Some(Cloud {
        time_ms: DURATION,
        effect,
        pos: [at[0], at[1].wrapping_add(LIFT), at[2]],
        scale: [SCALE; 3],
        born: tick,
        age: 0,
        rain_frames: RAIN_FRAMES,
    });
}

/// The effect the cloud over the driver has, or `rain_cloud::NONE` (`Driver::thCloud`).
pub fn effect(d: &Driver) -> u8 {
    d.cloud.map_or(rain_cloud::NONE, |c| c.effect)
}

/// `RB_RainCloud_ThTick` and `RB_RainCloud_FadeAway`, once per tick, after the drivers'. Needs the
/// roulette's random number for the weapon it rolls when the cloud ends.
pub fn tick(d: &mut Driver, racers: usize, elapsed_time_ms: i32, random_number: &mut i32) {
    let (_, at) = crate::baked::inst_matrix(d);

    // The clouds that are going away shrink, and follow the kart halfway.
    for cloud in &mut d.fading_clouds {
        cloud.pos[0] = cloud.pos[0].wrapping_add(at[0]) >> 1;
        cloud.pos[1] = cloud.pos[1].wrapping_add(at[1]).wrapping_add(LIFT) >> 1;
        cloud.pos[2] = cloud.pos[2].wrapping_add(at[2]) >> 1;
        for s in &mut cloud.scale {
            *s = s.wrapping_sub(FADE_STEP);
        }
        cloud.rain_frames = cloud.rain_frames.wrapping_sub(RAIN_FADE_STEP);
        cloud.age += 1;
    }
    d.fading_clouds.retain(|c| c.scale[0] >= 0);

    let Some(mut cloud) = d.cloud else { return };
    cloud.age += 1;
    let kart_scale = [d.inst_scale_x, d.inst_scale_y, d.inst_scale_z];
    for (s, k) in cloud.scale.iter_mut().zip(kart_scale) {
        *s = ((*s as i32 + k as i32) >> 1) as i16;
    }
    cloud.pos[0] = cloud.pos[0].wrapping_add(at[0]) >> 1;
    cloud.pos[1] = cloud.pos[1]
        .wrapping_add(at[1])
        .wrapping_add((cloud.scale[1] as i32 * 5) >> 7)
        >> 1;
    cloud.pos[2] = cloud.pos[2].wrapping_add(at[2]) >> 1;

    if d.actions_flag_set & actions::MASK_WEAPON != 0 {
        // The mask blows the cloud away.
        d.cloud = None;
        d.fading_clouds.push(cloud);
        return;
    }
    if cloud.time_ms != 0 {
        cloud.time_ms = (cloud.time_ms as i32 - elapsed_time_ms).max(0) as i16;
        if cloud.effect == rain_cloud::ITEM_ROLL
            && d.held_item_id != held::NONE
            && d.no_item_timer == 0
        {
            // The weapon in hand rolls again, held five ticks from picking until the cloud ends.
            d.held_item_id = held::ROULETTE;
            d.item_roll_timer = ROLL_TICKS;
            d.num_held_items = 0;
        }
        d.cloud = Some(cloud);
        return;
    }
    d.cloud = None;
    if cloud.effect == rain_cloud::ITEM_ROLL && d.held_item_id != held::NONE {
        d.item_roll_timer = 0;
        item::pick(d, racers, random_number);
    }
    d.fading_clouds.push(cloud);
}

/// `VehStuckProc_PlantEaten_Init`'s end of the cloud: it goes away at once.
pub fn dispel(d: &mut Driver) {
    if let Some(mut cloud) = d.cloud.take() {
        cloud.time_ms = 0;
        d.fading_clouds.push(cloud);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn driver(item: u8) -> Driver {
        Driver {
            held_item_id: item,
            ..Driver::default()
        }
    }

    #[test]
    fn a_kart_with_nothing_in_hand_is_slowed() {
        let mut d = driver(held::NONE);
        init(&mut d, 0, &mut 1);
        assert_eq!(effect(&d), rain_cloud::SLOW);
        let mut d = driver(held::TURBO);
        init(&mut d, 0, &mut 1);
        assert_eq!(effect(&d), rain_cloud::ITEM_ROLL);
        let mut d = Driver {
            no_item_timer: 5,
            ..driver(held::TURBO)
        };
        init(&mut d, 0, &mut 1);
        assert_eq!(effect(&d), rain_cloud::SLOW);
    }

    #[test]
    fn another_potion_restarts_the_cloud_and_draws_its_effect() {
        let mut d = driver(held::NONE);
        let mut rng = 1;
        init(&mut d, 0, &mut rng);
        d.cloud.as_mut().unwrap().time_ms = 100;
        init(&mut d, 0, &mut rng);
        assert_eq!(d.cloud.unwrap().time_ms, DURATION);
        assert!(effect(&d) < 4);
    }

    #[test]
    fn the_cloud_lasts_7_68_seconds_and_then_fades() {
        let mut d = driver(held::NONE);
        let mut rng = 1;
        init(&mut d, 0, &mut rng);
        // 0x1e00 / 32 ms: 240 ticks, and one more that sees it at zero.
        for _ in 0..240 {
            tick(&mut d, 8, 32, &mut rng);
            assert!(d.cloud.is_some());
        }
        tick(&mut d, 8, 32, &mut rng);
        assert_eq!(effect(&d), rain_cloud::NONE);
        assert_eq!(d.fading_clouds.len(), 1);
        // It shrinks 0x100 a tick from the kart's size, and is gone when it goes below zero.
        for _ in 0..40 {
            tick(&mut d, 8, 32, &mut rng);
        }
        assert!(d.fading_clouds.is_empty());
    }

    #[test]
    fn the_weapon_in_hand_rolls_again_under_the_cloud() {
        let mut d = driver(held::TURBO);
        let mut rng = 1;
        init(&mut d, 0, &mut rng);
        tick(&mut d, 8, 32, &mut rng);
        assert_eq!(d.held_item_id, held::ROULETTE);
        assert_eq!(d.item_roll_timer, ROLL_TICKS);
        // When the cloud ends, the roulette picks.
        d.cloud.as_mut().unwrap().time_ms = 0;
        tick(&mut d, 8, 32, &mut rng);
        assert_ne!(d.held_item_id, held::ROULETTE);
        assert_eq!(d.item_roll_timer, 0);
    }

    #[test]
    fn the_mask_blows_the_cloud_away() {
        let mut d = driver(held::NONE);
        let mut rng = 1;
        init(&mut d, 0, &mut rng);
        d.actions_flag_set |= actions::MASK_WEAPON;
        tick(&mut d, 8, 32, &mut rng);
        assert!(d.cloud.is_none());
        assert_eq!(d.fading_clouds.len(), 1);
    }
}
