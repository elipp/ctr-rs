//! `VehStuckProc`: the procedures that take the kart out of the player's hands
//! after it fell off the track. The mask grab (`VehStuckProc_MaskGrab_*`)
//! holds the kart for 1440 ms while the Aku Aku mask comes down to lift it,
//! then puts it on the track at the respawn point of the last quad block it
//! was validly on (`FindDestPos`, `VehBirth_TeleportSelf`), and the engine rev
//! (`VehStuckProc_RevEngine_*`) lowers it back to the ground, where the rev
//! gives the start boost.
//!
//! Not ported: the animation of the kart (the crash fall, the wheelie of the
//! rev: `matrix_array` stays as it was), the mask's sounds, the
//! whistle and the particles. The mask's own motion is ([`MaskHead`], [`mask_tick`]),
//! but not its model: the presentation draws it from [`MaskHead::shown`]. What the animation stage does to the simulation
//! (the kart frozen at its last position, lifted by the mask) is ported.
//!
//! Not modelled either:
//! - The race start's engine rev: a driver is spawned straight into driving
//!   ([`crate::sim::spawn`]), so the traffic lights the rev waits for do not
//!   exist and the rev here is only ever the mask grab's.
//! - Other drivers: [`find_dest_pos`] takes the positions of the other
//!   drivers, because it does not respawn the kart where one of them is.
//! - The camera: `CAMERA_FLAG_MASK_GRAB` (and its height offset) while lowered
//!   by the mask; [`crate::sim::TickResult::respawned`] stands for the camera's
//!   direction change.
//! - The instance's matrix, which `MaskGrab_Init` reads the kart's position
//!   from: the kart's position at the start of the tick stands in for it.

use coll::{Collision, search_flags, step_flags};
use loader::lev::{Level, SVec3, quad_flags};
use math::{Vec3, mips};

use crate::calc::{interp_by_speed, map_to_range};
use crate::fire;
use crate::sim::{driving_init, teleport_self};
use crate::spin::lose_wumpa;
use crate::{Driver, Effect, Env, PhysState, actions, kart_state};

/// How long a mask grab holds the driver (`VEH_STUCK_MASK_GRAB_INITIAL_TIMER`).
const MASK_GRAB_INITIAL_TIMER: i16 = 1440;
/// A fall from this high above the track (`VEH_STUCK_MASK_GRAB_FALL_HEIGHT_THRESHOLD`) is a
/// fall into the void: the kart stays put and the mask comes for it.
const FALL_HEIGHT_THRESHOLD: i32 = 0x8000;
/// The kart is held where it was until the timer has run down to here (`..._CRASH_TIME`).
const CRASH_TIME: i16 = 1296;
/// The mask comes down once the timer has run down to here (`..._WHISTLE_TIME`).
const MASK_DESCENT_TIME: i16 = 960;
const SQUISH_DECREASE_LIMIT: i16 = 0x3c1;
const SQUISH_DECREASE_STEP: i16 = 800;
const SQUISH_INCREASE_STEP: i16 = 0x2d0;
const SQUISH_MAX: i16 = 8000;
const LIFT_SHIFT: u32 = 7;
const MASK_HEAD_Y_OFFSET: i32 = 0x140;

const RESPAWN_Y_OFFSET: i32 = 0x80;
const RESPAWN_PLAYER_CLEARANCE_XZ: i32 = 0x2000;
/// How far below the respawn point `MaskGrab_SearchBsp` looks for the track.
const MASK_BSP_PROBE_HEIGHT: i32 = 0x100;
/// The checkpoint index of a quad block outside the track's progress.
const NO_CHECKPOINT: u8 = 0xff;
/// `LevelConfigFlags`: the track has a void to fall into.
const CONFIG_VOID_FALL: u32 = 2;

/// `MASK_HEAD_DURATION_NORMAL`, and with the wumpa fruit juiced.
const MASK_DURATION_NORMAL: i16 = 0x1e00;
const MASK_DURATION_JUICED: i16 = 0x2d00;
const WUMPA_JUICED_COUNT: i8 = 10;
/// `STATIC_AKUAKU` and `STATIC_UKAUKA` plus `MASK_SOUND_ID_OFFSET_FROM_MODEL`: what the mask says.
const AKU_SFX: u8 = 0x53;
const UKA_SFX: u8 = 0x54;
/// How much the mask's duration grows each tick of its fade (`RB_MaskWeapon_FadeAway`), and
/// where the fade ends.
const MASK_FADE_END: i16 = 0x200;
/// `MASK_HEAD_SCALE_NORMAL`
pub const MASK_SCALE_NORMAL: i16 = 0x1000;
/// The mask turns by this much around its own vertical a tick.
const MASK_TURN_STEP: i16 = 0x100;
/// The orbit's radius is this many units (`sin << 6 >> 12`), and the head floats this much above the kart.
const MASK_ORBIT_SHIFT: u32 = 6;
const MASK_HEIGHT: i16 = 0x40;
/// `maskPosArr` (`R231`): how high the mask floats above the kart, by the frame of the beam's animation.
const MASK_BOB: [i16; 24] = [0, 0, -2, -4, -8, -12, -16, -19, -23, -27, -29, -31, -32, -31, -29, -27, -23, -19, -16, -12, -8, -4, -2, 0];
/// The frames of the beam's animation (`turn`).
pub const MASK_BEAM_FRAMES: u16 = 24;
/// How much the fade shrinks the mask and the beam and makes the beam see-through a tick.
const MASK_FADE_SHRINK: i16 = 0x100;
const MASK_FADE_ALPHA_STEP: u16 = 0x200;
const MASK_ALPHA_GONE: u16 = 0x1000;
/// The mask comes down over this much of the grab's last stretch (`..._SCALE_RAMP_TIME`).
const MASK_GRAB_SCALE_RAMP_TIME: i32 = 0xf0;
const MASK_GRAB_SCALE_FULL_TIME: i16 = 721;
/// Characters that Aku Aku looks after (`VehPickupItem_MaskBoolGoodGuy`): Crash, Coco, Polar, Pura
/// and Penta. The rest get Uka Uka.
const GOOD_GUYS: [u8; 5] = [0, 3, 6, 7, 13];

const REV_MASK_GRAB_HEIGHT_TRIGGER: i32 = 0x1000;
const REV_MASK_RELEASE_HEIGHT: i32 = 0x4000;
const REV_MASK_DESCENT_STEP: i32 = 0x200;
const REV_START_BOOST_RESERVES: i32 = 960;
const REV_SMALL_BOOST_FIRE_LEVEL: i32 = 0x20;
const REV_BIG_BOOST_FIRE_LEVEL: i32 = 0x80;
const REV_OVERREV_SFX: u8 = 0xf;
const REV_FILL_STEP_MAX: i32 = 5000;
const REV_STEP_MIN: i32 = 0x100;
const REV_OVERREV_TIMEOUT: i16 = 192;
const REV_RELEASE_COOLDOWN: i16 = 0x100;
const REV_DECAY_STEP_MAX_NORMAL: i32 = 1000;
const REV_DECAY_STEP_MAX_LOCKED: i32 = 3000;
const REV_EMPTY_COOLDOWN: i16 = 0xc0;
const REV_TURBO_ROOM_SHIFT: u32 = 5;
const REV_SQUISH_SHIFT: u32 = 6;
const REV_SQUISH_LIMIT: i32 = 0x401;
const REV_SQUISH_MAX: i16 = 0x400;
const REV_MODEL_BASE_SCALE: i32 = 3276;

/// `RevEngineChargeState`
mod charge {
    pub const IDLE: u8 = 0;
    pub const RELEASED_ABOVE_ACCEL: u8 = 1;
    pub const ACTIVE: u8 = 2;
}

/// `RevEngineLockoutFlags`
mod lockout {
    pub const PEDAL_HELD: u8 = 0x1;
    pub const REV_DECAY: u8 = 0x2;
    pub const ALL: u8 = PEDAL_HELD | REV_DECAY;
}

/// The mask of a mask grab (`struct MaskHeadWeapon` and its thread): the kart is lifted by it,
/// and while it exists the driver has the `MASK_WEAPON` action. It floats around the kart
/// turning, with a beam of light in it, and is drawn as [`MaskHead::shown`] says.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MaskHead {
    /// How long the mask lasts, in milliseconds.
    pub duration: i16,
    /// `pos`: where the mask is while it is not following the kart (`world_space`); `pos[1]`
    /// is also how high the mask has come down to the kart, in whole units.
    pub pos: [i16; 3],
    /// The mask has been told to go (`RB_MaskWeapon_FadeAway`), and goes after a few ticks.
    pub fading: bool,
    /// `rot.y`: how far the mask has turned.
    pub rot_y: i16,
    /// `rot.z & MASK_HEAD_ROT_WORLD_SPACE`: placed in the world at `pos`, not on the kart.
    pub world_space: bool,
    /// `scale`: 0x1000 is the mask's size.
    pub scale: i16,
    /// Aku Aku, not Uka Uka.
    pub good_guy: bool,
    /// The frame of the beam's animation (`maskBeamInst->animFrame`).
    pub beam_frame: u16,
    /// The mask's instances as its thread's last tick left them; none before the first.
    pub shown: Option<MaskShown>,
}

/// Where an instance of the mask is, and how big: the offset from the kart (or from `pos`),
/// and the turn on top of the kart's own.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Placed {
    pub offset: [i16; 3],
    /// `ConvertRotToMatrix`'s rotation about the vertical, if it is turned.
    pub rot_y: Option<i16>,
    pub scale: i16,
}

/// The mask's head and its beam of light (`maskInst`, `maskBeamInst`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MaskShown {
    pub head: Placed,
    pub beam: Placed,
    /// The offsets are from `MaskHead::pos`, not from the kart.
    pub world_space: bool,
    /// The beam's `alphaScale`: 0 is opaque, 0x1000 is gone.
    pub beam_alpha: u16,
    /// The beam's animation frame.
    pub beam_frame: u16,
}

impl MaskHead {
    /// The rotation matrix of a turn of `rot_y` about the vertical (`ConvertRotToMatrix`).
    pub fn rotation(rot_y: i16) -> crate::Matrix {
        crate::rotmat::convert_rot_to_matrix(&mut gte::Gte::default(), SVec3 { x: 0, y: rot_y, z: 0 })
    }
}

/// `KartStates.MaskGrab`, without the particles and the whistle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MaskGrab {
    /// The mask is lifting the kart, now that it has come down to it.
    pub lifting_player: bool,
    /// The kart fell a long way and is still falling: it is held in place for a while.
    pub still_falling: bool,
    /// `AngleAxis_NormalVec`: the surface the model stands on once it is lifted.
    pub angle_axis_normal_vec: SVec3,
}

/// `KartStates.RevEngine`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RevEngine {
    /// The rev level at which the rev gives the big boost.
    pub boost_meter: i32,
    pub fire_level: i32,
    pub over_rev_timer_ms: i16,
    pub release_cooldown_timer_ms: i16,
    pub empty_cooldown_timer_ms: i16,
    pub charge_state: u8,
    pub lockout_flags: u8,
    /// The rev is a mask grab's, not the race start's.
    pub bool_mask_grab: bool,
}

/// `VehPickupItem_MaskUseWeapon`: the driver has a mask, a new one or the one it had.
fn use_mask(d: &mut Driver) {
    let duration = if d.num_wumpas < WUMPA_JUICED_COUNT { MASK_DURATION_NORMAL } else { MASK_DURATION_JUICED };
    match &mut d.mask {
        Some(mask) => {
            mask.duration = duration;
            mask.fading = false;
        }
        None => {
            let good_guy = GOOD_GUYS.contains(&d.character);
            d.mask = Some(MaskHead { duration, scale: MASK_SCALE_NORMAL, good_guy, ..MaskHead::default() });
        }
    }
}

/// `VehPickupItem_ShootNow` for the mask item: the driver has a mask, which lasts 7680 ms, or
/// 11520 ms with 10 wumpa fruit (juiced). While it lasts the kart is faster (`mask_speed`), is not
/// slowed by mud or by leaving the track, and shrugs off hits.
pub fn shoot_mask(d: &mut Driver, effects: &mut Vec<Effect>) {
    use_mask(d);
    let good_guy = d.mask.is_some_and(|m| m.good_guy);
    effects.push(Effect::Sound {
        id: if good_guy { AKU_SFX } else { UKA_SFX },
        echo: d.actions_flag_set & actions::ENGINE_ECHO != 0,
    });
}

/// `RB_MaskWeapon_ThTick` and `RB_MaskWeapon_FadeAway`: the mask turns round the kart, runs
/// out, fades away and is gone.
pub fn mask_tick(d: &mut Driver, env: &Env) {
    let Some(mask) = &mut d.mask else { return };
    let mut shown = mask.shown.unwrap_or_default();

    if !mask.fading {
        // The head goes round the kart with the beam in it, bobbing with the beam's animation.
        let radius = |trig: i32| mips::sra(mips::mul_lo(mips::sra(mips::sll(trig, MASK_ORBIT_SHIFT), 12), mask.scale as i32), 12) as i16;
        let (sin, cos) = math::sin_cos(mask.rot_y as i32 as u32);
        shown.head = Placed {
            offset: [radius(sin), MASK_BOB[mask.beam_frame as usize % MASK_BOB.len()] + MASK_HEIGHT, radius(cos)],
            rot_y: Some(mask.rot_y),
            scale: shown.head.scale,
        };
        shown.beam = Placed { offset: [0, MASK_HEIGHT, 0], rot_y: Some(mask.rot_y), scale: shown.beam.scale };
        shown.world_space = mask.world_space;

        mask.beam_frame = if mask.beam_frame + 1 < MASK_BEAM_FRAMES { mask.beam_frame + 1 } else { 0 };
        mask.rot_y = mask.rot_y.wrapping_sub(MASK_TURN_STEP);

        if mask.duration != 0 {
            mask.duration = (mask.duration as u16).wrapping_sub(env.elapsed_time_ms as u16) as i16;
            if mask.duration < 0 {
                mask.duration = 0;
            }
        } else {
            mask.fading = true;
            if fade_step(mask, &mut shown, env) {
                d.mask = None;
                return;
            }
        }

        shown.beam_alpha = 0;
        shown.head.scale = mask.scale;
        shown.beam.scale = mask.scale;
        shown.beam_frame = mask.beam_frame;
        mask.shown = Some(shown);
        return;
    }

    if fade_step(mask, &mut shown, env) {
        d.mask = None;
    } else {
        mask.shown = Some(shown);
    }
}

/// `RB_MaskWeapon_FadeAway`: the head shrinks as it spirals in to the kart, the beam shrinks
/// and fades. True once it is gone.
fn fade_step(mask: &mut MaskHead, shown: &mut MaskShown, env: &Env) -> bool {
    let (sin, cos) = math::sin_cos(mask.rot_y as i32 as u32);
    let radius = |trig: i32| mips::sra(mips::mul_lo(MASK_HEIGHT as i32 - (mask.duration as i32 >> 5) * 4, trig), 12) as i16;
    shown.world_space = false;
    shown.head = Placed { offset: [radius(sin), MASK_HEIGHT, radius(cos)], rot_y: None, scale: shown.head.scale - MASK_FADE_SHRINK };

    mask.rot_y = mask.rot_y.wrapping_sub(MASK_TURN_STEP);
    shown.beam = Placed { offset: [0, MASK_HEIGHT, 0], rot_y: Some(mask.rot_y), scale: shown.beam.scale - MASK_FADE_SHRINK };
    if shown.beam_alpha < MASK_ALPHA_GONE {
        shown.beam_alpha += MASK_FADE_ALPHA_STEP;
    }

    if mask.duration < MASK_FADE_END {
        mask.duration = (mask.duration as u16).wrapping_add(env.elapsed_time_ms as u16) as i16;
        if mask.duration > MASK_FADE_END {
            mask.duration = MASK_FADE_END;
        }
        false
    } else {
        true
    }
}

/// `VehStuckProc_MaskGrab_Init`: what the fixed search starts when `check_mask_grab_progress`
/// (or the fall below the track) asks for it.
pub fn mask_grab_init(d: &mut Driver, level: &Level) {
    d.kart_state = kart_state::MASK_GRABBED;
    d.phys_state = PhysState::MaskGrab;
    d.init_pending = false;
    d.grab = MaskGrab::default();

    use_mask(d);

    d.turbo_meter_room_left = 0;
    d.reserves = 0;
    d.turbo_outside_timer = 0;
    d.matrix_array = crate::MatrixArray::None;
    d.matrix_index = 0;

    d.no_input_timer = MASK_GRAB_INITIAL_TIMER;
    d.actions_flag_set &= !(actions::AIRBORNE | actions::HIGH_JUMP);

    lose_wumpa(d, 2);

    if mips::add_lo(d.quad_block_height, FALL_HEIGHT_THRESHOLD) < d.pos_curr.y {
        // (`numTimesMaskGrab` and the falling particles are not kept.)
        if d.pos_curr.y < -FALL_HEIGHT_THRESHOLD && level.config_flags & CONFIG_VOID_FALL != 0 {
            d.grab.angle_axis_normal_vec = d.axis_angle2_normal_vec;
        } else {
            d.grab.still_falling = true;
        }
    } else {
        d.grab.angle_axis_normal_vec = d.axis_angle2_normal_vec;
    }

    // The kart goes back to where it was drawn (the instance's position).
    d.pos_curr.x = mips::sll(mips::sra(d.pos_prev.x, 8), 8);
    d.pos_curr.y = mips::sll(mips::sra(d.pos_prev.y, 8), 8);
    d.pos_curr.z = mips::sll(mips::sra(d.pos_prev.z, 8), 8);
    d.pos_prev = d.pos_curr;

    if let Some(mask) = &mut d.mask {
        // The mask comes down from above the kart, in the world.
        mask.world_space = true;
        mask.pos = [
            mips::sra(d.pos_curr.x, 8) as i16,
            mips::add_lo(mips::sra(d.pos_curr.y, 8), MASK_HEAD_Y_OFFSET) as i16,
            mips::sra(d.pos_curr.z, 8) as i16,
        ];
    }
}

/// `VehStuckProc_MaskGrab_Update`: counts the grab down, then puts the kart back on the track
/// and starts the rev. Returns whether it did.
pub fn mask_grab_update(d: &mut Driver, coll: &mut Collision, level: &Level, env: &Env, others: &[Vec3]) -> bool {
    d.no_input_timer = (d.no_input_timer as u16).wrapping_sub(env.elapsed_time_ms as u16) as i16;
    if d.no_input_timer < 0 {
        d.no_input_timer = 0;
    }
    if d.no_input_timer != 0 {
        return false;
    }

    // The mask lets go, and the kart comes down on the track.
    if let Some(mask) = &mut d.mask {
        mask.world_space = false;
        mask.scale = MASK_SCALE_NORMAL;
    }
    find_dest_pos(d, coll, level, d.last_valid, others);
    teleport_self(d, coll, level, RESPAWN_Y_OFFSET);
    rev_engine_init(d);
    true
}

/// `VehStuckProc_MaskGrab_PhysLinear`: after the usual, nothing the player does counts.
pub fn mask_grab_after_phys_linear(d: &mut Driver) {
    d.fire_speed = 0;
    d.jump_ten_buffer = 0;
    d.base_speed = 0;
    d.simp_turn_state = 0;
    d.actions_flag_set = (d.actions_flag_set & !(actions::REVERSING_ENGINE | actions::BRAKE_WITH_ACCEL | actions::JUMP_BUTTON_HELD))
        | actions::ACCEL_PREVENTION;
}

/// `VehStuckProc_MaskGrab_Animate`, the parts that are not the animation: a kart that fell far
/// stays where it was for a while, and the mask comes down to the kart and lifts it.
pub fn mask_grab_animate(d: &mut Driver, env: &Env) {
    if d.grab.still_falling {
        if d.no_input_timer >= CRASH_TIME {
            d.speed = 0;
            d.speed_approx = 0;
            d.pos_curr = d.pos_prev;
        } else if d.no_input_timer >= SQUISH_DECREASE_LIMIT {
            d.jump_squish_stretch = (d.jump_squish_stretch as u16).wrapping_add(SQUISH_INCREASE_STEP as u16) as i16;
            if d.jump_squish_stretch > SQUISH_MAX {
                d.jump_squish_stretch = SQUISH_MAX;
            }
        } else {
            d.jump_squish_stretch = (d.jump_squish_stretch as u16).wrapping_sub(SQUISH_DECREASE_STEP as u16) as i16;
            if d.jump_squish_stretch < 0 {
                d.jump_squish_stretch = 0;
            }
        }
    } else {
        d.matrix_array = crate::MatrixArray::None;
        d.matrix_index = 0;
        d.axis_angle2_normal_vec = d.grab.angle_axis_normal_vec;
    }

    let Some(mask) = &mut d.mask else { return };
    mask.duration = MASK_DURATION_NORMAL;
    if d.no_input_timer > MASK_DESCENT_TIME {
        mask.scale = 0;
        return;
    }

    if d.grab.lifting_player {
        d.speed = 0;
        d.pos_curr.y = mips::add_lo(d.pos_curr.y, mips::sll(env.elapsed_time_ms, LIFT_SHIFT));
        d.pos_prev.y = d.pos_curr.y;
    } else {
        mask.pos[1] = (mask.pos[1] as u16).wrapping_sub(env.elapsed_time_ms as u16) as i16;
    }
    mask.pos[2] = mips::sra(d.pos_curr.z, 8) as i16;
    if (mask.pos[1] as i32) < mips::sra(d.pos_curr.y, 8) {
        mask.pos[1] = mips::sra(d.pos_curr.y, 8) as i16;
        d.grab.lifting_player = true;
    }
    mask.pos[0] = mips::sra(d.pos_curr.x, 8) as i16;

    // It grows as it comes down.
    mask.scale = if d.no_input_timer >= MASK_GRAB_SCALE_FULL_TIME {
        (mips::sll(mips::sub_lo(MASK_DESCENT_TIME as i32, d.no_input_timer as i32), 12) / MASK_GRAB_SCALE_RAMP_TIME) as i16
    } else {
        MASK_SCALE_NORMAL
    };
}

/// `COLL_SearchBSP_CallbackPARAM` with `COLL_FIXED_BSPLEAF_TestQuadblocks` over the segment from
/// the kart's position down (`VehStuckProc_MaskGrab_SearchBsp`).
fn search_bsp(d: &Driver, coll: &mut Collision, level: &Level) {
    let pos = [mips::sra(d.pos_curr.x, 8), mips::sra(d.pos_curr.y, 8), mips::sra(d.pos_curr.z, 8)];
    let top = SVec3 { x: pos[0] as i16, y: pos[1] as i16, z: pos[2] as i16 };
    let bottom = SVec3 { x: pos[0] as i16, y: mips::sub_lo(pos[1], MASK_BSP_PROBE_HEIGHT) as i16, z: pos[2] as i16 };

    let sps = &mut coll.sps;
    sps.quad_block_coll.pos = top;
    sps.input1.pos = bottom;
    // The two players-or-fewer split screens are searched at high detail; the sim only has those.
    sps.quad_block_coll.search_flags = search_flags::HIGH_LOD;
    sps.did_touch_quadblock = 0;
    sps.num_triangles_tested = 0;
    sps.hit_fraction = coll::FRACTION_ONE;
    sps.collision.step_flags = 0;

    sps.bbox.min = SVec3 { x: top.x.min(bottom.x), y: top.y.min(bottom.y), z: top.z.min(bottom.z) };
    sps.bbox.max = SVec3 { x: top.x.max(bottom.x), y: top.y.max(bottom.y), z: top.z.max(bottom.z) };
    sps.quad_block_coll.hit_pos = bottom;

    coll.search_fixed(level);
}

/// `VehStuckProc_MaskGrab_FindDestPos`: where the kart is put back, given the quad block it was
/// last validly on. Sets the kart's position and heading (`pos_curr`, `rot_curr`); the track
/// under it is found by [`teleport_self`] after.
///
/// The respawn point is the checkpoint node (`Level::restart_points`) of the quad block, or if
/// there is no track under it, or a kill plane, or another driver (`others`, their `pos_curr`)
/// is in the way, the next node forward along the track. A quad block without a checkpoint
/// makes it the quad block's own middle. (The original walks the nodes for as long as it takes:
/// here the walk ends after one round of the track.)
pub fn find_dest_pos(d: &mut Driver, coll: &mut Collision, level: &Level, quad: Option<u32>, others: &[Vec3]) {
    let Some(quad) = quad else { return };
    let q = &level.quad_blocks[quad as usize];
    let points = &level.restart_points;

    if points.is_empty() || q.checkpoint_index == NO_CHECKPOINT {
        // The middle of the quad block's two opposite corners, a little above the track.
        let vertex = |corner: usize| level.vertices[q.vertex_index[corner] as usize].pos;
        let (a, b) = (vertex(0), vertex(3));
        d.pos_curr.x = mips::sll(mips::add_lo(a.x as i32, b.x as i32), 7);
        d.pos_curr.y = mips::sll(mips::add_lo(mips::add_lo(a.y as i32, b.y as i32), RESPAWN_Y_OFFSET), 7);
        d.pos_curr.z = mips::sll(mips::add_lo(a.z as i32, b.z as i32), 7);
        return;
    }

    let radius = coll::PLAYER_HIT_RADIUS;
    let sps = &mut coll.sps;
    sps.input1.hit_radius = radius;
    sps.input1.hit_radius_squared = radius as i32 * radius as i32;
    sps.quad_block_coll.hit_radius = radius;
    sps.quad_block_coll.hit_radius_squared = radius as i32 * radius as i32;
    sps.quad_block_coll.quad_flags_ignored = (quad_flags::NO_CAMERA_RESPAWN_PROBE | quad_flags::NO_COLLISION_RESPONSE) as u32;
    sps.quad_block_coll.quad_flags_wanted = quad_flags::GROUND as u32;

    let node = |index: usize| points.get(index).unwrap_or(&points[0]);
    let mut respawn = node(q.checkpoint_index as usize);
    for _ in 0..=points.len() {
        let next = node(respawn.next_forward as usize);

        d.pos_curr.x = mips::sll(respawn.pos.x as i32, 8);
        d.pos_curr.y = mips::sll(mips::add_lo(respawn.pos.y as i32, RESPAWN_Y_OFFSET), 8);
        d.pos_curr.z = mips::sll(respawn.pos.z as i32, 8);
        d.rot_curr_x = 0;
        d.rot_curr_y = math::ratan2(
            mips::sub_lo(next.pos.x as i32, respawn.pos.x as i32),
            mips::sub_lo(next.pos.z as i32, respawn.pos.z as i32),
        ) as i16;
        d.rot_curr_z = 0;

        search_bsp(d, coll, level);
        if coll.sps.did_touch_quadblock == 0 || coll.sps.collision.step_flags & step_flags::FLAG_KILL_PLANE != 0 {
            respawn = next;
            continue;
        }

        let near = |other: &Vec3| {
            let abs = |v: i32| if v < 0 { mips::neg_lo(v) } else { v };
            abs(mips::sub_lo(d.pos_curr.x, other.x)) < RESPAWN_PLAYER_CLEARANCE_XZ
                || abs(mips::sub_lo(d.pos_curr.z, other.z)) < RESPAWN_PLAYER_CLEARANCE_XZ
        };
        if others.iter().any(near) {
            respawn = next;
            continue;
        }
        break;
    }
}

/// `VehStuckProc_RevEngine_Init`
pub fn rev_engine_init(d: &mut Driver) {
    d.kart_state = kart_state::ENGINE_REVVING;
    d.phys_state = PhysState::RevEngine;
    d.init_pending = false;

    // The reason for the rev: the start of the race, unless the kart is high above the track.
    d.rev = RevEngine::default();
    if mips::add_lo(d.quad_block_height, REV_MASK_GRAB_HEIGHT_TRIGGER) < d.pos_curr.y {
        d.rev.bool_mask_grab = true;
        use_mask(d);
        d.actions_flag_set &= !actions::TOUCH_GROUND;
    }

    d.bool_first_frame_since_rev_engine = 1;
    d.rev.boost_meter = d.stats.accel_speed_class_stat as i32 + d.stats.sacred_fire_speed as i32 / 3;
}

/// `VehStuckProc_RevEngine_Update`: once the mask has lowered the kart close enough to the
/// track, the rev ends, and gives a boost if it was high enough.
pub fn rev_engine_update(d: &mut Driver, effects: &mut Vec<Effect>) {
    // (The race start's rev would wait for the traffic lights here.)
    if d.rev.bool_mask_grab && mips::add_lo(d.quad_block_height, REV_MASK_RELEASE_HEIGHT) <= d.pos_curr.y {
        return;
    }

    if d.rev.bool_mask_grab {
        if let Some(mask) = &mut d.mask {
            mask.duration = 0;
        }
    }

    let accel = d.stats.accel_speed_class_stat as i32;
    if accel < d.rev.fire_level && d.rev.lockout_flags & lockout::ALL == 0 {
        // Revved a lot: the big boost, else the small one.
        if mips::add_lo(accel, d.stats.sacred_fire_speed as i32) <= d.rev.boost_meter {
            fire::increment(d, REV_START_BOOST_RESERVES, 0, REV_BIG_BOOST_FIRE_LEVEL, effects);
        } else {
            fire::increment(d, REV_START_BOOST_RESERVES, 0, REV_SMALL_BOOST_FIRE_LEVEL, effects);
        }
    }

    d.turbo_meter_room_left = 0;
    driving_init(d);
}

/// `VehStuckProc_RevEngine_PhysLinear`, before the usual: the cooldowns count down.
pub fn rev_engine_before_phys_linear(d: &mut Driver, env: &Env) {
    let count_down = |timer: &mut i16| {
        *timer = (*timer as u16).wrapping_sub(env.elapsed_time_ms as u16) as i16;
        if *timer < 0 {
            *timer = 0;
        }
    };
    count_down(&mut d.rev.release_cooldown_timer_ms);
    count_down(&mut d.rev.empty_cooldown_timer_ms);
}

/// `VehStuckProc_RevEngine_PhysLinear`, after the usual: a kart the mask carries is lowered.
pub fn rev_engine_after_phys_linear(d: &mut Driver) {
    if !d.rev.bool_mask_grab {
        return;
    }
    d.pos_curr.y = mips::sub_lo(d.pos_curr.y, REV_MASK_DESCENT_STEP);
    if let Some(mask) = &mut d.mask {
        mask.duration = MASK_DURATION_NORMAL;
    }
}

/// `VehStuckProc_RevEngine_Animate`: holding the gas revs the engine up towards the boost meter
/// (and over it, which locks the pedal); letting go lets it fall. The rev level shows in the
/// speedometer and the turbo meter, and squashes the kart.
pub fn rev_engine_animate(d: &mut Driver, env: &Env, effects: &mut Vec<Effect>) {
    let accel = d.stats.accel_speed_class_stat as i32;
    let sacred = d.stats.sacred_fire_speed as i32;
    let r = &mut d.rev;

    if d.fire_speed > 0 && r.release_cooldown_timer_ms == 0 && r.lockout_flags & lockout::ALL == 0 {
        // Revving: the meter fills fast when it is far from the boost meter.
        let mut delta = mips::sub_lo(r.fire_level, r.boost_meter);
        if delta < 0 {
            delta = mips::neg_lo(delta);
        }
        let mut fill_step = mips::sra(delta, 1);
        if REV_FILL_STEP_MAX < delta {
            fill_step = REV_FILL_STEP_MAX;
        }
        if fill_step < REV_STEP_MIN {
            fill_step = REV_STEP_MIN;
        }

        r.fire_level = interp_by_speed(r.fire_level, fill_step, r.boost_meter);
        r.charge_state = charge::ACTIVE;

        if r.boost_meter <= r.fire_level {
            r.over_rev_timer_ms = (r.over_rev_timer_ms as u16).wrapping_add(env.elapsed_time_ms as u16) as i16;
            if REV_OVERREV_TIMEOUT < r.over_rev_timer_ms {
                // Revved too long at the top: the engine gives out.
                r.charge_state = charge::IDLE;
                r.lockout_flags |= lockout::ALL;
                effects.push(Effect::Sound { id: REV_OVERREV_SFX, echo: d.actions_flag_set & actions::ENGINE_ECHO != 0 });
            }
        } else {
            r.over_rev_timer_ms = 0;
        }
    } else {
        r.over_rev_timer_ms = 0;

        if r.charge_state == charge::ACTIVE {
            r.release_cooldown_timer_ms = REV_RELEASE_COOLDOWN;
            r.charge_state = charge::IDLE;
            if accel < r.fire_level {
                r.charge_state = charge::RELEASED_ABOVE_ACCEL;
            }
        }

        if r.charge_state != charge::IDLE && r.fire_level < accel {
            // Let go below the boost: the boost meter creeps up for the next rev.
            let target = mips::add_lo(accel, sacred);
            r.charge_state = charge::IDLE;
            r.boost_meter = interp_by_speed(r.boost_meter, (target - accel) / 3 + 3, target);
        }

        if 0 < r.fire_level {
            let mut decay_step = mips::sra(r.fire_level, 1);
            let max = if r.lockout_flags & lockout::REV_DECAY != 0 { REV_DECAY_STEP_MAX_LOCKED } else { REV_DECAY_STEP_MAX_NORMAL };
            if max < decay_step {
                decay_step = max;
            }
            if decay_step < REV_STEP_MIN {
                decay_step = REV_STEP_MIN;
            }
            r.fire_level = mips::sub_lo(r.fire_level, decay_step);
            if r.fire_level < 1 {
                r.empty_cooldown_timer_ms = REV_EMPTY_COOLDOWN;
                r.fire_level = 0;
            }
        } else {
            r.lockout_flags &= !lockout::REV_DECAY;
            r.boost_meter = accel + sacred / 3;
        }

        if d.fire_speed < 1 {
            r.lockout_flags &= !lockout::PEDAL_HELD;
        }
    }

    // (`revEngineState`, which the HUD reads, is not kept.)
    d.speedometer_needle_value = r.fire_level as i16;

    let turbo_room = if r.fire_level < accel {
        let low_end = mips::sll(d.stats.turbo_low_room_warning as i32, REV_TURBO_ROOM_SHIFT) + 1;
        map_to_range(r.fire_level, 0, accel, mips::sll(d.stats.turbo_max_room as i32, REV_TURBO_ROOM_SHIFT), low_end)
    } else {
        map_to_range(
            r.fire_level,
            accel,
            mips::add_lo(accel, sacred),
            mips::sll(d.stats.turbo_low_room_warning as i32, REV_TURBO_ROOM_SHIFT),
            1,
        )
    };
    d.turbo_meter_room_left = turbo_room as i16;

    let squish = mips::sra(d.speedometer_needle_value as i32, REV_SQUISH_SHIFT);
    d.jump_squish_stretch = squish as i16;
    if REV_SQUISH_LIMIT <= squish {
        d.jump_squish_stretch = REV_SQUISH_MAX;
    } else if squish < 0 {
        d.jump_squish_stretch = 0;
    }

    // The basic squash and stretch of animation, before the motion: lower, and wider.
    d.inst_scale_y = mips::sub_lo(REV_MODEL_BASE_SCALE, d.jump_squish_stretch as u16 as i32) as i16;
    let width = (d.jump_squish_stretch as i32 * 6 / 10 + REV_MODEL_BASE_SCALE) as i16;
    d.inst_scale_x = width;
    d.inst_scale_z = width;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kart() -> Driver {
        Driver::new(0, 0)
    }

    #[test]
    fn the_mask_runs_out_and_fades_away() {
        let mut d = kart();
        let env = Env::default();
        use_mask(&mut d);
        assert_eq!(d.mask.unwrap().duration, MASK_DURATION_NORMAL);
        d.mask.as_mut().unwrap().duration = 3 * 32;
        for _ in 0..3 {
            mask_tick(&mut d, &env);
        }
        assert_eq!(d.mask.unwrap().duration, 0);
        assert!(!d.mask.unwrap().fading);

        // The fade lasts 16 ticks, the one that starts it included, and the mask is gone on the next.
        for _ in 0..16 {
            mask_tick(&mut d, &env);
            assert!(d.mask.unwrap().fading);
        }
        mask_tick(&mut d, &env);
        assert_eq!(d.mask, None);
    }

    #[test]
    fn a_new_mask_replaces_a_fading_one() {
        let mut d = kart();
        use_mask(&mut d);
        d.mask.as_mut().unwrap().fading = true;
        d.num_wumpas = 10;
        use_mask(&mut d);
        let mask = d.mask.unwrap();
        assert!(!mask.fading);
        assert_eq!(mask.duration, MASK_DURATION_JUICED);
    }

    #[test]
    fn the_rev_fills_while_the_gas_is_held_and_falls_when_let_go() {
        let mut d = kart();
        let env = Env::default();
        d.rev.boost_meter = d.stats.accel_speed_class_stat as i32 + d.stats.sacred_fire_speed as i32 / 3;
        d.fire_speed = 1;
        for _ in 0..5 {
            rev_engine_animate(&mut d, &env, &mut Vec::new());
        }
        let held = d.rev.fire_level;
        assert!(held > 0);
        assert_eq!(d.rev.charge_state, charge::ACTIVE);
        assert_eq!(d.speedometer_needle_value as i32, held);

        d.fire_speed = 0;
        rev_engine_animate(&mut d, &env, &mut Vec::new());
        assert!(d.rev.fire_level < held);
        assert_eq!(d.rev.release_cooldown_timer_ms, REV_RELEASE_COOLDOWN);
    }

    #[test]
    fn revving_too_long_locks_the_pedal_with_a_sound() {
        let mut d = kart();
        let env = Env::default();
        d.rev.boost_meter = 0x100;
        d.fire_speed = 1;
        let mut effects = Vec::new();
        let mut ticks = 0;
        while d.rev.lockout_flags == 0 {
            rev_engine_animate(&mut d, &env, &mut effects);
            ticks += 1;
            assert!(ticks < 40, "the engine never gave out");
        }
        assert_eq!(d.rev.lockout_flags, lockout::ALL);
        assert!(effects.contains(&Effect::Sound { id: REV_OVERREV_SFX, echo: false }));
    }

    #[test]
    fn aku_aku_looks_after_the_good_guys() {
        let good: Vec<usize> = (0..16)
            .filter(|&c| {
                let mut d = Driver::new(0, c);
                use_mask(&mut d);
                d.mask.unwrap().good_guy
            })
            .collect();
        // Crash, Coco, Polar, Pura, Penta.
        assert_eq!(good, [0, 3, 6, 7, 13]);
    }

    #[test]
    fn the_mask_turns_round_the_kart_and_the_beam_cycles() {
        let mut d = kart();
        let env = Env::default();
        use_mask(&mut d);
        mask_tick(&mut d, &env);
        let first = d.mask.unwrap().shown.unwrap();
        // The first tick shows the mask at its start: in front of the kart, `sin(0) = 0`.
        assert_eq!(first.head.offset, [0, MASK_HEIGHT, 0x40]);
        assert_eq!(first.head.rot_y, Some(0));
        assert_eq!(first.head.scale, MASK_SCALE_NORMAL);

        for _ in 0..MASK_BEAM_FRAMES - 1 {
            mask_tick(&mut d, &env);
        }
        let mask = d.mask.unwrap();
        assert_eq!(mask.beam_frame, 0, "the beam's animation has come round");
        assert_eq!(mask.rot_y, (MASK_BEAM_FRAMES as i16).wrapping_mul(-MASK_TURN_STEP));
        // Bobbing: lower in the middle of the beam's animation.
        let mut low = i16::MAX;
        for _ in 0..MASK_BEAM_FRAMES {
            mask_tick(&mut d, &env);
            low = low.min(d.mask.unwrap().shown.unwrap().head.offset[1]);
        }
        assert_eq!(low, MASK_HEIGHT - 32);
    }

    #[test]
    fn the_fading_mask_shrinks_and_the_beam_clears() {
        let mut d = kart();
        let env = Env::default();
        use_mask(&mut d);
        d.mask.as_mut().unwrap().duration = 0;
        mask_tick(&mut d, &env);
        // The tick that starts the fade ends with the mask at its own size.
        assert_eq!(d.mask.unwrap().shown.unwrap().head.scale, MASK_SCALE_NORMAL);
        for tick in 1..16 {
            mask_tick(&mut d, &env);
            let shown = d.mask.unwrap().shown.unwrap();
            assert_eq!(shown.head.scale, MASK_SCALE_NORMAL - tick * MASK_FADE_SHRINK, "tick {tick}");
            assert_eq!(shown.head.rot_y, None);
            assert!(!shown.world_space);
        }
        assert_eq!(d.mask.unwrap().shown.unwrap().beam_alpha, MASK_ALPHA_GONE);
        mask_tick(&mut d, &env);
        assert_eq!(d.mask, None);
    }

    #[test]
    fn the_mask_item_lasts_longer_with_ten_wumpa() {
        let lasts = |wumpas: i8| {
            let mut d = kart();
            d.num_wumpas = wumpas;
            shoot_mask(&mut d, &mut Vec::new());
            d.mask.unwrap().duration
        };
        assert_eq!(lasts(0), 0x1e00);
        assert_eq!(lasts(9), 0x1e00);
        assert_eq!(lasts(10), 0x2d00);
    }

    #[test]
    fn the_mask_item_makes_the_kart_faster() {
        let mut d = kart();
        let before = crate::linear::base_speed(&d);
        shoot_mask(&mut d, &mut Vec::new());
        // The action comes with the mask's thread at the next tick's `phys_linear`.
        d.actions_flag_set |= actions::MASK_WEAPON;
        assert_eq!(crate::linear::base_speed(&d) - before, d.stats.mask_speed as i32);
    }
}
