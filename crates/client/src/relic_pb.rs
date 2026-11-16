//! Personal bests of relic races: the best run (`race::relic::Run`) per map and lap count, in
//! `paths::relic_pbs()` natively, in the page's IndexedDB in a browser (`web/play.html`). No time
//! is stored: a best's time is whatever our sim makes of its run, so a sim change re-times (or
//! voids) old bests instead of leaving stale times next to runs that no longer make them.

use bevy::prelude::*;
use race::relic::{Outcome, Run};

use crate::props::Sim;
use crate::race_on::CurrentRace;

/// What the run just finished did to the personal best.
pub enum Verdict {
    /// None before (or the old one no longer finishes in our sim, or can't be read).
    First,
    Better { old: Outcome },
    Worse { best: Outcome },
}

/// The name of a best, a file's or a storage key: a run's header has both, so a browser's store
/// is read back without its keys.
fn name(map_hash: u64, laps: u8) -> String {
    format!("{map_hash:016x}-{laps}laps.relic")
}

#[cfg(not(target_arch = "wasm32"))]
mod store {
    /// The best's bytes; None if there is none, an error if there is one we can't read.
    pub fn load(name: &str) -> Result<Option<Vec<u8>>, String> {
        let path = crate::paths::relic_pbs().join(name);
        match std::fs::read(&path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    pub fn save(name: &str, bytes: &[u8]) -> Result<(), String> {
        let dir = crate::paths::relic_pbs();
        std::fs::create_dir_all(&dir)
            .and_then(|()| std::fs::write(dir.join(name), bytes))
            .map_err(|e| format!("{}: {e}", dir.join(name).display()))
    }
}

/// IndexedDB is async: the page reads every best before the game starts (`ctrTakeRelicPbs`), and
/// a save goes to it without waiting.
#[cfg(target_arch = "wasm32")]
mod store {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    static SAVED: OnceLock<Mutex<HashMap<String, Vec<u8>>>> = OnceLock::new();

    fn saved() -> &'static Mutex<HashMap<String, Vec<u8>>> {
        SAVED.get_or_init(|| {
            let saved = crate::web::take_relic_pbs().into_iter().filter_map(|bytes| {
                let run = race::relic::Run::from_bytes(&bytes).ok()?;
                Some((super::name(run.map_hash, run.laps), bytes))
            });
            Mutex::new(saved.collect())
        })
    }

    pub fn load(name: &str) -> Result<Option<Vec<u8>>, String> {
        Ok(saved().lock().map_err(|e| e.to_string())?.get(name).cloned())
    }

    pub fn save(name: &str, bytes: &[u8]) -> Result<(), String> {
        saved()
            .lock()
            .map_err(|e| e.to_string())?
            .insert(name.to_string(), bytes.to_vec());
        crate::web::save_relic_pb(name, bytes);
        Ok(())
    }
}

/// The time of the stored best of the race's map and laps, re-timed on the level loaded (the
/// race's); None if there is none (or it no longer finishes, or can't be read).
pub fn best(level: &loader::lev::Level, sim: &Sim, race: &CurrentRace) -> Option<Outcome> {
    let spec = *race.specs.first()?;
    stored(level, sim, race, &name(race.hash, race.rules.laps), spec)
}

/// Replays the best stored under `name`, with `spec` if it is of the same character.
fn stored(
    level: &loader::lev::Level,
    sim: &Sim,
    race: &CurrentRace,
    name: &str,
    spec: race::KartSpec,
) -> Option<Outcome> {
    match store::load(name).and_then(|b| b.map(|b| Run::from_bytes(&b)).transpose()) {
        Ok(Some(old)) if old.map_hash == race.hash && old.laps == race.rules.laps => {
            let old_spec = if old.demo.character == spec.character {
                spec
            } else {
                let (model, _) = crate::assets::load_racer(old.demo.character);
                race::KartSpec {
                    character: old.demo.character,
                    anim_num_frames: crate::scene::anim_num_frames(&model),
                    bot: false,
                    line: None,
                }
            };
            let outcome = old.replay(level, sim.start().clone(), old_spec);
            if outcome.is_none() {
                warn!(
                    "relic personal best {name}: no longer finishes (played on sim {}, ours is {})",
                    old.sim_version,
                    race::SIM_VERSION
                );
            }
            outcome
        }
        Ok(Some(_)) => {
            warn!("relic personal best {name}: of another map or lap count");
            None
        }
        Ok(None) => None,
        Err(e) => {
            warn!("relic personal best {name}: {e}");
            None
        }
    }
}

/// Re-times the stored best and the run just finished, on the level loaded (the race's), and
/// keeps the faster. `race` is a relic race whose kart has finished, and whose finish tick is
/// settled (the demo has every pad up to there). None if the run can't be timed: that is a bug,
/// and logged.
pub fn finished(level: &loader::lev::Level, sim: &Sim, race: &CurrentRace) -> Option<Verdict> {
    let spec = *race.specs.first()?;
    let demo = sim.race.recorded()?.clone();
    let Some(run) = Run::of_finished(&sim.race, race.hash, race.rules.seed, demo) else {
        error!("relic personal best: the recording doesn't reach the finish");
        return None;
    };
    // The live race's time is not trusted on its own: a best's time is always a replay's, and a
    // recording that replays differently would be a broken best.
    let new = run.replay(level, sim.start().clone(), spec);
    let live = sim.race.final_ms(0);
    let Some(new) = new.filter(|o| Some(o.final_ms) == live) else {
        error!("relic personal best: the run replays as {new:?}, not {live:?} ms; not kept");
        return None;
    };

    let name = name(race.hash, run.laps);
    let old = stored(level, sim, race, &name, spec);

    let verdict = match old {
        Some(best) if best.final_ms <= new.final_ms => return Some(Verdict::Worse { best }),
        Some(old) => Verdict::Better { old },
        None => Verdict::First,
    };
    match store::save(&name, &run.to_bytes()) {
        Ok(()) => info!("relic personal best {name}: {} ms", new.final_ms),
        Err(e) => error!("relic personal best {name}: {e}"),
    }
    Some(verdict)
}
