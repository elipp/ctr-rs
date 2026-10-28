//! The resources the client keeps: the track, the textures, the options and the scene's systems.

use bevy::prelude::*;
use loader::dram::DramFile;
use loader::icons::Icon;
use loader::lev::Level;
use loader::level::LevelId;
use loader::model::Model;
use loader::vram::Vram;

#[derive(Resource)]
pub struct Track(pub Level, pub LevelId);

#[derive(Resource)]
pub(crate) struct Textures {
    pub(crate) dram: DramFile,
    pub(crate) vram: Vram,
}

/// The racer model shown on the grid.
#[derive(Resource)]
pub(crate) struct Racer(pub(crate) Model);

#[derive(Resource)]
pub(crate) struct WheelIcons(pub(crate) Vec<Icon>);

#[derive(Resource)]
pub(crate) struct ShadowIcons(pub(crate) Option<[Icon; 2]>);

#[derive(Resource)]
pub(crate) struct SmokeIcon(pub(crate) Option<Icon>);

#[derive(Resource)]
pub(crate) struct TiremarkIcon(pub(crate) Option<Icon>);

#[derive(Resource)]
pub(crate) struct FlameModels(pub(crate) Vec<Model>);

/// How the level's mesh is built (`/debug level`).
#[derive(Resource, Clone, Copy)]
pub(crate) struct MeshOptions {
    /// Keep the blocks without textures and the faces with a pure red corner.
    pub(crate) hidden: bool,
    /// Cull the level's back faces, as the game does.
    pub(crate) cull: bool,
    /// The `color_lo` vertex colors instead of `color_hi`.
    pub(crate) low: bool,
}

impl Default for MeshOptions {
    fn default() -> Self {
        MeshOptions {
            hidden: false,
            cull: true,
            low: false,
        }
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

/// What to load the next level with.
#[derive(Resource, Clone, Copy)]
pub(crate) struct Options {
    /// The character of the racer on the grid.
    pub(crate) racer: u32,
    /// The race, or the viewer (`/debug viewer`).
    pub(crate) driving: bool,
}

/// The level shown, a `LevelId`.
#[derive(Resource)]
pub(crate) struct LevelNumber(pub(crate) u32);

/// The systems that build the scene, to run again for another level or the other mode: `setup`
/// and the kart, then the text overlay of the mode.
#[derive(Resource)]
pub(crate) struct SceneSystems {
    pub(crate) common: Vec<bevy::ecs::system::SystemId>,
    pub(crate) race: Vec<bevy::ecs::system::SystemId>,
    pub(crate) viewer: Vec<bevy::ecs::system::SystemId>,
}

impl SceneSystems {
    pub(crate) fn of(&self, driving: bool) -> Vec<bevy::ecs::system::SystemId> {
        let mode = if driving { &self.race } else { &self.viewer };
        self.common.iter().chain(mode).copied().collect()
    }
}
