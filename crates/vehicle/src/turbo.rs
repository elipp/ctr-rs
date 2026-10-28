//! The exhaust fire of a boost: `struct Turbo`, the thread `VehFire_Increment`
//! starts and `VehTurbo_ThTick` runs after the driver each tick. Two instances
//! (one per pipe) show animation frames of the `turbo` models; this is the
//! state that picks them, sizes them and fades them out.
//!
//! A player's fire roars as it burns ([`Effect::Recycle`] on [`audio_slot::TURBO`]).
//!
//! Its death (`VehTurbo_ThDestroy`) ends a turbo item's boost (`ACTION_TURBO_ITEM`).
//!
//! Not modelled: the rumble, the driver's own transparency
//! (`alphaScale` halving while the fire burns), and the water split and
//! reflection flags of the instances.

use crate::{Driver, Effect, actions, audio_slot, kart_state};

/// `Turbo`
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Turbo {
    /// `fireAnimIndex`: 0..8, which of `turbo` .. `turbo7` the first pipe shows.
    pub fire_anim_index: i16,
    /// `fireSize`: 4 (a small powerslide boost) to 8 (a turbo pad); 8 is the model at full size.
    pub fire_size: i16,
    /// `fireDisappearCountdown`: counts down to 0, where the fire starts to fade; -1 never does.
    pub fire_disappear_countdown: i8,
    /// `fireVisibilityCooldown` (ms): the fire is hidden while it counts down, so that
    /// each powerslide boost makes it pop.
    pub fire_visibility_cooldown: i16,
    /// The instances' `alphaScale`: 0 is opaque, 0x1000 gone.
    pub alpha_scale: u16,
    /// `HIDE_MODEL`
    pub hidden: bool,
    /// `fireAudioDistort`: the roar rises a step a tick from the boost that lit or fed the fire.
    pub fire_audio_distort: i16,
}

/// The frames of the animation (`STATIC_TURBO_EFFECT` .. `STATIC_TURBO_EFFECT7`).
pub const ANIM_FRAMES: i16 = 8;
/// The second pipe shows the animation this many frames ahead.
pub const SECOND_PIPE_FRAME_OFFSET: i16 = 3;

const FIRE_SIZE_MIN: i16 = 4;
const FIRE_SIZE_MAX: i16 = 8;
const RESERVES_DISAPPEAR_THRESHOLD: i16 = 0x10;
const ALPHA_FULL_MINUS_ONE: u16 = 0xfff;
const FADE_FAST_STEP: u16 = 0x100;
const FADE_SLOW_STEP: u16 = 0x40;

/// `TURBO_AUDIO_SFX_ID`: the roar of the fire, at most this loud, and fading with the fire.
const AUDIO_SFX: u8 = 0xe;
const AUDIO_VOLUME_BASE: i32 = 0x100;
const AUDIO_ALPHA_SHIFT: u32 = 4;
const AUDIO_VOLUME_MAX: i32 = 0x82;
/// Its distortion is this above `fire_audio_distort`, up to its own pitch, and rises to here.
const AUDIO_DISTORT_STEP: i32 = 0x10;
const AUDIO_DISTORT_MAX: i32 = 0x80;
const AUDIO_DISTORT_RISE_LIMIT: i16 = 0xc0;
/// The pan of the middle.
const CENTRE: u8 = 0x80;

/// Where the fire sits on the kart, in game units in the kart's frame
/// (right, up, forward), for the pipe on the right and the one on the left;
/// from the instance scale per axis (so they follow the squash and stretch).
pub fn pipe_offsets(instance_scale: [i32; 3]) -> [[i32; 3]; 2] {
    let [sx, sy, sz] = instance_scale;
    let y = (sy * 3) >> 8;
    let z = (sz * -0x34) >> 12;
    [[(sx * 9) >> 11, y, z], [(sx * -0x12) >> 12, y, z]]
}

impl Turbo {
    /// A fire that was just lit (`INSTANCE_BirthWithThread`); `lasts_short` for a hang time boost.
    pub fn new(lasts_short: bool) -> Turbo {
        Turbo {
            fire_disappear_countdown: if lasts_short { 2 } else { -1 },
            hidden: true,
            ..Turbo::default()
        }
    }

    /// The fire's scale against the model, 1.0 being a size of 8.
    pub fn scale(&self) -> f32 {
        self.fire_size.clamp(FIRE_SIZE_MIN, FIRE_SIZE_MAX) as f32 / 8.0
    }

    /// The animation frames the first and the second pipe show.
    pub fn frames(&self) -> [usize; 2] {
        [
            self.fire_anim_index,
            (self.fire_anim_index + SECOND_PIPE_FRAME_OFFSET) & (ANIM_FRAMES - 1),
        ]
        .map(|f| f as usize)
    }
}

/// `VehTurbo_ThTick`, once per game tick after the driver. The fire is gone
/// from `d.turbo` once its thread dies.
pub fn tick(d: &mut Driver, elapsed_time_ms: i16, effects: &mut Vec<Effect>) {
    let Some(mut t) = d.turbo else { return };
    let player = d.actions_flag_set & actions::BOT == 0;
    let echo = d.actions_flag_set & actions::ENGINE_ECHO != 0;

    // The cooldown runs out, and the fire shows.
    t.fire_visibility_cooldown = t
        .fire_visibility_cooldown
        .wrapping_sub(elapsed_time_ms)
        .max(0);
    if t.fire_visibility_cooldown == 0 {
        t.hidden = false;
    }

    t.fire_anim_index = (t.fire_anim_index + 1) % ANIM_FRAMES;
    if t.fire_disappear_countdown > 0 {
        t.fire_disappear_countdown -= 1;
    }

    if player {
        let volume = (AUDIO_VOLUME_BASE - (t.alpha_scale >> AUDIO_ALPHA_SHIFT) as i32)
            .clamp(0, AUDIO_VOLUME_MAX);
        let distort =
            (t.fire_audio_distort as i32 + AUDIO_DISTORT_STEP).clamp(0, AUDIO_DISTORT_MAX);
        effects.push(Effect::Recycle {
            slot: audio_slot::TURBO,
            id: Some(AUDIO_SFX),
            volume: volume as u8,
            distort: distort as u8,
            pan: CENTRE,
            echo,
        });
        if t.fire_audio_distort < AUDIO_DISTORT_RISE_LIMIT {
            t.fire_audio_distort += 1;
        }
    }

    let stop = if matches!(
        d.kart_state,
        kart_state::MASK_GRABBED | kart_state::CRASHING | kart_state::WARP_PAD
    ) {
        true
    } else if d.reserves < RESERVES_DISAPPEAR_THRESHOLD || t.fire_disappear_countdown == 0 {
        if t.alpha_scale > ALPHA_FULL_MINUS_ONE {
            true
        } else {
            t.alpha_scale += if t.fire_disappear_countdown == 0 {
                FADE_FAST_STEP
            } else {
                FADE_SLOW_STEP
            };
            false
        }
    } else {
        t.alpha_scale > ALPHA_FULL_MINUS_ONE
    };
    if stop && player {
        effects.push(Effect::Recycle {
            slot: audio_slot::TURBO,
            id: None,
            volume: 0,
            distort: AUDIO_DISTORT_MAX as u8,
            pan: CENTRE,
            echo,
        });
    }
    if stop {
        // `VehTurbo_ThDestroy`: a turbo item's boost ends with its fire.
        d.actions_flag_set &= !actions::TURBO_ITEM;
    }
    d.turbo = (!stop).then_some(t);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fire_sits_at_the_pipes() {
        assert_eq!(pipe_offsets([0xccc; 3]), [[14, 38, -42], [-15, 38, -42]]);
    }

    #[test]
    fn the_second_pipe_runs_three_frames_ahead_and_wraps() {
        let mut t = Turbo::new(false);
        assert_eq!(t.frames(), [0, 3]);
        t.fire_anim_index = 6;
        assert_eq!(t.frames(), [6, 1]);
    }

    #[test]
    fn a_new_fire_shows_once_its_cooldown_is_over() {
        let mut d = Driver::new(0, 0);
        d.reserves = 100;
        d.turbo = Some(Turbo {
            fire_visibility_cooldown: 96,
            ..Turbo::new(false)
        });
        for _ in 0..2 {
            tick(&mut d, 32, &mut vec![]);
            assert!(d.turbo.unwrap().hidden);
        }
        tick(&mut d, 32, &mut vec![]);
        assert!(!d.turbo.unwrap().hidden);
    }

    #[test]
    fn the_fire_fades_slowly_when_the_reserves_run_out_and_fast_after_a_hang_time_boost() {
        let mut d = Driver::new(0, 0);
        d.reserves = 5;
        d.turbo = Some(Turbo::new(false));
        let mut ticks = 0;
        while d.turbo.is_some() {
            tick(&mut d, 32, &mut vec![]);
            ticks += 1;
        }
        // 0x40 per tick up to 0x1000, and one more tick to notice.
        assert_eq!(ticks, 0x1000 / 0x40 + 1);

        d.reserves = 500;
        d.turbo = Some(Turbo::new(true));
        let mut ticks = 0;
        while d.turbo.is_some() {
            tick(&mut d, 32, &mut vec![]);
            ticks += 1;
        }
        // One tick of countdown, then 0x100 per tick, and one more tick to notice.
        assert_eq!(ticks, 1 + 0x1000 / 0x100 + 1);
    }

    #[test]
    fn the_roar_rises_from_the_boost_fades_with_the_fire_and_stops_with_it() {
        let mut d = Driver::new(0, 0);
        d.reserves = 5;
        d.turbo = Some(Turbo::new(false));
        let mut roars = vec![];
        while d.turbo.is_some() {
            let mut effects = vec![];
            tick(&mut d, 32, &mut effects);
            roars.extend(effects.into_iter().map(|e| match e {
                Effect::Recycle {
                    slot: audio_slot::TURBO,
                    id,
                    volume,
                    distort,
                    ..
                } => (id, volume, distort),
                _ => panic!("{e:?}"),
            }));
        }
        assert_eq!(roars[0], (Some(AUDIO_SFX), 0x82, 0x10));
        assert_eq!(roars[1], (Some(AUDIO_SFX), 0x82, 0x11));
        // Fading 0x40 a tick, the volume comes down 4 a tick once under 0x82.
        let (_, quieter, _) = roars[roars.len() - 3];
        assert_eq!(quieter as i32, 0x100 - (0xfc0 >> 4));
        assert_eq!(roars.last().unwrap().0, None);
    }

    #[test]
    fn a_turbo_item_boosts_until_its_fire_dies() {
        let mut d = Driver::new(0, 0);
        crate::fire::increment(
            &mut d,
            0x960,
            crate::fire::turbo_type::FREEZE_RESERVES_ON_TURBO_PAD | crate::fire::turbo_type::TURBO_ITEM,
            0x80,
            &mut vec![],
        );
        assert_ne!(d.actions_flag_set & actions::TURBO_ITEM, 0);
        tick(&mut d, 32, &mut vec![]);
        assert_ne!(d.actions_flag_set & actions::TURBO_ITEM, 0);
        d.kart_state = kart_state::CRASHING;
        tick(&mut d, 32, &mut vec![]);
        assert!(d.turbo.is_none());
        assert_eq!(d.actions_flag_set & actions::TURBO_ITEM, 0);
    }

    #[test]
    fn a_crash_stops_the_fire_at_once() {
        let mut d = Driver::new(0, 0);
        d.reserves = 500;
        d.turbo = Some(Turbo::new(false));
        d.kart_state = kart_state::CRASHING;
        tick(&mut d, 32, &mut vec![]);
        assert!(d.turbo.is_none());
    }
}
