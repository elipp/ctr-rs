//! The start-line fly-in: for the first 165 ticks of a race the camera flies along the path the
//! level holds (`CAM_StartLine_FlyIn`), in a frame set by the start of the grid, and over the
//! last 30 it blends into the chase camera (`CAM_ProcessTransition`).
//!
//! The fly-in is a function of the tick alone, and the chase camera runs underneath it all the
//! while, so the camera is the same for a race that is played again from a snapshot.

use gte::Gte;
use loader::lev::{FlyIn, Level, SVec3, quad_flags};
use math::mips;

use crate::{CameraDc, convert_rot_to_matrix};

/// `transitionFrame` at the start of the fly-in: it counts down one a tick, and at 0 the fly-in
/// is over.
pub const TRANSITION_FRAMES: i32 = 0xa5;
/// The frames of the fly-in path that are played (`FlyInData::frameCount1`), and where the
/// ratio of the path is taken from.
const PATH_FRAMES: i32 = 0x96;
/// How long the blend into the chase camera takes (`transitionFrameCount`).
const BLEND_FRAMES: i32 = 0x1e;

/// The grid spawns whose midpoint is the origin of the fly-in's frame.
const ORIGIN_SPAWNS: [usize; 2] = [1, 2];
/// Lifts the target of the camera by this much (`local_70.vy`), and the origin by `ORIGIN_LIFT`.
const TARGET_DROP: i32 = 0x60;
const ORIGIN_LIFT: i32 = 0x40;

impl CameraDc {
    /// `CAM_StartLine_FlyIn_FixY`: the height of the track under `pos`, which the spawn points
    /// may be above.
    fn track_height(&mut self, level: &Level, pos: SVec3) -> i16 {
        {
            let q = &mut self.coll.sps.quad_block_coll;
            q.quad_flags_wanted = (quad_flags::GROUND | quad_flags::COLLISION_SURFACE) as u32;
            q.quad_flags_ignored = 0;
            q.search_flags = coll::search_flags::HIGH_LOD;
        }
        for i in 0..8 {
            let offset = i * 0x400;
            let top = SVec3 {
                x: pos.x,
                y: (pos.y as i32 - (offset + 0x400)) as i16,
                z: pos.z,
            };
            let bottom = SVec3 {
                x: pos.x,
                y: (pos.y as i32 - (offset - 0x100)) as i16,
                z: pos.z,
            };
            self.coll.sps.did_touch_quadblock = 0;
            self.coll.search_quadblocks(level, top, bottom, 0);
            if self.coll.sps.did_touch_quadblock != 0 {
                return self.coll.sps.quad_block_coll.hit_pos.y;
            }
        }
        pos.y
    }

    /// `CAM_StartLine_FlyIn`: where the camera is and which way it looks at `frame` (0 to 150)
    /// of the fly-in.
    fn fly_in_view(&mut self, level: &Level, fly_in: &FlyIn, frame: i32) -> (SVec3, SVec3) {
        let frame_ratio = (frame << 12) / PATH_FRAMES;
        let eye_points = FlyIn::EYE_POINTS as i32;
        let target_points = FlyIn::TARGET_POINTS as i32;
        let count = target_points.max(eye_points);
        let index = (count * frame_ratio >> 12) as i16 as i32;
        let (eye_index, eye_ratio) = if index < eye_points - 1 {
            (index, frame_ratio)
        } else {
            (eye_points - 2, 0)
        };
        let (target_index, target_ratio) = if index < target_points - 1 {
            (index, frame_ratio)
        } else {
            (target_points - 2, 0)
        };
        let along = |path: &[SVec3], at: i32, ratio: i32, drop: i32| -> [i32; 3] {
            let (a, b) = (path[at as usize], path[at as usize + 1]);
            let ratio = count * ratio & 0xfff;
            let lerp = |a: i16, b: i16| {
                (a as i32 + ((b as i32 - a as i32) * ratio >> 12) as i16 as i32) as i16 as i32
            };
            [lerp(a.x, b.x), lerp(a.y, b.y) - drop, lerp(a.z, b.z)]
        };
        let eye = along(&fly_in.eye, eye_index, eye_ratio, 0);
        let target = along(&fly_in.target, target_index, target_ratio, TARGET_DROP);

        // The frame of the grid: turned like the first place, at the middle of the two next to it.
        let spawns = &level.driver_spawns;
        let rot = SVec3 {
            x: spawns[0].rot.x,
            y: spawns[0].rot.y.wrapping_add(0x400),
            z: spawns[0].rot.z,
        };
        let matrix = convert_rot_to_matrix(&mut Gte::new(), rot);
        let [a, b] = ORIGIN_SPAWNS.map(|i| {
            let mut pos = spawns[i].pos;
            pos.y = self.track_height(level, pos);
            pos
        });
        let origin = [
            a.x as i32 + (b.x as i32 - a.x as i32) / 2,
            a.y as i32 + (b.y as i32 - a.y as i32) / 2 + ORIGIN_LIFT,
            a.z as i32 + (b.z as i32 - a.z as i32) / 2,
        ];
        // `RotTrans`: the rotation of the vector, 4.12, and the translation.
        let rot_trans = |v: [i32; 3]| -> [i32; 3] {
            std::array::from_fn(|row| {
                let sum: i64 = (0..3)
                    .map(|col| matrix.m[row][col] as i64 * v[col] as i64)
                    .sum();
                (((origin[row] as i64) << 12) + sum >> 12) as i32
            })
        };
        let pos = rot_trans(eye).map(|c| c as i16);
        let looked_at = rot_trans(target);
        let delta = [0, 1, 2].map(|i| pos[i].wrapping_sub(looked_at[i] as i16) as i32);
        let flat = math::square_root0(mips::add_lo(
            mips::mul_lo(delta[0], delta[0]),
            mips::mul_lo(delta[2], delta[2]),
        ));
        let rot = SVec3 {
            x: (0x800 - math::ratan2(delta[1], flat)) as i16,
            y: math::ratan2(delta[0], delta[2]) as i16,
            z: 0,
        };
        (
            SVec3 {
                x: pos[0],
                y: pos[1],
                z: pos[2],
            },
            rot,
        )
    }

    /// The camera of the start of a race, over what the chase camera has just done:
    /// `transition_frame` counts down from [`TRANSITION_FRAMES`] to 0, and the fly-in is over
    /// then. Does nothing on a level without the fly-in.
    pub fn fly_in(&mut self, level: &Level, transition_frame: i32) {
        let Some(fly_in) = &level.fly_in else {
            return;
        };
        let frame = (TRANSITION_FRAMES - transition_frame).clamp(0, PATH_FRAMES);
        let (pos, rot) = self.fly_in_view(level, fly_in, frame);

        // `CAM_ProcessTransition`, from the fly-in (blend 0) to the chase camera (0x1000).
        let blend = self.transition_blend as i32;
        let (end_pos, end_rot) = (self.pb_pos, self.pb_rot);
        let step = |start: i16, end: i16| {
            (start as i32 + ((end as i32 - start as i32) * blend >> 12) as i16 as i32) as i16
        };
        let turn = |start: i16, end: i16| {
            let mut delta = (end as i32 - start as i32) & 0xfff;
            if delta > 0x7ff {
                delta -= 0x1000;
            }
            ((start as i32 + (delta * blend >> 12) as i16 as i32) & 0xfff) as i16
        };
        self.pb_pos = SVec3 {
            x: step(pos.x, end_pos.x),
            y: step(pos.y, end_pos.y),
            z: step(pos.z, end_pos.z),
        };
        self.pb_rot = SVec3 {
            x: turn(rot.x, end_rot.x),
            y: turn(rot.y, end_rot.y),
            z: turn(rot.z, end_rot.z),
        };

        // The blend for the next tick: a cosine from 0 to 0x1000 over the last 30 ticks.
        let half = BLEND_FRAMES / 2;
        if transition_frame <= BLEND_FRAMES {
            self.transition_blend = if transition_frame < half {
                (math::sin((0x400 - (transition_frame << 10) / half) as u32) / 2 + 0x800) as i16
            } else {
                let angle = (transition_frame - BLEND_FRAMES) * 0x400 / half;
                (0x800 - math::cos(angle as u32) / 2) as i16
            };
        }
    }
}
