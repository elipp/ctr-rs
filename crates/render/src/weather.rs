//! The rain and snow of a level (`RenderWeather`, `RainBuffer`): short streaks in a box around
//! the camera, which the game draws as lines of a PlayStation's GPU. Where they are comes from a
//! small pseudo-random generator, the scroll of the box (how far it has fallen) and the camera's
//! own movement, which the streaks trail behind. This is where they are, in game units from the
//! camera; the viewer draws them.

use loader::lev::{RainBuffer, SVec3};

const XY_MASK: u32 = 0xfffe_ffff;
const WRAP_MASK: u32 = 0x07fe_07ff;
/// The box is 0x800 units each way, and the point of the camera in it is nearer its middle.
const BOX_MIDDLE: u32 = 0x400;

/// The colour code of the GPU's shaded, semi-transparent line, over the top colour.
pub const LINE_COMMAND: u32 = 0x5200_0000;

/// A streak: where it begins (the dimmer end) and ends, in game units from the camera.
pub type Streak = [[i32; 3]; 2];

/// What the weather needs of a camera (`PushBuffer`).
#[derive(Clone, Copy, Debug, Default)]
pub struct Camera {
    pub pos: SVec3,
    /// `rot.y`: the heading.
    pub yaw: i16,
}

/// How the GPU blends the streaks onto the picture: the semi-transparency bits of their draw mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blend {
    /// Background plus the streak.
    Add,
    /// Background plus a quarter of the streak.
    AddQuarter,
    /// Not one of the two the levels use.
    Other,
}

/// `gGT->rainBuffer[i]`
#[derive(Clone, Debug)]
pub struct Weather {
    count: i32,
    max: i32,
    vanish_rate: i32,
    scroll_xy: u32,
    scroll_z: i32,
    velocity_xy: u32,
    velocity_z: i32,
    previous_xy: u32,
    previous_z: i32,
    color_top: u32,
    color_bottom: u32,
    fill_mode: u32,
}

/// `RenderWeather_NextRngByte`
fn next_byte(state0: &mut u32, state1: &mut u32) -> u32 {
    let shifted = *state1 >> 8;
    let mut mixed = state0.wrapping_add(shifted);
    *state0 = (*state0 >> 8) | (*state1 << 24);
    mixed = mixed.wrapping_add(*state0 >> 8);
    mixed <<= 24;
    *state1 = shifted | mixed;
    mixed
}

/// `RenderWeather_NextRng`: the packed `x` and `y`, and the `z`, of the next streak.
fn next(state0: &mut u32, state1: &mut u32) -> (u32, u32) {
    let x = next_byte(state0, state1);
    let y = next_byte(state0, state1);
    let z = next_byte(state0, state1);
    let xy = (((x as i32) >> 21) as u32 & 0xffff) | (((y as i32) >> 5) as u32);
    (xy, z >> 21)
}

fn point(xy: u32, z: u32) -> [i32; 3] {
    [
        (xy & 0xffff) as i16 as i32,
        (xy >> 16) as i16 as i32,
        z as i32,
    ]
}

impl Weather {
    /// `MainInit_RainBuffer`, for one player: the level's rain, shared out between `players`.
    pub fn new(level: &RainBuffer, players: u32) -> Weather {
        let players = players.max(1) as i32;
        Weather {
            count: level.num_particles_curr / players,
            max: i32::from(level.num_particles_max) / players,
            vanish_rate: i32::from(level.vanish_rate),
            scroll_xy: level.scroll_xy,
            scroll_z: level.scroll_z,
            velocity_xy: level.velocity_xy,
            velocity_z: level.velocity_z,
            previous_xy: 0,
            previous_z: 0,
            color_top: level.color_top,
            color_bottom: level.color_bottom,
            fill_mode: level.fill_mode,
        }
    }

    /// How many streaks there are now: they come in and go as the camera passes quad blocks with
    /// more or less weather.
    pub fn count(&self) -> i32 {
        self.count
    }

    /// The streaks' colours at their two ends, as `0x00BBGGRR`.
    pub fn colors(&self) -> [u32; 2] {
        [self.color_top & 0xff_ffff, self.color_bottom & 0xff_ffff]
    }

    pub fn blend(&self) -> Blend {
        match (self.fill_mode >> 5) & 3 {
            1 => Blend::Add,
            3 => Blend::AddQuarter,
            _ => Blend::Other,
        }
    }

    /// `RenderAllWeather`'s per-frame work, before `RenderWeather`: the quad block under the
    /// camera says how many streaks there should be, and how fast they come or go.
    pub fn set_intensity(&mut self, intensity: u8, vanish_rate: u8, players: u32) {
        let players = players.max(1) as i32;
        self.max = (i32::from(intensity) << 2) / players;
        self.vanish_rate = (i32::from(vanish_rate) << 2) / players;
    }

    /// `CAMERA_FLAG_RESET_RAIN_POS`: the camera jumped, so the streaks do not trail from where it
    /// was.
    pub fn reset(&mut self, camera: &Camera) {
        self.previous_xy = Self::camera_xy(camera);
        self.previous_z = i32::from(camera.pos.z);
    }

    fn camera_xy(camera: &Camera) -> u32 {
        (u32::from(camera.pos.x as u16) | (u32::from(camera.pos.y as u16) << 16)) & XY_MASK
    }

    /// `RenderWeather`: a frame of the weather. One entry for each of the streaks that there are
    /// (so a streak keeps its place in the list from one frame to the next), none for those that
    /// wrapped around the box this frame.
    pub fn tick(&mut self, camera: &Camera) -> Vec<Option<Streak>> {
        let (sin, cos) = math::sin_cos(camera.yaw as u16 as u32);
        let center_x = ((sin >> 2) + BOX_MIDDLE as i32) as u32;
        let packed_center = 0x0400_0000 | center_x;
        let center_z = (cos >> 2) + BOX_MIDDLE as i32;

        // The count goes toward the maximum.
        let diff = self.max - self.count;
        if diff != 0 {
            let rate = self.vanish_rate;
            if diff < 0 {
                self.count -= rate;
                if diff + rate > 0 {
                    self.count = self.max;
                }
            } else {
                self.count += rate;
                if diff - rate < 0 {
                    self.count = self.max;
                }
            }
        }

        let scroll_xy = self.scroll_xy;
        let scroll_z = self.scroll_z;
        let velocity_xy = self.velocity_xy & XY_MASK;
        let scroll_xy_end = scroll_xy.wrapping_add(velocity_xy) & XY_MASK;
        let scroll_z_end = scroll_z.wrapping_add(self.velocity_z);
        self.scroll_xy = scroll_xy_end;
        self.scroll_z = scroll_z_end;
        let scroll_xy_start =
            scroll_xy.wrapping_add(((velocity_xy as i32 >> 1) as u32) & XY_MASK) & XY_MASK;
        let scroll_z_start = scroll_z.wrapping_add(self.velocity_z >> 1);

        // The streaks trail behind the camera: their upper ends are drawn from where it was
        // (some of it), their lower ends from where it is.
        let camera_xy = Self::camera_xy(camera);
        let camera_z = i32::from(camera.pos.z);
        let (previous_xy, previous_z) = (self.previous_xy, self.previous_z);
        self.previous_xy = camera_xy;
        self.previous_z = camera_z;
        let delta = camera_xy.wrapping_sub(previous_xy);
        let correction = ((((delta as i32) >> 20) << 17) as u32)
            | ((((delta << 16) as i32) >> 19) as u32 & 0xffff);
        let smoothed_xy = camera_xy.wrapping_sub(correction) & XY_MASK;
        let smoothed_z = camera_z - ((camera_z - previous_z) >> 3);

        let start_xy =
            (scroll_xy_start.wrapping_sub(smoothed_xy) & XY_MASK).wrapping_add(center_x) & XY_MASK;
        let end_xy =
            (scroll_xy_end.wrapping_sub(camera_xy) & XY_MASK).wrapping_add(center_x) & XY_MASK;
        let start_z = scroll_z_start - smoothed_z + center_z;
        let end_z = scroll_z_end - camera_z + center_z;
        let span_xy = end_xy.wrapping_sub(start_xy);
        let span_z = end_z.wrapping_sub(start_z) as u32;

        let (mut state0, mut state1) = (0x3012_5400_u32, 0x4935_83fe_u32);
        let (mut xy, mut z) = next(&mut state0, &mut state1);
        let mut out = Vec::with_capacity(self.count.max(0) as usize);
        for _ in 0..self.count.max(0) {
            let xy0 = (xy.wrapping_add(start_xy) & WRAP_MASK).wrapping_sub(packed_center);
            let xy1 = (xy.wrapping_add(end_xy) & WRAP_MASK).wrapping_sub(packed_center);
            let z0 = z.wrapping_add(start_z as u32) & 0x7ff;
            let z1 = z.wrapping_add(end_z as u32) & 0x7ff;
            // A streak that wrapped round the box this frame has its ends far apart.
            let whole = xy1.wrapping_sub(xy0) == span_xy && z1.wrapping_sub(z0) == span_z;
            out.push(whole.then(|| {
                [
                    point(xy0 & XY_MASK, z0.wrapping_sub(center_z as u32)),
                    point(xy1 & XY_MASK, z1.wrapping_sub(center_z as u32)),
                ]
            }));
            (xy, z) = next(&mut state0, &mut state1);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Blizzard Bluff's snow, as the level file has it.
    fn snow() -> RainBuffer {
        RainBuffer {
            num_particles_max: 0xf0,
            vanish_rate: 0xf0,
            velocity_xy: 0xfff8_0000,
            color_top: 0x40_4040,
            color_bottom: 0xff_ffff,
            fill_mode: 0xe100_0a20,
            offset_ot: 1,
            ..Default::default()
        }
    }

    fn camera() -> Camera {
        Camera {
            pos: SVec3 {
                x: 1000,
                y: 300,
                z: -2000,
            },
            yaw: 0x300,
        }
    }

    fn steady(mut weather: Weather, camera: &Camera) -> Vec<Option<Streak>> {
        weather.reset(camera);
        let mut out = Vec::new();
        for _ in 0..10 {
            out = weather.tick(camera);
        }
        out
    }

    #[test]
    fn it_starts_with_none_and_comes_in_at_the_vanish_rate() {
        let mut weather = Weather::new(&snow(), 1);
        weather.set_intensity(0x60, 0x10, 1);
        assert_eq!(weather.count(), 0);
        let c = camera();
        weather.reset(&c);
        assert_eq!(weather.tick(&c).len(), 0x40);
        assert_eq!(weather.count(), 0x40);
        for _ in 0..10 {
            weather.tick(&c);
        }
        assert_eq!(weather.count(), 0x180);
        // And goes as it came.
        weather.set_intensity(0, 0x10, 1);
        weather.tick(&c);
        assert_eq!(weather.count(), 0x140);
        for _ in 0..10 {
            weather.tick(&c);
        }
        assert_eq!(weather.count(), 0);
    }

    #[test]
    fn a_still_camera_sees_snow_fall_in_a_box_around_it() {
        let mut weather = Weather::new(&snow(), 1);
        weather.set_intensity(0x60, 0xff, 1);
        let streaks = steady(weather, &camera());
        assert_eq!(streaks.len(), 0x180);
        let whole: Vec<_> = streaks.iter().flatten().collect();
        assert!(whole.len() > 0x100, "{} whole", whole.len());
        for [begin, end] in whole {
            // It falls 8 a frame, so the streak is 4 long (from the middle of the last to the
            // end of this), and straight down.
            assert_eq!(begin[0], end[0]);
            assert_eq!(begin[2], end[2]);
            assert_eq!(begin[1] - end[1], 4, "{begin:?} {end:?}");
            for p in [begin, end] {
                assert!((-0x800..0x800).contains(&p[0]), "{p:?}");
                assert!((-0x800..0x800).contains(&p[1]), "{p:?}");
                assert!((-0x800..0x800).contains(&p[2]), "{p:?}");
            }
        }
    }

    #[test]
    fn a_moving_camera_drags_the_streaks_back() {
        let mut weather = Weather::new(&snow(), 1);
        weather.set_intensity(0x60, 0xff, 1);
        let mut c = camera();
        weather.reset(&c);
        weather.tick(&c);
        c.pos.x += 80;
        let moving = weather.tick(&c);
        let dragged = moving
            .iter()
            .flatten()
            .filter(|[begin, end]| begin[0] != end[0])
            .count();
        assert!(dragged > 0x100, "{dragged}");
    }

    #[test]
    fn players_share_the_streaks() {
        let weather = Weather::new(
            &RainBuffer {
                num_particles_curr: 100,
                num_particles_max: 200,
                ..snow()
            },
            4,
        );
        assert_eq!(weather.count(), 25);
        let mut weather = weather;
        weather.set_intensity(0x60, 4, 4);
        assert_eq!(weather.max, 0x60);
    }

    #[test]
    fn the_blend_is_the_draw_modes() {
        assert_eq!(Weather::new(&snow(), 1).blend(), Blend::Add);
        let rain = RainBuffer {
            fill_mode: 0xe100_0a60,
            ..snow()
        };
        assert_eq!(Weather::new(&rain, 1).blend(), Blend::AddQuarter);
    }
}
