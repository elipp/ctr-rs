//! `game/Vehicle/VehFrame.c`: the kart model's animation. The steering
//! animation plays the frame that matches the steering, and the spin-out
//! animations play the model through its frames. Not presentation only: the
//! spin-out ends when its animation has reached the end (see
//! [`crate::spin`]).
//!
//! Left out: the TNT crate on the head (`instTntRecv` is never set, since the
//! driver holds no items) and the burn smoke particles.

use math::mips;

use crate::calc::{interp_by_speed, map_to_range};
use crate::{Driver, MatrixArray, kart_state};

const ANIM_DRIVE: u8 = 0;
const ANIM_STATIONARY: u8 = 1;
const ANIM_MATRIX_FIRST: u8 = 2;
const ANIM_AIRBORNE: u8 = 3;
const ANIM_END_START: u8 = 4;
const MATRIX_ANIM_COUNT: u32 = 2;
const NUM_FRAMES_MASK: u16 = 0x7fff;
const JUMP_AIR_THRESHOLD: i32 = 0x600;
const AIR_GAP_THRESHOLD: i32 = 0x8000;
const BURN_TIMER_LIMIT: i32 = 0x1e0;
const BURN_TIMER_SHIFT: u32 = 5;
const BURN_FRAME_PERIOD: i32 = 5;
const BURN_FRAME_SHIFT: u32 = 2;
const BURN_FRAME_BIAS: i32 = 8;
const TURN_ACCEL_PREVENTION_LIMIT: i32 = 0x40;
const TRANSITION_DEFAULT_SPEED: i32 = 2;
const TRANSITION_DRIVE_SPEED: i32 = 6;
const TRANSITION_MATRIX_SPEED: i32 = 1;
const INTERP_SPEED_NORMAL: i32 = 1;
const SPIN_INTERP_SPEED: i32 = 4;
const LAST_SPIN_INTERP_SPEED: i32 = 3;
const AIRBORNE_MATRIX_BASE: u8 = 7;

/// `CharacterID` values the airborne pose maps to others.
const COCO_BANDICOOT: u8 = 3;
const CRASH_BANDICOOT: u8 = 0;
const PENTA_PENGUIN: u8 = 13;
const FAKE_CRASH: u8 = 14;
const NITROS_OXIDE: u8 = 15;

/// `VehFrameInst_GetStartFrame`
pub fn start_frame(anim_index: i32, num_frames: i32) -> i32 {
    match anim_index {
        i if i == ANIM_DRIVE as i32 => num_frames >> 1, // midpoint
        i if i == ANIM_END_START as i32 => num_frames - 1, // end
        _ => 0,
    }
}

/// `VehFrameInst_GetNumAnimFrames`
pub fn num_anim_frames(d: &Driver, anim_index: u8) -> i32 {
    (d.anim_num_frames.get(anim_index as usize).copied().unwrap_or(0) & NUM_FRAMES_MASK) as i32
}

/// `VehFrameProc_Driving`
pub fn driving(d: &mut Driver) {
    let mut desired = ANIM_DRIVE;

    if d.kart_state != kart_state::WARP_PAD {
        if d.fire_speed < 0 {
            desired = if d.speed_approx < 1 { ANIM_STATIONARY } else { ANIM_DRIVE };
        }

        if (d.jump_height_curr as i32 > JUMP_AIR_THRESHOLD || d.anim_index == ANIM_AIRBORNE)
            && mips::sub_lo(d.pos_curr.y, d.quad_block_height) > AIR_GAP_THRESHOLD
        {
            desired = ANIM_AIRBORNE;
        }
    }

    let mut num_frames = num_anim_frames(d, d.anim_index);
    if num_frames <= 0 {
        return;
    }

    let current = d.anim_index;
    if desired != current {
        // Play the current animation to its resting frame, then switch.
        num_frames = if current == ANIM_MATRIX_FIRST {
            num_anim_frames(d, current) - 1
        } else {
            start_frame(current as i32, num_frames)
        };

        if d.anim_frame as i32 == num_frames {
            num_frames = num_anim_frames(d, desired);
            if num_frames <= 0 {
                return;
            }
            d.anim_index = desired;
            d.anim_frame = start_frame(desired as i32, num_frames) as i16;
            d.matrix_array = MatrixArray::None;
            d.matrix_index = 0;
            // ... and plays the new animation.
        } else {
            let speed = match d.anim_index {
                ANIM_DRIVE => TRANSITION_DRIVE_SPEED,
                ANIM_MATRIX_FIRST => {
                    d.matrix_index = d.anim_frame as u8;
                    TRANSITION_MATRIX_SPEED
                }
                _ => TRANSITION_DEFAULT_SPEED,
            };
            d.anim_frame = interp_by_speed(d.anim_frame as i32, speed, num_frames) as i16;

            if (d.anim_index as u32).wrapping_sub(ANIM_MATRIX_FIRST as u32) < MATRIX_ANIM_COUNT {
                d.matrix_index = d.anim_frame as u8;
                if d.matrix_index == 0 {
                    d.matrix_array = MatrixArray::None;
                    d.matrix_index = 0;
                }
            }
            return;
        }
    }

    match desired {
        ANIM_DRIVE => {
            let burn_timer = d.burn_timer as i32;
            let target;
            if burn_timer != 0 && burn_timer < BURN_TIMER_LIMIT {
                target = mips::add_lo(
                    mips::sub_lo(mips::sll((burn_timer >> BURN_TIMER_SHIFT) % BURN_FRAME_PERIOD, BURN_FRAME_SHIFT), BURN_FRAME_BIAS),
                    mips::sra(num_frames, 1),
                );
                d.anim_frame = target as i16;
            } else if d.actions_flag_set & crate::actions::ACCEL_PREVENTION != 0 {
                target = map_to_range(
                    -(d.amp_turn_state as i32),
                    -TURN_ACCEL_PREVENTION_LIMIT,
                    TURN_ACCEL_PREVENTION_LIMIT,
                    0,
                    num_frames - 1,
                );
            } else {
                let turn_input = d.simp_turn_state as i32;
                let turn_max = d.stats.turn_rate as i32;
                target = map_to_range(-turn_input, -turn_max, turn_max, 0, num_frames - 1);
            }
            d.anim_frame = interp_by_speed(d.anim_frame as i32, INTERP_SPEED_NORMAL, target) as i16;
        }
        ANIM_AIRBORNE => {
            d.anim_frame = interp_by_speed(d.anim_frame as i32, INTERP_SPEED_NORMAL, num_frames - 1) as i16;

            if d.kart_state == kart_state::MASK_GRABBED {
                return;
            }

            let mut character = d.character;
            if character == PENTA_PENGUIN {
                character = COCO_BANDICOOT;
            }
            if character == FAKE_CRASH {
                character = CRASH_BANDICOOT;
            }
            // Oxide's pose is the first of the jump matrices.
            let index = if character == NITROS_OXIDE { 0 } else { character };
            d.matrix_array = MatrixArray::Jump(index + AIRBORNE_MATRIX_BASE);
            d.matrix_index = d.anim_frame as u8;
        }
        _ => {
            d.anim_frame = interp_by_speed(d.anim_frame as i32, INTERP_SPEED_NORMAL, num_frames - 1) as i16;
        }
    }
}

/// `VehFrameProc_Spinning`
pub fn spinning(d: &mut Driver) {
    let mut num_frames = num_anim_frames(d, d.anim_index);
    if num_frames <= 0 {
        return;
    }

    if d.anim_index != ANIM_DRIVE {
        let target = start_frame(d.anim_index as i32, num_frames);

        if (d.anim_index as u32).wrapping_sub(ANIM_MATRIX_FIRST as u32) < MATRIX_ANIM_COUNT {
            d.matrix_array = MatrixArray::None;
            d.matrix_index = 0;
        }

        if d.anim_frame as i32 == target {
            num_frames = num_anim_frames(d, ANIM_DRIVE);
            if num_frames <= 0 {
                return;
            }
            d.anim_index = ANIM_DRIVE;
        }

        if d.anim_index != ANIM_DRIVE {
            d.anim_frame = interp_by_speed(d.anim_frame as i32, SPIN_INTERP_SPEED, target) as i16;
            return;
        }
    }

    let target = if d.spin_dir() >= 0 { num_frames - 1 } else { 0 };
    d.anim_frame = interp_by_speed(d.anim_frame as i32, SPIN_INTERP_SPEED, target) as i16;
}

/// `VehFrameProc_LastSpin`
pub fn last_spin(d: &mut Driver) {
    if d.anim_index != ANIM_DRIVE {
        spinning(d);
        return;
    }

    let num_frames = num_anim_frames(d, ANIM_DRIVE);
    if num_frames <= 0 {
        return;
    }

    let mut target = d.anim_frame as i32;
    if d.turn_angle_curr > 0 && d.turn_angle_lerp_vel < 0 {
        target = num_frames - 1;
    }
    if d.turn_angle_curr < 0 && d.turn_angle_lerp_vel > 0 {
        target = 0;
    }
    d.anim_frame = interp_by_speed(d.anim_frame as i32, LAST_SPIN_INTERP_SPEED, target) as i16;
}
