//! The levels runs are verified on: a folder of `.ctrmap`s, by `Map::hash()`.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use map::Map;
use race::solo::{Outcome, Run};
use tracing::{info, warn};

pub struct Track {
    pub map: Map,
    pub number: u32,
    pub file: String,
}

pub struct Tracks {
    by_hash: HashMap<u64, Arc<Track>>,
}

impl Tracks {
    pub fn load(maps: &Path) -> Result<Tracks, String> {
        let mut by_hash = HashMap::new();
        let entries = std::fs::read_dir(maps).map_err(|e| format!("{}: {e}", maps.display()))?;
        for entry in entries {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.extension().is_none_or(|e| e != "ctrmap") {
                continue;
            }
            let loaded = Map::read(&path).and_then(|map| {
                let hash = map.hash()?;
                let number = race::track::track_of(&map)?;
                Ok((map, hash, number))
            });
            let (map, hash, number) = match loaded {
                Ok(loaded) => loaded,
                Err(e) => {
                    warn!("{}: {e}", path.display());
                    continue;
                }
            };
            let file = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
            info!("map {hash:016x} {file} ({})", map.name);
            if let Some(other) = by_hash.insert(hash, Arc::new(Track { map, number, file })) {
                warn!("{} is the same map as {}", path.display(), other.file);
            }
        }
        if by_hash.is_empty() {
            return Err(format!("no maps in {}", maps.display()));
        }
        Ok(Tracks { by_hash })
    }

    pub fn get(&self, hash: u64) -> Option<Arc<Track>> {
        self.by_hash.get(&hash).cloned()
    }
}

/// `run` replayed on `track` as the client plays it. Slow (a whole race): off the async threads.
pub fn replay(track: &Track, run: &Run, anim_num_frames: [u16; 8]) -> Option<Outcome> {
    let made = race::track::sim_level(&track.map, track.number, run.kind.lev_mode());
    let objects = instance::Objects::new(&made.level, made.dram);
    let spec = race::KartSpec {
        character: run.demo.character,
        anim_num_frames,
        bot: false,
        line: None,
    };
    run.replay(&made.level, objects, spec)
}
