//! Telling when clients have stopped playing the same race. Each hashes its sim at the same
//! ticks (`Rollback::check_every`) and sends the hash to the others; a hash that differs from
//! the one a player made for the same tick means one of the two sims went its own way, from
//! that tick or before it.

use std::collections::BTreeMap;

/// The hashes kept for each player: a few seconds' worth, as the others' come in about that late.
const KEPT: usize = 64;

/// Our hash of a tick is not the one `slot` made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mismatch {
    pub tick: u32,
    pub slot: usize,
    /// Our hash of the tick and the player's, when the mismatch is of hashes.
    pub hashes: Option<Hashes>,
}

/// Two hashes of a tick that differ.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hashes {
    pub ours: u64,
    pub theirs: u64,
}

/// The hashes of one race, ours and the others'.
pub struct Checks {
    own: usize,
    /// By tick.
    mine: BTreeMap<u32, u64>,
    /// By slot (ours stays empty), then by tick.
    theirs: Vec<BTreeMap<u32, u64>>,
}

impl Checks {
    /// `players` in the race, ours being slot `own`.
    pub fn new(players: usize, own: usize) -> Checks {
        Checks {
            own,
            mine: BTreeMap::new(),
            theirs: vec![BTreeMap::new(); players],
        }
    }

    /// Our hash of `tick`, against the ones the others have sent for it.
    pub fn mine(&mut self, tick: u32, hash: u64) -> Vec<Mismatch> {
        self.mine.insert(tick, hash);
        keep(&mut self.mine);
        self.theirs
            .iter()
            .enumerate()
            .filter_map(|(slot, theirs)| {
                let &other = theirs.get(&tick).filter(|&&h| h != hash)?;
                Some(Mismatch {
                    tick,
                    slot,
                    hashes: Some(Hashes { ours: hash, theirs: other }),
                })
            })
            .collect()
    }

    /// The hash `slot` made of `tick`, against ours if we have made it. (If not, ours is
    /// checked when it comes.)
    pub fn theirs(&mut self, slot: usize, tick: u32, hash: u64) -> Option<Mismatch> {
        // Not a player of the race, or ourselves.
        if slot == self.own {
            return None;
        }
        let theirs = self.theirs.get_mut(slot)?;
        theirs.insert(tick, hash);
        keep(theirs);
        let &ours = self.mine.get(&tick).filter(|&&h| h != hash)?;
        Some(Mismatch {
            tick,
            slot,
            hashes: Some(Hashes { ours, theirs: hash }),
        })
    }
}

fn keep(hashes: &mut BTreeMap<u32, u64>) {
    while hashes.len() > KEPT {
        hashes.pop_first();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_that_agree_say_nothing_whoever_comes_first() {
        let mut a = Checks::new(3, 0);
        assert!(a.mine(0, 7).is_empty());
        assert_eq!(a.theirs(1, 0, 7), None);
        assert_eq!(a.theirs(2, 30, 9), None);
        assert!(a.mine(30, 9).is_empty());
    }

    #[test]
    fn a_hash_that_differs_names_the_player_whichever_side_is_late() {
        let mut a = Checks::new(3, 1);
        assert_eq!(a.theirs(2, 30, 5), None);
        assert_eq!(a.mine(30, 6), vec![Mismatch { tick: 30, slot: 2, hashes: Some(Hashes { ours: 6, theirs: 5 }) }]);
        assert_eq!(a.theirs(0, 60, 1), None);
        assert!(a.mine(60, 1).is_empty());
        assert_eq!(a.theirs(2, 60, 2), Some(Mismatch { tick: 60, slot: 2, hashes: Some(Hashes { ours: 1, theirs: 2 }) }));
    }

    #[test]
    fn strangers_and_ourselves_are_ignored() {
        let mut a = Checks::new(2, 0);
        a.mine(0, 1);
        assert_eq!(a.theirs(0, 0, 2), None);
        assert_eq!(a.theirs(5, 0, 2), None);
    }

    #[test]
    fn old_hashes_are_let_go() {
        let mut a = Checks::new(2, 0);
        for tick in 0..200 {
            a.mine(tick * 30, 1);
        }
        assert_eq!(a.mine.len(), KEPT);
        // The oldest is forgotten, so a late one for it finds nothing to differ from.
        assert_eq!(a.theirs(1, 0, 2), None);
    }
}
