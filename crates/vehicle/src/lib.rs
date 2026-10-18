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

pub mod calc;
pub mod crash;
pub mod scrub;
pub mod search;

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
    pub const TURBO_INPUT_LATCH: u32 = 0x80;
    pub const DRIVING_AGAINST_WALL: u32 = 0x2000;
    pub const ENGINE_ECHO: u32 = 0x10000;
}

/// `KartState`
pub mod kart_state {
    pub const CRASHING: u8 = 1;
    pub const DRIFTING: u8 = 2;
    pub const MASK_GRABBED: u8 = 5;
}

/// `BakedGteMatrix`: which table of baked rotations the animation draws from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MatrixArray {
    #[default]
    None,
    CrashFall,
}

/// The set of procedures a driver is currently running (the reference's
/// `funcPtrs`). Only the states reached by the ported code exist so far.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PhysState {
    #[default]
    Driving,
    SlamWall,
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
}

/// The part of `struct Driver` the ported code uses. Positions are 24.8
/// fixed point (`pos_curr`) unless noted; quad blocks are indices into
/// `Level::quad_blocks`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Driver {
    pub driver_id: u8,
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
    pub jump_height_curr: i16,
    pub x_speed: i32,
    pub y_speed: i32,
    pub z_speed: i32,
    pub speedometer_needle_value: i16,

    pub reserves: i16,
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
}
