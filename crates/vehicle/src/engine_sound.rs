//! The engine's sound: `EngineSound_Player` for the player's own kart, and `EngineSound_AI` with
//! the per-voice easing of `EngineSound_NearestAIs` for the karts heard from a distance. Every
//! tick they turn how the kart drives into what its engine's loop plays at
//! (`EngineAudio_Recalculate`): a volume, a "distortion" (the pitch, in 64ths of an octave around
//! 0x80) and a pan.
//!
//! The engine keeps three numbers between ticks (`engineSoundVolumeState`,
//! `engineSoundPitchState` and `sfxDistortOffset` in the reference, `game/HOWL/HOWL_Engine.c`)
//! that nothing but the sound reads, so they are in [`EngineSound`], not in [`Driver`].

use crate::calc::map_to_range;
use crate::{Driver, actions, kart_state};

/// `engineSoundVolumeState` swells by this while the revs are far from where the kart wants them, up to
/// [`SWELL_MAX`], and settles by [`SWELL_SETTLE`] once they are close.
const SWELL_RISE: i16 = 2000;
const SWELL_MAX: i16 = 14000;
const SWELL_SETTLE: u16 = 500;
/// A drifting engine stays this loud.
const SWELL_DRIFT_MIN: i16 = 2000;
/// The revs are close to the target within this.
const REVS_CLOSE: i32 = 0x600;
/// Off the ground or drifting, the revs go this much above the speed.
const REVS_FREE: i32 = 0xf00;
/// On the grid the boost adds to the revs.
const REV_FIRE: i32 = 0x3000;
const REV_FIRE_REVS: i32 = 0x1000;

/// The volume of an engine at rest, and at its loudest (a human's, a bot's).
const VOLUME_MIN: i32 = 0x82;
const VOLUME_MAX: i32 = 0xe6;
const VOLUME_MAX_BOT: i32 = 0xbe;
/// The distortion of an engine at rest, and at its highest (a human's, a bot's, one heard from a
/// distance).
const DISTORT_MIN: i32 = 0x3c;
const DISTORT_MAX: i32 = 200;
const DISTORT_MAX_BOT: i32 = 0xbe;
const DISTORT_MAX_FAR: i32 = 0xaa;

/// The pan of the middle, and how far the steering may move the player's engine from it.
const CENTRE: i32 = 0x80;
const PAN_SWING: i32 = 0x40;

/// An engine heard from a distance is at full volume this near the camera and silent from this
/// far, in level units.
const FAR_FULL_WITHIN: i32 = 200;
const FAR_HEARD_WITHIN: i32 = 2000;
/// How fast the volume and pan of such an engine follow, a tick.
const FAR_EASE: i32 = 10;
/// How much its distance changing can bend its pitch, either way.
const DOPPLER_MAX: i32 = 0x14;

/// What an engine's loop plays at: the flags of `EngineAudio_Recalculate`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EngineVoice {
    /// 0 to 255.
    pub volume: u8,
    /// The pitch: 0x80 is the loop's own, each 64 an octave.
    pub distort: u8,
    /// 0 all left, 0x80 the middle, 255 all right.
    pub pan: u8,
    /// The kart is in a tunnel (`ENGINE_ECHO`).
    pub echo: bool,
}

/// What an engine keeps between ticks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EngineSound {
    /// `engineSoundVolumeState`: how loud the engine is, 0 to 14000. It swells while the revs chase the
    /// speed and settles when they have caught up.
    pub swell: i16,
    /// `engineSoundPitchState`: the revs, which follow the speed.
    pub revs: i16,
    /// `sfxDistortOffset`: how much the drift's turbo meter raises the pitch. Only a drift
    /// changes it, so it stays after the drift until the next one.
    pub drift_pitch: i16,
}

/// What `EngineSound_NearestAIs` keeps for each of its two voices (`audioDefaults`): the
/// volume and pan they last played at, and how far their kart was.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FarVoice {
    pub volume: i32,
    pub pan: i32,
    pub distance: i32,
}

impl EngineSound {
    /// `EngineSound_Player`: the engine of the player's own kart.
    pub fn player(&mut self, d: &Driver) -> EngineVoice {
        let drifting = d.kart_state == kart_state::DRIFTING;
        let target = if d.kart_state == kart_state::ENGINE_REVVING {
            let fire = if d.fire_speed > 0 { REV_FIRE } else { 0 };
            let revs =
                (self.revs as i32 * 0x40 + fire * 0x30 + d.speedometer_needle_value as i32 * 0x90)
                    >> 8;
            if d.fire_speed > 0 {
                revs + REV_FIRE_REVS
            } else {
                revs
            }
        } else {
            self.speed_target(d, (d.fire_speed as i32).abs())
        };
        self.follow(d, target);

        let bot = d.actions_flag_set & actions::BOT != 0;
        let stats = &d.stats;
        let accel = stats.accel_speed_class_stat as i32;
        let loudest = if bot { VOLUME_MAX_BOT } else { VOLUME_MAX };
        let mut volume = map_to_range(self.swell as i32, 0, accel, VOLUME_MIN, loudest);
        // Steering is heard, except in a drift.
        let steer = (d.wheel_rotation as i32).abs();
        if !drifting && d.actions_flag_set & actions::ACCEL_PREVENTION == 0 {
            volume += steer >> 3;
        }

        let highest = if bot { DISTORT_MAX_BOT } else { DISTORT_MAX };
        let top = accel + stats.sacred_fire_speed as i32 + REVS_FREE;
        let mut distort = map_to_range(self.revs as i32, 0, top, DISTORT_MIN, highest);
        let wobble = (d.turn_wobble_angle as i32 >> 3).abs();
        if bot {
            distort = (distort - wobble).max(0);
        } else {
            if drifting {
                if d.turbo_meter_room_left != 0 {
                    self.drift_pitch =
                        (stats.turbo_max_room >> 1) as i16 - (d.turbo_meter_room_left >> 6);
                } else if self.drift_pitch != 0 {
                    self.drift_pitch -= 1;
                }
                distort = (distort - wobble).max(0);
            }
            distort += self.drift_pitch as i32;
        }

        let pan =
            (CENTRE - (d.wheel_rotation as i32 >> 3)).clamp(CENTRE - PAN_SWING, CENTRE + PAN_SWING);
        EngineVoice {
            volume: volume as u8,
            distort: distort.min(0xff) as u8,
            pan: pan as u8,
            echo: d.actions_flag_set & actions::ENGINE_ECHO != 0,
        }
    }

    /// `EngineSound_AI` and the easing of `EngineSound_NearestAIs`: the engine of a kart heard
    /// from the camera, `distance` level units away at the pan `pan` (0 to 0x100). `voice` is
    /// what the voice it plays on played last; `echo` is the listener's.
    pub fn far(
        &mut self,
        d: &Driver,
        distance: i32,
        pan: i32,
        voice: &mut FarVoice,
        echo: bool,
    ) -> EngineVoice {
        voice.pan = ease(pan, voice.pan, FAR_EASE);
        let approach = distance - voice.distance;
        voice.distance = distance;

        // The bots' fire speed means little, so the class's speed stands in for it.
        let accel = d.stats.accel_speed_class_stat as i32;
        self.follow(d, self.speed_target(d, accel.abs()));

        let near = map_to_range(self.swell as i32, 0, accel, VOLUME_MIN, VOLUME_MAX);
        let volume = if distance >= FAR_HEARD_WITHIN {
            0
        } else if distance > FAR_FULL_WITHIN {
            map_to_range(distance, FAR_FULL_WITHIN, FAR_HEARD_WITHIN, near, 0)
        } else {
            near
        };
        voice.volume = ease(volume, voice.volume, FAR_EASE);

        // Coming closer raises the pitch, going away lowers it.
        let doppler = (approach >> 3).clamp(-DOPPLER_MAX, DOPPLER_MAX);
        let distort = map_to_range(self.revs as i32, 0, accel, DISTORT_MIN, DISTORT_MAX_FAR);
        EngineVoice {
            volume: voice.volume as u8,
            distort: (distort - doppler).clamp(0, 0xff) as u8,
            pan: voice.pan.clamp(0, 0xff) as u8,
            echo,
        }
    }

    /// Where the revs head from `speed`: above it off the ground or in a drift, between it and
    /// the kart's speed on the ground.
    fn speed_target(&self, d: &Driver, speed: i32) -> i32 {
        let grounded = d.actions_flag_set_prev_frame & actions::TOUCH_GROUND != 0;
        if !grounded || d.kart_state == kart_state::DRIFTING {
            speed + REVS_FREE
        } else {
            (speed + (d.speed_approx as i32).abs()) >> 1
        }
    }

    /// The swell and the revs follow `target`.
    fn follow(&mut self, d: &Driver, target: i32) {
        if (target - self.revs as i32).abs() <= REVS_CLOSE {
            let settled = (self.swell as u16).wrapping_sub(SWELL_SETTLE) as i16;
            self.swell = if d.kart_state == kart_state::DRIFTING {
                settled.max(SWELL_DRIFT_MIN)
            } else {
                settled.max(0)
            };
        } else {
            self.swell = (self.swell + SWELL_RISE).min(SWELL_MAX);
        }
        self.revs = ((target * 0x89 + self.revs as i32 * 0x177) as u32 >> 9) as i16;
    }
}

/// `EngineSound_VolumeAdjust`: `current` moves towards `desired` by at most `step`.
fn ease(desired: i32, current: i32, step: i32) -> i32 {
    let gap = desired - current;
    if gap.abs() > step {
        current + step * gap.signum()
    } else {
        desired
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::{EngineClass, Stats};

    fn driver() -> Driver {
        Driver {
            stats: Stats::for_class(EngineClass::Balanced),
            actions_flag_set_prev_frame: actions::TOUCH_GROUND,
            ..Driver::default()
        }
    }

    #[test]
    fn an_idle_engine_settles_quiet_and_low() {
        let (d, mut engine) = (driver(), EngineSound::default());
        let voice = (0..60).map(|_| engine.player(&d)).last().unwrap();
        assert_eq!(engine.swell, 0);
        assert_eq!(voice.volume as i32, VOLUME_MIN);
        assert_eq!(voice.distort as i32, DISTORT_MIN);
        assert_eq!(voice.pan as i32, CENTRE);
    }

    #[test]
    fn speeding_up_swells_and_raises_the_pitch() {
        let mut d = driver();
        let mut engine = EngineSound::default();
        engine.player(&d);
        d.fire_speed = d.stats.accel_speed_class_stat;
        d.speed_approx = d.stats.accel_speed_class_stat;
        let first = engine.player(&d);
        assert_eq!(engine.swell, SWELL_RISE);
        // The revs follow by 137/512 a tick.
        assert_eq!(engine.revs as i32, (d.fire_speed as i32 * 0x89) >> 9);
        let later = (0..30).map(|_| engine.player(&d)).last().unwrap();
        assert!(later.distort > first.distort);
        // Caught up: the swell settles again.
        assert!(engine.swell < SWELL_MAX);
        assert!((d.fire_speed - engine.revs).abs() as i32 <= REVS_CLOSE);
    }

    #[test]
    fn steering_pans_the_engine_and_makes_it_louder() {
        let mut d = driver();
        let mut engine = EngineSound::default();
        d.wheel_rotation = 64;
        let left = engine.player(&d);
        assert_eq!(left.pan as i32, CENTRE - 8);
        assert_eq!(left.volume as i32, VOLUME_MIN + 8);
        d.wheel_rotation = -0x400;
        assert_eq!(engine.player(&d).pan as i32, CENTRE + PAN_SWING);
    }

    #[test]
    fn a_filling_turbo_meter_raises_the_pitch_and_it_stays_after_the_drift() {
        let mut d = driver();
        let mut engine = EngineSound::default();
        d.kart_state = kart_state::DRIFTING;
        let half_room = (d.stats.turbo_max_room >> 1) as i16;
        let room = (d.stats.turbo_max_room as i16) << 5;
        d.turbo_meter_room_left = room;
        engine.player(&d);
        assert_eq!(engine.drift_pitch, 0);
        d.turbo_meter_room_left = room / 2;
        engine.player(&d);
        let filling = half_room - ((room / 2) >> 6);
        assert_eq!(engine.drift_pitch, filling);
        // Full: it comes down a step a tick.
        d.turbo_meter_room_left = 0;
        engine.player(&d);
        assert_eq!(engine.drift_pitch, filling - 1);
        d.kart_state = kart_state::NORMAL;
        engine.player(&d);
        assert_eq!(engine.drift_pitch, filling - 1);
    }

    #[test]
    fn a_far_engine_fades_with_distance_and_eases_in() {
        let d = driver();
        let mut engine = EngineSound::default();
        let mut voice = FarVoice::default();
        let near = engine.far(&d, 100, CENTRE, &mut voice, false);
        // From silence and the left, 10 a tick.
        assert_eq!((near.volume, near.pan), (10, 10));
        let settled = (0..60)
            .map(|_| engine.far(&d, 100, CENTRE, &mut voice, false))
            .last()
            .unwrap();
        assert_eq!(settled.volume as i32, VOLUME_MIN);
        assert_eq!(settled.pan as i32, CENTRE);
        let gone = (0..60)
            .map(|_| engine.far(&d, 2500, CENTRE, &mut voice, false))
            .last()
            .unwrap();
        assert_eq!(gone.volume, 0);
    }

    #[test]
    fn a_far_engine_coming_closer_sounds_higher() {
        let d = driver();
        let mut engine = EngineSound::default();
        let mut voice = FarVoice::default();
        let steady = (0..60)
            .map(|_| engine.far(&d, 1000, CENTRE, &mut voice, false))
            .last()
            .unwrap();
        let mut at = |distance| engine.far(&d, distance, CENTRE, &mut voice, false).distort as i32;
        // An eighth of the change in distance, rounded down, at most 20 either way.
        assert_eq!(at(900), steady.distort as i32 + 13);
        assert_eq!(at(1000), steady.distort as i32 - 12);
        assert_eq!(at(600), steady.distort as i32 + DOPPLER_MAX);
    }

    #[test]
    fn ease_moves_by_the_step_at_most() {
        assert_eq!(ease(100, 0, 10), 10);
        assert_eq!(ease(0, 100, 10), 90);
        assert_eq!(ease(95, 90, 10), 95);
    }
}
