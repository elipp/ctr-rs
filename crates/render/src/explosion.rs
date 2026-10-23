//! The break-up of a crate (`crate_explosion`, `RB_CrateAny_ThTick_Explode`): the model, how
//! it is tinted and turned, and how long it plays. When a crate breaks, the model of the racer
//! pack plays its animation at the crate once, a frame a tick, and is gone.
//!
//! Which crate it is, and so its colour, is `instance::pickup::crates`; the entities are `view`'s.

use std::io;

use loader::dram::DramFile;
use loader::model::{Model, parse_pack_models};

/// `STATIC_CRATE_EXPLOSION`
pub const MODEL_ID: i16 = 0x26;

/// The explosion of a racer pack: every racer's has the same.
pub fn model(pack: &DramFile) -> io::Result<Option<Model>> {
    Ok(parse_pack_models(pack)?
        .into_iter()
        .find(|m| m.id == MODEL_ID))
}

/// The tint of an instance's `colorRGBA`, which holds its three 8-bit channels at bits 20, 12
/// and 4 (`r << 20 | g << 12 | b << 4`).
pub fn tint(color_rgba: u32) -> [u8; 3] {
    [20, 12, 4].map(|shift| (color_rgba >> shift) as u8)
}

/// The angle the game turns some break-ups by around the vertical (`rand() % 0x1000`), in turns
/// (0..1). The sim has no use for the number, so it is a hash of which crate broke and how often,
/// the same for everyone.
pub fn break_yaw(breaks: u32, crate_index: usize) -> f32 {
    (breaks
        .wrapping_mul(0x9e37)
        .wrapping_add(crate_index as u32 * 0x6255)
        % 0x1000) as f32
        / 4096.0
}

/// An explosion's progress: `animFrame`, which `RB_CrateAny_ThTick_Explode` advances a tick until
/// it has played the animation's last frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Playback {
    pub frame: i16,
    frames: i16,
}

impl Playback {
    /// At the first frame of an animation of `frames`.
    pub fn new(frames: i16) -> Playback {
        Playback { frame: 0, frames }
    }

    /// One game tick. False once the animation is over, and the explosion is gone.
    pub fn advance(&mut self) -> bool {
        self.frame += 1;
        self.frame < self.frames
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_channels_sit_at_20_12_and_4() {
        // The three explosions of the crates (`colorRGBA` of `RB_Crate*_ThCollide`).
        assert_eq!(tint(0x0faf_afa0), [0xfa, 0xfa, 0xfa]);
        assert_eq!(tint(0x0f29_53a0), [0xf2, 0x95, 0x3a]);
        assert_eq!(tint(0x080f_f000), [0x80, 0xff, 0x00]);
    }

    #[test]
    fn the_angle_is_a_turn_at_most_and_varies() {
        let angles: Vec<f32> = (0..50).map(|n| break_yaw(n, 3)).collect();
        assert!(angles.iter().all(|a| (0.0..1.0).contains(a)));
        assert!(angles.windows(2).any(|w| w[0] != w[1]));
        assert_eq!(break_yaw(7, 2), break_yaw(7, 2));
    }

    #[test]
    fn twelve_frames_play_in_twelve_ticks() {
        let mut playback = Playback::new(12);
        assert_eq!(playback.frame, 0);
        // Frames 1 to 11 are shown after the first tick to the eleventh, and the twelfth tick ends it.
        for frame in 1..12 {
            assert!(playback.advance());
            assert_eq!(playback.frame, frame);
        }
        assert!(!playback.advance());
    }
}
