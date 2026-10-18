use crate::level::LevelId;

/// Index of a subfile in `BIGFILE.BIG` (the game's `enum BigIndex`, `BI_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BigIndex(pub u32);

/// Which file of a level's group to load (`enum LevVramIndex`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LevFile {
    Vram = 0,
    Lev = 1,
    Ptr = 2,
}

/// Player-count / relic variant of a track (`LOAD_LEVEL_LOD_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lod {
    OnePlayer = 1,
    TwoPlayer = 2,
    ThreePlayer = 3,
    FourPlayer = 4,
    Relic = 8,
}

/// `sdata->levBigLodIndex`, indexed by `lod - 1`.
const LEV_BIG_LOD_INDEX: [u32; 8] = [0, 2, 4, 4, 6, 6, 6, 6];

const TRACK_FILES_PER_LOD_GROUP: u32 = 8;
const CUTSCENE_FILES_PER_LEVEL: u32 = 3;
const OUTRO_FILES_PER_LEVEL: u32 = 2;
const BATTLE_TRACK_COUNT: u32 = 7;
const INTRO_CUTSCENE_COUNT: u32 = 9;
const OUTRO_CUTSCENE_COUNT: u32 = 2;
const CREDIT_LEVEL_COUNT: u32 = 20;

impl BigIndex {
    // Each group starts where the previous one ends; sizes are from the game's enum.
    pub const ARCADE_TRACKS: Self = Self(0); // 18 tracks * (1p, 2p, 4p, relic) * (vram, lev, ptr..)
    pub const BATTLE_TRACKS: Self = Self(Self::ARCADE_TRACKS.0 + 18 * 8);
    pub const ADVENTURE_HUB: Self = Self(Self::BATTLE_TRACKS.0 + 7 * 8);
    pub const MAIN_MENU_FILE: Self = Self(Self::ADVENTURE_HUB.0 + 5 * 3);
    pub const OVERLAY_SECT1: Self = Self(Self::MAIN_MENU_FILE.0 + 6);
    pub const OVERLAY_SECT2: Self = Self(Self::OVERLAY_SECT1.0 + 5);
    pub const OVERLAY_SECT3: Self = Self(Self::OVERLAY_SECT2.0 + 4);
    pub const LANGUAGE_FILE: Self = Self(Self::OVERLAY_SECT3.0 + 4);
    pub const RACER_MODEL_HI: Self = Self(Self::LANGUAGE_FILE.0 + 8);
    pub const SHARED_MPK_VRM: Self = Self(Self::RACER_MODEL_HI.0 + 16);
    pub const ARCADE_1P_PACK: Self = Self(Self::SHARED_MPK_VRM.0 + 2);
    pub const ADVENTURE_PACK: Self = Self(Self::ARCADE_1P_PACK.0 + 16);
    pub const TIME_TRIAL_PACK: Self = Self(Self::ADVENTURE_PACK.0 + 16);
    pub const RACER_MODEL_MED: Self = Self(Self::TIME_TRIAL_PACK.0 + 16);
    pub const ARCADE_2P_PACK: Self = Self(Self::RACER_MODEL_MED.0 + 16);
    pub const RACER_MODEL_LOW: Self = Self(Self::ARCADE_2P_PACK.0 + 8);
    pub const ARCADE_4P_PACK: Self = Self(Self::RACER_MODEL_LOW.0 + 16);
    pub const PODIUM_VRMS: Self = Self(Self::ARCADE_4P_PACK.0 + 16);
    pub const DANCE_MODEL_WIN: Self = Self(Self::PODIUM_VRMS.0 + 2);
    pub const DANCE_MODEL_LOSE: Self = Self(Self::DANCE_MODEL_WIN.0 + 16 * 2);
    pub const DANCE_TAWNA_GIRL: Self = Self(Self::DANCE_MODEL_LOSE.0 + 16 * 2);
    pub const UKA_HEAD: Self = Self(Self::DANCE_TAWNA_GIRL.0 + 4 * 2);
    pub const AKU_HEAD: Self = Self(Self::UKA_HEAD.0 + 2 * 2);
    pub const DINGO_FIRE: Self = Self(Self::AKU_HEAD.0 + 2 * 2);
    pub const PODIUM: Self = Self(Self::DINGO_FIRE.0 + 2);
    pub const BOSS_HEAD: Self = Self(Self::PODIUM.0 + 2);
    pub const LOSE_HEAD: Self = Self(Self::BOSS_HEAD.0 + 5 * 2);
    pub const BOSS_BODY: Self = Self(Self::LOSE_HEAD.0 + 5 * 2);
    pub const VLC_TABLE: Self = Self(Self::BOSS_BODY.0 + 5 * 2);
    pub const THUMBS: Self = Self(Self::VLC_TABLE.0 + 1);
    pub const LOADING_SCREENS: Self = Self(Self::THUMBS.0 + 26);
    pub const ND_BOX: Self = Self(Self::LOADING_SCREENS.0 + 6);
    pub const CUTSCENES_INTRO: Self = Self(Self::ND_BOX.0 + 2);
    pub const CUTSCENES_OUTRO: Self = Self(Self::CUTSCENES_INTRO.0 + 9 * 3);
    pub const CREDITS: Self = Self(Self::CUTSCENES_OUTRO.0 + 2 * 2);
    pub const SCRAPBOOK: Self = Self(Self::CREDITS.0 + 20 * 3);

    /// The `n`th entry of a group starting at `self`.
    pub const fn nth(self, n: u32) -> Self {
        Self(self.0 + n)
    }

    pub const fn get(self) -> usize {
        self.0 as usize
    }

    /// Port of `LOAD_GetBigfileIndex`: the subfile holding `file` of `level`.
    ///
    /// `lod` only matters for race and battle tracks.
    pub fn level(level: LevelId, lod: Lod, file: LevFile) -> Self {
        let id = level.0;
        let file = file as u32;
        let lod_off = LEV_BIG_LOD_INDEX[lod as usize - 1];
        let nitro = LevelId::NITRO_COURT.0;
        let intro = LevelId::INTRO_RACE_TODAY.0;
        let oxide_ending = LevelId::OXIDE_ENDING.0;
        let credits = LevelId::CREDITS_CRASH.0;

        if id < nitro {
            return Self(Self::ARCADE_TRACKS.0 + id * TRACK_FILES_PER_LOD_GROUP + lod_off + file);
        }
        // The game does unsigned wrapping subtraction for these range checks.
        if id.wrapping_sub(nitro) < BATTLE_TRACK_COUNT {
            return Self(
                Self::BATTLE_TRACKS.0 + (id - nitro) * TRACK_FILES_PER_LOD_GROUP + lod_off + file,
            );
        }
        if id.wrapping_sub(intro) < INTRO_CUTSCENE_COUNT {
            return Self(Self::CUTSCENES_INTRO.0 + (id - intro) * CUTSCENE_FILES_PER_LEVEL + file);
        }
        if id.wrapping_sub(oxide_ending) < OUTRO_CUTSCENE_COUNT {
            return Self(
                Self::CUTSCENES_OUTRO.0 + (id - oxide_ending) * OUTRO_FILES_PER_LEVEL + file,
            );
        }
        if level == LevelId::NAUGHTY_DOG_CRATE {
            return Self(Self::ND_BOX.0 + file);
        }
        if id.wrapping_sub(credits) < CREDIT_LEVEL_COUNT {
            return Self(Self::CREDITS.0 + (id - credits) * CUTSCENE_FILES_PER_LEVEL + file);
        }
        if level == LevelId::MAIN_MENU_LEVEL {
            return Self(Self::MAIN_MENU_FILE.0 + file);
        }
        if level == LevelId::SCRAPBOOK {
            return Self(Self::SCRAPBOOK.0 + file);
        }
        // Everything else is an adventure hub level.
        Self(
            Self::ADVENTURE_HUB.0
                + id.wrapping_sub(LevelId::GEM_STONE_VALLEY.0) * CUTSCENE_FILES_PER_LEVEL
                + file,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_bases_match_retail_layout() {
        assert_eq!(BigIndex::BATTLE_TRACKS.0, 144);
        assert_eq!(BigIndex::ADVENTURE_HUB.0, 200);
        assert_eq!(BigIndex::MAIN_MENU_FILE.0, 215);
        assert_eq!(BigIndex::OVERLAY_SECT1.0, 221);
    }

    #[test]
    fn level_lookup() {
        let l = |id, lod, f| BigIndex::level(id, lod, f).0;
        assert_eq!(l(LevelId::DINGO_CANYON, Lod::OnePlayer, LevFile::Vram), 0);
        assert_eq!(l(LevelId::DRAGON_MINES, Lod::FourPlayer, LevFile::Ptr), 8 + 4 + 2);
        assert_eq!(l(LevelId::NITRO_COURT, Lod::OnePlayer, LevFile::Lev), 144 + 1);
        assert_eq!(l(LevelId::GEM_STONE_VALLEY, Lod::OnePlayer, LevFile::Vram), 200);
        assert_eq!(l(LevelId::MAIN_MENU_LEVEL, Lod::OnePlayer, LevFile::Vram), 215);
        assert_eq!(
            l(LevelId::SCRAPBOOK, Lod::OnePlayer, LevFile::Vram),
            BigIndex::SCRAPBOOK.0
        );
    }
}
