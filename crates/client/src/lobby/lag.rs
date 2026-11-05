//! `/debug lag`: a pretend network delay on the pads of the others that come to this client.
//!
//! The pads are held back on their way to the sim and then passed on as they came, so the race
//! itself is the same and the checksums agree: only the predictions of the others' karts run
//! longer, and the rollbacks that put them right are bigger. Only the pads wait: the clock reports
//! (`Event::Lead`), the checksums and the rest of the session come as they are, and so does the
//! ping, which `/net smoothing ping` follows (use `replay` or a fixed time to try the smoothing).
//!
//! Each client sets its own, and it lasts until it is turned off: `/debug lag 75 15` on one of
//! two clients shows the other's kart as if the pads took 75 ms (± 15 ms) to come.

use std::collections::VecDeque;
use std::time::Duration;

use bevy::prelude::*;
use net::Pad;

/// The longest delay, and the most jitter, that can be set. A delay near `MAX_LEAD` (2 s, see
/// `drive/tick.rs`) would have the race waiting for the others.
pub const MAX_DELAY_MS: f64 = 1000.0;
pub const MAX_JITTER_MS: f64 = 500.0;

/// How long the pads of the others are held back: `delay`, give or take up to `jitter`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lag {
    pub delay: Duration,
    pub jitter: Duration,
}

impl std::fmt::Display for Lag {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{} ms", self.delay.as_millis())?;
        if !self.jitter.is_zero() {
            write!(f, " ± {} ms", self.jitter.as_millis())?;
        }
        Ok(())
    }
}

/// The lag set (`None` for none), with what draws the jitter.
#[derive(Resource)]
pub struct FakeLag {
    pub lag: Option<Lag>,
    rng: u64,
}

impl Default for FakeLag {
    fn default() -> FakeLag {
        use std::hash::BuildHasher;
        FakeLag {
            lag: None,
            rng: std::collections::hash_map::RandomState::new().hash_one(0u8),
        }
    }
}

impl FakeLag {
    /// How long a packet that comes now waits: the delay, moved by up to the jitter either way.
    fn wait(&mut self) -> Duration {
        let Some(lag) = self.lag else {
            return Duration::ZERO;
        };
        // (splitmix64)
        self.rng = self.rng.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.rng;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^= z >> 31;
        // In -1..1.
        let spread = (z >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0;
        let wait = lag.delay.as_secs_f64() + lag.jitter.as_secs_f64() * spread;
        Duration::from_secs_f64(wait.max(0.0))
    }
}

/// The pads held back, each packet with the time it is let through. The times only go up: a
/// packet is never let through before one that came earlier (the sim takes a player's pads in
/// order), so a jitter shows as packets that bunch up, as behind a slow one on a real line.
#[derive(Default)]
pub struct Delayed {
    queue: VecDeque<(Duration, (usize, u32, Vec<Pad>))>,
}

impl Delayed {
    /// Holds back the packets that came at `now`.
    pub fn hold(
        &mut self,
        packets: impl IntoIterator<Item = (usize, u32, Vec<Pad>)>,
        now: Duration,
        lag: &mut FakeLag,
    ) {
        for packet in packets {
            let after = self.queue.back().map_or(Duration::ZERO, |&(at, _)| at);
            self.queue.push_back(((now + lag.wait()).max(after), packet));
        }
    }

    /// The packets whose time has come by `now`: all of them once the lag is off.
    pub fn release(&mut self, now: Duration, lag: &FakeLag) -> Vec<(usize, u32, Vec<Pad>)> {
        let mut out = Vec::new();
        while let Some(&(at, _)) = self.queue.front() {
            if lag.lag.is_some() && at > now {
                break;
            }
            out.extend(self.queue.pop_front().map(|(_, packet)| packet));
        }
        out
    }

    /// Lets go of the packets of `slot` that wait.
    pub fn forget(&mut self, slot: usize) {
        self.queue.retain(|(_, (from, ..))| *from != slot);
    }
}
