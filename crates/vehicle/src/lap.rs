//! Laps: how far along the track a kart is (`VehLap_UpdateProgress`) and what crossing the
//! finish line does (the per-driver part of `PlayLevel_UpdateLapStats`).
//!
//! Not ported: the bots (their progress comes from their path), the lap times for the HUD, the
//! final-lap sound and text, the confetti and the end-of-race menu, and dropping the held item
//! of a driver that finished.

use loader::lev::Level;
use math::{SVec3, mips};

use crate::{Driver, actions};

/// `VEH_LAP_INVALID_CHECKPOINT`
const NO_CHECKPOINT: u8 = 0xff;
const WORLD_POS_SHIFT: u32 = 8;
const MOVING_DIR_SHIFT: u32 = 5;
const TRACK_DISTANCE_SCALE_SHIFT: u32 = 3;
const PROJECTED_DISTANCE_SHIFT: u32 = 0xc;
const WRONG_WAY_DOT_LIMIT: i32 = 0x5a801;

/// `PLAYLEVEL_FINISHLINE_NEAR_DISTANCE` and `_FAR_DISTANCE`: a step from the near side of the
/// track's distance to the far side (or back) is the finish line, not driving.
const FINISH_LINE_NEAR: i32 = 1200;
const FINISH_LINE_FAR: i32 = 32000;
const REVERSE_DISTANCE_MAX: i32 = 1000;
const REVERSE_CROSSING_PENALTY: u32 = 600;

/// What a lap update found out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Lap {
    /// The kart crossed the line and started the lap `lap_index` (0 based).
    pub new_lap: bool,
    /// How long the lap it finished took, in milliseconds (with `new_lap`).
    pub time: i32,
    /// That lap is the last one.
    pub final_lap: bool,
    /// The kart has just finished the race.
    pub finished: bool,
}

/// `VehLap_UpdateProgress` for a human driver: `distance_to_finish_curr` is the distance left
/// to the finish line (in the same units as the checkpoints', 1/8 of the unit the nodes have),
/// wrapped by the length of the track; and whether the kart drives against the track.
pub fn update_progress(d: &mut Driver, level: &Level) {
    let Some(quad) = d.last_valid else {
        return;
    };
    let checkpoint_index = level.quad_blocks[quad as usize].checkpoint_index;
    let points = &level.restart_points;
    if checkpoint_index == NO_CHECKPOINT
        || (points.len() as u32).wrapping_sub(1) >= NO_CHECKPOINT as u32
    {
        return;
    }

    let checkpoint = &points[checkpoint_index as usize];
    let progress_node = &points[checkpoint.next_forward as usize];
    let next_node = &points[progress_node.next_forward as usize];

    let position = [
        mips::sra(d.pos_curr.x, WORLD_POS_SHIFT) as i16,
        mips::sra(d.pos_curr.y, WORLD_POS_SHIFT) as i16,
        mips::sra(d.pos_curr.z, WORLD_POS_SHIFT) as i16,
    ];
    let mut direction = SVec3 {
        x: progress_node.pos.x.wrapping_sub(next_node.pos.x),
        y: progress_node.pos.y.wrapping_sub(next_node.pos.y),
        z: progress_node.pos.z.wrapping_sub(next_node.pos.z),
    };
    math::vector_normalize(&mut direction);
    let delta = [
        position[0].wrapping_sub(progress_node.pos.x),
        position[1].wrapping_sub(progress_node.pos.y),
        position[2].wrapping_sub(progress_node.pos.z),
    ];

    // The GTE did these two dot products: the first row of its matrix was the offset from the
    // node, the second the kart's heading (the z column of the moving direction, scaled down).
    let dot = |a: [i32; 3]| -> i32 {
        (a[0] as i64 * direction.x as i64
            + a[1] as i64 * direction.y as i64
            + a[2] as i64 * direction.z as i64) as i32
    };
    let m = &d.matrix_moving_dir.m;
    let projection = dot(delta.map(i32::from));
    let wrong_way =
        dot([0, 1, 2].map(|row| mips::sra(m[row][2] as i32, MOVING_DIR_SHIFT) as i16 as i32));

    let progress = mips::add_lo(
        ((progress_node.dist_to_finish as u32) << TRACK_DISTANCE_SCALE_SHIFT) as i32,
        mips::sra(projection, PROJECTED_DISTANCE_SHIFT),
    );
    let track_length = (points[0].dist_to_finish as i32) << TRACK_DISTANCE_SCALE_SHIFT;
    d.distance_to_finish_curr = if track_length == 0 {
        progress
    } else {
        progress.wrapping_rem(track_length)
    } as u32;

    if wrong_way >= WRONG_WAY_DOT_LIMIT {
        d.actions_flag_set |= actions::DRIVING_WRONG_WAY;
    } else {
        d.actions_flag_set &= !actions::DRIVING_WRONG_WAY;
    }

    if d.actions_flag_set & actions::CHECKPOINT_BRANCH_PENDING != 0
        && d.checkpoint_current_index != checkpoint_index
    {
        d.checkpoint_branch_choice_index = checkpoint_index;
        d.actions_flag_set &= !actions::CHECKPOINT_BRANCH_PENDING;
    }
    if checkpoint.next_left != NO_CHECKPOINT {
        d.actions_flag_set |= actions::CHECKPOINT_BRANCH_PENDING;
    }
    d.checkpoint_current_index = checkpoint_index;
}

/// The per-driver part of `PlayLevel_UpdateLapStats`: the progress, the distance driven
/// backwards, and the finish line. `laps` is the length of the race, and `event_time` the
/// milliseconds since it started.
pub fn update(d: &mut Driver, level: &Level, laps: u8, event_time: i32) -> Lap {
    let mut lap = Lap::default();
    let before = d.distance_to_finish_curr as i32;
    update_progress(d, level);
    let after = d.distance_to_finish_curr as i32;

    let backwards = (d.distance_driven_backwards as i32)
        .wrapping_add(after.wrapping_sub(before))
        .clamp(0, REVERSE_DISTANCE_MAX);
    d.distance_driven_backwards = backwards as u32;

    let Some(start) = level.restart_points.first() else {
        return lap;
    };
    if before < FINISH_LINE_NEAR && after > FINISH_LINE_FAR {
        // Over the line, forwards.
        d.distance_driven_backwards = 0;
        if d.actions_flag_set & actions::BEHIND_START_LINE != 0 {
            // The start line itself: the lap has not begun.
            d.actions_flag_set &= !actions::BEHIND_START_LINE;
        } else {
            d.distance_to_finish_checkpoint = after as u32;
            // A kart that finished does not count more laps.
            if (laps as i32) >= d.lap_index as i32 + 1 {
                lap.time = event_time.wrapping_sub(d.lap_time);
                d.lap_time = event_time;
                d.lap_index += 1;
                lap.new_lap = true;
                lap.final_lap = d.lap_index as i32 == laps as i32 - 1;
            }
            if d.lap_index == laps && d.actions_flag_set & actions::RACE_FINISHED == 0 {
                d.actions_flag_set |= actions::RACE_FINISHED;
                lap.finished = true;
            }
        }
    } else if after < FINISH_LINE_NEAR && before > FINISH_LINE_FAR {
        // Backwards over the line.
        d.distance_driven_backwards = REVERSE_CROSSING_PENALTY;
        d.actions_flag_set |= actions::BEHIND_START_LINE;
    } else {
        let track_length = start.dist_to_finish as u32;
        if d.actions_flag_set & actions::BEHIND_START_LINE == 0
            && d.distance_to_finish_checkpoint.wrapping_sub(after as u32)
                <= (track_length >> 2) << 3
        {
            d.distance_to_finish_checkpoint = after as u32;
        }
    }
    lap
}
