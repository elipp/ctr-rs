//! `data.MetaDataTerrain`: how each surface type changes the physics.

/// `TerrainFlags`
pub mod terrain_flags {
    pub const RAISE_GROUND_OFFSET: u32 = 0x1;
    pub const ACCEL_WHILE_REVERSE_SLIDING: u32 = 0x4;
    pub const FORCE_SKIDMARKS: u32 = 0x8;
    pub const SKIP_TURN_ASSIST: u32 = 0x10;
    pub const ONESHOT_GROUND_SOUND: u32 = 0x20;
    pub const LANDING_SPARKS: u32 = 0x40;
    pub const MUD_PHYSICS: u32 = 0x80;
    pub const SIDESLIP_FRICTION: u32 = 0x100;
}

/// `TerrainType` of the surfaces the table has an entry for.
pub mod terrain_type {

    pub const ASPHALT: u8 = 0;
    pub const DIRT: u8 = 1;
    pub const GRASS: u8 = 2;
    pub const WOOD: u8 = 3;
    pub const WATER: u8 = 4;
    pub const STONE: u8 = 5;
    pub const ICE: u8 = 6;
    pub const TRACK: u8 = 7;
    pub const ICY_ROAD: u8 = 8;
    pub const SNOW: u8 = 9;
    pub const NONE: u8 = 10;
    pub const HARDPACK: u8 = 11;
    pub const METAL: u8 = 12;
    pub const FASTWATER: u8 = 13;
    pub const MUD: u8 = 14;
    pub const SIDESLIP: u8 = 15;
    pub const RIVERASPHALT: u8 = 16;
    pub const STEAMASPHALT: u8 = 17;
    pub const OCEANASPHALT: u8 = 18;
    pub const SLOWGRASS: u8 = 19;
    pub const SLOWDIRT: u8 = 20;
}

/// `struct Terrain`, without the particle emitters, skid sounds and bot data
/// (none of which affect the simulation).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Terrain {
    pub name: &'static str,
    pub flags: u32,
    /// 0x100 is neutral.
    pub speed_multiplier: i32,
    pub slow_until_speed: i32,
    /// 0% ice, 100% road.
    pub counter_steer_ratio: i32,
    pub turn_lean_scale: i32,
    pub ground_friction_scale: i32,
    pub turn_angle_scale: i32,
    pub turn_response_scale: i32,
    /// The sound played when the kart leaves the ground offset (`sound`).
    pub sound: i16,
}

/// `data.MetaDataTerrain`, indexed by terrain type.
pub const TERRAINS: [Terrain; 21] = [
    Terrain {
        name: "asphalt",
        flags: 64,
        speed_multiplier: 256,
        slow_until_speed: 256,
        counter_steer_ratio: 256,
        turn_lean_scale: 256,
        ground_friction_scale: 256,
        turn_angle_scale: 256,
        turn_response_scale: 256,
        sound: -1,
    },
    Terrain {
        name: "dirt",
        flags: 0,
        speed_multiplier: 140,
        slow_until_speed: 180,
        counter_steer_ratio: 256,
        turn_lean_scale: 256,
        ground_friction_scale: 256,
        turn_angle_scale: 256,
        turn_response_scale: 256,
        sound: 24,
    },
    Terrain {
        name: "grass",
        flags: 0,
        speed_multiplier: 140,
        slow_until_speed: 180,
        counter_steer_ratio: 256,
        turn_lean_scale: 256,
        ground_friction_scale: 256,
        turn_angle_scale: 256,
        turn_response_scale: 256,
        sound: 24,
    },
    Terrain {
        name: "wood",
        flags: 33,
        speed_multiplier: 256,
        slow_until_speed: 256,
        counter_steer_ratio: 256,
        turn_lean_scale: 256,
        ground_friction_scale: 256,
        turn_angle_scale: 256,
        turn_response_scale: 256,
        sound: 21,
    },
    Terrain {
        name: "water",
        flags: 0,
        speed_multiplier: 180,
        slow_until_speed: 200,
        counter_steer_ratio: 256,
        turn_lean_scale: 256,
        ground_friction_scale: 256,
        turn_angle_scale: 256,
        turn_response_scale: 256,
        sound: 25,
    },
    Terrain {
        name: "stone",
        flags: 64,
        speed_multiplier: 256,
        slow_until_speed: 256,
        counter_steer_ratio: 256,
        turn_lean_scale: 256,
        ground_friction_scale: 256,
        turn_angle_scale: 256,
        turn_response_scale: 256,
        sound: -1,
    },
    Terrain {
        name: "ice",
        flags: 28,
        speed_multiplier: 256,
        slow_until_speed: 100,
        counter_steer_ratio: 0,
        turn_lean_scale: 0,
        ground_friction_scale: 32,
        turn_angle_scale: 128,
        turn_response_scale: 32,
        sound: 22,
    },
    Terrain {
        name: "track",
        flags: 33,
        speed_multiplier: 140,
        slow_until_speed: 180,
        counter_steer_ratio: 256,
        turn_lean_scale: 256,
        ground_friction_scale: 256,
        turn_angle_scale: 256,
        turn_response_scale: 256,
        sound: 21,
    },
    Terrain {
        name: "icy_road",
        flags: 64,
        speed_multiplier: 256,
        slow_until_speed: 256,
        counter_steer_ratio: 256,
        turn_lean_scale: 256,
        ground_friction_scale: 128,
        turn_angle_scale: 256,
        turn_response_scale: 256,
        sound: -1,
    },
    Terrain {
        name: "snow",
        flags: 0,
        speed_multiplier: 140,
        slow_until_speed: 180,
        counter_steer_ratio: 256,
        turn_lean_scale: 256,
        ground_friction_scale: 128,
        turn_angle_scale: 256,
        turn_response_scale: 256,
        sound: 23,
    },
    Terrain {
        name: "none",
        flags: 0,
        speed_multiplier: 256,
        slow_until_speed: 256,
        counter_steer_ratio: 0,
        turn_lean_scale: 256,
        ground_friction_scale: 256,
        turn_angle_scale: 256,
        turn_response_scale: 256,
        sound: -1,
    },
    Terrain {
        name: "hardpack",
        flags: 0,
        speed_multiplier: 256,
        slow_until_speed: 256,
        counter_steer_ratio: 256,
        turn_lean_scale: 256,
        ground_friction_scale: 256,
        turn_angle_scale: 256,
        turn_response_scale: 256,
        sound: -1,
    },
    Terrain {
        name: "metal",
        flags: 64,
        speed_multiplier: 256,
        slow_until_speed: 256,
        counter_steer_ratio: 256,
        turn_lean_scale: 256,
        ground_friction_scale: 256,
        turn_angle_scale: 256,
        turn_response_scale: 256,
        sound: 26,
    },
    Terrain {
        name: "fastwater",
        flags: 0,
        speed_multiplier: 256,
        slow_until_speed: 256,
        counter_steer_ratio: 256,
        turn_lean_scale: 256,
        ground_friction_scale: 256,
        turn_angle_scale: 256,
        turn_response_scale: 256,
        sound: 25,
    },
    Terrain {
        name: "mud",
        flags: 128,
        speed_multiplier: 140,
        slow_until_speed: 180,
        counter_steer_ratio: 256,
        turn_lean_scale: 256,
        ground_friction_scale: 512,
        turn_angle_scale: 256,
        turn_response_scale: 256,
        sound: 27,
    },
    Terrain {
        name: "sideslip",
        flags: 320,
        speed_multiplier: 256,
        slow_until_speed: 256,
        counter_steer_ratio: 256,
        turn_lean_scale: 256,
        ground_friction_scale: 256,
        turn_angle_scale: 256,
        turn_response_scale: 256,
        sound: -1,
    },
    Terrain {
        name: "riverasphalt",
        flags: 64,
        speed_multiplier: 256,
        slow_until_speed: 256,
        counter_steer_ratio: 256,
        turn_lean_scale: 256,
        ground_friction_scale: 256,
        turn_angle_scale: 256,
        turn_response_scale: 256,
        sound: -1,
    },
    Terrain {
        name: "steamasphalt",
        flags: 64,
        speed_multiplier: 256,
        slow_until_speed: 256,
        counter_steer_ratio: 256,
        turn_lean_scale: 256,
        ground_friction_scale: 256,
        turn_angle_scale: 256,
        turn_response_scale: 256,
        sound: -1,
    },
    Terrain {
        name: "oceanasphalt",
        flags: 64,
        speed_multiplier: 256,
        slow_until_speed: 256,
        counter_steer_ratio: 256,
        turn_lean_scale: 256,
        ground_friction_scale: 256,
        turn_angle_scale: 256,
        turn_response_scale: 256,
        sound: -1,
    },
    Terrain {
        name: "slowgrass",
        flags: 0,
        speed_multiplier: 100,
        slow_until_speed: 180,
        counter_steer_ratio: 256,
        turn_lean_scale: 256,
        ground_friction_scale: 256,
        turn_angle_scale: 256,
        turn_response_scale: 256,
        sound: 24,
    },
    Terrain {
        name: "slowdirt",
        flags: 0,
        speed_multiplier: 100,
        slow_until_speed: 180,
        counter_steer_ratio: 256,
        turn_lean_scale: 256,
        ground_friction_scale: 256,
        turn_angle_scale: 256,
        turn_response_scale: 256,
        sound: 24,
    },
];

/// `VehAfterColl_GetTerrain`: out-of-range types are the first entry.
pub fn terrain(terrain_type: u8) -> &'static Terrain {
    &TERRAINS[if (terrain_type as usize) < TERRAINS.len() {
        terrain_type as usize
    } else {
        0
    }]
}
