//! A relic race's relics: retail's times for each, and which a run earns. The run itself, and
//! its personal best, are a solo run's (`solo.rs`).

/// The relics a run can earn, from the slowest time to the fastest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    Sapphire,
    Gold,
    Platinum,
}

/// Retail's relic times of the racing tracks (by `LevelId`), sapphire, gold, platinum, in its
/// clock's units (960 a second): `data.RelicTime`. They are for its 3 laps.
const RELIC_TIMES: [[u32; 3]; 18] = [
    [0x13ec0, 0x102c0, 0xc6c0],
    [0x14a00, 0x10a40, 0xca80],
    [0x15180, 0xff00, 0xa8c0],
    [0x120c0, 0xf3c0, 0xc300],
    [0x12c00, 0xe880, 0xa140],
    [0x16080, 0x102c0, 0x9d80],
    [0x11940, 0xf3c0, 0xce40],
    [0x2b5c0, 0x24180, 0x1c980],
    [0x15cc0, 0xf3c0, 0x8ac0],
    [0x1af40, 0x18600, 0x15900],
    [0x24540, 0x1d100, 0x15900],
    [0x1fa40, 0x16080, 0xc6c0],
    [0x2a300, 0x23dc0, 0x1d4c0],
    [0x2e2c0, 0x29400, 0x24180],
    [0x16440, 0x10e00, 0xb7c0],
    [0x34bc0, 0x2f580, 0x29b80],
    [0x1af40, 0x189c0, 0x17700],
    [0x189c0, 0x15900, 0x12840],
];

/// Retail's relic times of `track` (a `LevelId`) with `laps`, slowest first, in the clock's units
/// like `Race::final_ms`. None on a track or lap count retail has none for.
fn times(track: u32, laps: u8) -> Option<[(Tier, i32); 3]> {
    let times = RELIC_TIMES.get(track as usize).filter(|_| laps == 3)?;
    let tiers = [Tier::Sapphire, Tier::Gold, Tier::Platinum];
    Some(std::array::from_fn(|i| (tiers[i], times[i] as i32)))
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
        // Dingo Canyon: 1:25, 1:09, 0:53 (960 units a second).
        assert_eq!(target(0, 3, None), Some((Tier::Sapphire, 81_600)));
        assert_eq!(target(0, 3, Some(81_600)), Some((Tier::Gold, 66_240)));
        assert_eq!(target(0, 3, Some(50_881)), Some((Tier::Platinum, 50_880)));
        assert_eq!(target(0, 3, Some(50_880)), None);
        // Turbo Track's platinum: 1:19.
        assert_eq!(target(17, 3, Some(80_000)), Some((Tier::Platinum, 75_840)));
        assert_eq!(target(0, 5, None), None);
        assert_eq!(target(18, 3, None), None);
    }

    #[test]
    fn a_run_earns_the_fastest_relic_it_is_under() {
        assert_eq!(earned(0, 3, 81_601), None);
        assert_eq!(earned(0, 3, 81_600), Some(Tier::Sapphire));
        assert_eq!(earned(0, 3, 66_240), Some(Tier::Gold));
        assert_eq!(earned(0, 3, 40_000), Some(Tier::Platinum));
        assert_eq!(earned(0, 5, 1), None);
    }
}
