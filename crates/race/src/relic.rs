//! A relic race's relics: retail's times for each, and which a run earns. The run itself, and
//! its personal best, are a solo run's (`solo.rs`).

use crate::clock::SECOND;

/// The relics a run can earn, from the slowest time to the fastest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    Sapphire,
    Gold,
    Platinum,
}

/// Retail's relic times of the racing tracks (by `LevelId`), sapphire, gold, platinum, in seconds:
/// `data.RelicTime`. They are for its 3 laps.
const RELIC_SECONDS: [[i32; 3]; 18] = [
    [85, 69, 53],
    [88, 71, 54],
    [90, 68, 45],
    [77, 65, 52],
    [80, 62, 43],
    [94, 69, 42],
    [75, 65, 55],
    [185, 154, 122],
    [93, 65, 37],
    [115, 104, 92],
    [155, 124, 92],
    [135, 94, 53],
    [180, 153, 125],
    [197, 176, 154],
    [95, 72, 49],
    [225, 202, 178],
    [115, 105, 100],
    [105, 92, 79],
];

/// Retail's relic times of `track` (a `LevelId`) with `laps`, slowest first, in the clock's units
/// like `Race::final_ms`. None on a track or lap count retail has none for.
fn times(track: u32, laps: u8) -> Option<[(Tier, i32); 3]> {
    let seconds = RELIC_SECONDS.get(track as usize).filter(|_| laps == 3)?;
    let tiers = [Tier::Sapphire, Tier::Gold, Tier::Platinum];
    Some(std::array::from_fn(|i| (tiers[i], seconds[i] * SECOND)))
}

/// The relic to go for on `track` with `laps`, and its time: the slowest one `best`
/// (`Outcome::final_ms`) hasn't earned. Retail earns a relic at its time or under
/// (`RR_EndEvent_UnlockAward`). None where retail has no times, or once the platinum is earned.
pub fn target(track: u32, laps: u8, best: Option<i32>) -> Option<(Tier, i32)> {
    times(track, laps)?
        .into_iter()
        .find(|&(_, time)| best.is_none_or(|best| best > time))
}

/// The best relic a run of `final_ms` earns on `track` with `laps`.
pub fn earned(track: u32, laps: u8, final_ms: i32) -> Option<Tier> {
    times(track, laps)?
        .into_iter()
        .rev()
        .find(|&(_, time)| final_ms <= time)
        .map(|(tier, _)| tier)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_target_is_the_next_relic_not_earned() {
        // Dingo Canyon: 1:25, 1:09, 0:53.
        assert_eq!(target(0, 3, None), Some((Tier::Sapphire, 85 * SECOND)));
        assert_eq!(target(0, 3, Some(85 * SECOND)), Some((Tier::Gold, 69 * SECOND)));
        assert_eq!(target(0, 3, Some(53 * SECOND + 1)), Some((Tier::Platinum, 53 * SECOND)));
        assert_eq!(target(0, 3, Some(53 * SECOND)), None);
        // Turbo Track's platinum: 1:19.
        assert_eq!(target(17, 3, Some(80_000)), Some((Tier::Platinum, 79 * SECOND)));
        assert_eq!(target(0, 5, None), None);
        assert_eq!(target(18, 3, None), None);
    }

    #[test]
    fn a_run_earns_the_fastest_relic_it_is_under() {
        assert_eq!(earned(0, 3, 85 * SECOND + 1), None);
        assert_eq!(earned(0, 3, 85 * SECOND), Some(Tier::Sapphire));
        assert_eq!(earned(0, 3, 69 * SECOND), Some(Tier::Gold));
        assert_eq!(earned(0, 3, 40_000), Some(Tier::Platinum));
        assert_eq!(earned(0, 5, 1), None);
    }
}
