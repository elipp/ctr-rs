//! Personal bests of solo runs: the best run (`race::solo::Run`) per kind, map and lap count, in
//! `paths::pbs()` natively, in the page's IndexedDB in a browser (`web/play.html`). No time
//! is stored: a best's time is whatever our sim makes of its run, so a sim change re-times (or
//! voids) old bests instead of leaving stale times next to runs that no longer make them.

use bevy::prelude::*;
use race::solo::{Kind, Outcome, Run};

use crate::props::Sim;
use crate::race_on::CurrentRace;

/// What the run just finished did to the personal best.
pub enum Verdict {
    /// None before (or the old one no longer finishes in our sim, or can't be read).
    First,
    Better { old: Outcome },
    Worse { best: Outcome },
}

/// The name of a best's files without the extension, a file's or a storage key: a run's header
/// and a [`Timed`] both have its parts, so a browser's store is read back without its keys.
fn stem(kind: Kind, map_hash: u64, laps: u8) -> String {
    let kind = match kind {
        Kind::Relic => "relic",
        Kind::TimeTrial => "trial",
    };
    format!("{map_hash:016x}-{laps}laps-{kind}")
}

/// What a best's run timed at, kept next to it (`.time` beside `.run`) so that a best can be
/// shown without replaying it: on a track in a free drive the relic LEV isn't even loaded. It is
/// only believed for the run bytes it was made of (`run_digest`) and our `SIM_VERSION`; anything
/// else replays the run and writes it anew, so the times-are-the-sim's rule above still holds as
/// long as sim changes bump `SIM_VERSION`.
#[derive(Debug, PartialEq)]
struct Timed {
    kind: Kind,
    map_hash: u64,
    laps: u8,
    sim_version: u32,
    run_digest: u64,
    /// None: the run no longer finishes. Kept too, or a dead best would replay on every look.
    outcome: Option<Outcome>,
}

impl Timed {
    const MAGIC: &[u8; 4] = b"CTRT";
    const LEN: usize = 36;

    fn to_bytes(&self) -> Vec<u8> {
        let mut b = Self::MAGIC.to_vec();
        b.push(match self.kind {
            Kind::Relic => 0,
            Kind::TimeTrial => 1,
        });
        b.extend(self.map_hash.to_le_bytes());
        b.push(self.laps);
        b.extend(self.sim_version.to_le_bytes());
        b.extend(self.run_digest.to_le_bytes());
        let o = self.outcome;
        b.push(o.is_some() as u8);
        b.extend(o.map_or(0, |o| o.final_ms).to_le_bytes());
        b.push(o.is_some_and(|o| o.perfect) as u8);
        b.extend(o.map_or(0, |o| o.finish_tick).to_le_bytes());
        debug_assert_eq!(b.len(), Self::LEN);
        b
    }

    fn from_bytes(bytes: &[u8]) -> Option<Timed> {
        let rest = bytes.strip_prefix(Self::MAGIC).filter(|_| bytes.len() == Self::LEN)?;
        let u32_at = |at: usize| u32::from_le_bytes(rest[at..at + 4].try_into().unwrap());
        let u64_at = |at: usize| u64::from_le_bytes(rest[at..at + 8].try_into().unwrap());
        let kind = match rest[0] {
            0 => Kind::Relic,
            1 => Kind::TimeTrial,
            _ => return None,
        };
        let outcome = (rest[22] != 0).then(|| Outcome {
            final_ms: u32_at(23) as i32,
            perfect: rest[27] != 0,
            finish_tick: u32_at(28),
        });
        Some(Timed {
            kind,
            map_hash: u64_at(1),
            laps: rest[9],
            sim_version: u32_at(10),
            run_digest: u64_at(14),
            outcome,
        })
    }

    fn of(run: &Run, run_bytes: &[u8], outcome: Option<Outcome>) -> Timed {
        Timed {
            kind: run.kind,
            map_hash: run.map_hash,
            laps: run.laps,
            sim_version: race::SIM_VERSION,
            run_digest: map::fnv1a(run_bytes),
            outcome,
        }
    }

    fn save(&self) {
        let name = format!("{}.time", stem(self.kind, self.map_hash, self.laps));
        if let Err(e) = store::save(&name, &self.to_bytes()) {
            warn!("personal best {name}: {e}");
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod store {
    /// The best's bytes; None if there is none, an error if there is one we can't read.
    pub fn load(name: &str) -> Result<Option<Vec<u8>>, String> {
        let path = crate::paths::pbs().join(name);
        match std::fs::read(&path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    pub fn save(name: &str, bytes: &[u8]) -> Result<(), String> {
        let dir = crate::paths::pbs();
        std::fs::create_dir_all(&dir)
            .and_then(|()| std::fs::write(dir.join(name), bytes))
            .map_err(|e| format!("{}: {e}", dir.join(name).display()))
    }
}

/// IndexedDB is async: the page reads every best before the game starts (`ctrTakePbs`), and
/// a save goes to it without waiting.
#[cfg(target_arch = "wasm32")]
mod store {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    static SAVED: OnceLock<Mutex<HashMap<String, Vec<u8>>>> = OnceLock::new();

    fn saved() -> &'static Mutex<HashMap<String, Vec<u8>>> {
        SAVED.get_or_init(|| {
            let saved = crate::web::take_pbs().into_iter().filter_map(|bytes| {
                let name = if let Some(t) = super::Timed::from_bytes(&bytes) {
                    format!("{}.time", super::stem(t.kind, t.map_hash, t.laps))
                } else {
                    let run = race::solo::Run::from_bytes(&bytes).ok()?;
                    format!("{}.run", super::stem(run.kind, run.map_hash, run.laps))
                };
                Some((name, bytes))
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
        crate::web::save_pb(name, bytes);
        Ok(())
    }
}

/// The time of the stored best of the solo race's kind, map and laps, on the level loaded (the
/// race's); None if there is none (or it no longer finishes, or can't be read).
pub fn best(level: &loader::lev::Level, sim: &Sim, race: &CurrentRace) -> Option<Outcome> {
    let spec = *race.specs.first()?;
    let kind = race.solo?;
    stored(kind, race.hash, race.rules.laps, |run| {
        run.replay(level, sim.start().clone(), spec_for(run, spec))
    })
}

/// The time of the stored best of `kind` over `laps` on track `id`, whatever level is loaded: a
/// replay, if one is needed, builds the race's level off the scene (`assets::sim_level`).
pub fn best_on(id: u32, kind: Kind, laps: u8) -> Option<Outcome> {
    let hash = crate::maps::hash(id)?;
    stored(kind, hash, laps, |run| {
        let map = crate::maps::get(id)?;
        let (level, dram, _) = crate::assets::sim_level(map, id, kind.lev_mode());
        let objects = instance::Objects::new(&level, dram);
        run.replay(&level, objects, racer_spec(run.demo.character))
    })
}

/// `spec`, if the run is of its character; else the run's own character's.
fn spec_for(run: &Run, spec: race::KartSpec) -> race::KartSpec {
    if run.demo.character == spec.character {
        spec
    } else {
        racer_spec(run.demo.character)
    }
}

fn racer_spec(character: usize) -> race::KartSpec {
    let (model, _) = crate::assets::load_racer(character);
    race::KartSpec {
        character,
        anim_num_frames: crate::scene::anim_num_frames(&model),
        bot: false,
        line: None,
    }
}

/// The best stored for `kind`, `map_hash` and `laps`: its [`Timed`], or if that is missing or
/// stale, what `replay` makes of it (and the `Timed` written for next time).
fn stored(
    kind: Kind,
    map_hash: u64,
    laps: u8,
    replay: impl FnOnce(&Run) -> Option<Outcome>,
) -> Option<Outcome> {
    let stem = stem(kind, map_hash, laps);
    let name = format!("{stem}.run");
    let bytes = match store::load(&name) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return None,
        Err(e) => {
            warn!("personal best {name}: {e}");
            return None;
        }
    };
    let run = match Run::from_bytes(&bytes) {
        Ok(run) if run.map_hash == map_hash && run.laps == laps && run.kind == kind => run,
        Ok(_) => {
            warn!("personal best {name}: of another kind, map or lap count");
            return None;
        }
        Err(e) => {
            warn!("personal best {name}: {e}");
            return None;
        }
    };
    let fresh = Timed::of(&run, &bytes, None);
    let timed = store::load(&format!("{stem}.time")).ok().flatten();
    if let Some(timed) = timed.as_deref().and_then(Timed::from_bytes)
        && timed == (Timed { outcome: timed.outcome, ..fresh })
    {
        return timed.outcome;
    }
    let outcome = replay(&run);
    if outcome.is_none() {
        warn!(
            "personal best {name}: no longer finishes (played on sim {}, ours is {})",
            run.sim_version,
            race::SIM_VERSION
        );
    }
    Timed { outcome, ..fresh }.save();
    outcome
}

/// Re-times the run just finished on the level loaded (the race's), and keeps it if it beats
/// the stored best. `race` is a solo race whose kart has finished, and whose finish tick is
/// settled (the demo has every pad up to there). None if the run can't be timed: that is a bug,
/// and logged.
pub fn finished(level: &loader::lev::Level, sim: &Sim, race: &CurrentRace) -> Option<Verdict> {
    let spec = *race.specs.first()?;
    let demo = sim.race.recorded()?.clone();
    let Some(run) = Run::of_finished(&sim.race, race.hash, race.rules.seed, demo) else {
        error!("personal best: the recording doesn't reach the finish");
        return None;
    };
    // The live race's time is not trusted on its own: a best's time is always a replay's, and a
    // recording that replays differently would be a broken best.
    let new = run.replay(level, sim.start().clone(), spec);
    let live = sim.race.final_ms(0);
    let Some(new) = new.filter(|o| Some(o.final_ms) == live) else {
        error!("personal best: the run replays as {new:?}, not {live:?} ms; not kept");
        return None;
    };

    let old = best(level, sim, race);
    let verdict = match old {
        Some(best) if best.final_ms <= new.final_ms => return Some(Verdict::Worse { best }),
        Some(old) => Verdict::Better { old },
        None => Verdict::First,
    };
    let name = format!("{}.run", stem(run.kind, race.hash, run.laps));
    let bytes = run.to_bytes();
    match store::save(&name, &bytes) {
        Ok(()) => {
            info!("personal best {name}: {} ms", new.final_ms);
            Timed::of(&run, &bytes, Some(new)).save();
        }
        Err(e) => error!("personal best {name}: {e}"),
    }
    Some(verdict)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_time_reads_back_as_written() {
        for outcome in [
            None,
            Some(Outcome {
                final_ms: 81_600,
                perfect: true,
                finish_tick: 2_550,
            }),
        ] {
            let timed = Timed {
                kind: Kind::TimeTrial,
                map_hash: 0x0123_4567_89ab_cdef,
                laps: 3,
                sim_version: 12,
                run_digest: 0xfedc_ba98_7654_3210,
                outcome,
            };
            assert_eq!(Timed::from_bytes(&timed.to_bytes()), Some(timed));
        }
    }
}
