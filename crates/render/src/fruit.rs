//! The wumpa fruit of the HUD's counter (`STATIC_FRUITDISP`), which every racer's pack has. A level
//! has the fruit of its own (placed in the track), but a map made by a community tool may not.

use std::io;

use loader::dram::DramFile;
use loader::model::{Model, parse_pack_models};

/// `STATIC_FRUITDISP`
pub const DISPLAY: i16 = 0x37;

/// The counter's fruit from a racer's pack.
pub fn display_model(pack: &DramFile) -> io::Result<Model> {
    parse_pack_models(pack)?
        .into_iter()
        .find(|m| m.id == DISPLAY)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "the pack has no fruitdisp"))
}
