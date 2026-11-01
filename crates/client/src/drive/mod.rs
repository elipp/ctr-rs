//! Drive mode (the default; `/debug viewer` leaves it): the kart on the keyboard, simulated by the
//! `vehicle` crate once per game tick (30 Hz), drawn between the last two
//! ticks, with a chase camera.
//!
//! * W / Up: gas (Cross), S / Down: brake and reverse (Square, stick back)
//! * A / D, Left / Right: steer
//! * Space / Shift: hop (R1), Q / Ctrl: hop (L1). Land while steering hard
//!   with one still held to drift; tap the other when the meter is nearly
//!   empty for the boost
//! * M: use the item in hand (Circle): the mask is longer with 10 wumpa fruit. A bomb or a
//!   potion is thrown backward with S held; a potion forward with E held (dropped with neither)
//! * `/debug set wumpa <N>`: (debug) set the wumpa count (0 to 10, clamped)
//! * `/debug set weapon <N>`: (debug) drop what is in hand and roll item N (0 to 4, 6, 7, 10 or 11), as the
//!   roulette would (the item ids; 10 and 11 are the triple bomb and missile). Not online, not in
//!   a demo
//! * Left mouse button held and mouse moves: turn the camera around the kart; scroll wheel:
//!   closer or farther; double click or Home: back to the default view
//! * R: back to the starting grid
//! * Escape, P or Start: the pause menu (`pause.rs`)
//! * F5 / 1: start recording a demo from the grid, again to save it to `demo.ctr`
//! * F6 / 2: play `demo.ctr` back from the grid (any F5 or R ends it, the kart stays where it is)
//!
//! Not in the sim yet: turbo pads (they are applied in kart-against-kart), items, the
//! start-of-race countdown, other karts. Falling off the track is a mask grab: Aku Aku
//! or Uka Uka (`mask.rs`) carries the kart back to the track.

mod follow;
mod input;
mod overlay;
mod pose;
mod tick;

use bevy::prelude::*;
use race::KartSpec;
use vehicle::Driver;
use vehicle::demo::Demo;

use crate::Track;
use follow::{View, chase, view_of};
use overlay::{hud, reserves_hud, split_hud, wumpa_hud};
use tick::{debug_actions, drive_tick};

pub use follow::ChaseCamera;
pub(crate) use input::Pads;
pub(crate) use overlay::spawn_hud;
pub(crate) use pose::{Pose, animation_pose, pose_of, rotation_of};

/// The kart model's entity: its pose is the interpolated pose of the sim.
#[derive(Component)]
pub struct Kart;

/// What the viewer keeps of driving: the camera, the poses to draw between ticks, the demo. The
/// race itself, our kart in it included, is [`crate::props::Sim`].
#[derive(Resource)]
pub struct Drive {
    pub(crate) character: usize,
    anim_num_frames: [u16; 8],
    demo: DemoMode,
    camera: camera::CameraDc,
    weather: crate::weather::WeatherView,
    confetti: crate::confetti::ConfettiView,
    previous_view: View,
    current_view: View,
}

impl Drive {
    /// Driving for `driver`, the kart of `character` on the grid.
    pub fn new(
        track: &Track,
        driver: &Driver,
        character: usize,
        anim_num_frames: [u16; 8],
    ) -> Drive {
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
        Drive {
            camera,
            weather: crate::weather::WeatherView::new(&track.0),
            confetti: default(),
            previous_view: view,
            current_view: view,
            character,
            anim_num_frames,
            demo: DemoMode::Off,
        }
    }

    pub(crate) fn weather(&self) -> &crate::weather::WeatherView {
        &self.weather
    }

    pub(crate) fn confetti(&self) -> &crate::confetti::ConfettiView {
        &self.confetti
    }

    /// Our kart, as a race takes it.
    pub fn spec(&self) -> KartSpec {
        KartSpec {
            character: self.character,
            anim_num_frames: self.anim_num_frames,
        }
    }
}

/// Where the sim gets its input from.
enum DemoMode {
    Off,
    Recording(Demo),
    Playing(Demo),
}

const DEMO_FILE: &str = "demo.ctr";

/// Whether the race is on, and not the viewer (`/debug viewer`): the systems of the race run
/// only then.
pub(crate) fn driving(drive: Option<Res<Drive>>) -> bool {
    drive.is_some()
}

pub struct DrivePlugin;

impl Plugin for DrivePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(crate::exhaust::Particles::new())
            .init_gizmo_group::<crate::weather::WeatherGizmos>()
            .init_resource::<crate::flash::Flash>()
            .init_resource::<overlay::DebugText>()
            .add_systems(Update, overlay::debug_text_command)
            .add_systems(Startup, spawn_hud)
            .add_systems(
                FixedUpdate,
                (drive_tick, crate::flash::step, crate::projectile::tick)
                    .chain()
                    .run_if(driving),
            )
            .add_systems(
                Update,
                (
                    crate::kart::reap,
                    crate::kart::animate,
                    crate::kart::show,
                    crate::submerge::cut,
                    chase,
                    debug_actions,
                    crate::projectile::place,
                    crate::rain::draw.run_if(crate::fx::beaker_rain),
                    crate::weather::set_width.before(crate::weather::draw),
                    crate::weather::draw.run_if(crate::fx::weather),
                    crate::confetti::draw,
                    crate::spider::draw_webs,
                    crate::spider::shadows,
                    crate::flash::show,
                    hud,
                    wumpa_hud,
                    split_hud,
                    reserves_hud,
                )
                    .chain()
                    .distributive_run_if(driving),
            )
            .add_systems(
                Update,
                (
                    crate::shadow::update,
                    crate::flame::update,
                    crate::skid::update,
                    crate::mask::update.after(crate::kart::show),
                    crate::shield::update.after(crate::kart::show).after(chase),
                    crate::names::overhead.after(crate::kart::show).after(chase),
                )
                    .run_if(driving),
            )
            .add_systems(
                PostUpdate,
                (crate::exhaust::update, crate::exhaust::update_sparks)
                    .before(TransformSystems::Propagate)
                    .run_if(driving),
            );
    }
}
