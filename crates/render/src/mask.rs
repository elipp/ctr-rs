//! The models of the Aku Aku and Uka Uka masks (`VehPickupItem_MaskUseWeapon`): the head
//! (`akumouth`, `ukamouth`) and the beam of light that turns in it (`akubeam`, `ukabeam`). Every
//! racer's pack has them. Where they are and how big is `vehicle::stuck`'s.

use std::io;

use loader::dram::DramFile;
use loader::model::{Model, parse_pack_models};

/// `STATIC_AKUAKU`, `STATIC_UKAUKA`, `STATIC_AKUBEAM`, `STATIC_UKABEAM`
pub const AKU_HEAD: i16 = 0x39;
pub const UKA_HEAD: i16 = 0x3a;
pub const AKU_BEAM: i16 = 0x3e;
pub const UKA_BEAM: i16 = 0x40;

/// One mask: the head, and the beam in it.
#[derive(Debug, Clone)]
pub struct Mask {
    pub head: Model,
    pub beam: Model,
}

/// Both masks.
#[derive(Debug, Clone)]
pub struct Masks {
    pub aku: Mask,
    pub uka: Mask,
}

impl Masks {
    /// Aku Aku looks after the good guys, Uka Uka the rest.
    pub fn of(&self, good_guy: bool) -> &Mask {
        if good_guy { &self.aku } else { &self.uka }
    }
}

/// The masks of a racer's pack.
pub fn models(pack: &DramFile) -> io::Result<Masks> {
    let mut models = parse_pack_models(pack)?;
    let mut take = |id: i16| {
        let at = models
            .iter()
            .position(|m| m.id == id)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, format!("the pack has no model {id:#x}")))?;
        Ok::<_, io::Error>(models.swap_remove(at))
    };
    Ok(Masks {
        aku: Mask { head: take(AKU_HEAD)?, beam: take(AKU_BEAM)? },
        uka: Mask { head: take(UKA_HEAD)?, beam: take(UKA_BEAM)? },
    })
}
