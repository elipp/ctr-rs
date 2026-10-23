//! Rollback: playing a tick before everyone's pad for it is in, and putting it right when the
//! pad that was missing arrives.
//!
//! A [`Rollback`] holds a simulation and the pads of its players. A pad that is not in yet is
//! predicted as the last one the player made. Each tick played on a prediction keeps the state
//! it started from; when the real pad turns out to differ, the simulation goes back to that
//! state and plays on from there with what is known now. Ticks whose pads are all in need
//! nothing kept, so the snapshots are only as many as the ticks that wait on someone.
//!
//! What the simulation does for the outside (sounds, particles) comes out of the first play of a
//! tick only. The plays that put a tick right give theirs up: the sound has been heard.

use std::collections::VecDeque;

use crate::Pad;

/// A simulation to roll back: a state that steps from everyone's pads, and is `Clone` so that a
/// copy of it is a snapshot. Stepping must depend on nothing but the state and the pads.
pub trait Simulation: Clone {
    /// What a step reads besides the state, such as the level.
    type Context: ?Sized;
    /// What a step asks of the outside.
    type Output;

    /// One tick, from the pad of each player (by slot).
    fn step(&mut self, cx: &Self::Context, pads: &[Pad]) -> Self::Output;

    /// A hash of the state, to compare with other clients' (see [`Rollback::check_every`]).
    fn checksum(&self) -> u64;
}

/// A pad came for a tick after the one that should have been next from that player.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hole {
    /// The first tick we have no pad for: what the sender has to start from.
    pub expected: u32,
}

/// What [`Rollback::advance`] did.
#[derive(Debug)]
pub struct Advance<T> {
    /// The ticks played again because a pad turned out to differ from the prediction.
    pub replayed: u32,
    pub step: Step<T>,
}

#[derive(Debug)]
pub enum Step<T> {
    /// A new tick was played; this is what it asked of the outside.
    Played(T),
    /// Our own pad for the tick is not in: [`Rollback::confirm`] it first.
    NoInput,
    /// The window of unconfirmed ticks is full: nothing more is played until pads arrive.
    Full,
}

pub struct Rollback<S: Simulation> {
    /// The simulation at `tick`.
    now: S,
    /// The ticks played.
    played: u32,
    /// The tick of the oldest snapshot: everything before it has all its pads and is final.
    base: u32,
    /// `snapshots[i]` is the state before tick `base + i` was played.
    snapshots: VecDeque<S>,
    /// `used[i]` is the pads tick `base + i` was played with.
    used: VecDeque<Vec<Pad>>,
    /// The pads we know for each slot, from tick `base`.
    known: Vec<VecDeque<Pad>>,
    /// The latest pad known for each slot: what the ticks past the known ones are played with.
    last: Vec<Pad>,
    /// Our slot: its pads are never predicted, so a tick waits for them.
    local: Option<usize>,
    /// The most unconfirmed ticks that are kept.
    window: usize,
    /// The earliest played tick whose pad turned out different.
    dirty: Option<u32>,
    /// A hash is taken of the state at every multiple of this many ticks; 0 for none.
    every: u32,
    /// The hashes taken and not yet collected: (tick, hash of the state before that tick).
    sums: Vec<(u32, u64)>,
}

impl<S: Simulation> Rollback<S> {
    /// `initial` is the simulation at tick 0, with `players` pads a tick, `local` of them ours.
    /// At most `window` ticks are played past the last one everyone's pad is in for.
    pub fn new(initial: S, players: usize, local: Option<usize>, window: usize) -> Rollback<S> {
        Rollback {
            now: initial,
            played: 0,
            base: 0,
            snapshots: VecDeque::new(),
            used: VecDeque::new(),
            known: vec![VecDeque::new(); players],
            last: vec![Pad::default(); players],
            local,
            window,
            dirty: None,
            every: 0,
            sums: Vec::new(),
        }
    }

    /// Hashes the state before every `ticks`th tick (the first is tick 0), once that tick is
    /// final: every pad before it is in, so the hash is the same on every client that plays the
    /// same race. [`Rollback::take_checksums`] hands them out.
    pub fn check_every(mut self, ticks: u32) -> Rollback<S> {
        self.every = ticks;
        self
    }

    /// The hashes taken since the last call, by tick, each once.
    pub fn take_checksums(&mut self) -> Vec<(u32, u64)> {
        std::mem::take(&mut self.sums)
    }

    /// The simulation as far as it has been played.
    pub fn state(&self) -> &S {
        &self.now
    }

    /// The ticks played.
    pub fn tick(&self) -> u32 {
        self.played
    }

    /// The number of players.
    pub fn players(&self) -> usize {
        self.known.len()
    }

    /// The first tick that has no pad in from `slot`.
    pub fn confirmed_until(&self, slot: usize) -> u32 {
        self.base + self.known[slot].len() as u32
    }

    /// The first tick that is not final: the ticks before it have everyone's pad.
    pub fn settled(&self) -> u32 {
        (0..self.known.len())
            .map(|slot| self.confirmed_until(slot))
            .min()
            .map_or(self.played, |until| until.min(self.played))
    }

    /// The ticks played on a prediction (or a pad that is not known to be right yet).
    pub fn unsettled(&self) -> u32 {
        self.played - self.settled()
    }

    /// The snapshots kept.
    pub fn kept(&self) -> usize {
        self.snapshots.len()
    }

    /// Whether a correction waits for [`Rollback::advance`].
    pub fn dirty(&self) -> bool {
        self.dirty.is_some()
    }

    /// The pad of `slot` for `tick`. It has to be the next one we do not have (the ones before
    /// it are taken as repeats), or [`Hole`] says where to start again.
    pub fn confirm(&mut self, slot: usize, tick: u32, pad: Pad) -> Result<(), Hole> {
        let expected = self.confirmed_until(slot);
        if tick < expected {
            return Ok(());
        }
        if tick > expected {
            return Err(Hole { expected });
        }
        self.known[slot].push_back(pad);
        self.last[slot] = pad;
        // A tick that was played has to have been played with this pad.
        if tick < self.played && self.used[(tick - self.base) as usize][slot] != pad {
            self.dirty = Some(self.dirty.map_or(tick, |d| d.min(tick)));
        }
        Ok(())
    }

    /// The pads of `slot` from tick `first` on, some of which we may have: a packet that repeats
    /// the last few, say.
    pub fn confirm_all(&mut self, slot: usize, first: u32, pads: &[Pad]) -> Result<(), Hole> {
        for (tick, pad) in (first..).zip(pads) {
            self.confirm(slot, tick, *pad)?;
        }
        Ok(())
    }

    /// Puts right the ticks that were played on a pad that turned out different. The count is
    /// the ticks played again.
    pub fn correct(&mut self, cx: &S::Context) -> u32 {
        let Some(from) = self.dirty.take() else {
            return 0;
        };
        let target = self.played;
        let index = (from - self.base) as usize;
        self.now = self.snapshots[index].clone();
        self.snapshots.truncate(index);
        self.used.truncate(index);
        self.played = from;
        while self.played < target {
            self.play(cx);
        }
        target - from
    }

    /// Corrects what needs it, then plays the next tick if it can.
    pub fn advance(&mut self, cx: &S::Context) -> Advance<S::Output> {
        let replayed = self.correct(cx);
        self.trim();
        let step = if self
            .local
            .is_some_and(|l| self.confirmed_until(l) <= self.played)
        {
            Step::NoInput
        } else if self.snapshots.len() >= self.window {
            Step::Full
        } else {
            Step::Played(self.play(cx))
        };
        Advance { replayed, step }
    }

    /// The pads of tick `tick` (not before `base`): those known, then the last of each slot.
    fn pads_for(&self, tick: u32) -> Vec<Pad> {
        let index = (tick - self.base) as usize;
        (0..self.known.len())
            .map(|slot| {
                self.known[slot]
                    .get(index)
                    .copied()
                    .unwrap_or(self.last[slot])
            })
            .collect()
    }

    /// Whether every slot has a pad in for `tick`.
    fn all_known(&self, tick: u32) -> bool {
        (0..self.known.len()).all(|slot| self.confirmed_until(slot) > tick)
    }

    fn play(&mut self, cx: &S::Context) -> S::Output {
        let pads = self.pads_for(self.played);
        // A tick with every pad in, after ones that are final, can never be put right: it is
        // final at once and keeps nothing.
        if self.snapshots.is_empty() && self.all_known(self.played) {
            if self.every != 0 && self.played % self.every == 0 {
                self.sums.push((self.played, self.now.checksum()));
            }
            let output = self.now.step(cx, &pads);
            for known in &mut self.known {
                known.pop_front();
            }
            self.played += 1;
            self.base += 1;
            return output;
        }
        self.snapshots.push_back(self.now.clone());
        let output = self.now.step(cx, &pads);
        self.used.push_back(pads);
        self.played += 1;
        output
    }

    /// Lets go of the snapshots of the ticks that are final.
    fn trim(&mut self) {
        let settled = self.settled();
        while self.base < settled {
            if self.every != 0 && self.base % self.every == 0 {
                self.sums.push((self.base, self.snapshots[0].checksum()));
            }
            self.snapshots.pop_front();
            self.used.pop_front();
            for known in &mut self.known {
                known.pop_front();
            }
            self.base += 1;
        }
    }
}

impl<S: Simulation> std::ops::Deref for Rollback<S> {
    type Target = S;

    fn deref(&self) -> &S {
        &self.now
    }
}
