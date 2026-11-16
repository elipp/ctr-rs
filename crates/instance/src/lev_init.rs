//! `INSTANCE_LevInitAll`'s mode filter: which of a level's instances the game mode leaves out,
//! by clearing their `DRAW_COLLISION_MASK` bits (`0x1` draws, the rest collide). Everything that
//! reads `InstDef::flags` (the collision searches, the blasts, the props) honours it, so it is
//! applied once, to a fresh copy of the level file's flags: running it twice is harmless only
//! because it never sets back what it cleared.
//!
//! Retail's arcade does not hide time crates: its 1P LEVs have none. The relic LEVs
//! (`Lod::Relic`) have them with flags 0x8 (not drawn) until this sets `0x1`, and still have the
//! wumpa fruit, which this hides.

use coll::instance::DRAW_COLLISION_MASK;
use loader::lev::InstDef;

/// `PU_WUMPA_FRUIT`
const WUMPA_FRUIT: i32 = 2;
/// `PU_FRUIT_CRATE`, `PU_RANDOM_CRATE`.
const FRUIT_CRATE: i32 = 7;
const WEAPON_CRATE: i32 = 8;
/// `STATIC_TIME_CRATE_01..03`.
pub const TIME_CRATE_01: i32 = 0x5c;
pub const TIME_CRATE_02: i32 = 0x64;
pub const TIME_CRATE_03: i32 = 0x65;
/// `PU_EXPLOSIVE_CRATE` (nitro), `STATIC_CRATE_TNT`, `STATIC_CRYSTAL`.
const NITRO_CRATE: i32 = 6;
const TNT_CRATE: i32 = 0x27;
const CRYSTAL: i32 = 0x60;
/// `STATIC_C`; T and R are the two after it.
const LETTER_C: i32 = 0x93;

/// The part of `gGT->gameMode1` the filter reads. (No time trial, crystal challenge or token race
/// yet: the arcade branch is retail's "none of those".)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LevMode {
    #[default]
    Arcade,
    Relic,
}

/// Whether `model_id` is a time crate.
pub fn is_time_crate(model_id: i32) -> bool {
    matches!(model_id, TIME_CRATE_01 | TIME_CRATE_02 | TIME_CRATE_03)
}

/// The filter, over a level's instances as the file has them.
pub fn lev_init_all(instances: &mut [InstDef], mode: LevMode) {
    for inst in instances {
        let id = inst.model_id;
        let arcade_only = matches!(id, WUMPA_FRUIT | FRUIT_CRATE | WEAPON_CRATE);
        match mode {
            LevMode::Relic => {
                if arcade_only {
                    inst.flags &= !DRAW_COLLISION_MASK;
                }
                if is_time_crate(id) {
                    inst.flags |= 1;
                }
            }
            LevMode::Arcade => {
                if matches!(id, CRYSTAL | TNT_CRATE | NITRO_CRATE) {
                    inst.flags &= !DRAW_COLLISION_MASK;
                }
            }
        }
        if (LETTER_C..LETTER_C + 3).contains(&id) {
            inst.flags &= !DRAW_COLLISION_MASK;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn level(defs: &[(i32, u32)]) -> Vec<InstDef> {
        let instances = defs.iter().map(|&(model_id, flags)| InstDef {
            name: String::new(),
            model: 0,
            model_name: String::new(),
            scale: Default::default(),
            color_rgba: 0,
            flags,
            pos: Default::default(),
            rot: Default::default(),
            model_id,
        });
        instances.collect()
    }

    fn flags(instances: &[InstDef]) -> Vec<u32> {
        instances.iter().map(|i| i.flags).collect()
    }

    /// The relic LEV's: wumpa fruit, a weapon crate (a custom map's), time crates as the file
    /// has them, a C.
    const RELIC_LEV: [(i32, u32); 5] = [
        (WUMPA_FRUIT, 0xf),
        (WEAPON_CRATE, 0x7),
        (TIME_CRATE_01, 0x8),
        (TIME_CRATE_03, 0x8),
        (LETTER_C, 0x7),
    ];

    #[test]
    fn a_relic_race_draws_the_time_crates_and_hides_the_arcade_pickups() {
        let mut relic = level(&RELIC_LEV);
        lev_init_all(&mut relic, LevMode::Relic);
        assert_eq!(flags(&relic), [0, 0, 0x9, 0x9, 0]);
    }

    #[test]
    fn arcade_leaves_time_crates_as_they_are_and_hides_the_crystal_challenges() {
        let mut arcade = level(&RELIC_LEV);
        lev_init_all(&mut arcade, LevMode::Arcade);
        assert_eq!(flags(&arcade), [0xf, 0x7, 0x8, 0x8, 0]);
        let mut crystal = level(&[(CRYSTAL, 0x107), (TNT_CRATE, 0x7), (NITRO_CRATE, 0xf)]);
        lev_init_all(&mut crystal, LevMode::Arcade);
        assert_eq!(flags(&crystal), [0x100, 0, 0]);
    }
}
