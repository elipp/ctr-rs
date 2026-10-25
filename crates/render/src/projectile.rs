//! The models of the weapons in flight and of how they go off: the bomb (`bomb`), the missile
//! (`rocket`, which loops its `thrust` animation), and the burst of a blast (`RB_Burst_Init`):
//! two copies of `warpedburst` (the second turned a quarter on the screen) and a `shockwavered`
//! ring, all three facing the camera. The mines are the TNT (`crate_tnt`) and Nitro (`crate_nitro`)
//! crates, which go off in the break-up of a crate (`crate_explosion`, tinted red or green) with a
//! red or green shockwave (`RB_Blowup_Init`). The potions are the red and green beakers
//! (`beakerred`, `beakergreen`, which loop their `bubble` animation), which shatter in the same
//! shockwaves. The shield bomb is the shield's own model. Every racer's pack has them. Where the weapons are is
//! `weapon`'s; the entities are `view`'s.

use std::io;

use loader::dram::DramFile;
use loader::model::{Model, parse_pack_models};

/// `DYNAMIC_BOMB`, `DYNAMIC_ROCKET`, `STATIC_WARPEDBURST`, `STATIC_SHOCKWAVE_RED`
pub const BOMB: i16 = 0x3b;
pub const ROCKET: i16 = 0x29;
pub const BURST: i16 = 0x2b;
pub const SHOCKWAVE: i16 = 0x44;

/// `STATIC_CRATE_TNT`, `PU_EXPLOSIVE_CRATE`, `STATIC_CRATE_EXPLOSION`, `STATIC_SHOCKWAVE_GREEN`
pub const TNT: i16 = 0x27;
pub const NITRO: i16 = 0x06;
pub const CRATE_EXPLOSION: i16 = 0x26;
pub const SHOCKWAVE_GREEN: i16 = 0x45;

/// `STATIC_CLOUD`: the red beaker's rain cloud, which loops its `billow` animation.
pub const CLOUD: i16 = 0x42;

/// `STATIC_BEAKER_RED`, `STATIC_BEAKER_GREEN`
pub const BEAKER_RED: i16 = 0x46;
pub const BEAKER_GREEN: i16 = 0x47;

/// The `colorRGBA` of the break-up of a TNT crate (red) and of a Nitro crate (green), which
/// `explosion::tint` reads.
pub const TNT_COLOR: u32 = 0x0ad1_0000;
pub const NITRO_COLOR: u32 = 0x01ea_c000;

/// `RB_Burst_SetPosition`: the blast sits this far under the weapon.
pub const BURST_DROP: i32 = 0x30;

/// The weapons' models.
#[derive(Debug, Clone)]
pub struct Models {
    pub bomb: Model,
    pub rocket: Model,
    pub burst: Model,
    pub shockwave: Model,
    pub tnt: Model,
    pub nitro: Model,
    pub crate_explosion: Model,
    pub shockwave_green: Model,
    pub beaker_green: Model,
    pub beaker_red: Model,
    /// The shield bomb: the blue shield's model, and the green one's.
    pub shield_blue: Model,
    pub shield_green: Model,
    pub cloud: Model,
}

/// The weapons' models of a racer's pack.
pub fn models(pack: &DramFile) -> io::Result<Models> {
    let mut models = parse_pack_models(pack)?;
    let mut take = |id: i16| {
        let at = models.iter().position(|m| m.id == id).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("the pack has no model {id:#x}"),
            )
        })?;
        Ok::<_, io::Error>(models.swap_remove(at))
    };
    Ok(Models {
        bomb: take(BOMB)?,
        rocket: take(ROCKET)?,
        burst: take(BURST)?,
        shockwave: take(SHOCKWAVE)?,
        tnt: take(TNT)?,
        nitro: take(NITRO)?,
        crate_explosion: take(CRATE_EXPLOSION)?,
        shockwave_green: take(SHOCKWAVE_GREEN)?,
        beaker_green: take(BEAKER_GREEN)?,
        beaker_red: take(BEAKER_RED)?,
        shield_blue: take(crate::shield::BLUE)?,
        shield_green: take(crate::shield::GREEN)?,
        cloud: take(CLOUD)?,
    })
}

/// A looping animation's frame after a tick (`RB_MovingExplosive_ThTick`): the next, and the
/// first after the last.
pub fn loop_frame(frame: i16, frames: i16) -> i16 {
    if frame + 1 < frames { frame + 1 } else { 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_thrust_goes_round() {
        let mut frame = 0;
        let mut seen = Vec::new();
        for _ in 0..12 {
            seen.push(frame);
            frame = loop_frame(frame, 10);
        }
        assert_eq!(seen, [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 0, 1]);
    }
}
