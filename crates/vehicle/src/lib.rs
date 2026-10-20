//! Driver state and the vehicle code that runs after collision, ported from
//! the reference `ctr-native` (`game/COLL.c`, `game/Vehicle/`).
//!
//! The reference's `struct Driver` is ~0x640 bytes and carries rendering,
//! audio and HUD state next to the physics. Here [`Driver`] holds only the
//! fields the ported code reads or writes, named after the C fields in
//! snake case, and grows as more of the vehicle code is ported. Anything the
//! original does to the outside world (sound, rumble, voice lines) is
//! reported as an [`Effect`] instead, so the simulation stays free of I/O and
//! can be rolled back and replayed.
//!
//! Function pointers in the reference (`d->funcPtrs[...]`) become
//! [`PhysState`], one variant per set of procedures.
//!
//! `tests/reference.rs` checks this port against the C code.

pub mod angular;
pub mod calc;
pub mod crash;
pub mod exhaust;
pub mod fixed;
pub mod fire;
pub mod force;
pub mod frame;
pub mod jump;
pub mod jump_meter;
pub mod joystick;
pub mod linear;
pub mod powerslide;
pub mod scrub;
pub mod search;
pub mod shadow;
pub mod sim;
pub mod spin;
pub mod stats;
pub mod terrain;
pub mod turbo;

use loader::lev::SVec3;
use math::Vec3;

/// `DriverCollisionFlags`
pub mod collision_flags {
    pub const MASK_GRAB_REQUEST: u16 = 0x1;
    pub const SURFACE_PUSHBACK: u16 = 0x2;
    pub const TOUCHED_QUADBLOCK: u16 = 0x4;
    pub const GROUNDED: u16 = 0x8;
}

/// `Actions` (`Driver::actionsFlagSet`)
pub mod actions {
    pub const TOUCH_GROUND: u32 = 0x1;
    pub const STARTED_TOUCH_GROUND: u32 = 0x2;
    pub const JUMP_BUTTON_HELD: u32 = 0x4;
    pub const ACCEL_PREVENTION: u32 = 0x8;
    pub const STEER_LEFT: u32 = 0x10;
    pub const BRAKE_WITH_ACCEL: u32 = 0x20;
    pub const HIGH_JUMP: u32 = 0x40;
    pub const TURBO_INPUT_LATCH: u32 = 0x80;
    pub const DRIVING_WRONG_WAY: u32 = 0x100;
    pub const TURBO_ITEM: u32 = 0x200;
    pub const JUMP_STARTED: u32 = 0x400;
    pub const BACK_SKID: u32 = 0x800;
    pub const FRONT_SKID: u32 = 0x1000;
    pub const DRIVING_AGAINST_WALL: u32 = 0x2000;
    pub const WARP: u32 = 0x4000;
    pub const WEAPON_FIRE_REQUEST: u32 = 0x8000;
    pub const ENGINE_ECHO: u32 = 0x10000;
    pub const REVERSING_ENGINE: u32 = 0x20000;
    pub const RACE_TIMER_FROZEN: u32 = 0x40000;
    pub const AIRBORNE: u32 = 0x80000;
    pub const BOT: u32 = 0x100000;
    pub const NEW_BOOST: u32 = 0x200000;
    pub const ACCEL_RELEASED_WITH_RESERVES: u32 = 0x400000;
    pub const MASK_WEAPON: u32 = 0x800000;
    pub const BEHIND_START_LINE: u32 = 0x1000000;
    pub const RACE_FINISHED: u32 = 0x2000000;
    pub const TRACKER_TARGETED: u32 = 0x4000000;
    pub const CHECKPOINT_BRANCH_PENDING: u32 = 0x8000000;
    pub const HUMAN_HUMAN_COLLISION: u32 = 0x10000000;
    pub const REVERSE_STEER_LEFT: u32 = 0x20000000;
    pub const REVERSE_STEER_RIGHT: u32 = 0x40000000;
}

/// Gamepad buttons (`GamepadButtons`) as the driver reads them.
pub mod buttons {
    pub const CROSS: u32 = 0x10;
    pub const SQUARE: u32 = 0x20;
    pub const CIRCLE: u32 = 0x40;
    pub const R1: u32 = 0x400;
    pub const L1: u32 = 0x800;
}

/// `COLL_STEP_TRIGGER_*`: bits of `step_flag_set` set by trigger quad blocks.
pub mod step_triggers {
    pub const TURBO_PAD: u32 = 0x1;
    pub const SUPER_TURBO_PAD: u32 = 0x2;
    pub const TURBO_PAD_MASK: u32 = TURBO_PAD | SUPER_TURBO_PAD;
    pub const SUPPRESS_MASK_GRAB: u32 = 0x8;
}

/// `RainCloudEffect`
pub mod rain_cloud {
    pub const SLOW: u8 = 0;
    pub const ITEM_ROLL: u8 = 1;
    pub const ICE_TERRAIN: u8 = 2;
    pub const JUMP_LOCKOUT: u8 = 3;
    pub const NONE: u8 = 4;
    pub const HEAVY_FRICTION: u8 = 5;
    pub const RESERVE_RELEASE: u8 = 6;
}

/// `ForcedJumpType`
pub mod forced_jump {
    pub const NONE: u8 = 0;
    pub const LOW: u8 = 1;
    pub const HIGH: u8 = 2;
}

/// A `MATRIX` without the translation: `m[row][column]`, 4.12 fixed point.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Matrix {
    pub m: [[i16; 3]; 3],
}

impl Matrix {
    /// The five words the GTE rotation registers (`R11R12` .. `R33`) hold.
    pub fn words(&self) -> [u32; 5] {
        let m = &self.m;
        let pack = |lo: i16, hi: i16| lo as u16 as u32 | (hi as u16 as u32) << 16;
        [pack(m[0][0], m[0][1]), pack(m[0][2], m[1][0]), pack(m[1][1], m[1][2]), pack(m[2][0], m[2][1]), m[2][2] as u16 as u32]
    }
}

/// What the gamepad gives the driver in one tick (`GamepadBuffer`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Input {
    pub buttons_held: u32,
    pub buttons_tapped: u32,
    /// Sticks are 0..=255 with 0x80 centred; LX is steering (0 is left), LY
    /// reverses, RY is an analogue gas.
    pub stick_lx: u8,
    pub stick_ly: u8,
    pub stick_ry: u8,
}

impl Default for Input {
    fn default() -> Self {
        Input { buttons_held: 0, buttons_tapped: 0, stick_lx: 0x80, stick_ly: 0x80, stick_ry: 0x80 }
    }
}

/// The parts of `GameTracker` the physics reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Env {
    /// `elapsedTimeMS`. Retail derives it from the wall clock (clamped to
    /// 32..=64); here every 30 Hz tick is 32 so the simulation is deterministic.
    pub elapsed_time_ms: i32,
    /// `elapsedEventTime`: milliseconds since the race started.
    pub elapsed_event_time: i32,
    /// `timer`: the frame counter.
    pub timer: u32,
    /// `gameMode1 & END_OF_RACE`: the driver ignores the gamepad.
    pub end_of_race: bool,
}

impl Default for Env {
    fn default() -> Self {
        Env { elapsed_time_ms: 32, elapsed_event_time: 0, timer: 0, end_of_race: false }
    }
}

/// `KartState`
pub mod kart_state {
    pub const NORMAL: u8 = 0;
    pub const CRASHING: u8 = 1;
    pub const DRIFTING: u8 = 2;
    pub const SPINNING: u8 = 3;
    pub const ENGINE_REVVING: u8 = 4;
    pub const MASK_GRABBED: u8 = 5;
    pub const BLASTED: u8 = 6;
    pub const ANTIVSHIFT: u8 = 9;
    pub const WARP_PAD: u8 = 10;
    pub const FREEZE: u8 = 11;
}

/// `BakedGteMatrix`: which table of baked rotations the animation draws from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MatrixArray {
    #[default]
    None,
    CrashFall,
    /// `BAKED_GTE_MATRIX_JUMP_BASE + n`: the airborne pose of a character.
    Jump(u8),
}

/// The set of procedures a driver is currently running (the reference's
/// `funcPtrs`). Only the states reached by the ported code exist so far.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PhysState {
    #[default]
    Driving,
    SlamWall,
    /// `VehPhysProc_FreezeVShift_*`: the kart jittered between quad blocks and
    /// is held still until the player presses the gas or the brake again.
    FreezeVShift,
    /// `VehPhysProc_PowerSlide_*`: drifting.
    PowerSlide,
    /// `VehPhysProc_SpinFirst_*`: spinning out after a drift that lasted too long.
    SpinFirst,
    /// `VehPhysProc_SpinLast_*`: the spin winds down.
    SpinLast,
    /// `VehPhysProc_SpinStop_*`: the model straightens up, then driving resumes.
    SpinStop,
}

/// Something the original does that is not simulation: for the presentation
/// layer to play, or to ignore (headless, other peers).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// `GAMEPAD_JogCon1`
    JogCon { driver_id: u8, pattern: u8, strength: u8 },
    /// `OtherFX_Play_LowLevel(id, 1, HowlSfx_Pack(center, none, max, echo))`
    Sound { id: u8, echo: bool },
    /// `Voiceline_RequestPlay(id, characterIDs[driver_id], priority)`
    Voiceline { id: u8, driver_id: u8, priority: u8 },
    /// `GAMEPAD_ShockFreq`
    ShockFreq { driver_id: u8, a: u8, b: u8 },
    /// `GAMEPAD_ShockForce1`
    ShockForce { driver_id: u8, a: u8, b: u8 },
    /// `cameraDC[driver_id].flags |= CAMERA_FLAG_FIRE_SPEED_ZOOM`: a boost pulls the camera back.
    CameraFireSpeedZoom { driver_id: u8 },
}

impl Driver {
    /// `KartStates.Spinning.driftSpinRate`
    pub fn spin_rate(&self) -> i16 {
        self.num_frames_drifting
    }

    pub fn set_spin_rate(&mut self, value: i16) {
        self.num_frames_drifting = value;
    }

    /// `KartStates.Spinning.spinDir`
    pub fn spin_dir(&self) -> i16 {
        self.drift_boost_time_ms
    }

    pub fn set_spin_dir(&mut self, value: i16) {
        self.drift_boost_time_ms = value;
    }
}

/// The part of `struct Driver` the ported code uses. Positions are 24.8
/// fixed point (`pos_curr`) unless noted; quad blocks are indices into
/// `Level::quad_blocks`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Driver {
    pub driver_id: u8,
    /// `GAME_CHARACTER_IDS[driver_id]`: the character (in `GAME_CHARACTER_IDS`
    /// order) the driver plays.
    pub character: u8,
    pub phys_state: PhysState,
    pub kart_state: u8,
    pub simp_turn_state: i8,

    pub pos_curr: Vec3,
    pub velocity: Vec3,
    /// Offset from the model origin to the kart's centre (the collision sphere).
    pub origin_to_center: Vec3,

    pub collision_flags: u16,
    pub step_flag_set: u32,
    pub actions_flag_set: u32,
    pub actions_flag_set_prev_frame: u32,

    /// `currBlockTouching`: zero while airborne.
    pub curr_block_touching: Option<u32>,
    pub under_driver: Option<u32>,
    pub last_valid: Option<u32>,
    pub normal_vec_up: SVec3,
    pub axis_angle1_normal_vec: SVec3,
    pub sps_hit_pos: SVec3,
    pub sps_normal_vec: SVec3,
    pub pos_wall_coll: SVec3,

    pub v_shift_count: i16,
    pub speed: i16,
    pub speed_approx: i16,
    pub base_speed: i16,
    pub fire_speed: i16,
    pub fire_speed_cap: i16,
    pub jump_height_curr: i16,
    pub x_speed: i32,
    pub y_speed: i32,
    pub z_speed: i32,
    pub speedometer_needle_value: i16,

    pub reserves: i16,
    /// The exhaust fire of the boost, while there is one.
    pub turbo: Option<turbo::Turbo>,
    pub turbo_outside_timer: i16,
    pub turbo_meter_room_left: i16,
    pub wall_rub_timer: i16,
    pub wall_rub_speed_limit: i16,

    /// `angle`: the direction of travel; `rot_curr_w` is the render rotation's
    /// spin component.
    pub angle: i16,
    pub rot_curr_w: i16,
    pub turn_angle_curr: i16,
    pub turn_angle_lerp_vel: i16,
    pub turn_wobble_angle: i16,
    pub turn_wobble_velocity: i16,
    pub turn_wobble_timer: i16,
    pub amp_turn_state: i16,
    pub num_frames_spent_steering: i16,
    pub rotation_spin_rate: i16,
    pub veh_fire_audio_cooldown: i16,
    pub screen_offset_y: i8,
    pub distance_from_ground: i16,
    pub reserved_0x40e: i16,
    pub jump_squish_stretch: i16,
    pub jump_squish_stretch2: i16,

    /// The kart instance's animation (`instSelf->animIndex`, `animFrame`)
    /// and X/Y scale, which the crash code drives.
    pub anim_index: u8,
    pub anim_frame: i16,
    pub inst_scale_x: i16,
    pub inst_scale_y: i16,
    pub matrix_array: MatrixArray,
    pub matrix_index: u8,

    // --- the driving physics ---
    /// The `const_*` fields: the engine class's physics constants.
    pub stats: stats::Stats,
    pub num_wumpas: i8,
    pub accel_const: i8,
    pub turn_const: i8,
    pub turbo_const: i8,

    pub clock_receive: i16,
    pub hazard_timer: i16,
    /// `RainCloudEffect`; `rain_cloud::NONE` when there is no cloud.
    pub rain_cloud_effect: u8,
    pub burn_timer: i16,
    pub squish_timer: i16,
    pub super_engine_timer: i16,
    pub accel_tap_window_timer: i16,
    pub accel_tap_count: i16,
    pub v_shift_start_guard_timer: i16,
    pub v_shift_window_timer: i16,
    pub driver_rank: i16,

    pub jump_ten_buffer: i16,
    pub jump_cooldown_ms: i16,
    pub jump_coyote_timer_ms: i16,
    pub jump_forced_ms: i16,
    pub jump_initial_vel_y: i16,
    pub jump_high_jump_timer_ms: i16,
    pub jump_landing_boost: i16,
    /// `jumpMeter`: the hang time of the jump in the air or just landed, for the hang-time turbo and the HUD.
    pub jump_meter: i16,
    pub jump_meter_timer: i16,
    pub number_of_jumps: i16,
    pub forced_jump_type: u8,
    pub button_used_to_start_drift: i16,

    pub rot_curr_x: i16,
    pub rot_curr_y: i16,
    pub rot_curr_z: i16,
    pub rot_prev: [i16; 4],
    pub pos_prev: Vec3,
    pub jump_height_prev: i16,
    pub turn_angle_prev: i16,
    pub turn_angle_lerp_target: i16,
    pub matrix_moving_dir: Matrix,
    pub matrix_facing_dir: Matrix,

    /// Indices into [`terrain::TERRAINS`] (`terrainMeta1`, `terrainMeta2`).
    pub terrain_meta1: u8,
    pub terrain_meta2: u8,
    pub current_terrain: u8,
    pub terrain_friction_timer: i16,
    pub terrain_scaled_base_speed: i16,

    pub axis_angle2_normal_vec: SVec3,
    pub axis_angle3_normal_vec: SVec3,
    pub axis_angle4_normal_vec: SVec3,
    pub normal_vec_id: i8,
    pub axis_rotation_x: i16,
    pub axis_rotation_y: i16,
    pub forward_dir: i16,
    pub forward_accel_vector: SVec3,
    pub forward_accel_impulse: i16,
    pub accel: SVec3,
    pub wheel_rotation: i16,
    pub bool_first_frame_since_rev_engine: u8,
    pub quad_block_height: i32,
    pub distance_to_finish_checkpoint: u32,

    pub time_until_drift_spinout: i16,
    pub previous_frame_mult_drift: i16,

    /// `multDrift`: the drift's turn, negative when drifting right.
    pub mult_drift: i16,
    /// `NoInputTimer`: how long the spin-out keeps the kart from recovering.
    pub no_input_timer: i16,
    pub failed_boost_exhaust_timer: u8,
    /// `KartStates`, a union in the reference; this is the `Drifting` view,
    /// and [`Driver::spin_rate`] / [`Driver::spin_dir`] are the `Spinning`
    /// view of the same words.
    pub num_frames_drifting: i16,
    pub drift_boost_time_ms: i16,
    pub drift_total_time_ms: i16,
    pub num_boosts_attempted: i8,
    pub num_boosts_success: i8,
    /// Why the last drift stopped, for the HUD; not read by the sim.
    pub last_drift_end: Option<powerslide::DriftEnd>,

    /// Set when the original sets `funcPtrs[DRIVER_FUNC_INIT]` to
    /// `VehPhysProc_Driving_Init`: the next tick starts by driving again.
    pub init_pending: bool,
    /// The number of frames of the kart model's animations (`anim_index`),
    /// which time the crash animation.
    pub anim_num_frames: [u16; 8],
}
