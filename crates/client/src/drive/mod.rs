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
//! * `/debug set weapon <N>`: (debug) drop what is in hand and roll item N (0 to 4, 6 to 8, 10 or 11), as the
//!   roulette would (the item ids; 10 and 11 are the triple bomb and missile). Not online
//! * Left mouse button held and mouse moves: turn the camera around the kart; scroll wheel:
//!   closer or farther; double click or Home: back to the default view
//! * R: back to the starting grid
//! * H or Triangle: swap the minimap and the speedometer (`speedometer.rs`)
//! * Escape, P or Start: the pause menu (`pause.rs`)
//! * Select or C (B for the second keyboard player): that player's racer menu (`chooser.rs`)
//! * `/players <1 to 4>`: share the screen (offline). The second player takes a free pad, or the
//!   keyboard's I J K L (U, O hop, `;` item, Y aim); the others take pads (`split.rs`)
//!
//! Not in the sim yet: turbo pads (they are applied in kart-against-kart), items, the
//! start-of-race countdown, other karts. Falling off the track is a mask grab: Aku Aku
//! or Uka Uka (`mask.rs`) carries the kart back to the track.

mod chooser;
mod follow;
mod input;
mod local;
mod overlay;
mod pose;
mod split;
mod tick;

use bevy::prelude::*;
use race::KartSpec;
use vehicle::Driver;

use crate::Track;
use follow::{chase, chase_extra};
use overlay::{hud, reserves_hud, reward_hud, split_toast, wumpa_hud};
use tick::{debug_actions, drive_tick};

pub use follow::{ChaseCamera, MainView, ViewOf};
pub use chooser::Chosen;
pub(crate) use input::Pads;
pub(crate) use local::LocalView;
pub(crate) use split::{HudCamera, UnderCamera, window_rect};
pub(crate) use overlay::spawn_hud;
pub(crate) use pose::{Pose, animation_pose, pose_of, rotation_of};

/// The kart model's entity: its pose is the interpolated pose of the sim.
#[derive(Component)]
pub struct Kart;

/// A kart of another local player (`/players`), as opposed to one of a lobby (`remote.rs`).
#[derive(Component)]
pub struct Extra;

/// The most players that can share the screen.
pub const MAX_PLAYERS: usize = 4;

/// How many players share the screen (`/players`): the first is ours.
#[derive(Resource)]
pub struct LocalPlayers(pub usize);

/// What the viewer keeps of driving: a view for each local player, the first of them ours. The
/// race itself, our kart in it included, is [`crate::props::Sim`].
#[derive(Resource)]
pub struct Drive {
    pub(crate) character: usize,
    anim_num_frames: [u16; 8],
    /// The local players' views (never none): the camera, the weather and the poses to draw
    /// between ticks, of each.
    views: Vec<LocalView>,
}

impl Drive {
    /// Driving for `driver`, the kart of `character` on the grid.
    pub fn new(
        track: &Track,
        driver: &Driver,
        character: usize,
        anim_num_frames: [u16; 8],
    ) -> Drive {
        Drive {
            views: vec![LocalView::new(track, driver, 0, input::Device::Primary)],
            character,
            anim_num_frames,
        }
    }

    /// Our view, the first one: it is what the HUD and the sounds go by.
    pub(crate) fn own(&mut self) -> &mut LocalView {
        &mut self.views[0]
    }

    /// The local players' views, ours first.
    pub(crate) fn views(&self) -> &[LocalView] {
        &self.views
    }

    pub(crate) fn weather(&self) -> &crate::weather::WeatherView {
        &self.views[0].weather
    }

    pub(crate) fn confetti(&self) -> &crate::confetti::ConfettiView {
        &self.views[0].confetti
    }

    /// Our kart is of `character` now, with a model whose animations have `anim_num_frames`.
    pub(crate) fn set_character(&mut self, character: usize, anim_num_frames: [u16; 8]) {
        self.character = character;
        self.anim_num_frames = anim_num_frames;
    }

    /// The karts of the local players, in the order of their slots.
    pub fn specs(&self) -> Vec<KartSpec> {
        let extras = self.views.iter().skip(1).filter_map(|view| view.spec);
        std::iter::once(self.spec()).chain(extras).collect()
    }

    /// Our kart, as a race takes it.
    pub fn spec(&self) -> KartSpec {
        KartSpec {
            character: self.character,
            anim_num_frames: self.anim_num_frames,
            bot: false,
            line: None,
        }
    }
}

/// Whether the race is on, and not the viewer (`/debug viewer`): the systems of the race run
/// only then.
pub(crate) fn driving(drive: Option<Res<Drive>>) -> bool {
    drive.is_some()
}

pub struct DrivePlugin;

impl Plugin for DrivePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(LocalPlayers(1))
            .init_resource::<Chosen>()
            .add_systems(Update, (chooser::input, chooser::sync, chooser::show).chain().run_if(driving))
            .insert_resource(crate::exhaust::Particles::new())
            .init_gizmo_group::<crate::weather::WeatherGizmos>()
            .init_resource::<crate::flash::Flashes>()
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
                    chase_extra,
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
                    split_toast,
                    reserves_hud,
                    reward_hud,
                )
                    .chain()
                    .distributive_run_if(driving),
            )
            .add_systems(
                Update,
                (
                    split::sync,
                    split::layout,
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
