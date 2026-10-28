//! The driver's tick: the stages `MainFrame` runs over `funcPtrs`, in order,
//! for the procedures a driver running normally (or crashing into a wall, or
//! frozen by the V-shift guard) uses. Plus the birth and spawn of a driver.
//!
//! Left out of the tick: kart-against-kart (the rest of `CollideDrivers`), the audio and particle stages (presentation), and
//! the other states a driver can be in (the race start's engine rev, being blasted).
//! A fall off the track is the mask grab and the rev that follows it ([`crate::stuck`]).

use coll::Collision;
use loader::lev::{Level, SVec3};
use math::mips;

use crate::angular::phys_angular;
use crate::calc::interp_by_speed;
use crate::collide::{self, Feedback, Others};
use crate::fixed::fixed_player_search;
use crate::force::{
    KART_SCALE_BASE, collide_drivers, on_apply_forces, rot_axis_angle, translate_matrix,
};
use crate::frame;
use crate::jump::jump_and_friction;
use crate::linear::phys_linear;
use crate::scrub::IDENTITY_LIGHT_MATRIX;
use crate::search::{Instances, NoInstances, moved_player_search};
use crate::stats::{CHARACTER_ENGINES, Stats};
use crate::stuck;
use crate::terrain::terrain_type;
use crate::{Driver, Effect, Env, Input, PhysState, actions, kart_state, rain_cloud};
use crate::{powerslide, spin, tumble};

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
            character: character as u8,
            stats: Stats::for_class(CHARACTER_ENGINES[character]),
            axis_angle1_normal_vec: SVec3 {
                x: 0,
                y: 0x1000,
                z: 0,
            },
            axis_angle2_normal_vec: SVec3 {
                x: 0,
                y: 0x1000,
                z: 0,
            },
            inst_scale_x: KART_SCALE_BASE as i16,
            inst_scale_y: KART_SCALE_BASE as i16,
            inst_scale_z: KART_SCALE_BASE as i16,
            num_frames_spent_steering: STEER_RESET_FRAMES,
            terrain_meta1: terrain_type::NONE,
            terrain_meta2: terrain_type::NONE,
            current_terrain: terrain_type::NONE,
            rain_cloud_effect: rain_cloud::NONE,
            held_item_id: crate::item::held::NONE,
            ..Driver::default()
        }
    }
}

/// `VehBirth_TeleportSelf` for the start of a race: puts the driver on the
/// track at spawn point `spawn_index`, facing along the track, standing still.
pub fn spawn(d: &mut Driver, coll: &mut Collision, level: &Level, spawn_index: usize) {
    let spawn = &level.driver_spawns[spawn_index];

    d.actions_flag_set |= actions::BEHIND_START_LINE;
    d.distance_to_finish_checkpoint = level
        .restart_points
        .first()
        .map_or(0, |p| (p.dist_to_finish as u32) << 3);

    let bottom = SVec3 {
        x: spawn.pos.x,
        y: (spawn.pos.y as i32 + SPAWN_BOTTOM_Y_OFFSET) as i16,
        z: spawn.pos.z,
    };
    probe_track(d, coll, level, bottom, 0);

    d.rot_curr_x = spawn.rot.x;
    d.rot_curr_y = ((spawn.rot.y as i32 + 0x400) & 0xfff) as i16;
    d.rot_curr_z = spawn.rot.z;
    reset_after_teleport(d);

    d.num_wumpas = 0;

    // Straight into driving (the original starts with the engine-revving
    // state of the traffic lights).
    d.init_pending = true;
}

/// `VehBirth_TeleportSelf` without the level's spawn points: puts the driver on the track under
/// where it is (`pos_curr`, `rot_curr`), `spawn_y` above it. The respawn of a mask grab.
pub(crate) fn teleport_self(d: &mut Driver, coll: &mut Collision, level: &Level, spawn_y: i32) {
    let bottom = SVec3 {
        x: mips::sra(d.pos_curr.x, 8) as i16,
        y: mips::add_lo(mips::sra(d.pos_curr.y, 8), SPAWN_BOTTOM_Y_OFFSET) as i16,
        z: mips::sra(d.pos_curr.z, 8) as i16,
    };
    probe_track(d, coll, level, bottom, spawn_y);
    reset_after_teleport(d);
}

/// The first half of `VehBirth_TeleportSelf`: finds the track straight below `bottom`, and
/// puts the driver `spawn_y` above it with the surface's normal.
fn probe_track(d: &mut Driver, coll: &mut Collision, level: &Level, bottom: SVec3, spawn_y: i32) {
    use loader::lev::quad_flags;

    {
        let q = &mut coll.sps.quad_block_coll;
        q.quad_flags_wanted = (quad_flags::GROUND | quad_flags::COLLISION_SURFACE) as u32;
        q.quad_flags_ignored = 0;
        q.search_flags = coll::search_flags::HIGH_LOD;
    }

    let top = SVec3 {
        x: bottom.x,
        y: (bottom.y as i32 - SPAWN_PROBE_TOP_Y_OFFSET) as i16,
        z: bottom.z,
    };
    coll.search_quadblocks(level, top, bottom, 0);

    if coll.sps.did_touch_quadblock != 0 {
        d.axis_angle3_normal_vec = coll.sps.hit.plane.normal;
        d.last_valid = coll.sps.hit.quad;
    } else {
        d.axis_angle3_normal_vec = SVec3 {
            x: 0,
            y: 0x1000,
            z: 0,
        };
    }
    d.axis_angle1_normal_vec = d.axis_angle3_normal_vec;
    d.axis_angle2_normal_vec = d.axis_angle3_normal_vec;
    d.axis_angle4_normal_vec = d.axis_angle2_normal_vec;

    let hit_pos = coll.sps.quad_block_coll.hit_pos;
    d.pos_curr.x = mips::sll(hit_pos.x as i32, 8);
    d.pos_curr.y = mips::sll(mips::add_lo(hit_pos.y as i32, spawn_y), 8);
    d.pos_curr.z = mips::sll(hit_pos.z as i32, 8);
    d.pos_prev = d.pos_curr;
    d.quad_block_height = mips::sll(hit_pos.y as i32, 8);
}

/// The second half of `VehBirth_TeleportSelf` (for a teleport that is not a race start): the
/// driver stands still, facing `rot_curr`.
fn reset_after_teleport(d: &mut Driver) {
    d.speed = 0;
    d.speed_approx = 0;
    d.jump_height_curr = 0;
    d.jump_height_prev = 0;
    d.forward_accel_impulse = 0;
    d.angle = d.rot_curr_y;
    d.rot_prev = [d.rot_curr_x, d.rot_curr_y, d.rot_curr_z, 0];

    d.anim_index = 0;
    d.anim_frame = frame::start_frame(0, frame::num_anim_frames(d, 0)) as i16;
    d.inst_scale_x = KART_SCALE_BASE as i16;
    d.inst_scale_y = KART_SCALE_BASE as i16;
    d.inst_scale_z = KART_SCALE_BASE as i16;
    d.matrix_array = crate::MatrixArray::None;
    d.matrix_index = 0;

    d.jump_landing_boost = 0;
    d.jump_meter = 0;
    d.jump_meter_timer = 0;
    d.turn_angle_curr = 0;
    d.turn_angle_lerp_vel = 0;
    d.turn_angle_prev = 0;
    d.rot_curr_w = 0;
    d.jump_squish_stretch = 0;
    d.under_driver = None;
    d.clock_receive = 0;
    d.actions_flag_set &= !(actions::AIRBORNE | actions::HIGH_JUMP);
}

/// `VehPhysProc_Driving_Init`
pub(crate) fn driving_init(d: &mut Driver) {
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

/// The init stage (`DRIVER_FUNC_INIT`): what a procedure that was started
/// last tick sets up. The drift and the spin-out only arm their update; the
/// rest start driving.
fn init_stage(d: &mut Driver) {
    match d.phys_state {
        PhysState::PowerSlide
        | PhysState::SpinFirst
        | PhysState::MaskGrab
        | PhysState::PlantEaten
        | PhysState::RevEngine
        | PhysState::Tumble => d.init_pending = false,
        _ => driving_init(d),
    }
}

/// The update stage: `VehPhysProc_Driving_Update`,
/// `VehPhysProc_FreezeVShift_Update`, `VehPhysProc_PowerSlide_Update` and the
/// spin-out's.
fn update(d: &mut Driver, env: &Env, input: &Input, effects: &mut Vec<Effect>) {
    match d.phys_state {
        PhysState::Driving => {
            // A drift starts on landing, if steering hard with the hop button held.
            if powerslide::start_if_drifting(d, input) {
                return;
            }

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
                || d.actions_flag_set
                    & (actions::HUMAN_HUMAN_COLLISION
                        | actions::ACCEL_PREVENTION
                        | actions::BRAKE_WITH_ACCEL)
                    != 0
            {
                driving_init(d);
            } else {
                d.speed = 0;
                d.speed_approx = 0;
            }
        }
        PhysState::SlamWall | PhysState::SpinStop => {}
        // These start with their own updates, which have more to work with: see `tick_with`.
        PhysState::MaskGrab | PhysState::PlantEaten | PhysState::RevEngine => {}
        PhysState::Tumble => tumble::update(d),
        PhysState::PowerSlide => powerslide::update(d, input, env, effects),
        PhysState::SpinFirst => spin::first_update(d),
        PhysState::SpinLast => spin::last_update(d),
    }
}

/// `VehPhysProc_SlamWall_PhysAngular`
fn slam_wall_phys_angular(d: &mut Driver, gte: &mut gte::Gte, env: &Env) {
    let elapsed = env.elapsed_time_ms;
    let angle = mips::add_lo(
        d.angle as u16 as i32,
        mips::sra(mips::mul_lo(d.amp_turn_state as i32, elapsed), 13),
    ) & 0xfff;
    d.angle = angle as i16;

    d.rot_curr_y = mips::add_lo(
        d.turn_wobble_angle as u16 as i32,
        mips::add_lo(angle, d.turn_angle_curr as u16 as i32),
    ) as i16;
    d.rot_curr_w =
        interp_by_speed(d.rot_curr_w as i32, mips::sra(mips::sll(elapsed, 5), 5), 0) as i16;
    d.turn_angle_curr = interp_by_speed(
        d.turn_angle_curr as i32,
        mips::sra(mips::sll(elapsed, 7), 5),
        0,
    ) as i16;

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
    /// The driver fell off the track or touched a kill plane, and the original
    /// starts `VehStuckProc_MaskGrab_Init` (see [`Env::respawns`]).
    pub mask_grab: bool,
    /// The mask grab let go of the driver and put it back on the track: the
    /// camera's direction changed.
    pub respawned: bool,
}

/// One 30 Hz tick of a driver: every stage of `MainFrame`'s loop over the
/// driver's procedures. `effects` collects the sounds and rumble.
pub fn tick(
    d: &mut Driver,
    coll: &mut Collision,
    level: &Level,
    env: &Env,
    input: &Input,
    effects: &mut Vec<Effect>,
) -> TickResult {
    tick_with(d, coll, level, env, input, &mut NoInstances, effects)
}

/// [`tick`], where `instances` is what touching the hitboxes of the level's instances does.
pub fn tick_with(
    d: &mut Driver,
    coll: &mut Collision,
    level: &Level,
    env: &Env,
    input: &Input,
    instances: &mut dyn Instances,
    effects: &mut Vec<Effect>,
) -> TickResult {
    let mut feedback = Feedback::default();
    tick_among(
        d,
        coll,
        level,
        env,
        input,
        instances,
        &mut Others {
            drivers: &mut [],
            feedback: &mut feedback,
        },
        effects,
    )
}

/// [`tick_with`], where `others` are the drivers that tick after this one, which it can bump into.
///
/// (The original runs each stage over all the drivers before the next stage: see [`tick_all`],
/// which does that, and the three parts this one runs.)
pub fn tick_among(
    d: &mut Driver,
    coll: &mut Collision,
    level: &Level,
    env: &Env,
    input: &Input,
    instances: &mut dyn Instances,
    others: &mut Others,
    effects: &mut Vec<Effect>,
) -> TickResult {
    collide::take_pending_damage(d, effects);
    let respawned = tick_to_coll_moved(d, coll, level, env, input, instances, effects);
    collide_drivers_stage(d, others, env, effects);
    let result = tick_from_coll_fixed(d, coll, level, env, respawned, effects);
    after_logic(d, env, effects);
    result
}

/// What runs after the weapons of a tick (whose hits land in their buckets, after the drivers'
/// stages): the exhaust fire's thread (TURBO, after the weapons' buckets; it dies at the end of
/// the frame's logic, in `PROC_CheckAllForDead`), and then the HUD's jump meter, in the frame's
/// render, so that a fire it lights or feeds first ticks on the next frame. [`tick`] runs it;
/// after a [`tick_all`], the caller does, once the weapons have hit.
pub fn after_logic(d: &mut Driver, env: &Env, effects: &mut Vec<Effect>) {
    crate::turbo::tick(d, env.elapsed_time_ms as i16, effects);
    crate::jump_meter::update(d, env, effects);
}

/// A driver of a [`tick_all`].
pub struct Racer<'a> {
    pub driver: &'a mut Driver,
    pub coll: &'a mut Collision,
    pub input: &'a Input,
    /// What the driver's stages ask for. What a bump asks for goes to the driver that bumped.
    pub effects: &'a mut Vec<Effect>,
}

/// One 30 Hz tick of all the drivers (without [`after_logic`]), in `MainFrame`'s order: each stage over every driver, in
/// the thread list's order, before the next stage. A driver bumps into the ones after it in the
/// list (the original's sibling threads), which have run the stages before it and not those after.
/// `feedback` times the bump sounds.
pub fn tick_all(
    racers: &mut [Racer],
    level: &Level,
    env: &Env,
    instances: &mut dyn Instances,
    feedback: &mut Feedback,
) -> Vec<TickResult> {
    // `VehPickupItem_ShootOnCirclePress`, for all first: the hits queued by the last bumps land.
    for r in racers.iter_mut() {
        collide::take_pending_damage(r.driver, r.effects);
    }
    let respawned: Vec<bool> = racers
        .iter_mut()
        .map(|r| tick_to_coll_moved(r.driver, r.coll, level, env, r.input, instances, r.effects))
        .collect();
    for i in 0..racers.len() {
        let (head, tail) = racers.split_at_mut(i + 1);
        let r = &mut head[i];
        let mut later: Vec<&mut Driver> = tail.iter_mut().map(|r| &mut *r.driver).collect();
        collide_drivers_stage(
            r.driver,
            &mut Others {
                drivers: &mut later,
                feedback,
            },
            env,
            r.effects,
        );
    }
    racers
        .iter_mut()
        .zip(respawned)
        .map(|(r, respawned)| {
            tick_from_coll_fixed(r.driver, r.coll, level, env, respawned, r.effects)
        })
        .collect()
}

/// The stages up to `DRIVER_FUNC_COLL_MOVED`. The result is whether the mask grab let go of the
/// driver and put it back on the track.
fn tick_to_coll_moved(
    d: &mut Driver,
    coll: &mut Collision,
    level: &Level,
    env: &Env,
    input: &Input,
    instances: &mut dyn Instances,
    effects: &mut Vec<Effect>,
) -> bool {
    let mut respawned = false;

    // DRIVER_FUNC_INIT
    if d.pending_eaten {
        // (Its update runs this tick, as the tumble's does.)
        d.pending_eaten = false;
        stuck::plant_eaten_init(d);
    }
    if d.pending_tumble {
        // (Its update runs this tick, unlike the spin-out's.)
        d.pending_tumble = false;
        tumble::init(d, effects);
    }
    if d.pending_spin {
        // `SpinFirst_Init` also clears the update procedure, so there is none this tick.
        d.pending_spin = false;
        spin::first_init(d, effects);
    } else {
        if d.init_pending {
            init_stage(d);
        }

        // DRIVER_FUNC_UPDATE
        match d.phys_state {
            // (There are no other drivers to keep the respawn off of.)
            PhysState::MaskGrab => respawned = stuck::mask_grab_update(d, coll, level, env, &[]),
            PhysState::PlantEaten => {
                respawned = stuck::plant_eaten_update(d, coll, level, env, &[])
            }
            PhysState::RevEngine => stuck::rev_engine_update(d, env, effects),
            _ => update(d, env, input, effects),
        }
    }

    // DRIVER_FUNC_PHYS_LINEAR
    match d.phys_state {
        PhysState::SpinFirst => spin::first_before_phys_linear(d, env),
        PhysState::RevEngine => stuck::rev_engine_before_phys_linear(d, env),
        PhysState::Tumble => tumble::before_phys_linear(d, env),
        _ => {}
    }
    phys_linear(d, env, input, effects);
    match d.phys_state {
        PhysState::SlamWall => {
            d.base_speed = 0;
            d.fire_speed = 0;
        }
        PhysState::PowerSlide => powerslide::after_phys_linear(d),
        PhysState::SpinFirst => spin::first_after_phys_linear(d),
        PhysState::SpinLast => spin::last_after_phys_linear(d),
        PhysState::SpinStop => spin::stop_after_phys_linear(d),
        PhysState::MaskGrab | PhysState::PlantEaten => stuck::mask_grab_after_phys_linear(d),
        PhysState::RevEngine => stuck::rev_engine_after_phys_linear(d),
        PhysState::Tumble => tumble::after_phys_linear(d),
        PhysState::Driving | PhysState::FreezeVShift => {}
    }

    // DRIVER_FUNC_PHYS_ANGULAR
    match d.phys_state {
        PhysState::SlamWall => slam_wall_phys_angular(d, &mut coll.gte, env),
        PhysState::PowerSlide => powerslide::phys_angular(d, &mut coll.gte, env),
        PhysState::SpinFirst => spin::first_phys_angular(d, &mut coll.gte, env),
        PhysState::SpinLast => spin::last_phys_angular(d, &mut coll.gte, env),
        PhysState::SpinStop => spin::stop_phys_angular(d, &mut coll.gte, env),
        PhysState::Driving | PhysState::FreezeVShift | PhysState::MaskGrab => {
            phys_angular(d, &mut coll.gte, env)
        }
        PhysState::Tumble => tumble::phys_angular(d, &mut coll.gte, env),
        PhysState::RevEngine | PhysState::PlantEaten => {}
    }

    // The rev has no forces and no collisions: the mask lowers the kart, or it stands still. Nor
    // has the kart in a plant.
    if !matches!(d.phys_state, PhysState::RevEngine | PhysState::PlantEaten) {
        // DRIVER_FUNC_APPLY_FORCES
        on_apply_forces(d, coll, env, level, effects);

        // DRIVER_FUNC_COLL_MOVED
        moved_player_search(
            d,
            coll,
            level,
            &IDENTITY_LIGHT_MATRIX,
            env.elapsed_time_ms,
            instances,
            effects,
        );
    }
    respawned
}

/// `DRIVER_FUNC_COLLIDE_DRIVERS`: the other karts, and the turbo pads are here too.
fn collide_drivers_stage(
    d: &mut Driver,
    others: &mut Others,
    env: &Env,
    effects: &mut Vec<Effect>,
) {
    if !matches!(d.phys_state, PhysState::RevEngine | PhysState::PlantEaten) {
        collide_drivers(d, others, env, effects);
    }
}

/// The stages from `DRIVER_FUNC_COLL_FIXED` on.
fn tick_from_coll_fixed(
    d: &mut Driver,
    coll: &mut Collision,
    level: &Level,
    env: &Env,
    respawned: bool,
    effects: &mut Vec<Effect>,
) -> TickResult {
    let mut mask_grab = false;
    if !matches!(d.phys_state, PhysState::RevEngine | PhysState::PlantEaten) {
        // DRIVER_FUNC_COLL_FIXED
        mask_grab = fixed_player_search(d, coll, level, env, effects);
        if mask_grab && env.respawns {
            // Starts in the middle of the tick: the stages after this one are the mask grab's.
            stuck::mask_grab_init(d, level);
        }

        // DRIVER_FUNC_JUMP_FRICTION
        jump_and_friction(d, &mut coll.gte, env, level, effects);
    }
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

    // DRIVER_FUNC_TRANSLATE_MATRIX (the kart in a plant is not drawn)
    if d.phys_state != PhysState::PlantEaten {
        translate_matrix(d, &mut coll.gte, effects);
    }

    // DRIVER_FUNC_ANIMATE
    match d.phys_state {
        PhysState::SlamWall => slam_wall_animate(d),
        PhysState::Driving | PhysState::FreezeVShift | PhysState::PowerSlide => frame::driving(d),
        PhysState::SpinFirst => frame::spinning(d),
        PhysState::SpinLast => frame::last_spin(d),
        PhysState::SpinStop => spin::stop_animate(d),
        PhysState::MaskGrab => stuck::mask_grab_animate(d, env, effects),
        // (Its animation only moves the camera to look at the plant.)
        PhysState::PlantEaten => {}
        PhysState::RevEngine => stuck::rev_engine_animate(d, env, effects),
        PhysState::Tumble => tumble::animate(d),
    }

    // The mask's own thread.
    stuck::mask_tick(d, env);

    // The shield's.
    crate::shield::tick(d, env.timer, effects);

    TickResult {
        mask_grab,
        respawned,
    }
}
