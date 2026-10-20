//! Skid marks: `VehEmitter_Skidmarks` (the emitting half of
//! `VehEmitter_DriverMain`, after the physics) and the segment walk of
//! `VehGroundSkids_Main`. The driver keeps a ring of eight ticks of marks, one
//! per tire: a strip across the tire's width at where it touched the ground.
//! The renderer joins each tick's strip to the one before it.
//!
//! Not ported: the marks' draw order bytes (`depthBiasNormal`/`Secondary`; the
//! renderer's depth test stands in).

use crate::exhaust::rotate;
use crate::terrain::{TERRAINS, terrain_flags};
use crate::{Driver, MatrixArray, actions};

/// One tire's strip: the two ends across the tire, in game units.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Mark {
    pub edge: [[i16; 3]; 2],
    /// `VehEmitterSkidmark::flags`: bit 0 on terrain that forces skid marks.
    pub flags: u8,
}

/// `DriverSkidmarks`
pub mod tire {
    pub const BACK_LEFT: usize = 0;
    pub const BACK_RIGHT: usize = 1;
    pub const FRONT_LEFT: usize = 2;
    pub const FRONT_RIGHT: usize = 3;
}

const FRAME_COUNT: u8 = 8;
const CURRENT_FRAME_MASK: u32 = 0xf;
const HISTORY_MASK: u32 = 0xff_ffff;
const HISTORY_SHIFT: u32 = 4;

const LATERAL_SCALE: i32 = 15;
const WIDTH_SCALE: i32 = 10;
/// Below this speed the kart leaves no marks (`FP8_ONE * 2 + 1`).
const MIN_SPEED: i32 = 513;
const TIRE_LEFT_X: i32 = -0x1e;
const TIRE_RIGHT_X: i32 = 0x1e;
const TIRE_BACK_Z: i32 = -0x14;
const TIRE_FRONT_Z: i32 = 0x28;

/// `VehGroundSkids_Main`'s `FULL_INTENSITY_DEPTH` and its brightness.
const FULL_INTENSITY_DEPTH: i32 = 0x180;
const FULL_INTENSITY: i32 = 0x7f;
const MIN_INTENSITY: i32 = 0x10;

/// The skid marks part of `VehEmitter_DriverMain`: call it once per tick after the physics.
pub fn skidmarks(d: &mut Driver) {
    // The history moves on a tick, and the ring's current slot with it.
    d.skidmark_enable_flags = (d.skidmark_enable_flags << HISTORY_SHIFT) & HISTORY_MASK;
    d.skidmark_frame_index = d.skidmark_frame_index.wrapping_sub(1) & (FRAME_COUNT - 1);

    let terrain_flags = TERRAINS[d.terrain_meta1 as usize].flags;
    let forced = terrain_flags & terrain_flags::FORCE_SKIDMARKS != 0;
    if forced {
        d.actions_flag_set |= actions::BACK_SKID | actions::FRONT_SKID;
    }

    // The wheelie: rearing up the kart skids on its back tires, and a lifted nose on neither front tire.
    match d.matrix_array {
        MatrixArray::WheelieStart => {
            d.actions_flag_set |= actions::BACK_SKID;
            d.actions_flag_set &= !actions::FRONT_SKID;
        }
        MatrixArray::WheelieHold | MatrixArray::WheelieRecover => d.actions_flag_set &= !actions::FRONT_SKID,
        _ => {}
    }

    let skidding = d.actions_flag_set & (actions::BACK_SKID | actions::FRONT_SKID) != 0;
    if d.actions_flag_set & actions::TOUCH_GROUND == 0 || !skidding || (d.speed_approx as i32).abs() < MIN_SPEED {
        return;
    }

    let angle = d.axis_rotation_x as i32 as u32;
    let (sin, cos) = (math::sin(angle) * LATERAL_SCALE, math::cos(angle) * LATERAL_SCALE);
    // Across the tire, and the extension in front of the strip.
    let width = [(math::cos(angle) * WIDTH_SCALE) >> 12, (math::sin(angle) * -WIDTH_SCALE) >> 12];
    let lateral = [sin >> 12, cos >> 12];
    let flags = forced as u8;

    let (m, origin) = crate::baked::inst_matrix(d);
    let current = d.skidmark_frame_index as usize;
    let previous = (current + FRAME_COUNT as usize - 1) % FRAME_COUNT as usize;

    let put = |d: &mut Driver, slot: usize, t: usize, x: i32, y: i32, z: i32, w: [i32; 2]| {
        d.skidmarks[slot][t] = Mark {
            edge: [[(x + w[0]) as i16, y as i16, (z + w[1]) as i16], [(x - w[0]) as i16, y as i16, (z - w[1]) as i16]],
            flags,
        };
    };

    let wheels = [
        (actions::BACK_SKID, tire::BACK_LEFT, TIRE_LEFT_X, TIRE_BACK_Z),
        (actions::BACK_SKID, tire::BACK_RIGHT, TIRE_RIGHT_X, TIRE_BACK_Z),
        (actions::FRONT_SKID, tire::FRONT_LEFT, TIRE_LEFT_X, TIRE_FRONT_Z),
        (actions::FRONT_SKID, tire::FRONT_RIGHT, TIRE_RIGHT_X, TIRE_FRONT_Z),
    ];
    for (action, t, local_x, local_z) in wheels {
        if d.actions_flag_set & action == 0 {
            continue;
        }
        d.skidmark_enable_flags |= 1 << t;
        let at = rotate(&m, [local_x, 0, local_z]);
        let (pos_x, y, pos_z) = (origin[0] + at[0], origin[1] + at[1], origin[2] + at[2]);
        // The strip is centred half the extension behind the tire (the left rear
        // tire takes the unrounded half of it).
        let half_x = if t == tire::BACK_LEFT { sin >> 13 } else { lateral[0] >> 1 };
        let (x, z) = (pos_x - half_x, pos_z - (lateral[1] >> 1));
        put(d, current, t, x, y, z, width);
        // The slot the next tick takes carries the strip's far end, a step ahead and narrower.
        let half_width = [width[0] >> 1, width[1] >> 1];
        put(d, previous, t, x + lateral[0], y, z + lateral[1], half_width);
    }
}

/// A strip between two ticks' marks of one tire: `curr` is the newer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Segment {
    pub tire: usize,
    /// The two ends of the newer tick's strip, then the older one's.
    pub corners: [[i16; 3]; 4],
    /// How many ticks older the segment's newer end is: its brightness is halved that many times.
    pub age: u32,
    /// Terrain that forces marks draws them light instead of dark.
    pub light: bool,
}

/// The segments `VehGroundSkids_Main` draws for `d`, newest first. The
/// brightness of a segment is `intensity >> age` at its newer end and
/// `intensity >> (age - 1)` at the older one (the first segments of a
/// run have the same at both); it ends where that is 0.
pub fn segments(d: &Driver) -> Vec<Segment> {
    let mut out = Vec::new();
    let mut flags = d.skidmark_enable_flags;
    if flags <= CURRENT_FRAME_MASK {
        return out;
    }
    let ring = |slot: usize| (slot + FRAME_COUNT as usize) % FRAME_COUNT as usize;
    // The walk starts in the slot ahead of the current one and goes back in time;
    // it only has something to join from its second step on.
    let current = d.skidmark_frame_index as usize;
    let mut slot = ring(current + FRAME_COUNT as usize - 1);
    let mut previous_flags = 0;
    for step in 0..FRAME_COUNT as u32 {
        if flags == 0 {
            break;
        }
        // The extension slot ahead and the slot in front of it are the same strip's two ends.
        let older = ring(slot + FRAME_COUNT as usize - 1);
        if step >= 1 {
            for t in 0..4 {
                if flags & previous_flags & (1 << t) == 0 {
                    continue;
                }
                let (a, b) = (&d.skidmarks[slot][t], &d.skidmarks[older][t]);
                out.push(Segment {
                    tire: t,
                    corners: [a.edge[0], a.edge[1], b.edge[0], b.edge[1]],
                    age: step - 1,
                    light: a.flags & 1 != 0,
                });
            }
        }
        slot = ring(slot + 1);
        if step == 0 {
            previous_flags = CURRENT_FRAME_MASK;
        } else {
            previous_flags = flags;
            flags >>= HISTORY_SHIFT;
        }
    }
    out
}

/// A point on the newest marks, whose distance from the camera sets how
/// bright all of the driver's marks are: the first tire's, in the slot ahead.
pub fn fade_anchor(d: &Driver) -> [i16; 3] {
    let slot = (d.skidmark_frame_index as usize + FRAME_COUNT as usize - 1) % FRAME_COUNT as usize;
    d.skidmarks[slot][0].edge[0]
}

/// How bright the newest marks are (0..=0x7f) at `depth` game units in
/// front of the camera, or none once they are too faint to draw.
pub fn intensity(depth: i32) -> Option<i32> {
    if depth < FULL_INTENSITY_DEPTH {
        return Some(FULL_INTENSITY);
    }
    // One step down for every doubling of the distance past the full-intensity one.
    let bits = 32 - ((depth - FULL_INTENSITY_DEPTH) as u32).leading_zeros() as i32;
    let value = FULL_INTENSITY >> (bits - 6).max(0);
    (value >= MIN_INTENSITY).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Matrix, terrain::terrain_type};

    fn skidding_driver() -> Driver {
        let mut d = Driver::new(0, 0);
        d.matrix_facing_dir = Matrix { m: [[0x1000, 0, 0], [0, 0x1000, 0], [0, 0, 0x1000]] };
        d.actions_flag_set = actions::TOUCH_GROUND | actions::BACK_SKID;
        d.speed_approx = 0x1000;
        d.terrain_meta1 = terrain_type::ASPHALT;
        d.pos_curr = math::Vec3 { x: 100 << 8, y: 50 << 8, z: 200 << 8 };
        d
    }

    #[test]
    fn the_back_tires_leave_a_strip_and_the_front_ones_do_not() {
        let mut d = skidding_driver();
        skidmarks(&mut d);
        assert_eq!(d.skidmark_enable_flags, 0b0011);
        assert_eq!(d.skidmark_frame_index, 7);
        // Facing along +Z, the back tires are behind (-20) and 30 to either side.
        let left = d.skidmarks[7][tire::BACK_LEFT];
        assert_eq!(left.edge[0][1], 50);
        assert!((left.edge[0][2] as i32 - 180).abs() <= 16, "{left:?}");
        assert_eq!(d.skidmarks[7][tire::FRONT_LEFT], Mark::default());
    }

    #[test]
    fn the_marks_need_speed_and_the_ground() {
        let mut d = skidding_driver();
        d.speed_approx = 512;
        skidmarks(&mut d);
        assert_eq!(d.skidmark_enable_flags, 0);
        let mut d = skidding_driver();
        d.actions_flag_set &= !actions::TOUCH_GROUND;
        skidmarks(&mut d);
        assert_eq!(d.skidmark_enable_flags, 0);
    }

    #[test]
    fn a_run_of_ticks_gives_segments_that_age() {
        let mut d = skidding_driver();
        for _ in 0..3 {
            skidmarks(&mut d);
        }
        // Back left and back right, joining the current tick to the extension and two older ones.
        let segments = segments(&d);
        let ages: Vec<_> = segments.iter().filter(|s| s.tire == tire::BACK_LEFT).map(|s| s.age).collect();
        assert_eq!(ages, [0, 1, 2]);
        // Nothing is drawn for a single tick: only the current nibble is set.
        let mut d = skidding_driver();
        skidmarks(&mut d);
        assert!(segments_of(&d).is_empty());
    }

    fn segments_of(d: &Driver) -> Vec<Segment> {
        segments(d)
    }

    #[test]
    fn terrain_that_forces_marks_gives_light_ones_on_all_tires() {
        let mut d = skidding_driver();
        d.terrain_meta1 = terrain_type::ICE;
        skidmarks(&mut d);
        skidmarks(&mut d);
        let forced = TERRAINS[terrain_type::ICE as usize].flags & terrain_flags::FORCE_SKIDMARKS != 0;
        assert_eq!(d.skidmark_enable_flags & 0xf == 0xf, forced);
        assert_eq!(segments(&d).iter().all(|s| s.light), forced);
    }

    #[test]
    fn marks_fade_with_distance() {
        assert_eq!(intensity(100), Some(0x7f));
        assert_eq!(intensity(0x180 + 8), Some(0x7f));
        assert_eq!(intensity(0x180 + 40), Some(0x7f >> 0));
        assert_eq!(intensity(0x180 + 100), Some(0x7f >> 1));
        assert_eq!(intensity(0x180 + 255), Some(0x7f >> 2));
        assert_eq!(intensity(0x180 + 256), None);
    }
}
