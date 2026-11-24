//! The resources the client keeps: the track, the textures, the options and the scene's systems.

use bevy::prelude::*;
use loader::dram::DramFile;
use loader::icons::Icon;
use loader::lev::Level;
use loader::level::LevelId;
use loader::model::Model;
use loader::vram::Vram;

/// The level shown, its id, and the mode its instances were filtered for (`load_level`): a race
/// of another mode on the same track has to load it again.
#[derive(Resource)]
pub struct Track(pub Level, pub LevelId, pub instance::lev_init::LevMode);

#[derive(Resource)]
pub(crate) struct Textures {
    pub(crate) dram: DramFile,
    pub(crate) vram: Vram,
}

/// What a kart has of its racer's pack. Every retail pack has the same of these (`ctr-query
/// packs`), so they are character 0's, loaded once (`assets::load_shared`); what is a racer's
/// own is in its [`Kit`](crate::assets::Kit).
#[derive(Resource)]
pub(crate) struct KartParts {
    /// The wheel sprites (`tireAnim`). The level's fire takes its pictures from them too.
    pub(crate) wheels: Vec<Icon>,
    /// Global icons 0 and 1 (`shadow1`, `shadow2`).
    pub(crate) shadow: Option<[Icon; 2]>,
    /// The first picture of the `dustpuff` icon group: the exhaust smoke.
    pub(crate) smoke: Option<Icon>,
    /// Global icon `tiremark`: the skid marks.
    pub(crate) tiremark: Option<Icon>,
    /// `turbo` .. `turbo7`: the frames of the exhaust fire.
    pub(crate) flames: Vec<Model>,
}

/// How the level's mesh is built (`/debug level`).
#[derive(Resource, Clone, Copy)]
pub(crate) struct MeshOptions {
    /// Keep the blocks without textures and the faces with a pure red corner.
    pub(crate) hidden: bool,
    /// Cull the level's back faces, as the game does.
    pub(crate) cull: bool,
    /// The `color_lo` vertex colors instead of `color_hi`.
    pub(crate) low: bool,
    /// Move the crates the level places in the ground to where they belong (`props::crate_fixup`).
    pub(crate) crate_fixup: bool,
}

impl Default for MeshOptions {
    fn default() -> Self {
        MeshOptions {
            hidden: false,
            cull: true,
            low: false,
            crate_fixup: true,
        }
    }
}

/// The constant depth bias that keeps a crate in front of the ground it sinks into (`/debug crate
/// bias`; see `DEPTH_BIAS_PER_ORDER` for the unit).
#[derive(Resource, Clone, Copy)]
pub(crate) struct CrateBias(pub(crate) f32);

impl Default for CrateBias {
    fn default() -> Self {
        CrateBias(100_000.0)
    }
}

/// The racers in `GAME_CHARACTER_IDS` order.
pub(crate) const CHARACTERS: [&str; 16] = [
    "Crash",
    "Cortex",
    "Tiny",
    "Coco",
    "N. Gin",
    "Dingo",
    "Polar",
    "Pura",
    "Pinstripe",
    "Papu",
    "Roo",
    "Joe",
    "N. Tropy",
    "Penta",
    "Fake Crash",
    "Oxide",
];

/// The colour of each racer's name in the chat, in the same order as `CHARACTERS`.
pub(crate) const CHARACTER_COLORS: [Color; 16] = [
    Color::srgb(1.0, 0.6, 0.2),    // Crash
    Color::srgb(0.95, 0.9, 0.35),  // Cortex
    Color::srgb(0.85, 0.55, 0.3),  // Tiny
    Color::srgb(1.0, 0.5, 0.75),   // Coco
    Color::srgb(0.6, 0.75, 0.9),   // N. Gin
    Color::srgb(0.95, 0.35, 0.3),  // Dingo
    Color::srgb(0.75, 0.95, 1.0),  // Polar
    Color::srgb(0.3, 0.85, 0.8),   // Pura
    Color::srgb(0.7, 0.7, 0.85),   // Pinstripe
    Color::srgb(0.9, 0.8, 0.5),    // Papu
    Color::srgb(0.55, 0.95, 0.45), // Roo
    Color::srgb(0.5, 0.75, 0.35),  // Joe
    Color::srgb(0.4, 0.45, 1.0),   // N. Tropy
    Color::srgb(0.75, 0.55, 1.0),  // Penta
    Color::srgb(0.8, 0.45, 0.2),   // Fake Crash
    Color::srgb(0.4, 1.0, 0.6),    // Oxide
];

/// What to load the next level with.
#[derive(Resource, Clone, Copy)]
pub(crate) struct Options {
    /// Our pick: the free drive's racer. A race's racer for us is the race's (its sim has it).
    pub(crate) racer: u32,
}

/// What the client is showing: the race (free drive, a race, a lobby), the viewer (`/debug
/// viewer`) or the podium. The only record of it: `drive::Drive` exists in `Drive` alone (put
/// in and taken away by `OnEnter`/`OnExit`), and `podium::Podium` in `Podium` alone. Whatever
/// changes it sets `NextState` and leaves the scene to the `OnEnter` of the new mode (`scene.rs`,
/// `podium.rs`).
#[derive(States, Default, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum Mode {
    #[default]
    Drive,
    Viewer,
    Podium,
}

/// The level shown, a `LevelId`.
#[derive(Resource)]
pub(crate) struct LevelNumber(pub(crate) u32);

/// The systems that build the scene, to run again for another level or the other mode: `setup`,
/// the kart and the console (`console.rs`), then the text overlay of the mode.
#[derive(Resource)]
pub(crate) struct SceneSystems {
    pub(crate) common: Vec<bevy::ecs::system::SystemId>,
    pub(crate) race: Vec<bevy::ecs::system::SystemId>,
    pub(crate) viewer: Vec<bevy::ecs::system::SystemId>,
}

impl SceneSystems {
    /// The podium has the common ones only: not the viewer's text.
    pub(crate) fn of(&self, mode: Mode) -> Vec<bevy::ecs::system::SystemId> {
        let own: &[_] = match mode {
            Mode::Drive => &self.race,
            Mode::Viewer => &self.viewer,
            Mode::Podium => &[],
        };
        self.common.iter().chain(own).copied().collect()
    }
}
