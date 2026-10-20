//! The exhaust fire of a boost: `struct Turbo`, the thread `VehFire_Increment`
//! starts and `VehTurbo_ThTick` runs after the driver each tick. Two instances
//! (one per pipe) show animation frames of the `turbo` models; this is the
//! state that picks them, sizes them and fades them out.
//!
//! Not modelled: the sound and rumble, the driver's own transparency
//! (`alphaScale` halving while the fire burns), and the water split and
//! reflection flags of the instances.

use crate::{Driver, kart_state};

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
        [self.fire_anim_index, (self.fire_anim_index + SECOND_PIPE_FRAME_OFFSET) & (ANIM_FRAMES - 1)].map(|f| f as usize)
    }
}

/// `VehTurbo_ThTick`, once per game tick after the driver. The fire is gone
/// from `d.turbo` once its thread dies.
pub fn tick(d: &mut Driver, elapsed_time_ms: i16) {
    let Some(mut t) = d.turbo else { return };

    // The cooldown runs out, and the fire shows.
    t.fire_visibility_cooldown = t.fire_visibility_cooldown.wrapping_sub(elapsed_time_ms).max(0);
    if t.fire_visibility_cooldown == 0 {
        t.hidden = false;
    }

    t.fire_anim_index = (t.fire_anim_index + 1) % ANIM_FRAMES;
    if t.fire_disappear_countdown > 0 {
        t.fire_disappear_countdown -= 1;
    }

    let stop = if matches!(d.kart_state, kart_state::MASK_GRABBED | kart_state::CRASHING | kart_state::WARP_PAD) {
        true
    } else if d.reserves < RESERVES_DISAPPEAR_THRESHOLD || t.fire_disappear_countdown == 0 {
        if t.alpha_scale > ALPHA_FULL_MINUS_ONE {
            true
        } else {
            t.alpha_scale += if t.fire_disappear_countdown == 0 { FADE_FAST_STEP } else { FADE_SLOW_STEP };
            false
        }
    } else {
        t.alpha_scale > ALPHA_FULL_MINUS_ONE
    };
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
        d.turbo = Some(Turbo { fire_visibility_cooldown: 96, ..Turbo::new(false) });
        for _ in 0..2 {
            tick(&mut d, 32);
            assert!(d.turbo.unwrap().hidden);
        }
        tick(&mut d, 32);
        assert!(!d.turbo.unwrap().hidden);
    }

    #[test]
    fn the_fire_fades_slowly_when_the_reserves_run_out_and_fast_after_a_hang_time_boost() {
        let mut d = Driver::new(0, 0);
        d.reserves = 5;
        d.turbo = Some(Turbo::new(false));
        let mut ticks = 0;
        while d.turbo.is_some() {
            tick(&mut d, 32);
            ticks += 1;
        }
        // 0x40 per tick up to 0x1000, and one more tick to notice.
        assert_eq!(ticks, 0x1000 / 0x40 + 1);

        d.reserves = 500;
        d.turbo = Some(Turbo::new(true));
        let mut ticks = 0;
        while d.turbo.is_some() {
            tick(&mut d, 32);
            ticks += 1;
        }
        // One tick of countdown, then 0x100 per tick, and one more tick to notice.
        assert_eq!(ticks, 1 + 0x1000 / 0x100 + 1);
    }

    #[test]
    fn a_crash_stops_the_fire_at_once() {
        let mut d = Driver::new(0, 0);
        d.reserves = 500;
        d.turbo = Some(Turbo::new(false));
        d.kart_state = kart_state::CRASHING;
        tick(&mut d, 32);
        assert!(d.turbo.is_none());
    }
}
