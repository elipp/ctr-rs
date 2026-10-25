//! `VehStuckProc_Tumble_*`: the kart that a bomb or a missile blasted tumbles through the air
//! until its no-input timer runs out, and then drives again. (`VehPickState_NewState` starts it;
//! see [`crate::pick`].)
//!
//! The blasted animation's table is not ported (see [`crate::baked`]), so only the index into it
//! is kept.

use gte::Gte;
use math::mips;

use crate::calc::interp_by_speed;
use crate::force::rot_axis_angle;
use crate::frame::{num_anim_frames, start_frame};
use crate::sim::driving_init;
use crate::spin::lose_wumpa;
use crate::{Driver, Effect, Env, MatrixArray, PhysState, actions, kart_state};

const FORCED_JUMP_MS: i16 = 0x60;
const JUMP_VELOCITY_BASE: i32 = 6000;
const STEERING_FRAME_SENTINEL: i16 = 10000;
const ANGULAR_DAMP_SHIFT: u32 = 3;
const TURN_WRAP_BIAS: i32 = 0x800;
const TURN_MASK: i32 = 0xfff;
const SPIN_RATE_SHIFT: u32 = 0xd;
const WUMPA_PENALTY: i32 = 3;
const BACKWARDS_RNG_MASK: i32 = 4;
const RUMBLE_WEAK: u8 = 0x19;
const RUMBLE_STRONG: u8 = 0x29;
const RUMBLE_DURATION: u8 = 0x60;

/// `data.bakedGteMath[BAKED_GTE_MATRIX_BLASTED].numEntries` (`matArr06[0x1B]`).
const BLASTED_ENTRIES: i32 = 0x1b;

/// `KartStates.Blasted.boolPlayBackwards`, the first byte of the union.
fn set_play_backwards(d: &mut Driver, value: u8) {
    d.num_frames_drifting = (d.num_frames_drifting & !0xff) | value as i16;
}

fn play_backwards(d: &Driver) -> bool {
    d.num_frames_drifting & 0xff != 0
}

/// The draw of `VehStuckProc_Tumble_Init` (`MixRNG_Scramble`), made where the init stage of the
/// tick begins so that the shared random number advances in the order of the game. A kart with a
/// tumble to start plays the animation backwards on a 4 in the number.
pub fn roll_direction(d: &mut Driver, random_number: &mut i32) {
    if d.pending_tumble {
        let rng = math::mix_rng_scramble(random_number);
        set_play_backwards(d, (rng & BACKWARDS_RNG_MASK) as u8);
    }
}

/// `VehStuckProc_Tumble_Init`
pub fn init(d: &mut Driver, effects: &mut Vec<Effect>) {
    d.kart_state = kart_state::BLASTED;
    d.turbo_meter_room_left = 0;
    lose_wumpa(d, WUMPA_PENALTY);

    d.anim_index = 0;
    d.anim_frame = start_frame(0, num_anim_frames(d, 0)) as i16;

    d.phys_state = PhysState::Tumble;
    d.init_pending = false;

    effects.push(Effect::JogCon {
        driver_id: d.driver_id,
        pattern: if d.simp_turn_state > 0 {
            RUMBLE_STRONG
        } else {
            RUMBLE_WEAK
        },
        strength: RUMBLE_DURATION,
    });
}

/// `VehStuckProc_Tumble_Update`: when the timer is out the kart drives again.
pub fn update(d: &mut Driver) {
    if d.no_input_timer != 0 {
        return;
    }
    d.matrix_array = MatrixArray::None;
    d.matrix_index = 0;
    driving_init(d);
}

/// The start of `VehStuckProc_Tumble_PhysLinear`, before the driving one.
pub fn before_phys_linear(d: &mut Driver, env: &Env) {
    d.no_input_timer = mips::sub_lo(
        d.no_input_timer as u16 as i32,
        env.elapsed_time_ms as u16 as i32,
    ) as i16;
    if d.no_input_timer < 0 {
        d.no_input_timer = 0;
    }
}

/// The rest of it, after the driving one.
pub fn after_phys_linear(d: &mut Driver) {
    d.jump_forced_ms = FORCED_JUMP_MS;
    d.base_speed = 0;
    d.fire_speed = 0;
    d.actions_flag_set |=
        actions::WARP | actions::FRONT_SKID | actions::BACK_SKID | actions::ACCEL_PREVENTION;
    d.jump_initial_vel_y = mips::add_lo(
        mips::sll(d.no_input_timer as u16 as i32, 1),
        JUMP_VELOCITY_BASE,
    ) as i16;
}

/// `VehStuckProc_Tumble_PhysAngular`
pub fn phys_angular(d: &mut Driver, gte: &mut Gte, env: &Env) {
    let elapsed = env.elapsed_time_ms;

    d.num_frames_spent_steering = STEERING_FRAME_SENTINEL;

    d.rotation_spin_rate = d
        .rotation_spin_rate
        .wrapping_sub(d.rotation_spin_rate >> ANGULAR_DAMP_SHIFT);

    d.turn_angle_lerp_vel = d
        .turn_angle_lerp_vel
        .wrapping_sub(d.turn_angle_lerp_vel >> ANGULAR_DAMP_SHIFT);
    d.amp_turn_state = d.rotation_spin_rate;

    let turn = d.turn_angle_curr as u16 as i32 + d.turn_angle_lerp_vel as i32 + TURN_WRAP_BIAS;
    d.turn_angle_curr = ((turn & TURN_MASK) - TURN_WRAP_BIAS) as i16;

    d.turn_wobble_angle = d
        .turn_wobble_angle
        .wrapping_sub(d.turn_wobble_angle >> ANGULAR_DAMP_SHIFT);

    let step = mips::sra(
        mips::mul_lo(d.rotation_spin_rate as i32, elapsed),
        SPIN_RATE_SHIFT,
    ) as i16;
    d.angle = (d.angle.wrapping_add(step) as i32 & TURN_MASK) as i16;

    d.rot_curr_y = d
        .angle
        .wrapping_add(d.turn_angle_curr)
        .wrapping_add(d.turn_wobble_angle);
    d.rot_curr_w =
        interp_by_speed(d.rot_curr_w as i32, mips::sra(mips::sll(elapsed, 5), 5), 0) as i16;

    let mut moving = d.matrix_moving_dir;
    rot_axis_angle(&mut moving, gte, d.axis_angle1_normal_vec, d.angle as i32);
    d.matrix_moving_dir = moving;
}

/// `VehStuckProc_Tumble_Animate`: the frame of the blasted animation, a step every 32 ms.
pub fn animate(d: &mut Driver) {
    let mut index = mips::sra(d.no_input_timer as i32, 5);
    d.matrix_array = MatrixArray::Blasted;
    index %= BLASTED_ENTRIES;
    if play_backwards(d) {
        index = BLASTED_ENTRIES - (index + 1);
    }
    d.matrix_index = index as _;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blasted() -> Driver {
        let mut d = Driver::default();
        d.pending_tumble = true;
        d.no_input_timer = 0x960;
        d
    }

    #[test]
    fn the_tumble_starts_blasted_and_costs_three_wumpa() {
        let mut d = blasted();
        d.num_wumpas = 5;
        let mut effects = Vec::new();
        init(&mut d, &mut effects);
        assert!(d.kart_state == kart_state::BLASTED && d.phys_state == PhysState::Tumble);
        assert_eq!(d.num_wumpas, 2);
        assert!(matches!(
            effects[0],
            Effect::JogCon {
                pattern: RUMBLE_WEAK,
                ..
            }
        ));
    }

    #[test]
    fn the_direction_is_drawn_once_for_a_pending_tumble_only() {
        let mut rng = 1;
        let mut d = Driver::default();
        roll_direction(&mut d, &mut rng);
        assert_eq!(rng, 1);
        let mut d = blasted();
        roll_direction(&mut d, &mut rng);
        assert_ne!(rng, 1);
        assert_eq!(
            play_backwards(&d),
            (math::mix_rng_scramble(&mut 1) & 4) != 0
        );
    }

    #[test]
    fn the_kart_drives_again_when_the_timer_is_out() {
        let mut d = blasted();
        d.phys_state = PhysState::Tumble;
        let env = Env::default();
        for _ in 0..0x960 / env.elapsed_time_ms + 1 {
            before_phys_linear(&mut d, &env);
            update(&mut d);
        }
        assert_eq!(d.no_input_timer, 0);
        assert_eq!(d.phys_state, PhysState::Driving);
    }

    #[test]
    fn the_animation_wraps_and_can_run_backwards() {
        let mut d = Driver::default();
        d.no_input_timer = 0x960;
        animate(&mut d);
        assert_eq!(d.matrix_index as i32, (0x960 >> 5) % BLASTED_ENTRIES);
        set_play_backwards(&mut d, 4);
        animate(&mut d);
        assert_eq!(
            d.matrix_index as i32,
            BLASTED_ENTRIES - ((0x960 >> 5) % BLASTED_ENTRIES + 1)
        );
    }
}
