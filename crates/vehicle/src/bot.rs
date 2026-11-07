//! The bots (`BOTS.c`): karts that do not run the driving physics, but slide along one of the
//! level's three nav paths ([`loader::lev::NavPath`]) at a speed of their own, which a rubber
//! band ties to the best human's place.
//!
//! [`Bots`] is what `sdata` keeps for all of them (the difficulty, the paths and grid order, their
//! RNG); a bot's own state is [`BotData`] in its [`Driver`]. [`Bots::spawn`] is
//! `BOTS_Driver_Init`, [`tick_all`] runs `BOTS_ThTick_Drive` (or `BOTS_ThTick_RevEngine`) of each,
//! and [`Bots::update_globals`] is `BOTS_UpdateGlobals`.
//!
//! [`hurt`] is `BOTS_ChangeState`, what a hit does to a bot: the drive tick plays it out.
//!
//! A bump pushes a bot off its path ([`crash_ai`], from [`crate::collide`]), and of two bots that
//! bump the one behind slows down ([`collide_with_other_ai`]).
//!
//! What the bots fire (`PickupBots_UpdateArcade`) is the race's (`race::Race`): it needs the
//! weapons.
//!
//! A bot that fell is brought back by the mask (`BOTS_MaskGrab`), the player's
//! [`crate::stuck::MaskHead`]: it carries the kart down to its path and lets go there.
//!
//! Not ported yet: the level's objects (`BOTS_LevInstColl`), the camera and sounds of a player's
//! kart that a bot drives, and the adventure and boss modes (their difficulty and
//! `PickupBots_UpdateBoss`).

mod difficulty;

use coll::Collision;
use gte::Gte;
use loader::lev::{
    Level, NAV_PATH_COUNT, NavFrame, NavPath, SVec3, nav_flags, nav_special, quad_flags,
};
use math::{Vec3, mips};

use crate::force::{KART_SCALE_BASE, mvmva, rot_axis_angle, translate_matrix};
use crate::rotmat::{convert_rot_to_matrix, rot_to_matrix};
use crate::{Driver, Effect, actions, fire, forced_jump, frame, kart_state};

/// `BotFlags` (`BotData::botFlags`)
pub mod flags {
    /// The bot is between its path's frames, heading for `nav_frame` from where it was put
    /// ([`BotData::estimate`]).
    pub const ESTIMATE_NAV: u32 = 0x1;
    pub const DAMAGE_ACTIVE: u32 = 0x2;
    pub const DAMAGE_SUPPRESS_EMITTER: u32 = 0x4;
    /// Knocked off the path: it slides on the ground until it stops.
    pub const FREE_PHYSICS: u32 = 0x8;
    pub const NAV_BOOST_ACTIVE: u32 = 0x10;
    pub const MOON_GRAVITY: u32 = 0x20;
    pub const BOSS_PATH_REQUESTED: u32 = 0x40;
    pub const BOSS_PATH_ACTIVE: u32 = 0x80;
    pub const DEMO_CAMERA_STARTED: u32 = 0x100;
    pub const STARTLINE_INIT_DONE: u32 = 0x200;
}

/// `BOTS_DAMAGE_STATE_*` (`BotData::aiDamageState`)
pub mod damage_state {
    pub const SPIN: i16 = 1;
    pub const BLAST: i16 = 2;
    pub const SQUISH: i16 = 3;
    pub const MASK_GRAB: i16 = 5;
}

/// The arcade difficulties of the menu (`D230.cupDifficulty.speed`), and the cheat's.
pub mod difficulty_level {
    pub const EASY: i16 = 0x50;
    pub const MEDIUM: i16 = 0xa0;
    pub const HARD: i16 = 0xf0;
    /// `BOTS_DIFFICULTY_SUPERHARD`
    pub const SUPER_HARD: i16 = 0x140;
}

/// `GAME_CHARACTER_IDS` of Nitros Oxide, who cheats.
pub const NITROS_OXIDE: u8 = 15;

const MAX_KARTS: usize = 8;
/// `BOTS_DIFFICULTY_BLEND_DENOMINATOR`
const DIFFICULTY_BLEND: i32 = 0xf0;
/// `BOTS_DIFFICULTY_CUP_OFFSET`
const CUP_DIFFICULTY_OFFSET: i16 = 0x50;
/// `sdata->AI_AccelFrameCount` and `AI_AccelFrameSteps`: how long a bot's start is slowed, and by
/// how much for each place in the acceleration order.
const ACCEL_FRAME_COUNT: i32 = 0x0a;
const ACCEL_FRAME_STEPS: i32 = 0x18;
/// `BOTS_RACE_START_AI_COLLISION_DELAY_FRAMES`: the bots keep their paths for 15 seconds.
const PATH_CHANGE_DELAY_FRAMES: i32 = 15 * 30;
/// `gGT->constVal_9000`: the speed of a bot that has nobody to chase.
const UNCHASED_SPEED: i32 = 9000;
/// `BOTS_MASK_DROP_HEIGHT`
const MASK_DROP_HEIGHT: i32 = 0x10000;
/// `BOTS_PATH_CHANGE_PATH_SHIFT`, `_FRAME_MASK`
const PATH_CHANGE_PATH_SHIFT: u32 = 10;
const PATH_CHANGE_FRAME_MASK: i32 = 0x3ff;
/// `NavFrame::flags` bit the kill plane's step back skips over.
const NAV_FLAG_NO_RESPAWN: u16 = 0x4000;

/// `data.botsThrottle`: the frame of each path from which a bot brakes for ten frames, on the
/// three tracks that have one.
const THROTTLE_HOT_AIR_SKYWAY: [i32; 3] = [0x1cc, 0x1bd, 0x1b4];
const THROTTLE_POLAR_PASS: [i32; 3] = [0x15e, 0x151, 0x156];
const THROTTLE_PAPU_PYRAMID: [i32; 3] = [0xf0, 0xe9, 0xf6];

/// `Terrain::bot` and `botFrictionScale`, by terrain type: how a bot's speed takes the surface.
#[derive(Clone, Copy)]
struct BotTerrain {
    /// `TERRAIN_BOT_FLAG_DECEL_TO_TARGET_SPEED`: the bot slows to the surface's speed at once.
    decel_to_target: bool,
    target_speed_scale: i32,
    acceleration_scale: i32,
    friction_scale: i32,
}

const fn terrain(decel_to_target: bool, target: i32, accel: i32, friction: i32) -> BotTerrain {
    BotTerrain {
        decel_to_target,
        target_speed_scale: target,
        acceleration_scale: accel,
        friction_scale: friction,
    }
}

/// In [`crate::terrain::TERRAINS`] order.
const BOT_TERRAINS: [BotTerrain; 21] = [
    terrain(false, 0x100, 0x100, 0x100), // asphalt
    terrain(false, 0x8c, 0xb4, 0x100),   // dirt
    terrain(false, 0x8c, 0xb4, 0x100),   // grass
    terrain(false, 0x100, 0x100, 0x100), // wood
    terrain(false, 0xb4, 0xc8, 0x100),   // water
    terrain(false, 0x100, 0x100, 0x100), // stone
    terrain(false, 0x100, 0x64, 0x20),   // ice
    terrain(false, 0x8c, 0xb4, 0x100),   // track
    terrain(false, 0x100, 0x100, 0x80),  // icy road
    terrain(false, 0x8c, 0xb4, 0x80),    // snow
    terrain(false, 0x100, 0x100, 0x100), // none
    terrain(false, 0x100, 0x100, 0x100), // hardpack
    terrain(false, 0x100, 0x100, 0x100), // metal
    terrain(false, 0x100, 0x100, 0x100), // fastwater
    terrain(true, 0x8c, 0xb4, 0x200),    // mud
    terrain(false, 0x100, 0x100, 0x100), // sideslip
    terrain(false, 0x100, 0x100, 0x100), // riverasphalt
    terrain(false, 0x100, 0x100, 0x100), // steamasphalt
    terrain(false, 0x100, 0x100, 0x100), // oceanasphalt
    terrain(false, 0x64, 0xb4, 0x100),   // slowgrass
    terrain(false, 0x64, 0xb4, 0x100),   // slowdirt
];

/// `VehAfterColl_GetTerrain`: a type past the table is asphalt.
fn terrain_index(terrain_type: u8) -> u8 {
    if (terrain_type as usize) < BOT_TERRAINS.len() {
        terrain_type
    } else {
        0
    }
}

/// `struct RngDeadCoedState` and `RngDeadCoed`: the bots' own RNG (`sdata->advRng`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rng {
    state0: u32,
    state1: u32,
}

impl Default for Rng {
    /// The seed `BOTS_Adv_AdjustDifficulty` gives a state that is still zero.
    fn default() -> Rng {
        Rng {
            state0: 0x3021_5400,
            state1: 0x4935_83fe,
        }
    }
}

impl Rng {
    pub fn draw(&mut self) -> i32 {
        let mix = self.state1 << 24;
        let mut state1 = self.state1 >> 8;
        let mut feedback = self.state0.wrapping_add(state1);
        let state0 = (self.state0 >> 8) | mix;
        feedback = feedback.wrapping_add(state0 >> 8);
        state1 |= feedback << 24;
        state1 ^= 0xdead_c0ed;
        self.state0 = state0;
        self.state1 = state1;
        state1 as i32
    }
}

/// `struct BotPhysics`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Physics {
    /// The turn of a blasted kart, and the timer of a squish and of a mask grab.
    pub rot_xz: i16,
    pub drift_target: i16,
    pub mul_drift: i16,
    pub simp_turn_state: i16,
    /// How long the bot has been drifting, in milliseconds: it boosts as a player would.
    pub turbo_meter: i16,
    pub fire_level: i16,
    pub squish_cooldown: i32,
    pub speed_y: i32,
    /// The speed along the path.
    pub speed_linear: i32,
    pub accel: Vec3,
    /// A push off the path (with [`flags::FREE_PHYSICS`]).
    pub velocity: Vec3,
}

/// `struct BotData`: a bot's own state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BotData {
    /// `botPath`: which of the level's nav paths it follows.
    pub path: usize,
    /// `botNavFrame`: the frame of the path it last passed (or, with [`flags::ESTIMATE_NAV`],
    /// is heading for).
    pub nav_frame: usize,
    /// `navProgressRemainder`: how far past that frame it is, 24.8.
    pub nav_progress_remainder: i32,
    /// [`flags`]
    pub flags: u32,
    /// `botAccel`: ticks left of the slow start.
    pub accel: i32,
    /// [`damage_state`]
    pub damage_state: i16,
    pub phys: Physics,
    /// `positionBackup`: where the path puts the kart, before [`Physics::velocity`].
    pub position_backup: Vec3,
    /// `aiRot`
    pub rot: [i16; 3],
    /// `ai_progress_cooldown`: ticks the bot waits before it moves on.
    pub progress_cooldown: i32,
    /// `ai_rotY_608`: the heading the camera of a kart a bot drives lags behind.
    pub rot_y_608: i16,
    /// `ai_quadblock_checkpointIndex`: the restart point it is at, for the lap counter.
    pub checkpoint_index: u8,
    /// `estimateNavFrame`: a frame made up where the bot was put, aimed at `nav_frame`.
    pub estimate: NavFrame,
    pub weapon_cooldown: i16,
    pub blast_bounce_count: u8,
    /// The thread runs `BOTS_ThTick_RevEngine`: the mask lowers the kart back onto the path.
    pub revving: bool,
}

/// How the bots of a race are set up (what `BOTS_Adv_AdjustDifficulty` reads of the game mode).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Setup {
    /// `gGT->arcadeDifficulty`: see [`difficulty_level`].
    pub difficulty: i16,
    /// The racing track (a `LevelId`, 0 to 17) whose rubber band, brakes and quirks the bots
    /// use; none for a track of no slot of the game's (the first's are taken).
    pub track: Option<u8>,
    /// The race is one of a cup: the bots chase by their start order instead of by place.
    pub cup: bool,
    /// `numPlyrCurrGame`: the human karts, which have the slots before the bots'.
    pub players: usize,
}

/// What `sdata` keeps for the bots of a race.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bots {
    pub setup: Setup,
    /// `advRng`
    pub rng: Rng,
    /// `arcade_difficultyParams`: the rubber band by place (0 to 7), then the speed it gives
    /// (8 to 10) and the slack by lap (11 to 13).
    pub arcade: [i16; 14],
    /// `cup_difficultyParams`, in a cup.
    pub cup: [i16; 14],
    /// `accelerateOrder`, by driver.
    pub accelerate_order: [u8; MAX_KARTS],
    /// `driver_pathIndexIDs`
    pub driver_paths: [u8; MAX_KARTS],
    /// `kartSpawnOrderArray`
    pub spawn_order: [u8; MAX_KARTS],
    /// `navBotList`: the drivers on each path, the newest first.
    pub lists: [Vec<u8>; NAV_PATH_COUNT],
    /// `aiCollisionDelayFrameCount`
    pub path_change_delay: i32,
    /// `bestHumanRank`, as a slot.
    pub best_human: Option<usize>,
}

impl Bots {
    /// The arcade part of `BOTS_Adv_AdjustDifficulty`, for a race at the start of a cup (or of
    /// none): the rubber band, the paths and the acceleration order of the grid.
    pub fn new(setup: Setup) -> Bots {
        let row = difficulty::ARCADE_DIFFICULTY[setup.track.map_or(0, |t| t as usize)];
        let blend = |factor: i16| -> [i16; 14] {
            let (lo, hi) = (row[0], row[1]);
            std::array::from_fn(|i| {
                let part = factor as i32 * (hi[i] as i32 - lo[i] as i32) / DIFFICULTY_BLEND;
                (lo[i] as u16 as i32).wrapping_add(part) as i16
            })
        };
        let difficulty = setup.difficulty.max(0);
        let arcade = blend(difficulty);
        let cup =
            blend((setup.difficulty as u16).wrapping_add(CUP_DIFFICULTY_OFFSET as u16) as i16);

        let mut rng = Rng::default();
        let spawn_order = crate::grid::spawn_order(
            crate::grid::GridMode::Arcade,
            setup.players,
            std::array::from_fn(|i| i as u8),
        );

        let mut path_order = [0u8, 0, 0, 2, 0, 0, 0, 2];
        let first = (rng.draw() >> 8) as u8 & 1;
        path_order[1] = first;
        path_order[5] = first ^ 1;
        let second = (rng.draw() >> 8) as u8 & 1;
        path_order[2] = second + 1;
        path_order[6] = (second ^ 1) + 1;
        let driver_paths = spawn_order.map(|place| path_order[place as usize]);

        let mut accel_order = [0u8; MAX_KARTS];
        let mut accel = (rng.draw() >> 8) as u8 & 3;
        let mut rear = (rng.draw() >> 8) as u8 & 3;
        for i in 0..4 {
            accel_order[i] = accel;
            accel = (accel + 1) & 3;
            accel_order[i + 4] = rear + 4;
            rear = rear.wrapping_sub(1) & 3;
        }
        let accelerate_order = spawn_order.map(|place| accel_order[place as usize]);

        Bots {
            setup,
            rng,
            arcade,
            cup,
            accelerate_order,
            driver_paths,
            spawn_order,
            lists: Default::default(),
            path_change_delay: 0,
            best_human: None,
        }
    }

    /// `BOTS_Driver_Init` and `BOTS_GotoStartingLine`: makes `d` (a new driver, its slot its
    /// `driver_id`) a bot on the grid. False when the level has no path for it to follow, and it
    /// is no bot.
    pub fn spawn(&mut self, d: &mut Driver, coll: &mut Collision, level: &Level) -> bool {
        let id = d.driver_id as usize;
        let first = self.driver_paths[id] as usize % NAV_PATH_COUNT;
        let mut path = first;
        // The path the grid gives it, or the one before that has frames.
        while level.nav_paths[path].frames.len() <= 1 {
            path = (path + NAV_PATH_COUNT - 1) % NAV_PATH_COUNT;
            if path == first {
                return false;
            }
        }
        d.actions_flag_set |= actions::BOT;
        d.bot = Some(BotData {
            path,
            ..BotData::default()
        });
        self.lists[path].insert(0, d.driver_id);
        self.goto_starting_line(d, coll, level);
        true
    }

    /// `BOTS_GotoStartingLine`
    fn goto_starting_line(&mut self, d: &mut Driver, coll: &mut Collision, level: &Level) {
        self.path_change_delay = 0;
        let place = self.spawn_order[d.driver_id as usize];
        crate::sim::spawn(d, coll, level, place as usize);
        d.init_pending = false;
        let mut b = d.bot.take().expect("a bot");
        b.position_backup = d.pos_curr;
        b.phys.velocity = Vec3::default();
        b.phys.accel = Vec3::default();
        b.phys.speed_linear = 0;
        b.phys.speed_y = 0;
        b.nav_progress_remainder = 0;
        b.nav_frame = 0;
        set_rotation(d, &mut b, level, true);
        let accel = self.accelerate_order[place as usize] as i32;
        d.rot_curr_z = 0;
        d.rot_curr_x = 0;
        d.rot_prev[0] = 0;
        d.rot_prev[2] = 0;
        b.rot[0] = 0;
        b.rot[2] = 0;
        d.turn_angle_curr = 0;
        d.actions_flag_set |= actions::BOT;
        let rot_y = mips::sll(b.estimate.rot[1] as i32, 4) as i16;
        b.rot_y_608 = rot_y;
        d.angle = rot_y;
        d.rot_curr_y = rot_y;
        d.rot_prev[1] = rot_y;
        b.rot[1] = rot_y;
        b.accel = mips::mul_lo(ACCEL_FRAME_COUNT, accel);
        let rng = self.rng.draw();
        b.weapon_cooldown = mips::add_lo(mips::sra(rng, 8) & 0xff, 300) as i16;
        d.bot = Some(b);
    }

    /// `BOTS_UpdateGlobals`, after every thread of a tick: the best human (or, with none, the
    /// last of the bots) is the one the bots chase. `drivers` by slot.
    pub fn update_globals(&mut self, drivers: &[&Driver]) {
        self.path_change_delay += 1;
        let by_rank = |rank: usize| drivers.iter().position(|d| d.driver_rank == rank as i16);
        let mut best_human = None;
        let mut worst_bot = None;
        for rank in (0..MAX_KARTS).rev() {
            let Some(slot) = by_rank(rank) else {
                continue;
            };
            if drivers[slot].actions_flag_set & actions::BOT != 0 {
                worst_bot.get_or_insert(slot);
            } else {
                best_human = Some(slot);
            }
        }
        self.best_human = best_human.or(worst_bot);
    }

    /// `BOTS_GetTrackDistanceToFinish`
    fn track_distance(level: &Level) -> i32 {
        level
            .restart_points
            .first()
            .map_or(0, |p| mips::sll(p.dist_to_finish as i32, 3))
    }

    fn throttle(&self) -> Option<[i32; 3]> {
        match self.setup.track? {
            7 => Some(THROTTLE_HOT_AIR_SKYWAY),
            12 => Some(THROTTLE_POLAR_PASS),
            5 => Some(THROTTLE_PAPU_PYRAMID),
            _ => None,
        }
    }

    /// Tiny Arena, whose kill planes put a bot back by checkpoint (`BOTS_Killplane`).
    fn tiny_arena(&self) -> bool {
        self.setup.track == Some(15)
    }
}

/// What a bot reads of another driver: a copy taken just before the bot's tick.
#[derive(Clone, Copy, Debug)]
struct Peer {
    bot: bool,
    nav_frame: usize,
    distance_to_finish_curr: i32,
    lap_index: i32,
    actions_flag_set: u32,
    num_wumpas: i32,
    turbo_const: i32,
    speed_class_stat: i32,
    accel_speed_class_stat: i32,
    driver_rank: i32,
}

impl Peer {
    fn of(d: &Driver) -> Peer {
        Peer {
            bot: d.bot.is_some(),
            nav_frame: d.bot.as_ref().map_or(0, |b| b.nav_frame),
            distance_to_finish_curr: d.distance_to_finish_curr as i32,
            lap_index: d.lap_index as i32,
            actions_flag_set: d.actions_flag_set,
            num_wumpas: d.num_wumpas as i32,
            turbo_const: d.turbo_const as i32,
            speed_class_stat: d.stats.speed_class_stat as i32,
            accel_speed_class_stat: d.stats.accel_speed_class_stat as i32,
            driver_rank: d.driver_rank as i32,
        }
    }
}

/// What the game tells a bot's tick.
#[derive(Clone, Copy, Debug)]
pub struct Env {
    pub elapsed_time_ms: i32,
    /// `trafficLightsTimer`: the bots wait while it is above 0.
    pub traffic_lights_timer: i32,
    /// `numLaps`
    pub laps: u8,
    /// `gGT->timer`
    pub timer: u32,
    /// Whether karts bump into each other (see `vehicle::Env::kart_collisions`).
    pub kart_collisions: bool,
}

/// A kart of [`tick_all`].
pub struct Kart<'a> {
    pub driver: &'a mut Driver,
    pub coll: &'a mut Collision,
    pub effects: &'a mut Vec<Effect>,
}

/// The `ROBOT` bucket's threads: every bot's tick, in the bucket's order (the last born first).
/// `karts` is every kart by slot; the humans' are only read.
pub fn tick_all(karts: &mut [Kart], bots: &mut Bots, level: &Level, env: &Env) {
    for slot in (0..karts.len()).rev() {
        if karts[slot].driver.bot.is_none() {
            continue;
        }
        let peers: Vec<Peer> = karts.iter().map(|k| Peer::of(k.driver)).collect();
        // The threads after this one in the bucket: the bots born before it.
        let (before, from) = karts.split_at_mut(slot);
        let siblings: Vec<&mut Driver> = before
            .iter_mut()
            .rev()
            .filter(|k| k.driver.bot.is_some())
            .map(|k| &mut *k.driver)
            .collect();
        let kart = &mut from[0];
        let mut b = kart.driver.bot.take().expect("a bot");
        let mut tick = Tick {
            d: kart.driver,
            b: &mut b,
            coll: kart.coll,
            bots,
            level,
            env,
            peers: &peers,
            siblings,
            effects: kart.effects,
        };
        if tick.b.revving {
            tick.rev_engine();
        } else {
            tick.drive();
        }
        // The mask's own thread.
        crate::stuck::mask_tick(kart.driver, env.elapsed_time_ms);
        kart.driver.bot = Some(b);
    }
}

/// `BOTS_SetRotation`: aims the bot from where it is at its nav frame, as a frame of its own
/// ([`BotData::estimate`]), facing the grid's way with `spawn_yaw`.
fn set_rotation(d: &mut Driver, b: &mut BotData, level: &Level, spawn_yaw: bool) {
    let nf = level.nav_paths[b.path].frames[b.nav_frame];
    b.phys.velocity = Vec3::default();
    let e = &mut b.estimate;
    e.pos = SVec3 {
        x: mips::sra(d.pos_curr.x, 8) as i16,
        y: mips::sra(d.pos_curr.y, 8) as i16,
        z: mips::sra(d.pos_curr.z, 8) as i16,
    };
    let dx = mips::sub_lo(nf.pos.x as i32, e.pos.x as i32);
    let dz = mips::sub_lo(nf.pos.z as i32, e.pos.z as i32);
    let dy = mips::sub_lo(nf.pos.y as i32, e.pos.y as i32);
    let (xx, zz, yy) = (
        mips::mul_lo(dx, dx),
        mips::mul_lo(dz, dz),
        mips::mul_lo(dy, dy),
    );
    e.dist_to_next_nav_xz = math::square_root0_stub(mips::add_lo(xx, zz)) as i16;
    e.dist_to_next_nav_xyz = math::square_root0_stub(mips::add_lo(mips::add_lo(xx, yy), zz)) as i16;
    let pitch = math::ratan2(
        mips::sll(dy, 12),
        mips::sll(e.dist_to_next_nav_xz as i32, 12),
    );
    e.rot[3] = mips::sra(pitch, 4) as u8;
    b.nav_progress_remainder = 0;
    if spawn_yaw {
        let yaw = level.driver_spawns[0].rot.y as i32;
        e.rot[1] = mips::sra(mips::add_lo(yaw, 0x400), 4) as u8;
    } else {
        e.rot[0] = nf.rot[0];
        let yaw = math::ratan2(mips::neg_lo(dx), mips::neg_lo(dz));
        e.rot[1] = mips::sra(mips::add_lo(yaw, 0x800), 4) as u8;
        e.rot[2] = nf.rot[2];
    }
    let v = mips::sll(e.rot[1] as i32, 4) as i16;
    b.rot_y_608 = v;
    d.angle = v;
    d.rot_curr_y = v;
    d.rot_prev[1] = v;
    b.rot[1] = v;
    b.flags |= flags::ESTIMATE_NAV;
}

/// `sdata->AI_VelY_WhenBlasted_0x3000`: how high a blast throws a bot.
const BLAST_SPEED_Y: i32 = 0x3000;
/// `BotPhysics::squishCooldown` of a spin and of a squish: the turn they wind down from.
const SPIN_COOLDOWN: i32 = 0x300;
/// `rotXZ` of a squish (how long it lasts) and of a mask grab.
const SQUISH_TIME: i16 = 0xf00;
const MASK_GRAB_TIME: i16 = 0xd20;
/// `burnTimer` and `squishTimer` that a burn and a squish give.
const BURN_TIMER: i16 = 0xf00;
const SQUISH_TIMER: i16 = 0xf00;

/// `BOTS_ChangeState`: what a hit of `damage` (a [`crate::pick::damage`]) does to the bot `d`.
/// `by_player` says whether a player's kart hit it (none for a hit nobody is to blame for): the
/// bot a player blasts says so. The result is the game's: whether the bot was hurt. (A bot has
/// no shield or mask to take it.)
pub fn hurt(
    d: &mut Driver,
    damage: u8,
    by_player: Option<bool>,
    effects: &mut Vec<Effect>,
) -> bool {
    let Some(mut b) = d.bot.take() else {
        return false;
    };
    let hurt = change_state(d, &mut b, damage, by_player, effects);
    d.bot = Some(b);
    hurt
}

fn change_state(
    d: &mut Driver,
    b: &mut BotData,
    damage: u8,
    by_player: Option<bool>,
    effects: &mut Vec<Effect>,
) -> bool {
    use crate::pick::damage as kind;
    d.pending_damage_type = 0;
    if d.kart_state == kart_state::MASK_GRABBED {
        return false;
    }
    d.kart_state = kart_state::NORMAL;
    let p = &mut b.phys;
    match damage {
        kind::SPIN | kind::BURN => {
            if b.flags & flags::DAMAGE_ACTIVE == 0 {
                b.damage_state = damage_state::SPIN;
                p.turbo_meter = 0;
                let oxide = d.character == NITROS_OXIDE;
                // Oxide in the air keeps his speed.
                if !oxide || d.actions_flag_set & actions::TOUCH_GROUND != 0 {
                    d.reserves = 0;
                    d.turbo_outside_timer = 0;
                    p.speed_linear = mips::sra(p.speed_linear, if oxide { 1 } else { 2 });
                }
                p.squish_cooldown = SPIN_COOLDOWN;
                b.progress_cooldown = 0;
                b.flags |= flags::DAMAGE_ACTIVE;
            }
            if damage == kind::BURN {
                d.burn_timer = BURN_TIMER;
            }
        }
        kind::BLAST => {
            let already = b.flags & flags::DAMAGE_ACTIVE != 0;
            b.blast_bounce_count = 0;
            p.turbo_meter = 0;
            b.damage_state = damage_state::BLAST;
            d.reserves = 0;
            d.turbo_outside_timer = 0;
            p.speed_y = BLAST_SPEED_Y;
            if !already {
                p.speed_linear = mips::sra(p.speed_linear, 3);
                b.position_backup.y = mips::add_lo(b.position_backup.y, 0x4000);
            }
            b.progress_cooldown = 0;
            d.matrix_array = crate::MatrixArray::None;
            b.flags |= flags::DAMAGE_ACTIVE;
            match by_player {
                // (Nor does it count for the attacker.)
                None => return true,
                Some(true) => effects.push(Effect::Voiceline {
                    id: crate::pick::VOICELINE_COMMON_DAMAGE,
                    driver_id: d.driver_id,
                    priority: crate::pick::VOICELINE_PRIORITY,
                }),
                Some(false) => {}
            }
        }
        kind::SQUISH => {
            p.turbo_meter = 0;
            b.damage_state = damage_state::SQUISH;
            p.squish_cooldown = SPIN_COOLDOWN;
            p.rot_xz = SQUISH_TIME;
            d.squish_timer = SQUISH_TIMER;
            d.reserves = 0;
            d.turbo_outside_timer = 0;
            p.speed_y = 0;
            b.progress_cooldown = 0;
            p.speed_linear = mips::sra(p.speed_linear, 1);
            b.flags |= flags::DAMAGE_ACTIVE | flags::DAMAGE_SUPPRESS_EMITTER;
        }
        kind::MASK_GRAB => {
            // Out of sight (in a plant's mouth) until the grab ends.
            p.turbo_meter = 0;
            p.speed_linear = 0;
            p.speed_y = 0;
            p.rot_xz = MASK_GRAB_TIME;
            b.damage_state = damage_state::MASK_GRAB;
            d.kart_state = kart_state::MASK_GRABBED;
            b.flags |= flags::DAMAGE_ACTIVE | flags::DAMAGE_SUPPRESS_EMITTER;
            d.collision_disabled = true;
        }
        kind::NONE => {
            if b.flags & flags::DAMAGE_ACTIVE != 0 {
                return false;
            }
        }
        _ => b.progress_cooldown = 0x3c,
    }
    true
}

/// `VehPhysCrash_AI`: a bump moves the bot at `vel`. What of it goes along its nav frame is its
/// speed on the path; the rest pushes it off ([`flags::FREE_PHYSICS`]).
pub fn crash_ai(b: &mut BotData, nav_paths: &[NavPath], vel: [i32; 3]) {
    let Some(frame) = nav_paths
        .get(b.path)
        .and_then(|path| path.frames.get(b.nav_frame))
    else {
        return;
    };
    let rot = frame.rot.map(|r| mips::sll(r as i32, 4) as i16);
    let m = rot_to_matrix(SVec3 {
        x: rot[0],
        y: rot[1],
        z: rot[2],
    });
    let forward = [0, 1, 2].map(|row| mips::sra(m.m[row][2] as i32, 4));
    let speed = mips::sra(
        mips::add_lo(
            mips::add_lo(
                mips::mul_lo(forward[0], vel[0]),
                mips::mul_lo(forward[1], vel[1]),
            ),
            mips::mul_lo(forward[2], vel[2]),
        ),
        8,
    );
    let p = &mut b.phys;
    p.speed_linear = speed;
    p.accel.x = mips::sub_lo(vel[0], mips::sra(mips::mul_lo(forward[0], speed), 8));
    p.accel.z = mips::sub_lo(vel[2], mips::sra(mips::mul_lo(forward[2], speed), 8));
    b.flags |= flags::FREE_PHYSICS;
}

/// `BOTS_AI_COLLISION_SPEED_PENALTY`
const COLLISION_SPEED_PENALTY: i32 = 3000;

/// `BOTS_CollideWithOtherAI`: of two bots that bumped, the one behind (along the path of the one
/// further back in the race) takes the speed of the one ahead, less a penalty.
pub fn collide_with_other_ai(
    one: (&Driver, &mut BotData),
    two: (&Driver, &mut BotData),
    nav_paths: &[NavPath],
) {
    let (one, two) = if one.0.driver_rank < two.0.driver_rank {
        (two, one)
    } else {
        (one, two)
    };
    let Some(frames) = nav_paths.get(one.1.path).map(|p| &p.frames) else {
        return;
    };
    let Some(&frame) = frames.get(one.1.nav_frame) else {
        return;
    };
    let (start, end) = if one.1.flags & flags::ESTIMATE_NAV != 0 {
        (one.1.estimate.pos, frame.pos)
    } else {
        (frame.pos, frames[(one.1.nav_frame + 1) % frames.len()].pos)
    };
    let along = |d: &Driver| {
        let at = [d.pos_curr.x, d.pos_curr.y, d.pos_curr.z].map(|v| mips::sra(v, 8) as i16);
        map_range_pos_points(end, start, at)
    };
    let (ahead_one, ahead_two) = (along(one.0), along(two.0));
    let ((_, behind), (_, front)) = if ahead_one < ahead_two {
        (one, two)
    } else {
        (two, one)
    };
    behind.phys.speed_linear =
        mips::sub_lo(front.phys.speed_linear, COLLISION_SPEED_PENALTY).max(0);
}

/// `CAM_MapRange_PosPoints`: how far along the line from `pos2` to `pos1` the point `curr` is,
/// past `pos1` (in level units, negative before it).
fn map_range_pos_points(pos1: SVec3, pos2: SVec3, curr: [i16; 3]) -> i32 {
    let mut path = math::SVec3 {
        x: pos1.x.wrapping_sub(pos2.x),
        y: pos1.y.wrapping_sub(pos2.y),
        z: pos1.z.wrapping_sub(pos2.z),
    };
    math::vector_normalize(&mut path);
    let delta = [
        curr[0].wrapping_sub(pos1.x),
        curr[1].wrapping_sub(pos1.y),
        curr[2].wrapping_sub(pos1.z),
    ];
    let dot = mips::add_lo(
        mips::add_lo(
            mips::mul_lo(path.x as i32, delta[0] as i32),
            mips::mul_lo(path.y as i32, delta[1] as i32),
        ),
        mips::mul_lo(path.z as i32, delta[2] as i32),
    );
    dot >> 12
}

/// Whether the bot `d` is out of sight: a plant has it (`HIDE_MODEL`).
pub fn hidden(d: &Driver) -> bool {
    d.bot.as_ref().is_some_and(|b| {
        b.flags & flags::DAMAGE_ACTIVE != 0 && b.damage_state == damage_state::MASK_GRAB
    })
}

/// The tumble a blast gives the bot `d`, as a rotation to put on its model and a lift (what the
/// drive tick does to `botInstance->matrix`): none unless it is blasted.
pub fn blast_tumble(d: &Driver) -> Option<(SVec3, i32)> {
    let b = d.bot.as_ref()?;
    if b.flags & flags::DAMAGE_ACTIVE == 0 || b.damage_state != damage_state::BLAST {
        return None;
    }
    let turn = b.phys.rot_xz as i32;
    let rot = SVec3 {
        x: mips::sll(turn, 8) as i16,
        y: mips::mul_lo(turn, 0xe0) as i16,
        z: 0,
    };
    Some((rot, 0x20))
}

/// Counts a millisecond timer down by `ms`, to 0.
fn count_down(timer: &mut i16, ms: i32) {
    *timer = mips::sub_lo(*timer as u16 as i32, ms) as i16;
    if *timer < 0 {
        *timer = 0;
    }
}

/// The signed shortest way from angle `a` to `b`, both 0x1000 a turn.
fn turn_between(a: i32, b: i32) -> i32 {
    let delta = mips::sub_lo(b, a) & 0xfff;
    if delta > 0x7ff { delta - 0x1000 } else { delta }
}

/// `delta` (12 bits, wrapped) clamped to ±0x20: how far a turn goes in a tick.
fn step_toward(delta: i32) -> i32 {
    let mut delta = delta & 0xfff;
    if delta > 0x7ff {
        delta -= 0x1000;
    }
    delta.clamp(-0x20, 0x20)
}

/// The frame a bot is at: one of its path's, or the one it made up.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Cur {
    Estimate,
    Frame(usize),
}

/// One bot's tick, with what it reads.
struct Tick<'a, 'b> {
    d: &'a mut Driver,
    b: &'a mut BotData,
    coll: &'a mut Collision,
    bots: &'a mut Bots,
    level: &'a Level,
    env: &'a Env,
    peers: &'b [Peer],
    /// The bots that tick after this one, which it can bump into.
    siblings: Vec<&'b mut Driver>,
    effects: &'a mut Vec<Effect>,
}

impl Tick<'_, '_> {
    fn frames(&self) -> &[NavFrame] {
        &self.level.nav_paths[self.b.path].frames
    }

    fn frame(&self, cur: Cur) -> NavFrame {
        match cur {
            Cur::Estimate => self.b.estimate,
            Cur::Frame(i) => self.frames()[i],
        }
    }

    fn after(&self, i: usize) -> usize {
        (i + 1) % self.frames().len()
    }

    fn oxide(&self) -> bool {
        self.d.character == NITROS_OXIDE
    }

    fn increment(&mut self, reserves: i32, ty: u32, fire_level: i32) {
        fire::increment(self.d, reserves, ty, fire_level, self.effects);
    }

    /// `BOTS_ThTick_Drive`
    fn drive(&mut self) {
        let ms = self.env.elapsed_time_ms;
        let id = self.d.driver_id as usize;
        self.d.turbo_meter_room_left = 0;
        self.d.forward_dir = 0;
        self.d.split_line = false;

        if self.b.weapon_cooldown != 0 {
            self.b.weapon_cooldown = self.b.weapon_cooldown.wrapping_sub(1);
        }
        if self.d.pending_damage_type == 0 {
            if self.d.actions_flag_set & actions::RACE_FINISHED == 0 && self.b.weapon_cooldown != 0
            {
                self.b.weapon_cooldown = self.b.weapon_cooldown.wrapping_sub(1);
            }
        } else {
            let by_player = self
                .d
                .pending_damage_attacker
                .and_then(|slot| self.peers.get(slot as usize))
                .map(|peer| !peer.bot);
            let damage = self.d.pending_damage_type;
            change_state(self.d, self.b, damage, by_player, self.effects);
        }

        let d = &mut *self.d;
        count_down(&mut d.reserves, ms);
        count_down(&mut d.turbo_outside_timer, ms);
        count_down(&mut d.squish_timer, ms);
        count_down(&mut d.burn_timer, ms);
        count_down(&mut d.clock_receive, ms);

        self.b.phys.drift_target = 0;
        d.kart_state = kart_state::NORMAL;
        d.actions_flag_set_prev_frame = d.actions_flag_set;
        d.actions_flag_set &=
            !(actions::DROPPING_MINE | actions::JUMP_STARTED | actions::STARTED_TOUCH_GROUND);

        let (mut cur, mut next) = if self.b.flags & flags::ESTIMATE_NAV == 0 {
            (Cur::Frame(self.b.nav_frame), self.after(self.b.nav_frame))
        } else {
            (Cur::Estimate, self.b.nav_frame)
        };

        if self.env.kart_collisions {
            let paths = &self.level.nav_paths;
            crate::collide::bump_bots(self.d, self.b, &mut self.siblings, paths);
        }

        // `local_38`: the rubber band's boost of the drift's and the landing's turbos.
        let mut boost_scale = 1;
        let mut delta;
        let mut lights = self.env.traffic_lights_timer;
        if self.oxide() {
            // He pretends the lights went out earlier.
            lights = mips::sub_lo(lights, 0x1e0);
        }
        if self.b.progress_cooldown == 0 && lights <= 0 {
            if self.b.flags & flags::STARTLINE_INIT_DONE == 0 {
                self.b.flags |= flags::STARTLINE_INIT_DONE;
                if self.oxide() {
                    self.effects.push(Effect::Voiceline {
                        id: 0,
                        driver_id: self.d.driver_id,
                        priority: 0x10,
                    });
                }
                // A bot of the front row starts with a boost one time in four; Oxide always.
                if (self.bots.spawn_order[id] < 3 && (self.bots.rng.draw() & 0xff) < 0x40)
                    || self.oxide()
                {
                    self.increment(0x2d0, 1, 0x180);
                    self.effects.push(Effect::FireAudio { fire_level: 0x180 });
                }
            }

            let bot_flags = self.b.flags;
            if bot_flags & (flags::BOSS_PATH_REQUESTED | flags::BOSS_PATH_ACTIVE)
                != flags::BOSS_PATH_REQUESTED
                && self.bots.path_change_delay > PATH_CHANGE_DELAY_FRAMES
                && bot_flags & flags::ESTIMATE_NAV == 0
                && self.overtake(next)
            {
                next = self.b.nav_frame;
                cur = Cur::Estimate;
            }

            if self.d.actions_flag_set & actions::TOUCH_GROUND == 0 {
                let p = &mut self.b.phys;
                let zy = mips::add_lo(
                    mips::mul_lo(p.speed_linear, p.speed_linear),
                    mips::mul_lo(p.speed_y, p.speed_y),
                );
                if zy > 0x2b11_0000 {
                    let length = math::square_root0_stub(zy);
                    p.speed_linear = mips::div(mips::mul_lo(p.speed_linear, 0x6900), length);
                    p.speed_y = mips::div(mips::mul_lo(p.speed_y, 0x6900), length);
                }
            } else {
                boost_scale = self.speed_on_ground(cur);
            }

            let p = &mut self.b.phys;
            if p.speed_linear > 0x6400 {
                p.speed_linear = 0x6400;
            }
            delta = mips::sra(mips::mul_lo(p.speed_linear, ms), 5).max(0);
            delta = mips::add_lo(delta, self.b.nav_progress_remainder);
        } else {
            // Waiting for the lights, or for the cooldown of a fall.
            self.d.inst_scale_x = KART_SCALE_BASE as i16;
            self.d.inst_scale_y = KART_SCALE_BASE as i16;
            self.d.inst_scale_z = KART_SCALE_BASE as i16;
            self.b.phys.speed_linear = 0;
            if self.b.progress_cooldown != 0 {
                self.b.progress_cooldown -= 1;
            }
            delta = self.b.nav_progress_remainder;
        }

        let f = self.frame(cur);
        let mut nav_flags_seen = f.flags;
        let mut special_seen = f.special_bits;
        if special_seen & nav_special::MOON_GRAVITY != 0 {
            self.b.flags |= flags::MOON_GRAVITY;
        }
        let gravity = if self.b.flags & flags::MOON_GRAVITY == 0 {
            self.d.stats.gravity as i32
        } else {
            mips::div(mips::mul_lo(self.d.stats.gravity as i32, 41), 100)
        };
        let p = &mut self.b.phys;
        p.speed_y = mips::sub_lo(p.speed_y, mips::sra(mips::mul_lo(gravity, ms), 5));
        if p.speed_y < -0x5000 {
            p.speed_y = -0x5000;
        }
        self.b.position_backup.y = mips::add_lo(
            self.b.position_backup.y,
            mips::sra(mips::mul_lo(self.b.phys.speed_y, ms), 5),
        );

        let grounded = self.d.actions_flag_set & actions::TOUCH_GROUND != 0;
        let mut nav_dist = if grounded {
            f.dist_to_next_nav_xyz
        } else {
            f.dist_to_next_nav_xz
        } as i32;
        let mut ramp_bits = if f.special_bits & nav_special::RAMP_PHYS != 0 {
            f.special_bits
        } else {
            0
        };
        let mut flat_distance = false;
        let mut left = mips::sra(delta, 8);
        // (A frame of no length is passed in the same tick; a path of nothing but those would
        // never end the loop.)
        let mut steps = 0;
        while nav_dist <= left && steps <= self.frames().len() {
            steps += 1;
            cur = Cur::Frame(next);
            delta = mips::sub_lo(delta, mips::sll(nav_dist, 8));
            next = self.after(next);
            left = mips::sub_lo(left, nav_dist);
            let f = self.frame(cur);
            if mips::sra(self.b.position_backup.y, 8) < self.frames()[next].pos.y as i32
                && f.flags & nav_flags::KILLPLANE != 0
            {
                self.killplane();
                return;
            }
            nav_flags_seen |= f.flags;
            special_seen |= f.special_bits;
            self.b.flags &= if self.b.flags & flags::BOSS_PATH_ACTIVE == 0 {
                !flags::ESTIMATE_NAV
            } else {
                !(flags::ESTIMATE_NAV | flags::BOSS_PATH_REQUESTED | flags::BOSS_PATH_ACTIVE)
            };
            // A frame that climbs or drops steeply is measured flat from here on.
            if !flat_distance && f.rot[3].wrapping_sub(0x31) > 0x9e {
                nav_dist = if self.d.actions_flag_set & actions::TOUCH_GROUND == 0 {
                    f.dist_to_next_nav_xz
                } else {
                    f.dist_to_next_nav_xyz
                } as i32;
            } else {
                flat_distance = true;
                nav_dist = f.dist_to_next_nav_xz as i32;
            }
            if f.special_bits & nav_special::RAMP_PHYS != 0 {
                ramp_bits = f.special_bits;
            }
        }
        self.b.nav_progress_remainder = delta;

        let d = &mut *self.d;
        let mut nav_actions = 0;
        if nav_flags_seen & nav_flags::BACK_SKID != 0 {
            nav_actions |= actions::BACK_SKID;
        }
        if nav_flags_seen & nav_flags::ENGINE_ECHO != 0 {
            nav_actions |= actions::ENGINE_ECHO;
        }
        if nav_flags_seen & nav_flags::FRONT_SKID != 0 {
            nav_actions |= actions::FRONT_SKID;
        }
        d.actions_flag_set &= !(actions::ENGINE_ECHO | actions::BACK_SKID | actions::FRONT_SKID);
        d.actions_flag_set |= nav_actions;

        let f = self.frame(cur);
        self.d.terrain_meta1 = terrain_index((f.flags as u8) >> 3);

        self.drift_boosts(nav_flags_seen, boost_scale);

        if nav_flags_seen & nav_flags::SUPER_TURBO != 0 {
            self.increment(0x78, 1, 0x900);
            self.b.flags |= flags::NAV_BOOST_ACTIVE;
        }
        if nav_flags_seen & nav_flags::TURBO != 0 {
            self.increment(0x2d0, 1, 0x180);
            self.b.flags |= flags::NAV_BOOST_ACTIVE;
        }

        if self.b.flags & flags::ESTIMATE_NAV == 0
            && let Cur::Frame(i) = cur
        {
            self.b.nav_frame = i;
            next = self.after(i);
        }

        let percentage = if nav_dist == 0 {
            0
        } else {
            mips::div(mips::sll(left, 12), nav_dist)
        };

        let d = &mut *self.d;
        d.pos_prev = d.pos_curr;
        d.rot_prev[0] = d.rot_curr_x;
        d.rot_prev[1] = d.rot_curr_y;
        d.rot_prev[2] = d.rot_curr_z;

        let f = self.frame(cur);
        let n = self.frames()[next];
        let lerp = |a: i16, b: i16| {
            mips::sll(
                mips::add_lo(
                    a as i32,
                    mips::sra(
                        mips::mul_lo(mips::sub_lo(b as i32, a as i32), percentage),
                        12,
                    ),
                ),
                8,
            )
        };
        self.b.position_backup.x = lerp(f.pos.x, n.pos.x);
        self.d.quad_block_height = lerp(f.pos.y, n.pos.y);
        self.b.position_backup.z = lerp(f.pos.z, n.pos.z);

        if self.b.flags & flags::FREE_PHYSICS != 0 {
            self.free_physics();
        }

        if self.b.flags & (flags::ESTIMATE_NAV | flags::FREE_PHYSICS) == 0 {
            self.b.checkpoint_index = f.go_back_count;
        } else {
            let turn = turn_between(mips::sll(f.rot[1] as i32, 4), mips::sll(n.rot[1] as i32, 4));
            self.b.rot[1] = (mips::add_lo(
                mips::sll(f.rot[1] as i32, 4),
                mips::sra(mips::mul_lo(turn, percentage), 12),
            ) & 0xfff) as i16;
            if self.probe_ground(0x80, quad_flags::NO_COLLISION_RESPONSE) {
                return;
            }
        }

        let delta = mips::sra(delta, 8);
        if self.b.position_backup.y < self.d.quad_block_height {
            if self.land(f, delta, nav_flags_seen, boost_scale) {
                return;
            }
        } else {
            let d = &mut *self.d;
            if !flat_distance && d.actions_flag_set & actions::TOUCH_GROUND != 0 {
                let xyz = f.dist_to_next_nav_xyz as i32;
                if xyz != 0 {
                    self.b.nav_progress_remainder = mips::sll(
                        mips::div(mips::mul_lo(delta, f.dist_to_next_nav_xz as i32), xyz),
                        8,
                    );
                }
            }
            d.actions_flag_set &= !actions::TOUCH_GROUND;
            d.actions_flag_set |= actions::AIRBORNE;
            d.jump_landing_boost = mips::add_lo(d.jump_landing_boost as u16 as i32, ms) as i16;
        }

        self.drift();
        if self.damage(ms) {
            return;
        }

        if self.d.forced_jump_type != forced_jump::NONE {
            let jump = self.d.stats.jump_force as i32;
            let mut force = mips::add_lo(mips::sll(jump, 1), jump);
            if self.d.forced_jump_type != forced_jump::HIGH {
                force = mips::div(force, 2);
            }
            if self.b.phys.speed_y < force {
                self.b.phys.speed_y = force;
            }
            self.d.forced_jump_type = forced_jump::NONE;
        }

        if special_seen & nav_special::RAMP_PHYS != 0
            && (self.b.phys.speed_linear > 0x1c1f || self.oxide())
        {
            let index = (ramp_bits & nav_special::INDEX_MASK) as usize;
            let path = &self.level.nav_paths[self.b.path];
            self.b.phys.speed_y = path.ramp_speed_y[index] as i32;
            self.b.phys.speed_linear = path.ramp_speed_linear[index] as i32;
            if self.oxide() {
                self.b.phys.squish_cooldown = 0;
                self.b.phys.mul_drift = 0;
                self.b.flags &= !(flags::DAMAGE_ACTIVE | flags::DAMAGE_SUPPRESS_EMITTER);
            }
        }

        if f.rot[3].wrapping_sub(0x31) > 0x9e && n.rot[3].wrapping_sub(0x31) > 0x9e {
            self.steer(f, n, percentage);
        }

        self.place(f, nav_flags_seen);
    }

    /// The part of the drive that changes path: a bot right behind another on its path takes
    /// the frame's way onto another, if it has one. True if it did.
    fn overtake(&mut self, next: usize) -> bool {
        let id = self.d.driver_id;
        let points = self.frames().len() as i32;
        let mut nearest = 1000;
        let mut ahead = None;
        for &other in &self.bots.lists[self.b.path] {
            if other == id {
                continue;
            }
            let peer = &self.peers[other as usize];
            let mut gap = peer.nav_frame as i32 - self.b.nav_frame as i32;
            if gap < 0 {
                gap += points;
            }
            if gap < nearest {
                nearest = gap;
                ahead = Some(peer);
            }
        }
        let Some(ahead) = ahead.filter(|_| nearest < 3) else {
            return false;
        };
        let mut gap = mips::sub_lo(
            self.d.distance_to_finish_curr as i32,
            ahead.distance_to_finish_curr,
        );
        if gap < 0 {
            gap = mips::add_lo(gap, Bots::track_distance(self.level));
        }
        if gap >= 0x200 {
            return false;
        }
        let opcode = self.frames()[next].path_change_opcode as i32;
        if opcode >= (NAV_PATH_COUNT as i32) << PATH_CHANGE_PATH_SHIFT {
            return false;
        }
        let path = mips::sra(opcode, PATH_CHANGE_PATH_SHIFT);
        let frame = (opcode & PATH_CHANGE_FRAME_MASK) as usize;
        // (Retail trusts the level's data here.)
        if path < 0 || frame >= self.level.nav_paths[path as usize].frames.len() {
            return false;
        }
        let old = self.b.path;
        self.bots.lists[old].retain(|&d| d != id);
        self.bots.lists[path as usize].insert(0, id);
        self.b.path = path as usize;
        self.b.nav_frame = frame;
        set_rotation(self.d, self.b, self.level, false);
        true
    }

    /// The speed a bot on the ground aims for: the rubber band, the boosts, the hurts, and the
    /// surface. The result is `local_38`, which scales its turbos.
    fn speed_on_ground(&mut self, cur: Cur) -> i32 {
        let mut boost_scale = 1;
        self.d.actions_flag_set &= !actions::AIRBORNE;
        let id = self.d.driver_id as usize;
        let best = self
            .bots
            .best_human
            .filter(|&slot| slot != id)
            .map(|slot| self.peers[slot]);
        let mut velocity = match best {
            None => UNCHASED_SPEED,
            Some(best) => {
                let (v, scale) = self.rubber_band(&best);
                boost_scale = scale;
                v
            }
        };

        let d = &mut *self.d;
        if d.reserves == 0 {
            self.b.flags &= !flags::NAV_BOOST_ACTIVE;
        } else if self.b.flags & flags::NAV_BOOST_ACTIVE == 0 {
            velocity = mips::add_lo(velocity, d.fire_speed_cap as i32);
        } else {
            velocity = mips::add_lo(d.fire_speed_cap as i32, 10000);
        }
        if d.clock_receive != 0 || d.squish_timer != 0 {
            velocity = mips::sra(mips::mul_lo(velocity, 0xc00), 12);
        }
        let damaged = d.stats.damaged_speed as i32;
        if self.b.flags & flags::DAMAGE_ACTIVE == 0 {
            if d.tnt_scale_y.is_some() || d.cloud.is_some() {
                velocity = mips::sub_lo(velocity, mips::sra(damaged, 1));
            }
        } else {
            self.b.phys.turbo_meter = 0;
            velocity = mips::sub_lo(
                velocity,
                if d.character == NITROS_OXIDE {
                    mips::sra(damaged, 2)
                } else {
                    damaged
                },
            );
        }
        velocity = velocity.max(0);

        let terrain = BOT_TERRAINS[terrain_index(d.terrain_meta1) as usize];
        let throttle = self.bots.throttle();
        let f = match cur {
            Cur::Estimate => self.b.estimate,
            Cur::Frame(i) => self.level.nav_paths[self.b.path].frames[i],
        };
        let p = &mut self.b.phys;
        p.speed_linear = mips::sub_lo(
            p.speed_linear,
            mips::sra(
                mips::mul_lo(
                    d.stats.pedal_friction_forward as i32,
                    terrain.friction_scale,
                ),
                8,
            ),
        )
        .max(0);
        velocity = velocity.min(0x6900);
        let target = mips::sra(mips::mul_lo(velocity, terrain.target_speed_scale), 8);

        let mut accelerate = p.speed_linear < target;
        if terrain.decel_to_target && !accelerate {
            // Mud halves the bot's speed each tick, down to its target.
            p.speed_linear = mips::sra(p.speed_linear, 1).max(target);
            accelerate = false;
        }
        if accelerate {
            let accel = if d.reserves < 1 {
                d.stats.accel_class_stat
            } else {
                d.stats.accel_reserves
            } as i32;
            let mut gain = mips::sra(mips::mul_lo(accel, terrain.acceleration_scale), 8);
            if self.b.accel != 0 {
                self.b.accel -= 1;
                let order = self.bots.accelerate_order[id] as i32;
                gain = mips::sra(
                    mips::mul_lo(
                        gain,
                        mips::sub_lo(0x100, mips::mul_lo(ACCEL_FRAME_STEPS, order)),
                    ),
                    8,
                );
            }
            p.speed_linear = mips::add_lo(p.speed_linear, gain);
        }

        if let (Some(throttle), Cur::Frame(index)) = (throttle, cur) {
            let from = throttle[self.b.path];
            let index = index as i32;
            if from <= index && index < from + 0xb && p.speed_linear > 9000 {
                p.turbo_meter = 0;
                p.speed_linear = mips::sub_lo(
                    p.speed_linear,
                    mips::add_lo(100, d.stats.accel_class_stat as i32),
                );
            }
        }

        if f.rot[3] > 0x80 {
            // Uphill.
            let target = mips::add_lo(target, d.stats.slope_forward_speed_bonus as i32);
            if p.speed_linear < target {
                let sine = math::sin(mips::sll(f.rot[3] as i32, 4) as u32);
                p.speed_linear = mips::sub_lo(
                    p.speed_linear,
                    mips::sra(mips::mul_lo(d.stats.gravity as i32, sine), 12),
                );
            }
            d.fire_speed = target as i16;
        }
        boost_scale
    }

    /// The rubber band: the speed that keeps the bot near `best`, by how far apart they are and
    /// the bot's place. Also whether the bot is behind (which doubles its turbos).
    fn rubber_band(&self, best: &Peer) -> (i32, i32) {
        let d = &*self.d;
        let id = d.driver_id as usize;
        let cup = self.bots.setup.cup;
        let laps = self.env.laps as i32;
        let params = |i: i32| -> i32 {
            // (A place below 0 reads before the table in retail.)
            usize::try_from(i)
                .ok()
                .and_then(|i| self.bots.arcade.get(i))
                .map_or(0, |&v| v as i32)
        };
        let cup_params = |i: usize| self.bots.cup[i] as i32;

        let mut rank = d.driver_rank as i32;
        if self.peers[0].driver_rank < d.driver_rank as i32 {
            rank -= 1;
        }
        if self.bots.setup.players > 1 && self.peers[1].driver_rank <= rank {
            rank -= 1;
        }
        if cup {
            let order = &self.bots.accelerate_order;
            rank = order[id] as i32;
            if order[0] < 2 {
                rank -= 1;
            }
            if order[1] < 2 {
                rank -= 1;
            }
        }

        let lap_stat = if best.lap_index == 0 {
            params(0xb)
        } else if best.lap_index == laps - 1 {
            params(0xd)
        } else {
            params(0xc)
        };
        let track = Bots::track_distance(self.level);
        // The lap, less one behind the start line.
        let lap =
            |p: &Peer| p.lap_index - (p.actions_flag_set & actions::BEHIND_START_LINE != 0) as i32;
        let progress = |p: &Peer| {
            mips::add_lo(
                mips::sub_lo(track, p.distance_to_finish_curr),
                mips::mul_lo(lap(p), track),
            )
        };
        let gap = mips::sub_lo(progress(best), progress(&Peer::of(d)));
        let gap = mips::sub_lo(gap, mips::add_lo(params(rank), lap_stat));

        let last_lap = d.lap_index as i32 >= laps - 1;
        let pull = if gap < 1 {
            params(9)
        } else if cup && (rank & 0xffff) == 0 {
            if last_lap {
                cup_params(8) + cup_params(0xa)
            } else {
                cup_params(8)
            }
        } else if last_lap {
            params(8) + params(0xa)
        } else {
            params(8)
        };

        let pull_abs = pull.wrapping_abs();
        let mut velocity = mips::sra(
            mips::mul_lo(
                pull_abs,
                mips::div(mips::sll(mips::add_lo(gap.wrapping_abs(), 0x80), 12), 0xa00),
            ),
            12,
        );
        let boost_scale = (gap > 0) as i32;
        velocity = mips::add_lo(
            velocity,
            mips::mul_lo(
                mips::div(mips::sll(velocity, 3), 100),
                mips::sub_lo(7, rank),
            ),
        );
        if pull_abs < velocity {
            velocity = pull_abs;
        }
        if pull < 0 {
            velocity = mips::neg_lo(pull_abs);
        }

        let wumpas = best.num_wumpas.min(9);
        let turbo = best.turbo_const.min(5);
        let net = mips::sub_lo(
            mips::div(
                mips::sll(
                    mips::sub_lo(best.accel_speed_class_stat, best.speed_class_stat),
                    12,
                ),
                5,
            ),
            1,
        );
        let extra = mips::add_lo(
            mips::div(mips::sll(best.speed_class_stat, 3), 10),
            mips::sra(
                mips::add_lo(
                    mips::div(mips::mul_lo(wumpas, net), 10),
                    mips::mul_lo(turbo, net),
                ),
                12,
            ),
        )
        .min(0x6900);
        velocity = mips::add_lo(velocity, extra);
        velocity = if velocity < 0x5dc1 {
            velocity.max(0x1c20)
        } else {
            24000
        };
        (velocity, boost_scale)
    }

    /// The bot's turbos of a drift the path asks for: the longer it slides, the bigger.
    fn drift_boosts(&mut self, nav_flags_seen: u16, boost_scale: i32) {
        let ms = self.env.elapsed_time_ms;
        if self.d.actions_flag_set & actions::FRONT_SKID == 0
            || nav_flags_seen & (nav_flags::DRIFT_LEFT | nav_flags::DRIFT_RIGHT) == 0
        {
            self.d.turbo_meter_room_left = 0;
            self.b.phys.turbo_meter = 0;
            self.b.phys.fire_level = 0;
            return;
        }
        self.d.actions_flag_set |= actions::TURBO_INPUT_LATCH;
        let p = &mut self.b.phys;
        p.turbo_meter = mips::add_lo(p.turbo_meter as u16 as i32, ms) as i16;
        let (meter, level) = (p.turbo_meter, p.fire_level);
        // The charge marks the bar between boosts; a boost fires at each threshold.
        let charge = |t: &mut Self, level: i16| {
            t.b.phys.fire_level = level;
            t.d.turbo_meter_room_left = 0xa0;
        };
        let fire = |t: &mut Self, level: i16, reserves: i32, fire_level: i32| {
            t.b.phys.fire_level = level;
            t.d.turbo_meter_room_left = 0;
            t.increment(reserves, 2, fire_level);
        };
        if meter >= 0xb41 && level <= 5 {
            fire(self, 6, 0x2d0, mips::mul_lo(boost_scale, 0x180));
        } else if meter >= 0x961 && level <= 4 {
            charge(self, 5);
        } else if meter >= 0x781 && level <= 3 {
            fire(self, 4, 0x1e0, mips::sll(boost_scale, 8));
        } else if meter > 0x5a0 && level < 3 {
            charge(self, 3);
        } else if meter >= 0x3c1 && level <= 1 {
            fire(self, 2, 0xf0, mips::sll(boost_scale, 7));
        } else if meter > 0x1e0 && level < 1 {
            charge(self, 1);
        }
    }

    /// A push off the path wears off (`BOT_FLAG_FREE_PHYSICS`).
    fn free_physics(&mut self) {
        let p = &mut self.b.phys;
        p.accel.y = 0;
        p.velocity.x = mips::add_lo(p.velocity.x, p.accel.x);
        p.velocity.y = mips::add_lo(p.velocity.y, p.accel.y);
        p.velocity.z = mips::add_lo(p.velocity.z, p.accel.z);
        let (accel_x, accel_z) = (p.accel.x, p.accel.z);
        p.accel.x = mips::sra(p.accel.x, 1);
        p.accel.z = mips::sra(p.accel.z, 1);
        p.velocity.y = mips::add_lo(p.velocity.y, p.accel.y);
        p.velocity.x = mips::add_lo(p.velocity.x, accel_x);
        p.velocity.z = mips::add_lo(p.velocity.z, accel_z);
        let settle = |velocity: &mut i32, accel: &mut i32| {
            let v = *velocity;
            if v == 0 {
                return;
            }
            if v < 1 {
                *velocity = mips::add_lo(v, 0x444);
                if *velocity > 0 {
                    *velocity = 0;
                } else if *accel == 0 {
                    *accel = 0x444;
                }
            } else {
                *velocity = mips::sub_lo(v, 0x444);
                if *velocity < 0 {
                    *velocity = 0;
                } else if *accel == 0 {
                    *accel = -0x444;
                }
            }
        };
        settle(&mut p.velocity.x, &mut p.accel.x);
        settle(&mut p.velocity.z, &mut p.accel.z);
        if p.velocity.x == 0 && p.velocity.z == 0 {
            self.b.flags &= !flags::FREE_PHYSICS;
        }
    }

    /// The ground under a bot that is off its path's frames: where it stands, and the restart
    /// point it is at. True if it is a kill plane, which ended the tick.
    fn probe_ground(&mut self, below: i32, ignored: u16) -> bool {
        let b = &*self.b;
        let at = |pos: i32, velocity: i32| mips::sra(mips::add_lo(pos, velocity), 8);
        let (x, y, z) = (
            at(b.position_backup.x, b.phys.velocity.x),
            at(b.position_backup.y, b.phys.velocity.y),
            at(b.position_backup.z, b.phys.velocity.z),
        );
        let top = SVec3 {
            x: x as i16,
            y: mips::sub_lo(y, 0x100) as i16,
            z: z as i16,
        };
        let bottom = SVec3 {
            x: x as i16,
            y: mips::add_lo(y, below) as i16,
            z: z as i16,
        };
        let q = &mut self.coll.sps.quad_block_coll;
        q.quad_flags_wanted = quad_flags::GROUND as u32;
        q.quad_flags_ignored = ignored as u32;
        q.search_flags = coll::search_flags::HIGH_LOD;
        self.coll.search_quadblocks(self.level, top, bottom, 0);
        if self.coll.sps.did_touch_quadblock == 0 {
            return false;
        }
        let Some(quad) = self.coll.sps.hit.quad else {
            return false;
        };
        let quad = &self.level.quad_blocks[quad as usize];
        self.d.quad_block_height = mips::sll(self.coll.sps.quad_block_coll.hit_pos.y as i32, 8);
        self.b.checkpoint_index = quad.checkpoint_index;
        self.d.axis_angle3_normal_vec = self.coll.sps.hit.plane.normal;
        if quad.flags & quad_flags::KILL_PLANE != 0 {
            self.killplane();
            return true;
        }
        false
    }

    /// The bot came down on its path: the landing's turbo, a jump the path asks for, the drift.
    /// True if it fell on a kill plane, which ended the tick.
    fn land(&mut self, f: NavFrame, delta: i32, nav_flags_seen: u16, boost_scale: i32) -> bool {
        let old = self.b.flags;
        self.b.flags &= !flags::MOON_GRAVITY;
        if f.flags & nav_flags::KILLPLANE != 0 {
            self.killplane();
            return true;
        }
        let airborne = self.d.actions_flag_set & actions::TOUCH_GROUND == 0;
        let remainder_flat = |t: &mut Self| {
            let xz = f.dist_to_next_nav_xz as i32;
            if xz != 0 {
                t.b.nav_progress_remainder = mips::sll(
                    mips::div(mips::mul_lo(delta, f.dist_to_next_nav_xyz as i32), xz),
                    8,
                );
            }
        };
        if old & flags::DAMAGE_ACTIVE == 0 {
            if airborne {
                remainder_flat(self);
                let boost = self.d.jump_landing_boost;
                let fire_level = if boost > 0x5a0 {
                    Some(0xc0)
                } else if boost > 0x3c0 {
                    Some(0x60)
                } else if boost > 0x280 {
                    Some(0)
                } else {
                    None
                };
                if let Some(level) = fire_level {
                    self.increment(0x2d0, 2, mips::mul_lo(boost_scale, level));
                }
                self.d.actions_flag_set |= actions::STARTED_TOUCH_GROUND;
            }
            let d = &mut *self.d;
            let rise = mips::sub_lo(d.quad_block_height, d.pos_prev.y);
            self.b.position_backup.y = d.quad_block_height;
            let p = &mut self.b.phys;
            p.speed_y = rise;
            if nav_flags_seen & nav_flags::JUMP != 0 || d.tnt_scale_y.is_some() {
                d.actions_flag_set |= actions::JUMP_STARTED;
                p.speed_y = mips::add_lo(rise, 0x1400);
            }
            if p.speed_y > 16000 {
                p.speed_y = 16000;
            }
            if nav_flags_seen & (nav_flags::DRIFT_LEFT | nav_flags::DRIFT_RIGHT) == 0 {
                p.drift_target = 0;
                d.kart_state = kart_state::NORMAL;
            } else {
                d.kart_state = kart_state::DRIFTING;
                p.drift_target = if nav_flags_seen & nav_flags::DRIFT_LEFT == 0 {
                    0x2aa
                } else {
                    -0x2aa
                };
            }
        } else {
            if airborne {
                remainder_flat(self);
            }
            let p = &mut self.b.phys;
            p.speed_y = mips::sra(mips::neg_lo(p.speed_y), 1).wrapping_abs();
            self.b.blast_bounce_count = self.b.blast_bounce_count.wrapping_add(1);
            if self.b.damage_state == damage_state::SPIN {
                self.d.actions_flag_set |= actions::BACK_SKID | actions::FRONT_SKID;
            } else if self.b.damage_state == damage_state::BLAST && self.b.blast_bounce_count > 2 {
                self.b.progress_cooldown = 10;
                self.b.phys.speed_linear = 0;
                self.b.flags &= !(flags::DAMAGE_ACTIVE | flags::DAMAGE_SUPPRESS_EMITTER);
            }
            self.b.position_backup.y = self.d.quad_block_height;
        }
        self.d.jump_landing_boost = 0;
        self.d.actions_flag_set |= actions::TOUCH_GROUND;
        false
    }

    /// The drift's turn eases toward what the path asks.
    fn drift(&mut self) {
        let p = &mut self.b.phys;
        let step = if p.drift_target != 0 { 0x60 } else { 0x18 };
        if p.drift_target < p.mul_drift {
            let down = mips::sub_lo(p.mul_drift as u16 as i32, step) as i16;
            p.mul_drift = down.max(p.drift_target);
        } else {
            let up = mips::add_lo(p.mul_drift as u16 as i32, step) as i16;
            p.mul_drift = up.min(p.drift_target);
        }
        self.d.mult_drift = p.mul_drift;
    }

    /// What a hit does to the bot while it lasts. True if a mask grab ended, which ended the tick.
    fn damage(&mut self, ms: i32) -> bool {
        if self.b.flags & flags::DAMAGE_ACTIVE == 0 {
            return false;
        }
        let p = &mut self.b.phys;
        let wind_down = |p: &mut Physics| {
            let before = p.squish_cooldown as u16 as i32;
            p.squish_cooldown = mips::sub_lo(p.squish_cooldown, 0xc);
            p.mul_drift = mips::add_lo(p.mul_drift as u16 as i32, before) as i16;
            p.squish_cooldown
        };
        match self.b.damage_state {
            damage_state::BLAST => p.rot_xz = p.rot_xz.wrapping_sub(1),
            damage_state::SPIN => {
                if wind_down(p) < 0x100 {
                    p.squish_cooldown = 0;
                    p.mul_drift = 0;
                    p.speed_linear = 0;
                    self.b.flags &= !(flags::DAMAGE_ACTIVE | flags::DAMAGE_SUPPRESS_EMITTER);
                }
                self.d.kart_state = kart_state::SPINNING;
            }
            damage_state::SQUISH => {
                if wind_down(p) < 0x200 {
                    p.squish_cooldown = 0;
                    p.mul_drift = 0;
                }
                p.rot_xz = mips::sub_lo(p.rot_xz as u16 as i32, ms) as i16;
                if p.rot_xz <= 0 {
                    p.speed_y = 0x1400;
                    self.b.flags &= !(flags::DAMAGE_ACTIVE | flags::DAMAGE_SUPPRESS_EMITTER);
                    self.d.actions_flag_set |= actions::JUMP_STARTED;
                }
            }
            damage_state::MASK_GRAB => {
                p.speed_linear = 0;
                p.rot_xz = mips::sub_lo(p.rot_xz as u16 as i32, ms) as i16;
                self.d.kart_state = kart_state::MASK_GRABBED;
                if p.rot_xz < 1 {
                    self.b.flags &= !(flags::DAMAGE_ACTIVE | flags::DAMAGE_SUPPRESS_EMITTER);
                    self.b.progress_cooldown = 1;
                    self.mask_grab();
                    return true;
                }
            }
            _ => {}
        }
        false
    }

    /// The turn of the kart, its lean and its wheels, eased along the frames.
    fn steer(&mut self, f: NavFrame, n: NavFrame, percentage: i32) {
        let lerp_angle = |a: u8, b: u8| -> i16 {
            let (a, b) = (mips::sll(a as i32, 4), mips::sll(b as i32, 4));
            (mips::add_lo(
                a,
                mips::sra(mips::mul_lo(turn_between(a, b), percentage), 12),
            ) & 0xfff) as i16
        };
        let estimating = self.b.flags & flags::ESTIMATE_NAV != 0;
        if !estimating && self.d.actions_flag_set & actions::TOUCH_GROUND != 0 {
            self.b.rot[0] = lerp_angle(f.rot[0], n.rot[0]);
            self.b.rot[2] = lerp_angle(f.rot[2], n.rot[2]);
        }
        let mut turn = turn_between(mips::sll(f.rot[1] as i32, 4), mips::sll(n.rot[1] as i32, 4));
        self.b.rot[1] = lerp_angle(f.rot[1], n.rot[1]);
        if estimating {
            turn = 0;
        }

        let d = &mut *self.d;
        let step = step_toward(mips::sub_lo(mips::sll(turn, 1), d.turn_angle_curr as i32));
        let turn_angle = mips::add_lo(d.turn_angle_curr as u16 as i32, step) as i16;
        d.turn_angle_curr = turn_angle;

        let p = &mut self.b.phys;
        let wrap = |v: i32| -> i16 {
            let v = v & 0xfff;
            (if v > 0x7ff { v - 0x1000 } else { v }) as i16
        };
        p.simp_turn_state = if p.drift_target == 0 {
            mips::sra(wrap(turn_angle as i32) as i32, 2) as i16
        } else {
            mips::sra(wrap(mips::neg_lo(p.mul_drift as u16 as i32)) as i32, 3) as i16
        };
        let step = step_toward(mips::sub_lo(
            p.simp_turn_state as i32,
            d.wheel_rotation as i32,
        ));
        d.wheel_rotation = mips::add_lo(d.wheel_rotation as u16 as i32, step) as i16;
        d.simp_turn_state = p.simp_turn_state as u8 as i8;
    }

    /// The end of the drive: the kart's rotation, matrices, speeds and position from where the
    /// path put it, its hazard timer, its matrices and animation.
    fn place(&mut self, f: NavFrame, nav_flags_seen: u16) {
        let ms = self.env.elapsed_time_ms;
        let d = &mut *self.d;
        let b = &mut *self.b;
        d.rot_curr_x = b.rot[0];
        d.rot_curr_y = b.rot[1];
        d.rot_curr_z = b.rot[2];

        // The shake of a clock or a cloud.
        let shake = if d.clock_receive != 0 {
            Some(d.clock_receive as i32)
        } else if d.squish_timer == 0 && d.cloud.is_some() {
            Some(mips::sll(self.env.timer as i32, 2))
        } else {
            None
        };
        if let Some(timer) = shake {
            let angle = mips::mul_lo(timer, 0xc) as u32;
            let x = mips::sra(mips::mul_lo(math::sin(angle), 50), 10);
            let z = mips::sra(mips::mul_lo(math::cos(angle), 50), 10);
            // It turns by the GTE's rotation as the last thread left it: here, this kart's.
            let gte: &mut Gte = &mut self.coll.gte;
            gte.mtc2(x as u32 & 0xffff, gte::data::VXY0);
            gte.mtc2(z as u32, gte::data::VZ0);
            gte.cop2(mvmva(1, 0, 0, 3, 0));
            b.phys.turbo_meter = 0;
            let [ix, iy, iz] = [0, 1, 2].map(|k| gte.mfc2(gte::data::IR1 + k) as i16 as i32);
            d.rot_curr_x = mips::add_lo(d.rot_curr_x as u16 as i32, ix) as i16;
            d.rot_curr_y = mips::add_lo(d.rot_curr_y as u16 as i32, iy) as i16;
            d.rot_curr_z = mips::add_lo(d.rot_curr_z as u16 as i32, iz) as i16;
        }

        if b.flags & (flags::ESTIMATE_NAV | flags::FREE_PHYSICS) == 0 {
            let m = rot_to_matrix(SVec3 {
                x: mips::sll(f.rot[0] as i32, 4) as i16,
                y: mips::sll(f.rot[1] as i32, 4) as i16,
                z: mips::sll(f.rot[2] as i32, 4) as i16,
            });
            d.axis_angle3_normal_vec = SVec3 {
                x: m.m[0][1],
                y: m.m[1][1],
                z: m.m[2][1],
            };
        }
        let m = convert_rot_to_matrix(
            &mut self.coll.gte,
            SVec3 {
                x: d.rot_curr_x,
                y: d.rot_curr_y,
                z: d.rot_curr_z,
            },
        );
        d.axis_angle2_normal_vec = SVec3 {
            x: m.m[0][1],
            y: m.m[1][1],
            z: m.m[2][1],
        };
        d.angle = d.rot_curr_y;
        let p = &b.phys;
        d.speed_approx = p.speed_linear as i16;
        d.speed = p.speed_linear as i16;
        d.jump_height_prev = d.jump_height_curr;
        d.axis_rotation_x = b.rot[1] & 0xfff;
        d.jump_height_curr =
            mips::sra(mips::mul_lo(p.speed_y, math::cos(f.rot[3] as u32)), 12) as i16;
        let heading = d.axis_rotation_x as u32;
        d.z_speed = mips::sra(mips::mul_lo(p.speed_linear, math::cos(heading)), 12);
        d.y_speed = p.speed_y;
        d.x_speed = mips::sra(mips::mul_lo(p.speed_linear, math::sin(heading)), 12);
        let z = d.rot_curr_z as i32 & 0xfff;
        d.rot_curr_z = (if z > 0x7ff { z - 0x1000 } else { z }) as i16;
        d.rot_curr_y = mips::add_lo(
            d.rot_curr_y as u16 as i32,
            mips::add_lo(p.mul_drift as u16 as i32, d.turn_angle_curr as u16 as i32),
        ) as i16;
        d.pos_curr = Vec3 {
            x: mips::add_lo(p.velocity.x, b.position_backup.x),
            y: mips::add_lo(p.velocity.y, b.position_backup.y),
            z: mips::add_lo(p.velocity.z, b.position_backup.z),
        };

        hazard_timer(d, ms);

        // (Here `BOTS_LevInstColl` would touch the level's objects, on the frames that ask for it
        // and off the path: it is not ported.)
        if nav_flags_seen & nav_flags::SPLIT_LINE != 0 {
            d.split_line = true;
        }

        translate_matrix(d, &mut self.coll.gte, self.effects);
        let mut moving = d.matrix_moving_dir;
        rot_axis_angle(
            &mut moving,
            &mut self.coll.gte,
            d.axis_angle2_normal_vec,
            d.angle as i32,
        );
        d.matrix_moving_dir = moving;
        frame::driving(d);

        let cam = mips::sub_lo(d.angle as i32, b.rot_y_608 as i32) & 0xfff;
        d.rot_curr_w = mips::neg_lo(cam) as i16;
        let cam = if cam > 0x7ff { cam | !0xfff } else { cam };
        b.rot_y_608 = (mips::add_lo(b.rot_y_608 as u16 as i32, mips::sra(cam, 3)) & 0xfff) as i16;
    }

    /// `BOTS_Killplane`: the bot fell: it is put back on the frame before the restart point it
    /// is at (or, on Tiny Arena, on one of two points), and the mask brings it.
    fn killplane(&mut self) {
        let frames = &self.level.nav_paths[self.b.path].frames;
        let back = |i: usize| if i == 0 { frames.len() - 1 } else { i - 1 };
        let mut i = self.b.nav_frame;
        let checkpoint = self.d.checkpoint_current_index;
        let tiny_override = match checkpoint {
            0x94 => Some(0x84),
            0xa0 => Some(0x80),
            _ => None,
        };
        // (Each walk ends at most a lap back: retail's would go on.)
        let mut steps = 0;
        if let Some(target) = tiny_override.filter(|_| self.bots.tiny_arena()) {
            while steps < frames.len() {
                let count = frames[i].go_back_count as i32;
                if count >= target - 1 && count <= target + 1 {
                    break;
                }
                i = back(i);
                steps += 1;
            }
        } else {
            while steps < frames.len()
                && (frames[i].go_back_count == checkpoint
                    || frames[i].flags & NAV_FLAG_NO_RESPAWN != 0)
            {
                i = back(i);
                steps += 1;
            }
        }
        self.b.nav_frame = i;
        self.mask_grab();
    }

    /// `BOTS_MaskGrab`: the bot is put halfway along its frame, high above the track, and drops.
    fn mask_grab(&mut self) {
        let frames = &self.level.nav_paths[self.b.path].frames;
        let f = frames[self.b.nav_frame];
        let n = frames[(self.b.nav_frame + 1) % frames.len()];
        let d = &mut *self.d;
        let b = &mut *self.b;
        d.kart_state = kart_state::MASK_GRABBED;
        b.nav_progress_remainder = mips::sll(f.dist_to_next_nav_xz as i32 / 2, 8);
        let mid = |a: i16, b: i16| mips::sll(mips::add_lo(a as i32, (b as i32 - a as i32) / 2), 8);
        b.position_backup = Vec3 {
            x: mid(f.pos.x, n.pos.x),
            y: mid(f.pos.y, n.pos.y),
            z: mid(f.pos.z, n.pos.z),
        };
        d.pos_prev = b.position_backup;
        d.quad_block_height = b.position_backup.y;
        b.phys.speed_linear = 0;
        b.phys.speed_y = 0;
        b.phys.velocity = Vec3::default();
        d.actions_flag_set |= actions::TOUCH_GROUND;
        b.flags &= !(flags::ESTIMATE_NAV
            | flags::DAMAGE_ACTIVE
            | flags::DAMAGE_SUPPRESS_EMITTER
            | flags::FREE_PHYSICS
            | flags::BOSS_PATH_REQUESTED);
        d.rot_curr_x = mips::sll(f.rot[0] as i32, 4) as i16;
        d.rot_curr_y = mips::sll(f.rot[1] as i32, 4) as i16;
        d.rot_curr_z = mips::sll(f.rot[2] as i32, 4) as i16;
        b.phys.squish_cooldown = 0;
        b.phys.mul_drift = 0;
        d.turbo_meter_room_left = 0;
        d.reserves = 0;
        d.clock_receive = 0;
        d.squish_timer = 0;
        d.turbo_outside_timer = 0;
        d.matrix_array = crate::MatrixArray::None;
        d.matrix_index = 0;
        d.actions_flag_set &= !(actions::AIRBORNE | actions::HIGH_JUMP);
        d.collision_disabled = false;
        d.pos_curr = b.position_backup;
        d.pos_curr.y = mips::add_lo(d.pos_curr.y, MASK_DROP_HEIGHT);
        crate::stuck::use_mask(d);
        if let Some(mask) = &mut d.mask {
            mask.duration = crate::stuck::MASK_DURATION_NORMAL;
            mask.world_space = true;
        }
        b.revving = true;
        self.rev_engine();
    }

    /// `BOTS_ThTick_RevEngine`: the kart sinks back to its path under the mask, which lets go
    /// there, and drives on from there.
    fn rev_engine(&mut self) {
        if self.b.position_backup.y >= self.d.pos_curr.y {
            if let Some(mask) = &mut self.d.mask {
                mask.scale = crate::stuck::MASK_SCALE_NORMAL;
                mask.duration = 0;
                mask.world_space = false;
            }
            self.d.kart_state = kart_state::ENGINE_REVVING;
            self.d.clock_receive = 0;
            self.d.squish_timer = 0;
            self.b.revving = false;
            self.drive();
            return;
        }
        let d = &mut *self.d;
        d.pos_curr.y = mips::sub_lo(
            d.pos_curr.y,
            mips::sra(mips::sll(self.env.elapsed_time_ms, 9), 5),
        );
        if let Some(mask) = &mut d.mask {
            mask.pos = [d.pos_curr.x, d.pos_curr.y, d.pos_curr.z].map(|v| mips::sra(v, 8) as i16);
        }
        translate_matrix(d, &mut self.coll.gte, self.effects);
        frame::driving(d);
    }
}

/// The hazard timer: it keeps the shakes of a clock or a squish from starting over, and counts
/// down while the bot moves.
fn hazard_timer(d: &mut Driver, ms: i32) {
    let mut bad = d.clock_receive as i32;
    if bad == 0 {
        bad = d.squish_timer as i32;
    }
    if bad == 0 {
        if d.speed_approx > 0x100 {
            d.hazard_timer = mips::sub_lo(d.hazard_timer as u16 as i32, ms) as i16;
        }
        d.hazard_timer = (d.hazard_timer as u16 & 0xfffe) as i16;
        if d.hazard_timer >= 0 {
            d.hazard_timer = -2;
        }
        return;
    }
    let moving = (d.speed_approx as i32).abs() > 0x100;
    if moving {
        if d.actions_flag_set & actions::TOUCH_GROUND == 0 {
            d.hazard_timer = mips::neg_lo(bad.abs()) as i16;
        } else {
            d.hazard_timer = bad.abs() as i16;
        }
    } else if d.hazard_timer > 0 {
        d.hazard_timer = mips::neg_lo(d.hazard_timer as i32) as i16;
    }
    d.hazard_timer = (d.hazard_timer as u16 | 1) as i16;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rng_is_retails() {
        // RngDeadCoed's first draws from the seed `BOTS_Adv_AdjustDifficulty` gives it.
        let mut rng = Rng::default();
        let state0 = (0x3021_5400u32 >> 8) | (0x4935_83feu32 << 24);
        let state1 = ((0x4935_83feu32 >> 8)
            | (0x3021_5400u32
                .wrapping_add(0x4935_83fe >> 8)
                .wrapping_add(state0 >> 8)
                << 24))
            ^ 0xdead_c0ed;
        assert_eq!(rng.draw(), state1 as i32);
        assert_eq!(rng.state0, state0);
    }

    #[test]
    fn the_rubber_band_blends_between_the_tables() {
        let at = |difficulty| {
            Bots::new(Setup {
                difficulty,
                track: Some(0),
                cup: false,
                players: 1,
            })
        };
        let row = difficulty::ARCADE_DIFFICULTY[0];
        assert_eq!(at(0).arcade, row[0]);
        assert_eq!(at(0xf0).arcade, row[1]);
        let medium = at(difficulty_level::MEDIUM).arcade;
        for i in 0..14 {
            let (lo, hi) = (row[0][i] as i32, row[1][i] as i32);
            assert_eq!(medium[i] as i32, lo + 0xa0 * (hi - lo) / 0xf0);
        }
    }

    #[test]
    fn the_grid_gives_each_row_its_paths_and_starts() {
        let bots = Bots::new(Setup {
            difficulty: difficulty_level::MEDIUM,
            track: Some(0),
            cup: false,
            players: 1,
        });
        // The outside places keep to their paths; the middle ones share the other two.
        assert_eq!([bots.driver_paths[0], bots.driver_paths[3]], [0, 2]);
        assert_eq!([bots.driver_paths[4], bots.driver_paths[7]], [0, 2]);
        assert_eq!(bots.driver_paths[1] ^ bots.driver_paths[5], 1);
        assert_eq!((bots.driver_paths[2] - 1) ^ (bots.driver_paths[6] - 1), 1);
        // Each row starts in a turn of four.
        let mut front: Vec<u8> = bots.accelerate_order[..4].to_vec();
        front.sort();
        assert_eq!(front, [0, 1, 2, 3]);
        let mut rear: Vec<u8> = bots.accelerate_order[4..].to_vec();
        rear.sort();
        assert_eq!(rear, [4, 5, 6, 7]);
    }

    #[test]
    fn of_two_bots_that_bump_the_one_behind_slows() {
        // A path along +x.
        let frame = |x| NavFrame {
            pos: SVec3 { x, y: 0, z: 0 },
            ..NavFrame::default()
        };
        let paths = [NavPath {
            frames: vec![frame(0), frame(1000), frame(2000)],
            ..NavPath::default()
        }];
        let kart = |x: i32, rank| {
            let mut d = Driver::new(0, 0);
            d.pos_curr.x = x << 8;
            d.driver_rank = rank;
            let mut b = BotData::default();
            b.phys.speed_linear = 10_000;
            (d, b)
        };
        let (front, mut front_bot) = kart(600, 0);
        let (back, mut back_bot) = kart(500, 1);
        front_bot.phys.speed_linear = 8_000;
        collide_with_other_ai((&front, &mut front_bot), (&back, &mut back_bot), &paths);
        assert_eq!(back_bot.phys.speed_linear, 8_000 - COLLISION_SPEED_PENALTY);
        assert_eq!(front_bot.phys.speed_linear, 8_000);
        assert_eq!(
            map_range_pos_points(frame(1000).pos, frame(0).pos, [1200, 0, 0]),
            200
        );
    }

    #[test]
    fn a_hazard_timer_runs_down_while_the_bot_moves() {
        let mut d = Driver::new(0, 0);
        d.speed_approx = 0x200;
        hazard_timer(&mut d, 32);
        assert_eq!(d.hazard_timer, -32);
        hazard_timer(&mut d, 32);
        assert_eq!(d.hazard_timer, -64);
    }
}
