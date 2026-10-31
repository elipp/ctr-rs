//! A limit on how fast something may be done: a bucket of tokens that fills at a steady rate, and
//! one is taken for each go. A burst up to the bucket's size is allowed, and then one go per
//! refill.

use n0_future::time::Instant;

/// How many chat lines a player can say at once, and how often it gets another after that.
pub const CHAT_BURST: u32 = 5;
pub const CHAT_PER_SECOND: f32 = 0.5;

/// What the host allows beyond [`CHAT_BURST`]: lines that left a player in time can arrive
/// bunched up, and the player is not at fault for it.
const HOST_SLACK: u32 = 2;

#[derive(Clone, Debug)]
pub struct RateLimit {
    burst: f32,
    per_second: f32,
    tokens: f32,
    at: Instant,
}

impl RateLimit {
    /// A full bucket of `burst` that gets `per_second` tokens a second.
    pub fn new(burst: u32, per_second: f32, now: Instant) -> RateLimit {
        RateLimit {
            burst: burst as f32,
            per_second,
            tokens: burst as f32,
            at: now,
        }
    }

    /// The limit a player puts on its own chat.
    pub fn chat(now: Instant) -> RateLimit {
        RateLimit::new(CHAT_BURST, CHAT_PER_SECOND, now)
    }

    /// The limit the host puts on a guest's chat: a little more than the guest keeps to.
    pub fn chat_for_host(now: Instant) -> RateLimit {
        RateLimit::new(CHAT_BURST + HOST_SLACK, CHAT_PER_SECOND, now)
    }

    /// Whether a go is allowed at `now`, which takes a token if so.
    pub fn allow(&mut self, now: Instant) -> bool {
        let passed = now.saturating_duration_since(self.at).as_secs_f32();
        self.at = now;
        self.tokens = (self.tokens + passed * self.per_second).min(self.burst);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

impl Default for RateLimit {
    fn default() -> RateLimit {
        RateLimit::chat(Instant::now())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn a_burst_goes_through_and_then_one_per_refill() {
        let start = Instant::now();
        let mut limit = RateLimit::new(3, 0.5, start);
        assert!((0..3).all(|_| limit.allow(start)));
        assert!(!limit.allow(start));
        // Half a token is not enough.
        assert!(!limit.allow(start + Duration::from_secs(1)));
        assert!(limit.allow(start + Duration::from_secs(2)));
        assert!(!limit.allow(start + Duration::from_secs(2)));
    }

    #[test]
    fn the_bucket_does_not_fill_past_its_size() {
        let start = Instant::now();
        let mut limit = RateLimit::new(2, 1.0, start);
        let later = start + Duration::from_secs(3600);
        assert!(limit.allow(later));
        assert!(limit.allow(later));
        assert!(!limit.allow(later));
    }

    #[test]
    fn the_host_allows_a_little_more() {
        let now = Instant::now();
        let mut guest = RateLimit::chat(now);
        let mut host = RateLimit::chat_for_host(now);
        let count = |limit: &mut RateLimit| (0..20).filter(|_| limit.allow(now)).count();
        assert_eq!(count(&mut guest), CHAT_BURST as usize);
        assert_eq!(count(&mut host), (CHAT_BURST + HOST_SLACK) as usize);
    }
}
