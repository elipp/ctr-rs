//! The chase camera, ported from the reference `ctr-native`
//! (`game/CAM.c`): `CAM_FollowDriver_Normal`, the camera of a race, as
//! `CAM_ThTick` runs it for a camera following a human driver.
//!
//! The camera is a state ([`CameraDc`], the reference's `CameraDC`) that is
//! stepped once per game tick after the driver's. It swings with the driver:
//! its heading is the direction of travel plus `rot_curr_w` (the camera spin
//! a powerslide sets) plus the model's own turn, it pulls back with speed and
//! boosts, rolls with the kart, and keeps above the track. Its result is the
//! push buffer's position and rotation ([`CameraDc::pb_pos`], `pb_rot`) and
//! the point it looks at ([`CameraDc::look_at_pos`]), in world units.
//!
//! While the mask holds the kart the camera stays put (`KS_MASK_GRABBED`).
//!
//! Left out: the start-of-race fly-in and the other cutscene and end-of-race
//! cameras, the transitions between them, being blasted (the camera's
//! lerp back), the mask grab and the rev-engine start, the two-player zoom.
//! The camera uses its own collision scratchpad.

mod matrix;

use coll::Collision;
use loader::lev::{Level, SVec3};
use math::{Vec3, mips};
use vehicle::Driver;
use vehicle::actions;
use vehicle::calc::map_to_range;
use vehicle::kart_state;

pub use matrix::convert_rot_to_matrix;

/// `CameraFlags` the follow camera uses.
pub mod flags {
    pub const DIRECTION_CHANGED: u32 = 0x8;
    pub const FIRE_SPEED_ZOOM: u32 = 0x80;
    pub const REVERSE: u32 = 0x10000;
}

/// `ZoomData`: how far behind the kart the camera sits and how it follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Zoom {
    /// The distance goes from `dist_min` at `speed_min` to `dist_max` at `speed_max`.
    pub dist_min: i16,
    pub dist_max: i16,
    pub speed_min: i16,
    pub speed_max: i16,
    /// How fast the camera's idea of the speed follows the kart's, out and in
    /// (out of 0x100).
    pub percentage1: u8,
    pub percentage2: u8,
    /// Roll scale, roll smoothing, height of the look-at point.
    pub angle: [i16; 3],
    pub vert_distance: i16,
}

/// `data.NearCam4x3`: the close camera (one, three or four players).
pub const NEAR_CAM: Zoom = Zoom {
    dist_min: 0xb3,
    dist_max: 0xe0,
    speed_min: 0x400,
    speed_max: 0x1400,
    percentage1: 0xf0,
    percentage2: 0xff,
    angle: [0x80, 0xe0, 0x70],
    vert_distance: 0x40,
};

/// `data.FarCam4x3`: the far camera.
pub const FAR_CAM: Zoom = Zoom {
    dist_min: 0xe0,
    dist_max: 0x120,
    speed_min: 0x400,
    speed_max: 0x1400,
    percentage1: 0xf0,
    percentage2: 0xff,
    angle: [0x80, 0xe0, 0x60],
    vert_distance: 0x60,
};

/// `QUADBLOCK_FLAG_CAMERA_SEARCH`
const QUAD_CAMERA_SEARCH: u32 = 0x0800;
/// `CAM_FOLLOW_DRIVER_QUAD_FLAGS_SKIP_TERRAIN_HEIGHT`
const QUAD_SKIP_TERRAIN_HEIGHT: u16 = 0x4100;
/// The camera looks down a little: `rot.x` of one player (`0xff9c`).
const PITCH: i16 = -100;
const TERRAIN_WATER: u8 = 4;
const TERRAIN_FAST_WATER: u8 = 0xd;
const TERRAIN_MUD: u8 = 0xe;
const DAMAGE_PITCH_MIN: i16 = -0x20;
const FIRE_ZOOM_OFFSET_MAX: i32 = 0x6000;
const HEIGHT_SMOOTHING_FRAMES: i16 = 8;
const PUSH_BUFFER_CORRECTION_LIMIT: i32 = 2;
/// `level.configFlags` bit for a track with water the mask grab can drop you in.
const CONFIG_UNDERWATER: u32 = 2;

/// `CameraFireSpeedZoom`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FireSpeedZoom {
    pub distance_offset: i32,
    pub timer: i32,
}

/// `CameraHeightSmoothing`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HeightSmoothing {
    pub start_offset: i16,
    pub frames_remaining: i16,
    pub current_offset: i16,
}

/// What a tick gives the camera beyond the driver.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Input {
    /// `elapsedTimeMS`
    pub elapsed_time_ms: i32,
    /// R2 is held: look backwards.
    pub reverse: bool,
    /// Use [`FAR_CAM`] instead of [`NEAR_CAM`] (`nearOrFar`).
    pub far: bool,
}

/// The part of `CameraDC` the follow camera uses, and the push buffer it moves.
#[derive(Clone, Debug)]
pub struct CameraDc {
    pub flags: u32,
    /// `desiredRot.x`: the smoothed roll of the kart.
    pub desired_rot_x: i16,
    pub damage_pitch_offset: i16,
    /// The quad block under the camera.
    pub quad_block: Option<u32>,
    pub camera_move_speed: i32,
    pub push_buffer_pos_correction: Vec3,
    pub camera_pos: Vec3,
    pub look_at_pos: Vec3,
    pub fire_speed_zoom: FireSpeedZoom,
    pub height_smoothing: HeightSmoothing,
    /// `PushBuffer::pos` and `rot`: where the camera is and which way it turns
    /// (`rot.x` is `0x800` minus the pitch, `rot.y` the heading, `rot.z` the roll).
    pub pb_pos: SVec3,
    pub pb_rot: SVec3,
    coll: Collision,
}

impl Default for CameraDc {
    /// `CAM_Init`
    fn default() -> CameraDc {
        CameraDc {
            flags: flags::DIRECTION_CHANGED,
            desired_rot_x: 0,
            damage_pitch_offset: 0,
            quad_block: None,
            camera_move_speed: 0,
            push_buffer_pos_correction: Vec3::default(),
            camera_pos: Vec3::default(),
            look_at_pos: Vec3::default(),
            fire_speed_zoom: FireSpeedZoom::default(),
            height_smoothing: HeightSmoothing::default(),
            pb_pos: SVec3::default(),
            pb_rot: SVec3::default(),
            coll: Collision::default(),
        }
    }
}

fn abs(value: i32) -> i32 {
    if value < 0 {
        mips::neg_lo(value)
    } else {
        value
    }
}

impl CameraDc {
    /// `CAM_FindClosestQuadblock`: the quad block below `pos`; the height of
    /// the ground there is `coll.sps.quad_block_coll.hit_pos.y`.
    fn find_closest_quadblock(&mut self, level: &Level, pos: Vec3) -> bool {
        let sps = &mut self.coll.sps;
        let low = SVec3 {
            x: pos.x as i16,
            y: (pos.y as i16).wrapping_sub(0x800),
            z: pos.z as i16,
        };
        let high = SVec3 {
            x: pos.x as i16,
            y: (pos.y as i16).wrapping_add(0x100),
            z: pos.z as i16,
        };

        sps.input1.pos = low;
        sps.quad_block_coll.pos = high;
        sps.quad_block_coll.hit_pos = low;
        sps.bbox.min = SVec3 {
            x: low.x.min(high.x),
            y: low.y.min(high.y),
            z: low.z.min(high.z),
        };
        sps.bbox.max = SVec3 {
            x: low.x.max(high.x),
            y: low.y.max(high.y),
            z: low.z.max(high.z),
        };
        sps.quad_block_coll.quad_flags_wanted = QUAD_CAMERA_SEARCH;
        sps.quad_block_coll.quad_flags_ignored = 0;
        sps.did_touch_quadblock = 0;
        sps.num_triangles_tested = 0;
        sps.quad_block_coll.search_flags = 0;

        if let Some(quad) = self.quad_block {
            self.coll.test_fixed_quadblock(level, quad);
        }
        if self.coll.sps.did_touch_quadblock == 0 {
            self.coll.search_fixed(level);
        }

        let hit = self.coll.sps.did_touch_quadblock != 0;
        if hit {
            self.quad_block = self.coll.sps.hit.quad;
        }
        hit
    }

    /// `CAM_FollowDriver_Normal` (followed by what `CAM_ThTick` does after
    /// it): moves the camera after `d`.
    pub fn follow_driver(&mut self, d: &Driver, level: &Level, input: &Input) {
        let zoom = if input.far { FAR_CAM } else { NEAR_CAM };
        let elapsed = input.elapsed_time_ms;

        // Looking backwards, while R2 is held.
        let was_reverse = self.flags & flags::REVERSE != 0;
        self.flags &= !flags::REVERSE;
        if input.reverse {
            self.flags |= flags::REVERSE;
        }
        if was_reverse != (self.flags & flags::REVERSE != 0) {
            self.flags |= flags::DIRECTION_CHANGED;
        }
        let reverse_angle: i32 = if self.flags & flags::REVERSE != 0 {
            0x800
        } else {
            0
        };

        // The camera's idea of the speed: follows the kart's, slower in than out.
        let speed = abs(d.speed_approx as i32);
        if self.flags & flags::DIRECTION_CHANGED == 0 {
            let (p13, p11) = if speed < self.camera_move_speed {
                (zoom.percentage2 as i32, zoom.percentage1 as i32)
            } else {
                (zoom.percentage1 as i32, zoom.percentage1 as i32)
            };
            self.camera_move_speed = mips::sra(
                mips::add_lo(
                    mips::mul_lo(p11, self.camera_move_speed),
                    mips::mul_lo(0x100 - p13, speed),
                ),
                8,
            );
        } else {
            self.camera_move_speed = speed;
        }

        let gte = &mut self.coll.gte;

        // The camera turns with the kart's heading and the camera spin of a drift.
        let mut rot = SVec3 {
            x: PITCH,
            y: ((d.rot_curr_w as i32 + d.angle as i32 + 0x800 + reverse_angle) & 0xfff) as i16,
            z: mips::mul_lo(self.desired_rot_x as i32, -2) as i16,
        };
        // (Leaves the matrix in the GTE.)
        convert_rot_to_matrix(gte, rot);

        if self.flags & flags::FIRE_SPEED_ZOOM != 0 {
            let boost = mips::sra(d.fire_speed_cap as i32, 4);
            if self.fire_speed_zoom.timer < boost {
                self.fire_speed_zoom.timer = boost;
            }
        }

        // How far behind: by speed, plus a pull-back that a boost starts.
        let mut distance = map_to_range(
            self.camera_move_speed,
            zoom.speed_min as i32,
            zoom.speed_max as i32,
            zoom.dist_min as i32,
            zoom.dist_max as i32,
        ) as i16 as i32;

        let fire = &mut self.fire_speed_zoom;
        if fire.timer == 0 {
            if fire.distance_offset != 0 {
                fire.distance_offset =
                    mips::sub_lo(fire.distance_offset, mips::mul_lo(elapsed, 0x10));
                if fire.distance_offset < 0 {
                    fire.distance_offset = 0;
                }
            }
        } else {
            fire.distance_offset = mips::add_lo(fire.distance_offset, mips::mul_lo(elapsed, 0x40));
            if fire.distance_offset > FIRE_ZOOM_OFFSET_MAX {
                fire.distance_offset = FIRE_ZOOM_OFFSET_MAX;
            }
            fire.timer = mips::sub_lo(fire.timer, elapsed);
            if fire.timer < 0 {
                fire.timer = 0;
            }
        }
        distance = (distance + (fire.distance_offset >> 8)) as i16 as i32;

        let rotate = |gte: &mut gte::Gte, v: SVec3| -> (i32, i32, i32) {
            gte.mtc2(
                v.x as u16 as u32 | (v.y as u16 as u32) << 16,
                gte::data::VXY0,
            );
            gte.mtc2(v.z as i32 as u32, gte::data::VZ0);
            gte.cop2(vehicle::force::mvmva(1, 0, 0, 3, 0));
            (
                gte.mfc2(gte::data::MAC1) as i32,
                gte.mfc2(gte::data::MAC2) as i32,
                gte.mfc2(gte::data::MAC3) as i32,
            )
        };

        // The camera is behind the look-at point by `distance`, turned by the matrix.
        let (mut pos_x, mut pos_y, mut pos_z) = rotate(
            gte,
            SVec3 {
                x: 0,
                y: 0,
                z: distance as i16,
            },
        );
        // ... and a little to the side by the kart's roll.
        let side = rotate(
            gte,
            SVec3 {
                x: 0,
                y: 0x40,
                z: 0,
            },
        );

        let mut delta = Vec3 {
            x: mips::sra(d.pos_curr.x, 8),
            y: mips::sra(d.pos_curr.y, 8),
            z: mips::sra(d.pos_curr.z, 8),
        };
        delta.x += side.0;
        delta.z += side.2;
        pos_x += delta.x;
        pos_y += delta.y + zoom.vert_distance as i32;
        pos_z += delta.z;

        // The look-at point turns with the model's own turn, too.
        rot.x = PITCH;
        rot.z = 0;
        rot.y = ((d.rot_curr_w as i32
            + d.angle as i32
            + d.turn_angle_curr as i32
            + 0x800
            + reverse_angle)
            & 0xfff) as i16;
        convert_rot_to_matrix(gte, rot);

        // A kart that is not damaged is looked at from slightly above.
        if d.actions_flag_set & actions::WARP == 0 {
            self.damage_pitch_offset -= 8;
            if self.damage_pitch_offset < DAMAGE_PITCH_MIN {
                self.damage_pitch_offset = DAMAGE_PITCH_MIN;
            }
        } else {
            self.damage_pitch_offset += 8;
            if self.damage_pitch_offset > 0 {
                self.damage_pitch_offset = 0;
            }
        }

        let (rx, ry, rz) = rotate(
            gte,
            SVec3 {
                x: 0,
                y: 0,
                z: self.damage_pitch_offset,
            },
        );
        let (rx, ry, rz) = (
            rx.clamp(-0x8000, 0x7fff) as i16 as i32,
            ry.clamp(-0x8000, 0x7fff) as i16 as i32,
            rz.clamp(-0x8000, 0x7fff) as i16 as i32,
        );
        delta.x += rx;
        delta.z += rz;
        delta.y += ry + zoom.angle[2] as i32;

        // The kart's roll, smoothed.
        self.desired_rot_x = mips::sra(
            mips::add_lo(
                mips::mul_lo(zoom.angle[1] as i32, self.desired_rot_x as i32),
                mips::mul_lo(0x100 - zoom.angle[1] as i32, d.rot_curr_z as i32),
            ),
            8,
        ) as i16;

        // A kart the mask holds (falling off the track, out of bounds): the
        // push buffer stays where it is and the camera starts from there.
        let grabbed = d.kart_state == kart_state::MASK_GRABBED;
        if grabbed {
            pos_x = self.pb_pos.x as i32;
            pos_y = self.pb_pos.y as i32;
            pos_z = self.pb_pos.z as i32;
            self.height_smoothing = HeightSmoothing::default();
        }

        // Keep above the track.
        let kart_y = mips::sra(d.pos_curr.y, 8);
        let hit = self.find_closest_quadblock(
            level,
            Vec3 {
                x: pos_x,
                y: pos_y,
                z: pos_z,
            },
        );
        let skip_height = match (hit, self.coll.sps.hit.quad) {
            (true, Some(q)) => level.quad_blocks[q as usize].flags & QUAD_SKIP_TERRAIN_HEIGHT != 0,
            _ => true,
        };
        let smoothing = &mut self.height_smoothing;
        let mut settled = false;
        if skip_height {
            if pos_y < smoothing.current_offset as i32 + kart_y {
                smoothing.frames_remaining = HEIGHT_SMOOTHING_FRAMES;
                smoothing.start_offset = smoothing.current_offset;
                pos_y = smoothing.current_offset as i32 + kart_y;
                settled = true;
            } else {
                smoothing.frames_remaining = HEIGHT_SMOOTHING_FRAMES;
                smoothing.start_offset = (pos_y as i16).wrapping_sub(kart_y as i16);
            }
        } else {
            let quad = &level.quad_blocks[self.coll.sps.hit.quad.unwrap() as usize];
            let mut floor = self.coll.sps.quad_block_coll.hit_pos.y as i32;
            if matches!(
                quad.terrain_type,
                TERRAIN_MUD | TERRAIN_WATER | TERRAIN_FAST_WATER
            ) {
                floor = 0;
            }
            let min_y = floor + zoom.vert_distance as i32;
            if pos_y < min_y {
                pos_y = min_y;
            }
            let frames = smoothing.frames_remaining as i32;
            if frames != 0 {
                pos_y = (mips::mul_lo(8 - frames, pos_y)
                    + mips::mul_lo(frames, smoothing.start_offset as i32 + kart_y))
                    >> 3;
                smoothing.frames_remaining -= 1;
            }
        }
        if !settled {
            smoothing.current_offset = (pos_y as i16).wrapping_sub(kart_y as i16);
        }

        // A track with water the mask grab can drop you in keeps the camera above sea level.
        if level.config_flags & CONFIG_UNDERWATER != 0 && pos_y < zoom.vert_distance as i32 {
            pos_y = zoom.vert_distance as i32;
        }

        if grabbed {
            // The camera keeps its heading, levels out and unrolls.
            self.pb_rot.z -= self.pb_rot.z >> 3;
            if self.pb_rot.x < 0x800 {
                self.pb_rot.x = (self.pb_rot.x + 0x10).min(0x800);
            }
        } else {
            // The direction from the look-at point to the camera is the push buffer's rotation.
            let dir = Vec3 {
                x: pos_x - delta.x,
                y: pos_y - delta.y,
                z: pos_z - delta.z,
            };
            self.pb_rot.y = math::ratan2(dir.x, dir.z) as i16;
            let flat = math::square_root0_stub(mips::add_lo(
                mips::mul_lo(dir.x, dir.x),
                mips::mul_lo(dir.z, dir.z),
            ));
            self.pb_rot.x = (0x800 - math::ratan2(dir.y, flat)) as i16;
            self.pb_rot.z = mips::sra(
                mips::mul_lo(zoom.angle[0] as i32, self.desired_rot_x as i32),
                8,
            ) as i16;
        }

        // The push buffer follows the camera, within a little slack.
        let pos = Vec3 {
            x: pos_x,
            y: pos_y,
            z: pos_z,
        };
        let copy = Vec3 {
            x: pos.x - self.pb_pos.x as i32,
            y: pos.y - self.pb_pos.y as i32,
            z: pos.z - self.pb_pos.z as i32,
        };
        let correction = &mut self.push_buffer_pos_correction;
        correction.x -= pos.x - self.camera_pos.x;
        correction.y -= pos.y - self.camera_pos.y;
        correction.z -= pos.z - self.camera_pos.z;
        correction.x = correction
            .x
            .clamp(-PUSH_BUFFER_CORRECTION_LIMIT, PUSH_BUFFER_CORRECTION_LIMIT);
        correction.y = correction
            .y
            .clamp(-PUSH_BUFFER_CORRECTION_LIMIT, PUSH_BUFFER_CORRECTION_LIMIT);
        correction.z = correction
            .z
            .clamp(-PUSH_BUFFER_CORRECTION_LIMIT, PUSH_BUFFER_CORRECTION_LIMIT);

        if !grabbed {
            self.pb_pos.x = self
                .pb_pos
                .x
                .wrapping_add((copy.x as i16).wrapping_add(correction.x as i16));
            self.pb_pos.y = self
                .pb_pos
                .y
                .wrapping_add((copy.y as i16).wrapping_add(correction.y as i16));
            self.pb_pos.z = self
                .pb_pos
                .z
                .wrapping_add((copy.z as i16).wrapping_add(correction.z as i16));
        }

        self.camera_pos = pos;
        self.look_at_pos = delta;

        // `CAM_ThTick` clears what was handled.
        self.flags &= !(flags::DIRECTION_CHANGED | flags::FIRE_SPEED_ZOOM);
    }
}
