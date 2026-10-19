//! The driver's tick: the stages `MainFrame` runs over `funcPtrs`, in order,
//! for the procedures a driver running normally (or crashing into a wall, or
//! frozen by the V-shift guard) uses. Plus the birth and spawn of a driver.
//!
//! Left out of the tick: kart-against-kart (`CollideDrivers`), the audio,
//! animation and particle stages (all presentation), drifting
//! (`VehPhysProc_PowerSlide_*`), and the other states a driver can be in
//! (spinning out, the rev-engine start, the mask grab, being blasted).

use coll::Collision;
use loader::lev::{Level, SVec3};
use math::mips;

use crate::angular::phys_angular;
use crate::calc::interp_by_speed;
use crate::fixed::fixed_player_search;
use crate::force::{on_apply_forces, rot_axis_angle, translate_matrix};
use crate::jump::jump_and_friction;
use crate::linear::phys_linear;
use crate::scrub::IDENTITY_LIGHT_MATRIX;
use crate::search::moved_player_search;
use crate::stats::{CHARACTER_ENGINES, Stats};
use crate::terrain::terrain_type;
use crate::{Driver, Effect, Env, Input, PhysState, actions, kart_state, rain_cloud};

const V_SHIFT_START_GUARD_TIMER: i16 = 0x60;
const V_SHIFT_WINDOW_TIMER: i16 = 0x280;
const V_SHIFT_MAX_COUNT: i16 = 5;
const STEER_RESET_FRAMES: i16 = 10000;
/// `VEH_BIRTH_DRIVER_BOTTOM_Y_OFFSET`
const SPAWN_BOTTOM_Y_OFFSET: i32 = 0x80;
/// `VEH_BIRTH_COLL_PROBE_TOP_Y_OFFSET`
const SPAWN_PROBE_TOP_Y_OFFSET: i32 = 0x100;

impl Driver {
    /// A new driver of the character (`GAME_CHARACTER_IDS` order), as
    /// `VehBirth_NonGhost` and `VehBirth_TireSprites` leave it. It is not on
    /// the track yet: see [`spawn`].
    pub fn new(driver_id: u8, character: usize) -> Driver {
        Driver {
            driver_id,
            stats: Stats::for_class(CHARACTER_ENGINES[character]),
            axis_angle1_normal_vec: SVec3 { x: 0, y: 0x1000, z: 0 },
            axis_angle2_normal_vec: SVec3 { x: 0, y: 0x1000, z: 0 },
            num_frames_spent_steering: STEER_RESET_FRAMES,
            terrain_meta1: terrain_type::NONE,
            terrain_meta2: terrain_type::NONE,
            current_terrain: terrain_type::NONE,
            rain_cloud_effect: rain_cloud::NONE,
            ..Driver::default()
        }
    }
}

/// `VehBirth_TeleportSelf` for the start of a race: puts the driver on the
/// track at spawn point `spawn_index`, facing along the track, standing still.
pub fn spawn(d: &mut Driver, coll: &mut Collision, level: &Level, spawn_index: usize) {
    use loader::lev::quad_flags;

    let spawn = &level.driver_spawns[spawn_index];
    {
        let q = &mut coll.sps.quad_block_coll;
        q.quad_flags_wanted = (quad_flags::GROUND | quad_flags::COLLISION_SURFACE) as u32;
        q.quad_flags_ignored = 0;
        q.search_flags = coll::search_flags::HIGH_LOD;
    }

    d.actions_flag_set |= actions::BEHIND_START_LINE;
    d.distance_to_finish_checkpoint = level.restart_points.first().map_or(0, |p| (p.dist_to_finish as u32) << 3);

    let bottom = SVec3 { x: spawn.pos.x, y: (spawn.pos.y as i32 + SPAWN_BOTTOM_Y_OFFSET) as i16, z: spawn.pos.z };
    let top = SVec3 { x: bottom.x, y: (bottom.y as i32 - SPAWN_PROBE_TOP_Y_OFFSET) as i16, z: bottom.z };
    coll.search_quadblocks(level, top, bottom, 0);

    if coll.sps.did_touch_quadblock != 0 {
        d.axis_angle3_normal_vec = coll.sps.hit.plane.normal;
        d.last_valid = coll.sps.hit.quad;
    } else {
        d.axis_angle3_normal_vec = SVec3 { x: 0, y: 0x1000, z: 0 };
    }
    d.axis_angle1_normal_vec = d.axis_angle3_normal_vec;
    d.axis_angle2_normal_vec = d.axis_angle3_normal_vec;
    d.axis_angle4_normal_vec = d.axis_angle2_normal_vec;

    let hit_pos = coll.sps.quad_block_coll.hit_pos;
    d.pos_curr.x = mips::sll(hit_pos.x as i32, 8);
    d.pos_curr.y = mips::sll(hit_pos.y as i32, 8);
    d.pos_curr.z = mips::sll(hit_pos.z as i32, 8);
    d.pos_prev = d.pos_curr;
    d.quad_block_height = mips::sll(hit_pos.y as i32, 8);

    d.rot_curr_x = spawn.rot.x;
    d.rot_curr_y = ((spawn.rot.y as i32 + 0x400) & 0xfff) as i16;
    d.rot_curr_z = spawn.rot.z;

    d.speed = 0;
    d.speed_approx = 0;
    d.jump_height_curr = 0;
    d.jump_height_prev = 0;
    d.forward_accel_impulse = 0;
    d.angle = d.rot_curr_y;
    d.rot_prev = [d.rot_curr_x, d.rot_curr_y, d.rot_curr_z, 0];
    d.jump_landing_boost = 0;
    d.turn_angle_curr = 0;
    d.turn_angle_lerp_vel = 0;
    d.turn_angle_prev = 0;
    d.rot_curr_w = 0;
    d.jump_squish_stretch = 0;
    d.under_driver = None;
    d.clock_receive = 0;
    d.actions_flag_set &= !(actions::AIRBORNE | actions::HIGH_JUMP);
    d.num_wumpas = 0;

    // Straight into driving (the original starts with the engine-revving
    // state of the traffic lights).
    d.init_pending = true;
}

/// `VehPhysProc_Driving_Init`
fn driving_init(d: &mut Driver) {
    d.v_shift_start_guard_timer = V_SHIFT_START_GUARD_TIMER;
    d.v_shift_window_timer = V_SHIFT_WINDOW_TIMER;
    d.phys_state = PhysState::Driving;
    d.kart_state = kart_state::NORMAL;
    d.turbo_meter_room_left = 0;
    d.v_shift_count = 0;
    d.init_pending = false;
}

/// `VehPhysProc_FreezeVShift_Init`
fn freeze_vshift_init(d: &mut Driver) {
    d.kart_state = kart_state::ANTIVSHIFT;
    d.phys_state = PhysState::FreezeVShift;
    d.turbo_meter_room_left = 0;
    d.init_pending = false;
    d.actions_flag_set &= !actions::HUMAN_HUMAN_COLLISION;
}

/// The update stage: `VehPhysProc_Driving_Update` (without starting a drift)
/// and `VehPhysProc_FreezeVShift_Update`.
fn update(d: &mut Driver) {
    match d.phys_state {
        PhysState::Driving => {
            // At this point, assume driver is not touching ground. Wait until
            // the V-shift startup guard has elapsed; if V_Shift happened too
            // many times, meaning you jitter between two quadblocks in a "V"
            // shape, stop driving until you press X.
            if d.v_shift_start_guard_timer == 0 && d.v_shift_count >= V_SHIFT_MAX_COUNT {
                freeze_vshift_init(d);
            } else if d.v_shift_window_timer == 0 {
                d.v_shift_count = 0;
            }
        }
        PhysState::FreezeVShift => {
            if d.fire_speed != 0
                || d.actions_flag_set & (actions::HUMAN_HUMAN_COLLISION | actions::ACCEL_PREVENTION | actions::BRAKE_WITH_ACCEL) != 0
            {
                driving_init(d);
            } else {
                d.speed = 0;
                d.speed_approx = 0;
            }
        }
        PhysState::SlamWall => {}
    }
}

/// `VehPhysProc_SlamWall_PhysAngular`
fn slam_wall_phys_angular(d: &mut Driver, gte: &mut gte::Gte, env: &Env) {
    let elapsed = env.elapsed_time_ms;
    let angle = mips::add_lo(d.angle as u16 as i32, mips::sra(mips::mul_lo(d.amp_turn_state as i32, elapsed), 13)) & 0xfff;
    d.angle = angle as i16;

    d.rot_curr_y = mips::add_lo(d.turn_wobble_angle as u16 as i32, mips::add_lo(angle, d.turn_angle_curr as u16 as i32)) as i16;
    d.rot_curr_w = interp_by_speed(d.rot_curr_w as i32, mips::sra(mips::sll(elapsed, 5), 5), 0) as i16;
    d.turn_angle_curr = interp_by_speed(d.turn_angle_curr as i32, mips::sra(mips::sll(elapsed, 7), 5), 0) as i16;

    let mut moving = d.matrix_moving_dir;
    rot_axis_angle(&mut moving, gte, d.axis_angle1_normal_vec, d.angle as i32);
    d.matrix_moving_dir = moving;
}

/// `VehPhysProc_SlamWall_Animate`: the crash animation plays through, then
/// the driver drives again.
fn slam_wall_animate(d: &mut Driver) {
    d.anim_frame = (d.anim_frame as u16).wrapping_add(1) as i16;
    d.matrix_index = d.matrix_index.wrapping_add(1);

    let frames = |index: u8| d.anim_num_frames.get(index as usize).copied().unwrap_or(0) as i32;
    if (d.anim_frame as i32) < frames(d.anim_index) - 1 {
        return;
    }

    let drive_frames = frames(0);
    if drive_frames > 0 {
        d.anim_index = 0;
        // `VehFrameInst_GetStartFrame`: the midpoint of the driving animation.
        d.anim_frame = (drive_frames >> 1) as i16;
        d.matrix_array = crate::MatrixArray::None;
        d.matrix_index = 0;
    }
    d.init_pending = true;
}

/// What a tick asks of the game beyond the driver.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TickResult {
    /// The driver fell off the track or touched a kill plane: the original
    /// starts `VehStuckProc_MaskGrab_Init`, which puts it back on the track.
    pub mask_grab: bool,
}

/// One 30 Hz tick of a driver: every stage of `MainFrame`'s loop over the
/// driver's procedures. `effects` collects the sounds and rumble.
pub fn tick(d: &mut Driver, coll: &mut Collision, level: &Level, env: &Env, input: &Input, effects: &mut Vec<Effect>) -> TickResult {
    // DRIVER_FUNC_INIT
    if d.init_pending {
        driving_init(d);
    }

    // DRIVER_FUNC_UPDATE
    update(d);

    // DRIVER_FUNC_PHYS_LINEAR
    phys_linear(d, env, input, effects);
    if d.phys_state == PhysState::SlamWall {
        d.base_speed = 0;
        d.fire_speed = 0;
    }

    // DRIVER_FUNC_PHYS_ANGULAR
    if d.phys_state == PhysState::SlamWall {
        slam_wall_phys_angular(d, &mut coll.gte, env);
    } else {
        phys_angular(d, &mut coll.gte, env);
    }

    // DRIVER_FUNC_APPLY_FORCES
    on_apply_forces(d, coll, env, level, effects);

    // DRIVER_FUNC_COLL_MOVED
    moved_player_search(d, coll, level, &IDENTITY_LIGHT_MATRIX, env.elapsed_time_ms, effects);

    // DRIVER_FUNC_COLLIDE_DRIVERS: nothing to collide with.

    // DRIVER_FUNC_COLL_FIXED
    let mask_grab = fixed_player_search(d, coll, level, env, effects);

    // DRIVER_FUNC_JUMP_FRICTION
    jump_and_friction(d, &mut coll.gte, env, level, effects);
    if d.phys_state == PhysState::FreezeVShift {
        // `VehPhysProc_FreezeVShift_ReverseOneFrame`: after the usual,
        if d.actions_flag_set & actions::JUMP_STARTED != 0 {
            driving_init(d);
        } else if d.actions_flag_set & actions::HUMAN_HUMAN_COLLISION == 0 {
            d.x_speed = 0;
            d.y_speed = 0;
            d.z_speed = 0;
            d.speed = 0;
            d.speed_approx = 0;
            d.pos_curr = d.pos_prev;
        }
    }

    // DRIVER_FUNC_TRANSLATE_MATRIX
    translate_matrix(d, &mut coll.gte);

    // DRIVER_FUNC_ANIMATE
    if d.phys_state == PhysState::SlamWall {
        slam_wall_animate(d);
    }

    TickResult { mask_grab }
}
