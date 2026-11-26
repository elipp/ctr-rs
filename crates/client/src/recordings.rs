//! The recordings of finished races that weren't solo runs (`race::multi::Recording`), for
//! `ctr-query replay`/`diff` and watching again. Only the latest [`KEPT`] stay.
//!
//! Not in a browser yet: it has no folder, and IndexedDB would need a store of its own
//! (`pb.rs` has one for the bests).

use bevy::prelude::*;

use crate::props::Sim;
use crate::race_on::CurrentRace;

/// A race of 8 for 3 laps is some tens of kB.
#[cfg(not(target_arch = "wasm32"))]
const KEPT: usize = 50;

#[cfg(not(target_arch = "wasm32"))]
const EXTENSION: &str = "race";

/// Saves the recording of `race`, which `sim` plays and whose ticks up to its end are settled.
pub fn save(sim: &Sim, race: &CurrentRace) {
    let Some(setup) = sim.setup.clone() else {
        return;
    };
    let mut names = race.labels.clone();
    names.truncate(race.characters.len());
    let recorder = (!race.spectating()).then_some(race.slot as u8);
    let Some(recording) = race::multi::Recording::of(&sim.race, setup, names, recorder) else {
        return;
    };
    write(race.track, &recording.to_bytes());
}

#[cfg(not(target_arch = "wasm32"))]
fn write(track: u32, bytes: &[u8]) {
    let now = jiff::Timestamp::from_second(net::score::unix_now() as i64).unwrap_or_default();
    let name = format!(
        "{}-{}.{EXTENSION}",
        now.strftime("%Y%m%d-%H%M%S"),
        crate::maps::stable_name(track),
    );
    let dir = crate::paths::recordings();
    let path = dir.join(name);
    match std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&path, bytes)) {
        Ok(()) => info!("race recorded: {} ({} bytes)", path.display(), bytes.len()),
        Err(e) => error!("race recording {}: {e}", path.display()),
    }
    prune(&dir);
}

#[cfg(target_arch = "wasm32")]
fn write(_track: u32, _bytes: &[u8]) {}

/// Deletes all but the latest [`KEPT`]: the names sort by when they were made.
#[cfg(not(target_arch = "wasm32"))]
fn prune(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<_> = entries
        .filter_map(|entry| Some(entry.ok()?.path()))
        .filter(|path| path.extension().is_some_and(|e| e == EXTENSION))
        .collect();
    paths.sort();
    let excess = paths.len().saturating_sub(KEPT);
    for path in &paths[..excess] {
        if let Err(e) = std::fs::remove_file(path) {
            warn!("{}: {e}", path.display());
        }
    }
}
