//! The loops of `VehEmitter_DriverMain` for a player's kart: the ground's (the dirt, grass,
//! water and the like it drives on), the wall rub's and the skid's, which it sets again every
//! tick (`OtherFX_RecycleNew` on `driverAudioPtrs`, [`Effect::Recycle`]).

use crate::calc::map_to_range;
use crate::terrain::{TERRAINS, terrain_flags};
use crate::{Driver, Effect, actions, audio_slot, kart_state};

/// The ground's loop is at full volume from this speed, and at its highest pitch from this one.
const GROUND_LOUDEST_SPEED: i32 = 5000;
const GROUND_VOLUME_MAX: i32 = 200;
const GROUND_HIGHEST_SPEED: i32 = 12000;
const GROUND_DISTORT_MIN: i32 = 0x6c;
const GROUND_DISTORT_MAX: i32 = 0xd2;

/// `wallRubTimer` on the tick the kart scrapes a wall.
const WALL_RUB_TIMER_FULL: i16 = 0xf0;
const WALL_RUB_SFX: u8 = 0x14;
/// The wall rub's volume swells and fades by this a tick.
const WALL_RUB_VOLUME_STEP: i16 = 20;
const WALL_RUB_VOLUME_MAX: i16 = 255;

/// The tires skid from just over this speed (2 units a tick).
const SKID_MIN_SPEED: i32 = 2 * 256 + 1;
/// Between these speeds the skid gets louder and lower.
const SKID_SPEED_MIN: i32 = 2000;
const SKID_SPEED_MAX: i32 = 12000;
const SKID_VOLUME_MIN: i32 = 0x14;
const SKID_VOLUME_MAX: i32 = 0xaa;
const SKID_DISTORT_SLOW: i32 = 0x92;
const SKID_DISTORT_FAST: i32 = 0x78;

/// The pan of the middle, and the distortion that keeps a sound at its own pitch.
const CENTRE: i32 = 0x80;
const NO_DISTORTION: i32 = 0x80;

/// The sounds of `VehEmitter_DriverMain`: call it once per tick for a player's kart, after the
/// skid marks ([`crate::skid::skidmarks`]), whose flags the skid reads.
pub fn tick(d: &mut Driver, effects: &mut Vec<Effect>) {
    let echo = d.actions_flag_set & actions::ENGINE_ECHO != 0;
    let terrain = &TERRAINS[d.terrain_meta1 as usize];
    let speed = (d.speed_approx as i32).abs();
    let touching = d.actions_flag_set & actions::TOUCH_GROUND != 0;
    let recycle = |slot, id: i16, volume: i32, distort: i32, pan: i32| Effect::Recycle {
        slot,
        id: u8::try_from(id).ok(),
        volume: volume as u8,
        distort: distort as u8,
        pan: pan as u8,
        echo,
    };

    // The ground rumbles under the kart, louder and higher the faster it goes. (A terrain that
    // makes one sound as the kart sets down makes none as it drives.)
    let ground = if touching && terrain.flags & terrain_flags::ONESHOT_GROUND_SOUND == 0 {
        terrain.sound
    } else {
        -1
    };
    effects.push(recycle(
        audio_slot::GROUND,
        ground,
        map_to_range(speed, 0, GROUND_LOUDEST_SPEED, 0, GROUND_VOLUME_MAX),
        map_to_range(
            speed,
            0,
            GROUND_HIGHEST_SPEED,
            GROUND_DISTORT_MIN,
            GROUND_DISTORT_MAX,
        ),
        CENTRE,
    ));

    // Scraping a wall swells the rub, which fades away from it.
    if d.wall_rub_timer == WALL_RUB_TIMER_FULL && d.kart_state != kart_state::MASK_GRABBED {
        d.engine_vol = (d.engine_vol + WALL_RUB_VOLUME_STEP).min(WALL_RUB_VOLUME_MAX);
    } else {
        d.engine_vol = (d.engine_vol - WALL_RUB_VOLUME_STEP).max(0);
    }
    let wall = if d.engine_vol == 0 {
        -1
    } else {
        WALL_RUB_SFX as i16
    };
    effects.push(recycle(
        audio_slot::WALL,
        wall,
        d.engine_vol as i32,
        NO_DISTORTION,
        CENTRE,
    ));

    // The tires skid on the ground: louder the faster and the harder the kart turns, lower the
    // faster, and lower still with the wobble of a drift. It is heard on the side the kart
    // turns away from.
    let skidding = d.actions_flag_set & (actions::BACK_SKID | actions::FRONT_SKID) != 0;
    if !touching || !skidding || speed < SKID_MIN_SPEED {
        effects.push(recycle(audio_slot::SKID, -1, 0, NO_DISTORTION, CENTRE));
        return;
    }
    let turn = (d.simp_turn_state as i32).abs();
    let volume = map_to_range(
        speed,
        SKID_SPEED_MIN,
        SKID_SPEED_MAX,
        SKID_VOLUME_MIN,
        SKID_VOLUME_MAX,
    ) + (turn >> 1);
    let mut distort = map_to_range(
        speed,
        SKID_SPEED_MIN,
        SKID_SPEED_MAX,
        SKID_DISTORT_SLOW,
        SKID_DISTORT_FAST,
    );
    if d.kart_state == kart_state::DRIFTING {
        distort = (distort - (d.turn_wobble_angle as i32).abs()).max(0);
    }
    distort = (distort + turn).min(SKID_DISTORT_SLOW);
    let pan = CENTRE - (d.simp_turn_state as i32 >> 2);
    effects.push(recycle(
        audio_slot::SKID,
        terrain.skid_sound,
        volume,
        distort,
        pan,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terrain::terrain_type;

    fn driver() -> Driver {
        Driver {
            actions_flag_set: actions::TOUCH_GROUND,
            terrain_meta1: terrain_type::DIRT,
            speed_approx: 5000,
            ..Driver::default()
        }
    }

    fn slot(effects: &[Effect], wanted: u8) -> Effect {
        *effects
            .iter()
            .find(|e| matches!(e, Effect::Recycle { slot, .. } if *slot == wanted))
            .unwrap()
    }

    #[test]
    fn dirt_rumbles_louder_and_higher_the_faster() {
        let mut d = driver();
        let mut effects = vec![];
        tick(&mut d, &mut effects);
        let Effect::Recycle {
            id,
            volume,
            distort,
            ..
        } = slot(&effects, audio_slot::GROUND)
        else {
            unreachable!()
        };
        assert_eq!((id, volume), (Some(24), 200));
        assert_eq!(distort as i32, map_to_range(5000, 0, 12000, 0x6c, 0xd2));
        // In the air it stops.
        d.actions_flag_set = 0;
        effects.clear();
        tick(&mut d, &mut effects);
        assert!(matches!(
            slot(&effects, audio_slot::GROUND),
            Effect::Recycle { id: None, .. }
        ));
    }

    #[test]
    fn the_wall_rub_swells_and_fades() {
        let mut d = driver();
        let mut effects = vec![];
        d.wall_rub_timer = WALL_RUB_TIMER_FULL;
        for _ in 0..20 {
            tick(&mut d, &mut effects);
        }
        assert_eq!(d.engine_vol, 255);
        d.wall_rub_timer = 0;
        tick(&mut d, &mut effects);
        assert_eq!(d.engine_vol, 235);
        assert!(matches!(
            effects.last().copied(),
            Some(Effect::Recycle { .. })
        ));
        for _ in 0..12 {
            effects.clear();
            tick(&mut d, &mut effects);
        }
        assert_eq!(d.engine_vol, 0);
        assert!(matches!(
            slot(&effects, audio_slot::WALL),
            Effect::Recycle { id: None, .. }
        ));
    }

    #[test]
    fn a_skid_is_heard_on_the_side_away_from_the_turn() {
        let mut d = driver();
        d.actions_flag_set |= actions::BACK_SKID;
        d.simp_turn_state = 64;
        let mut effects = vec![];
        tick(&mut d, &mut effects);
        let Effect::Recycle {
            id, volume, pan, ..
        } = slot(&effects, audio_slot::SKID)
        else {
            unreachable!()
        };
        assert_eq!(id, Some(18));
        assert_eq!(
            volume as i32,
            map_to_range(5000, 2000, 12000, 0x14, 0xaa) + 32
        );
        assert_eq!(pan as i32, CENTRE - 16);
        // Too slow, and it stops.
        d.speed_approx = 2 * 256;
        effects.clear();
        tick(&mut d, &mut effects);
        assert!(matches!(
            slot(&effects, audio_slot::SKID),
            Effect::Recycle { id: None, .. }
        ));
    }
}
