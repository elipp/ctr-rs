//! The maps that the host is sending: one to a guest at a time, and only a few at once, so that
//! a guest that asks over and over cannot make the host copy and send a map for each ask.

use std::collections::HashSet;

use iroh::EndpointId;

/// The most maps a host sends at once, to all its guests together.
pub(crate) const MAX_TRANSFERS: usize = 3;

/// A map (its track and hash) on its way to a guest.
pub(crate) type Transfer = (EndpointId, u32, u64);

/// What to do with a guest's ask for a map.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// Send it, and [`Transfers::finish`] when it is done.
    Send,
    /// It is on its way already. Say nothing: a refusal would make the guest drop the map when
    /// it arrives.
    Duplicate,
    /// Too many are being sent. Refuse, so that the guest can ask again later.
    Busy,
}

#[derive(Default)]
pub(crate) struct Transfers {
    active: HashSet<Transfer>,
}

impl Transfers {
    pub fn begin(&mut self, transfer: Transfer) -> Verdict {
        if self.active.contains(&transfer) {
            Verdict::Duplicate
        } else if self.active.len() >= MAX_TRANSFERS {
            Verdict::Busy
        } else {
            self.active.insert(transfer);
            Verdict::Send
        }
    }

    pub fn finish(&mut self, transfer: &Transfer) {
        self.active.remove(transfer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guest(n: u8) -> EndpointId {
        iroh::SecretKey::from_bytes(&[n; 32]).public()
    }

    #[test]
    fn a_map_is_sent_to_a_guest_once_at_a_time() {
        let mut transfers = Transfers::default();
        let one = (guest(1), 0x8000_0001, 5);
        assert_eq!(transfers.begin(one), Verdict::Send);
        assert_eq!(transfers.begin(one), Verdict::Duplicate);
        // Another guest, or another map, is another transfer.
        assert_eq!(transfers.begin((guest(2), 0x8000_0001, 5)), Verdict::Send);
        transfers.finish(&one);
        assert_eq!(transfers.begin(one), Verdict::Send);
    }

    #[test]
    fn only_so_many_are_sent_at_once() {
        let mut transfers = Transfers::default();
        for n in 0..MAX_TRANSFERS as u8 {
            assert_eq!(transfers.begin((guest(n), 1, 1)), Verdict::Send);
        }
        let late = (guest(99), 1, 1);
        assert_eq!(transfers.begin(late), Verdict::Busy);
        transfers.finish(&(guest(0), 1, 1));
        assert_eq!(transfers.begin(late), Verdict::Send);
    }
}
