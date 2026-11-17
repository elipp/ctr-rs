//! Signed writes: the signature check, the cache of signatures already used, and the per-key
//! submit limits. The key that signed is the player; nothing else claims an identity.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use score::{Action, Auth, AuthError, MAX_SKEW, PublicKey};

/// A signature is refused again for this long: past it, its `ts` is stale anyway.
const SEEN_FOR: u64 = 2 * MAX_SKEW;

/// Submits a key may make in [`SUBMIT_WINDOW`] seconds.
const SUBMITS_PER_WINDOW: usize = 20;
const SUBMIT_WINDOW: u64 = 3600;

#[derive(Default)]
pub struct Seen(Mutex<HashMap<[u8; 64], u64>>);

impl Seen {
    /// The key that signed `body` for `action`, if the signature holds, is fresh, and wasn't
    /// used before (it is remembered until it would be stale).
    pub fn check(&self, auth: &Auth, action: Action, body: &[u8], now: u64) -> Result<PublicKey, AuthError> {
        auth.fresh(now)?;
        let key = auth.verify(action, body)?;
        let mut seen = self.0.lock().unwrap();
        seen.retain(|_, until| *until > now);
        if seen.insert(auth.sig, now + SEEN_FOR).is_some() {
            return Err(AuthError::Replayed);
        }
        Ok(key)
    }
}

#[derive(Default)]
struct Submits {
    recent: VecDeque<u64>,
    busy: bool,
}

/// Per key: [`SUBMITS_PER_WINDOW`] an hour, one at a time.
#[derive(Default, Clone)]
pub struct Limits(Arc<Mutex<HashMap<[u8; 32], Submits>>>);

/// A submit under way: the key's next may start once this is dropped.
pub struct Slot {
    limits: Limits,
    key: [u8; 32],
}

impl Limits {
    pub fn start(&self, key: &PublicKey, now: u64) -> Option<Slot> {
        let mut all = self.0.lock().unwrap();
        all.retain(|_, s| s.busy || s.recent.back().is_some_and(|&t| t + SUBMIT_WINDOW > now));
        let submits = all.entry(*key.as_bytes()).or_default();
        while submits.recent.front().is_some_and(|&t| t + SUBMIT_WINDOW <= now) {
            submits.recent.pop_front();
        }
        if submits.busy || submits.recent.len() >= SUBMITS_PER_WINDOW {
            return None;
        }
        submits.busy = true;
        submits.recent.push_back(now);
        Some(Slot { limits: self.clone(), key: *key.as_bytes() })
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        if let Some(s) = self.limits.0.lock().unwrap().get_mut(&self.key) {
            s.busy = false;
        }
    }
}
