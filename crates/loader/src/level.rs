/// Level IDs as used by the game (`enum LevelID`). Not an enum: the original
/// accepts arbitrary values (e.g. synthetic adventure IDs >= 100) and falls
/// through to the adventure hub levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LevelId(pub u32);

impl LevelId {
    pub const DINGO_CANYON: Self = Self(0);
    pub const DRAGON_MINES: Self = Self(1);
    pub const BLIZZARD_BLUFF: Self = Self(2);
    pub const CRASH_COVE: Self = Self(3);
    pub const TIGER_TEMPLE: Self = Self(4);
    pub const PAPU_PYRAMID: Self = Self(5);
    pub const ROO_TUBES: Self = Self(6);
    pub const HOT_AIR_SKYWAY: Self = Self(7);
    pub const SEWER_SPEEDWAY: Self = Self(8);
    pub const MYSTERY_CAVES: Self = Self(9);
    pub const CORTEX_CASTLE: Self = Self(10);
    pub const N_GIN_LABS: Self = Self(11);
    pub const POLAR_PASS: Self = Self(12);
    pub const OXIDE_STATION: Self = Self(13);
    pub const COCO_PARK: Self = Self(14);
    pub const TINY_ARENA: Self = Self(15);
    pub const SLIDE_COLISEUM: Self = Self(16);
    pub const TURBO_TRACK: Self = Self(17);
    pub const NITRO_COURT: Self = Self(18);
    pub const RAMPAGE_RUINS: Self = Self(19);
    pub const PARKING_LOT: Self = Self(20);
    pub const SKULL_ROCK: Self = Self(21);
    pub const THE_NORTH_BOWL: Self = Self(22);
    pub const ROCKY_ROAD: Self = Self(23);
    pub const LAB_BASEMENT: Self = Self(24);
    pub const GEM_STONE_VALLEY: Self = Self(25);
    pub const N_SANITY_BEACH: Self = Self(26);
    pub const THE_LOST_RUINS: Self = Self(27);
    pub const GLACIER_PARK: Self = Self(28);
    pub const CITADEL_CITY: Self = Self(29);
    pub const INTRO_RACE_TODAY: Self = Self(30);
    pub const INTRO_COCO: Self = Self(31);
    pub const INTRO_TINY: Self = Self(32);
    pub const INTRO_POLAR: Self = Self(33);
    pub const INTRO_DINGODILE: Self = Self(34);
    pub const INTRO_CORTEX: Self = Self(35);
    pub const INTRO_SPACE: Self = Self(36);
    pub const INTRO_CRASH: Self = Self(37);
    pub const INTRO_OXIDE: Self = Self(38);
    pub const MAIN_MENU_LEVEL: Self = Self(39);
    pub const ADVENTURE_GARAGE: Self = Self(40);
    pub const NAUGHTY_DOG_CRATE: Self = Self(41);
    pub const OXIDE_ENDING: Self = Self(42);
    pub const OXIDE_TRUE_ENDING: Self = Self(43);
    pub const CREDITS_CRASH: Self = Self(44);
    pub const CREDITS_CORTEX: Self = Self(45);
    pub const CREDITS_TINY: Self = Self(46);
    pub const CREDITS_COCO: Self = Self(47);
    pub const CREDITS_N_GIN: Self = Self(48);
    pub const CREDITS_DINGO: Self = Self(49);
    pub const CREDITS_POLAR: Self = Self(50);
    pub const CREDITS_PURA: Self = Self(51);
    pub const CREDITS_PINSTRIPE: Self = Self(52);
    pub const CREDITS_PAPU: Self = Self(53);
    pub const CREDITS_ROO: Self = Self(54);
    pub const CREDITS_JOE: Self = Self(55);
    pub const CREDITS_TROPY: Self = Self(56);
    pub const CREDITS_PENTA: Self = Self(57);
    pub const CREDITS_FAKE_CRASH: Self = Self(58);
    pub const CREDITS_OXIDE: Self = Self(59);
    pub const CREDITS_AMI: Self = Self(60);
    pub const CREDITS_ISABELLA: Self = Self(61);
    pub const CREDITS_LIZ: Self = Self(62);
    pub const CREDITS_MEGUMI: Self = Self(63);
    pub const SCRAPBOOK: Self = Self(64);
}
