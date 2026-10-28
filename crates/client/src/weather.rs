//! The rain and snow of the level (`RenderWeather`): streaks around the camera, more or fewer
//! as the quad blocks it passes say. Where they are is `render::weather`'s. It is a tick of it
//! for each tick of the race, and the streaks are drawn between the last two, so they are as
//! smooth as the camera is. Drawn as gizmo lines, each from its dim end to its bright one (retail
//! adds them to the picture).

use bevy::prelude::*;
use camera::CameraDc;
use loader::lev::Level;
use render::mesh::SCALE;
use render::weather::{Blend, Camera, Streak, Weather};

/// How many times longer than retail's the streaks are drawn (from their lower end): a pixel of
/// the PlayStation's picture is several of ours, and its lines read as longer than their length.
const STREAK_STRETCH: f32 = 1.25;

/// The least alpha of a streak's end: additive white on a PlayStation fills the pixel, where our
/// blended one lets the background through.
const MIN_ALPHA: f32 = 0.95;

/// A streak that moved farther than this between two ticks wrapped round the box, and is not
/// drawn between the two places.
const WRAP_JUMP: i32 = 0x400;

/// What the weather of the race keeps: the state of the game's and the last two ticks of it.
pub struct WeatherView {
    weather: Weather,
    primed: bool,
    /// The camera at the last two ticks (the streaks are from the camera).
    eyes: [Vec3; 2],
    streaks: [Vec<Option<Streak>>; 2],
}

fn eye_of(camera: &CameraDc) -> Vec3 {
    let p = camera.pb_pos;
    Vec3::new(p.x as f32, p.y as f32, p.z as f32) * SCALE
}

impl WeatherView {
    /// `MainInit_RainBuffer`: the level's weather, for the one view.
    pub fn new(level: &Level) -> WeatherView {
        WeatherView {
            weather: Weather::new(&level.rain_buffer, 1),
            primed: false,
            eyes: [Vec3::ZERO; 2],
            streaks: [Vec::new(), Vec::new()],
        }
    }

    /// One tick of the race. `jumped` is for a camera that did not move there (it is behind
    /// another kart, or the kart respawned): the streaks do not trail from where it was.
    pub fn step(&mut self, level: &Level, camera: &CameraDc, jumped: bool) {
        // The quad block under the camera says how much weather there is.
        if let Some(quad) = camera
            .quad_block
            .and_then(|q| level.quad_blocks.get(q as usize))
        {
            self.weather
                .set_intensity(quad.weather_intensity, quad.weather_vanish_rate, 1);
        }
        let view = Camera {
            pos: camera.pb_pos,
            yaw: camera.pb_rot.y,
        };
        let jumped = jumped || !self.primed;
        if jumped {
            self.weather.reset(&view);
            self.primed = true;
        }
        let eye = eye_of(camera);
        self.eyes = [if jumped { eye } else { self.eyes[1] }, eye];
        self.streaks.swap(0, 1);
        if jumped {
            self.streaks[0].clear();
        }
        self.streaks[1] = self.weather.tick(&view);
    }

    /// The race is paused: nothing moves between ticks.
    pub fn freeze(&mut self) {
        self.eyes[0] = self.eyes[1];
        self.streaks[0] = self.streaks[1].clone();
    }

    /// The streaks `t` of the way from the last tick to this one, as the two ends in the world.
    fn lines(&self, t: f32) -> impl Iterator<Item = [Vec3; 2]> {
        let eye = self.eyes[0].lerp(self.eyes[1], t);
        self.streaks[1]
            .iter()
            .enumerate()
            .filter_map(move |(i, now)| {
                let now = (*now)?;
                let from = match self.streaks[0].get(i) {
                    Some(Some(before)) if before[0][0].abs_diff(now[0][0]) < WRAP_JUMP as u32 => {
                        *before
                    }
                    _ => now,
                };
                // Camera-relative, so the lerp of the eye and the lerp of the streak add up to the
                // streak's own movement through the world.
                let lerp = |a: [i32; 3], b: [i32; 3]| {
                    Vec3::new(a[0] as f32, a[1] as f32, a[2] as f32)
                        .lerp(Vec3::new(b[0] as f32, b[1] as f32, b[2] as f32), t)
                        * SCALE
                };
                let (begin, end) = (eye + lerp(from[0], now[0]), eye + lerp(from[1], now[1]));
                Some([end + (begin - end) * STREAK_STRETCH, end])
            })
    }

    /// The colours of the two ends of a streak. The GPU adds a streak to the picture (all of it,
    /// or a quarter), which alpha blending of a white one stands in for.
    fn colors(&self) -> [Color; 2] {
        let share = match self.weather.blend() {
            Blend::AddQuarter => 0.25,
            Blend::Add | Blend::Other => 1.0,
        };
        // Only the snow is held up: the rain is as dim as it should be.
        let min_alpha = if share == 1.0 { MIN_ALPHA } else { 0.0 };
        self.weather.colors().map(|rgb| {
            let [r, g, b] = [rgb & 0xff, (rgb >> 8) & 0xff, (rgb >> 16) & 0xff];
            let brightness = r.max(g).max(b) as f32 / 255.0;
            Color::srgba(1.0, 1.0, 1.0, (brightness * share).max(min_alpha))
        })
    }
}

/// The weather's own gizmos, so that the width of its lines is its own.
#[derive(Default, Reflect, GizmoConfigGroup)]
pub struct WeatherGizmos;

/// The lines of the PlayStation's picture are a pixel of its 240 rows. Snow is drawn that many
/// times thicker on a taller window, or it is a few specks.
const RETAIL_ROWS: f32 = 240.0;

/// The width of the rain's lines, in pixels.
const RAIN_WIDTH: f32 = 2.0;

/// Sets the width of the lines, before `draw` takes the gizmos (which read the config).
pub fn set_width(
    drive: Res<crate::drive::Drive>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut store: ResMut<GizmoConfigStore>,
) {
    let snow = drive.weather().weather.blend() == Blend::Add;
    store.config_mut::<WeatherGizmos>().0.line.width = match windows.single() {
        Ok(window) if snow => (window.height() / RETAIL_ROWS).max(RAIN_WIDTH),
        _ => RAIN_WIDTH,
    };
}

pub fn draw(
    drive: Res<crate::drive::Drive>,
    fixed: Res<Time<Fixed>>,
    mut gizmos: Gizmos<WeatherGizmos>,
) {
    let view = drive.weather();
    let [top, bottom] = view.colors();
    for [begin, end] in view.lines(fixed.overstep_fraction()) {
        gizmos.line_gradient(begin, end, top, bottom);
    }
}
