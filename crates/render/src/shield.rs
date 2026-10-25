//! The models of the shield bubble (`VehPickupItem_ShootNow`): the dark layer under it
//! (`shielddark`), the coloured one (`shield` for the blue, juiced bubble, `shield_green`) and the
//! highlight that sweeps round it. Every racer's pack has them. How big they are and how
//! transparent is `vehicle::shield`'s.

use std::io;

use loader::dram::DramFile;
use loader::model::{Model, parse_pack_models};

/// `DYNAMIC_SHIELD`, `DYNAMIC_SHIELD_DARK`, `DYNAMIC_HIGHLIGHT`, `DYNAMIC_SHIELD_GREEN`
pub const BLUE: i16 = 0x56;
pub const DARK: i16 = 0x5a;
pub const HIGHLIGHT: i16 = 0x5d;
pub const GREEN: i16 = 0x5e;

#[derive(Debug, Clone)]
pub struct Shields {
    pub dark: Model,
    pub blue: Model,
    pub green: Model,
    pub highlight: Model,
}

/// The shield's models of a racer's pack.
pub fn models(pack: &DramFile) -> io::Result<Shields> {
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
    Ok(Shields {
        dark: take(DARK)?,
        blue: take(BLUE)?,
        green: take(GREEN)?,
        highlight: take(HIGHLIGHT)?,
    })
}
