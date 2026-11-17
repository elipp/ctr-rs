//! A map as the sim races on it. Shared by the client's scene, its personal-best replays and the
//! leaderboard's verifier: anything here that the sim reads must come out the same for all of
//! them, or a run re-times differently off the scene.

use instance::lev_init::{LevMode, lev_init_all};
use loader::dram::DramFile;
use loader::lev::Level;
use loader::model::Model;
use loader::nav_amend::Amendments;
use loader::vram::VramImage;
use map::Map;

pub struct SimLevel<'a> {
    pub level: Level,
    pub dram: &'a DramFile,
    pub vram: &'a [VramImage],
    /// Bot path amendments that didn't apply: the caller logs them.
    pub left_out: Vec<String>,
}

/// The level a race of `mode` on `map` (track number `track`) runs on. A relic race and a time
/// trial are on the map's relic LEV where it has one (`Map::relic`), as retail's `levelLOD` 8 is
/// for both (`LOAD_TenStages`).
pub fn sim_level(map: &Map, track: u32, mode: LevMode) -> SimLevel<'_> {
    let (level, dram, vram) = match (&map.relic, mode) {
        (Some(r), LevMode::Relic | LevMode::TimeTrial) => (&r.level, &r.dram, &r.vram),
        _ => (&map.level, &map.dram, &map.vram),
    };
    let mut level = level.clone();
    lev_init_all(&mut level.instances, mode);
    let left_out = Amendments::builtin().apply(track, &mut level.nav_paths);
    SimLevel {
        level,
        dram,
        vram,
        left_out,
    }
}

/// The track number the client gives `map`: the game's track it stands in for, or its own id.
pub fn track_of(map: &Map) -> std::io::Result<u32> {
    match map::builtin::level_of(&map.name) {
        Some(level) => Ok(level.0),
        None => map.id(),
    }
}

/// The frames of the first eight animations of a kart `model` (`Driver::animNumFrames`).
pub fn anim_num_frames(model: &Model) -> [u16; 8] {
    let mut frames = [0; 8];
    for (n, anim) in frames.iter_mut().zip(&model.headers[0].animations) {
        *n = anim.num_frames;
    }
    frames
}

/// [`anim_num_frames`] of every character's kart (`GAME_CHARACTER_IDS` order), as the retail
/// disc's hi models have them: for whoever replays runs without the models (the leaderboard).
/// `tests/anim_frames.rs` checks them against the disc.
pub const ANIM_NUM_FRAMES: [[u16; 8]; loader::source::CHARACTERS as usize] = {
    const KART: [u16; 8] = [21, 7, 15, 4, 0, 0, 0, 0];
    const OXIDE: [u16; 8] = [21, 1, 1, 1, 0, 0, 0, 0];
    [
        KART, KART, KART, KART, KART, KART, KART, KART, KART, KART, KART, KART, KART, KART, KART,
        OXIDE,
    ]
};
