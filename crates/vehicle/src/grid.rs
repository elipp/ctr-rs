//! Which `driver_spawns` point each driver starts on: `BOTS_Adv_AdjustDifficulty` copies a
//! table of `data.kartSpawnOrder` into `sdata->kartSpawnOrderArray` by game mode, and
//! `VehBirth_TeleportSelf` then reads `DriverSpawn[order[driverID]]`.

/// The words of `data.kartSpawnOrder`, a byte per driver (little endian, as on the PSX).
mod table {
    pub const VS_2P: [u32; 2] = [0x0402_0100, 0x0703_0605];
    pub const VS_3P_4P: [u32; 2] = [0x0302_0100, 0x0706_0504];
    pub const TIME_TRIAL: [u32; 2] = [0x0300_0201, 0x0706_0504];
    pub const DEFAULT: [u32; 2] = [0x0302_0100, 0x0706_0504];
}

/// The `gameMode1` flags that pick a table, in the order the reference tests them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridMode {
    /// `RELIC_RACE` or `TIME_TRIAL`: the lone kart starts in the middle of the track.
    TimeTrial,
    CrystalChallenge,
    BossChallenge,
    /// `ADVENTURE_CUP` with `cupID == 4`.
    PurpleCup,
    /// Any other adventure mode race: the array keeps what it had.
    Adventure,
    /// Everything else: arcade races, versus and battle.
    Arcade,
}

/// `driver id -> index into driver_spawns`.
pub type SpawnOrder = [u8; 8];

fn bytes(words: [u32; 2]) -> SpawnOrder {
    let [a, b] = words.map(u32::to_le_bytes);
    [a[0], a[1], a[2], a[3], b[0], b[1], b[2], b[3]]
}

/// The order at the start of a race or of a cup (`cup.trackIndex == 0`), as the reference
/// fills it: the versus tables by player count first, then the mode's table over them.
///
/// `previous` is the array as it was, which only an adventure race that has no table of its own
/// leaves in place.
pub fn spawn_order(mode: GridMode, players: usize, previous: SpawnOrder) -> SpawnOrder {
    let mut order = previous;
    if players == 2 {
        order = bytes(table::VS_2P);
    } else if players > 2 {
        order = bytes(table::VS_3P_4P);
    }
    match mode {
        GridMode::TimeTrial => bytes(table::TIME_TRIAL),
        GridMode::CrystalChallenge | GridMode::BossChallenge | GridMode::PurpleCup => bytes(table::DEFAULT),
        GridMode::Adventure => order,
        // `else if ((gameMode1 & ADVENTURE_MODE) == 0)`: this replaces the versus tables
        // above, so versus starts in driver order too.
        GridMode::Arcade => bytes(table::DEFAULT),
    }
}

/// The later races of a cup (`MainGameEnd`): the driver who came `n`th starts on spawn `n`.
pub fn spawn_order_from_ranks(ranks: [u8; 8]) -> SpawnOrder {
    ranks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_decode_to_driver_order() {
        let identity = [0, 1, 2, 3, 4, 5, 6, 7];
        assert_eq!(spawn_order(GridMode::Arcade, 8, [0; 8]), identity);
        assert_eq!(spawn_order(GridMode::TimeTrial, 1, [0; 8]), [1, 2, 0, 3, 4, 5, 6, 7]);
        assert_eq!(bytes(table::VS_2P), [0, 1, 2, 4, 5, 6, 3, 7]);
    }

    #[test]
    fn arcade_overrides_the_versus_tables() {
        assert_eq!(spawn_order(GridMode::Arcade, 2, [0; 8]), [0, 1, 2, 3, 4, 5, 6, 7]);
        assert_eq!(spawn_order(GridMode::Adventure, 2, [0; 8]), [0, 1, 2, 4, 5, 6, 3, 7]);
        assert_eq!(spawn_order(GridMode::Adventure, 1, [9; 8]), [9; 8]);
    }
}
