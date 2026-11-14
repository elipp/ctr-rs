//! What one local player's view of the race keeps from tick to tick: the camera, the weather and
//! the confetti it sees, and the two views the drawing goes between. The race is everyone's
//! ([`crate::props::Sim`]); this is what is one player's own, so that each of the players on the
//! screen has one.

use super::follow::{View, view_of};
use super::input::Device;
use crate::Track;

/// How long the camera stays at the finish line watching our kart coast away, in played ticks.
const FINISH_HOLD_TICKS: u32 = 60;
/// How far above our kart's position the held camera looks.
const FINISH_HOLD_LOOK_HEIGHT: i32 = 0x40;

/// What a tick tells a view of the race, as [`LocalView::follow`] takes it.
pub(super) struct Follow<'a> {
    /// The tick just played.
    pub tick: u32,
    /// The kart the camera follows: ours, or another when we watch.
    pub kart: &'a race::Kart,
    /// Whether the start-line fly-in is on, as the race says.
    pub intro: bool,
    /// Whether our kart has finished, and we do not just watch.
    pub finished: bool,
    /// R1 or L1 was tapped (the camera picks another kart then, and the hold is over).
    pub cycle: bool,
    /// The camera follows a kart that is not ours.
    pub elsewhere: bool,
    /// Whether the confetti is on: our kart won.
    pub confetti: bool,
}

pub(crate) struct LocalView {
    /// The kart of the race this view follows and drives.
    pub(super) slot: usize,
    /// What makes its pads.
    pub(super) device: Device,
    /// Its racer menu (`chooser.rs`).
    pub(crate) chooser: super::chooser::Chooser,
    /// The kart it drives, if it is not ours (`Drive` has ours).
    pub(super) spec: Option<race::KartSpec>,
    pub(super) camera: camera::CameraDc,
    pub(super) weather: crate::weather::WeatherView,
    pub(super) confetti: crate::confetti::ConfettiView,
    pub(super) previous_view: View,
    pub(super) current_view: View,
    /// The kart had finished last tick, and the played ticks left of the camera's hold at the
    /// finish line.
    finish: (bool, u32),
}

impl LocalView {
    /// The kart of the race this view follows and drives.
    pub(crate) fn slot(&self) -> usize {
        self.slot
    }

    /// The view of `driver` on the grid, who is `slot` in the race and drives with `device`.
    pub(super) fn new(
        track: &Track,
        driver: &vehicle::Driver,
        slot: usize,
        device: Device,
    ) -> LocalView {
        let mut camera = camera::CameraDc::default();
        camera.follow_driver(
            driver,
            &track.0,
            &camera::Input {
                elapsed_time_ms: 32,
                ..Default::default()
            },
        );
        let view = view_of(&camera);
        LocalView {
            slot,
            device,
            chooser: Default::default(),
            spec: None,
            camera,
            weather: crate::weather::WeatherView::new(&track.0),
            confetti: Default::default(),
            previous_view: view,
            current_view: view,
            finish: (false, 0),
        }
    }

    /// The race starts over: the camera, the weather and the confetti begin again.
    pub(super) fn reset(&mut self, track: &Track) {
        self.camera = camera::CameraDc::default();
        self.weather = crate::weather::WeatherView::new(&track.0);
        self.confetti = Default::default();
    }

    /// The race is paused: nothing moves between ticks.
    pub(super) fn freeze(&mut self) {
        self.previous_view = self.current_view;
        self.weather.freeze();
        self.confetti.freeze();
    }

    /// The camera, the weather and the confetti after a tick.
    pub(super) fn follow(&mut self, track: &Track, f: Follow) {
        let camera_input = camera::Input {
            elapsed_time_ms: vehicle::Env::default().elapsed_time_ms,
            ..Default::default()
        };
        // Past the line the camera stays there a while and turns after the kart as it coasts
        // away, until R1 or L1 picks a kart to follow (or a rollback takes the finish back).
        let (was_finished, held) = &mut self.finish;
        if f.finished && !*was_finished {
            *held = FINISH_HOLD_TICKS;
        }
        *was_finished = f.finished;
        if !f.finished || f.cycle || f.elsewhere {
            *held = 0;
        }
        let holding = *held > 0;
        if holding {
            // (Counted in played ticks only: the wait for the others past the line holds still.)
            *held -= 1;
        }
        let jumped = !holding && self.camera.flags & camera::flags::DIRECTION_CHANGED != 0;
        if holding {
            let mut target = f.kart.driver.pos_curr;
            target.x >>= 8;
            target.y = (target.y >> 8) + FINISH_HOLD_LOOK_HEIGHT;
            target.z >>= 8;
            self.camera.look_at(target);
            // Once it is over, straight back behind the kart.
            self.camera.flags |= camera::flags::DIRECTION_CHANGED;
        } else {
            self.camera
                .follow_driver(&f.kart.driver, &track.0, &camera_input);
        }
        // The start-line fly-in, over the chase camera, for as long as the race says.
        let fly_in_frame = camera::TRANSITION_FRAMES - f.tick as i32;
        if f.intro && fly_in_frame >= 0 {
            self.camera.fly_in(&track.0, fly_in_frame);
        }
        self.weather.step(&track.0, &self.camera, jumped);
        self.confetti.step(&self.camera, f.confetti);
    }

    /// The tick's view is in: the drawing goes from the last one to it.
    pub(super) fn push(&mut self) {
        self.previous_view = self.current_view;
        self.current_view = view_of(&self.camera);
    }
}
