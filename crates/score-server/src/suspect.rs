//! Near-duplicates (TIME-TRIAL-RELIC-LEADERBOARDS.md, "Run theft"): a copied ghost with one
//! meaningless input changed gets past `runs_pads`, but still changes pads on almost the same
//! ticks as the run it was copied from. Only marked, never refused: two good players can drive
//! the same line, and the operator decides.

use std::collections::HashSet;

use race::solo::Run;

/// How near in time an earlier run must be to be compared: 3 ticks, of 32 clock units each.
pub const WITHIN: i32 = 3 * 32;

/// The most earlier runs a submit is compared with, so that a crowded time can't make a submit
/// slow.
pub const CANDIDATES: u32 = 32;

/// Of the pad changes of the longer stream, the percentage whose tick the other has too.
const SAME_PERCENT: usize = 95;

/// Whether `a` and `b` change pads on mostly the same ticks. By tick alone: a thief would rather
/// nudge a stick or a button than the timing, which is what makes the run.
pub fn alike(a: &Run, b: &Run) -> bool {
    let ticks = |run: &Run| run.demo.changes.iter().map(|c| c.tick).collect::<HashSet<u32>>();
    let (a, b) = (ticks(a), ticks(b));
    let longer = a.len().max(b.len());
    longer > 0 && a.intersection(&b).count() * 100 >= longer * SAME_PERCENT
}

#[cfg(test)]
mod tests {
    use super::*;
    use race::solo::Kind;
    use vehicle::demo::{Change, Demo};

    fn run(ticks: impl IntoIterator<Item = u32>) -> Run {
        let changes = ticks
            .into_iter()
            .map(|tick| Change { tick, buttons_held: tick, stick_lx: 0, stick_ly: 0, stick_ry: 0 })
            .collect();
        Run {
            sim_version: race::SIM_VERSION,
            kind: Kind::Relic,
            map_hash: 0,
            laps: 3,
            seed: 0,
            demo: Demo { character: 0, changes, length: 4000 },
        }
    }

    #[test]
    fn a_nudged_copy_is_alike() {
        let original = run((0..200).map(|t| t * 10));
        let mut copy = run((0..200).map(|t| t * 10));
        copy.demo.changes[17].stick_lx = 1;
        assert!(alike(&original, &copy));
        let shifted = run((0..200).map(|t| t * 10 + 1));
        assert!(!alike(&original, &shifted));
    }

    #[test]
    fn a_few_more_changes_still_count() {
        let original = run((0..200).map(|t| t * 10));
        let extra = run((0..200).map(|t| t * 10).chain((0..5).map(|t| t * 10 + 5)));
        assert!(alike(&original, &extra));
        let many_more = run((0..200).map(|t| t * 10).chain((0..20).map(|t| t * 10 + 5)));
        assert!(!alike(&original, &many_more));
    }
}
