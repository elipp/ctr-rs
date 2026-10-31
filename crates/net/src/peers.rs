//! What the session keeps of the other players: the lobby's guests, the race's roster, the
//! players that left it, and the link of pads to each player in it.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use iroh::EndpointId;
use iroh::endpoint::Connection;

use crate::conn::send_datagram;
use crate::wire::{InputReceiver, InputSender, Message};
use crate::realm::Publisher;
use crate::transfers::Transfers;
use crate::{KICK_AFTER, RateLimit, Ticket};
use crate::{Pad, Tick};

#[derive(Clone)]
pub(crate) enum Role {
    Host,
    Join(Ticket),
}

/// What only the lobby's host keeps: the guests, and what it takes to run the lobby and the
/// races started from it. A peer is a host while it has one of these; the rest of a session
/// (the connections, the race) is the same for every peer.
#[derive(Default)]
pub(crate) struct HostState {
    /// The players that joined, in the order they did.
    pub(crate) guests: Vec<Guest>,
    /// How fast each guest has been talking.
    pub(crate) talkers: HashMap<EndpointId, RateLimit>,
    /// The players of the race that left it. They are not told of in the lobby, and go from
    /// `guests` when the next race starts.
    pub(crate) gone: HashSet<EndpointId>,
    /// The track of a start that waits for the Hellos.
    pub(crate) start_wanted: Option<(u32, u64)>,
    /// The maps being sent.
    pub(crate) transfers: Transfers,
    /// Who is ready, before tick 0.
    pub(crate) ready_from: HashSet<EndpointId>,
    /// A race has been started and the game has not said it is over.
    pub(crate) racing: bool,
    /// The level the lobby drives on between races, as last set.
    pub(crate) lobby_track: Option<(u32, u64)>,
    /// The players that left the race, whose pads are still being settled, by slot.
    pub(crate) departures: HashMap<usize, Departure>,
    /// The listing of the lobby in the realm, if it is public.
    pub(crate) publisher: Option<Publisher>,
}

/// A player in the lobby, as the host knows it.
pub(crate) struct Guest {
    pub(crate) id: EndpointId,
    pub(crate) connection: Connection,
    /// What it said it drives, and the number of the Hello that said it; none  until it comes.
    pub(crate) hello: Option<(u32, u8)>,
    /// What it wants to be called, and the number of the Hello or Nick that said it.
    pub(crate) nick: (u32, String),
    /// It only watches: no kart, no pads (from its Hello).
    pub(crate) spectator: bool,
}

/// The race as agreed at the start.
pub(crate) struct Plan {
    /// The endpoint of each slot. The host is 0, and the spectators come after the players.
    pub(crate) roster: Vec<EndpointId>,
    /// How many of the roster race: the slots from here on are spectators.
    pub(crate) players: usize,
}

impl Plan {
    pub(crate) fn slot_of(&self, id: EndpointId) -> Option<usize> {
        self.roster.iter().position(|&r| r == id)
    }

    /// Whether `id` is in the roster as a spectator: it sends no pads, so nothing is waited for
    /// from it.
    pub(crate) fn is_spectator(&self, id: EndpointId) -> bool {
        self.slot_of(id).is_some_and(|slot| slot >= self.players)
    }
}

/// The host's tally of a player that left the race: who has not said yet what they hold of
/// its pads, and the most anyone held.
pub(crate) struct Departure {
    /// The slots to hear from.
    pub(crate) waiting: HashSet<usize>,
    /// The first tick of `pads`, and the pads: the longest run any client has.
    pub(crate) first: Tick,
    pub(crate) pads: Vec<Pad>,
}

impl Departure {
    /// `slot` holds `pads` of the player that left, from `first`.
    pub(crate) fn heard(&mut self, slot: usize, first: Tick, pads: Vec<Pad>) {
        self.waiting.remove(&slot);
        let end = |first: Tick, pads: &[Pad]| first as usize + pads.len();
        if end(first, &pads) > end(self.first, &self.pads) {
            self.first = first;
            self.pads = pads;
        }
    }
}

/// The connection to another player in the race, and the pads that go over it.
pub(crate) struct Link {
    pub(crate) connection: Connection,
    /// Our pads, until it confirms them.
    pub(crate) sender: InputSender,
    /// Its pads, as far as they have come in.
    pub(crate) receiver: InputReceiver,
    /// The first of our ticks it has not confirmed.
    pub(crate) peer_has: Tick,
    /// The race the link is for.
    pub(crate) epoch: u32,
    /// When its pads last went further (or the link began).
    pub(crate) advanced: Instant,
}

impl Link {
    pub(crate) fn new(connection: Connection, epoch: u32) -> Link {
        Link {
            epoch,
            connection,
            sender: InputSender::default(),
            receiver: InputReceiver::default(),
            peer_has: 0,
            advanced: Instant::now(),
        }
    }

    /// Sends the pads it lacks of ours, and what we have of its.
    pub(crate) fn send(&mut self) {
        let mut packet = self.sender.packet(self.peer_has, self.receiver.next());
        packet.epoch = self.epoch;
        send_datagram(&self.connection, &Message::Inputs(packet));
    }
}

/// The players to remove from a race: the ones whose pads stopped coming for [`KICK_AFTER`], and
/// are behind everyone else. (When one player hangs, the rest wait for it and stop sending as
/// well, so being silent is not enough: the one to blame is the one that is furthest back.) `own`
/// is how far our pads go, and `guests` has for each player how far theirs go and when that
/// last changed.
pub(crate) fn stalled(
    own: Tick,
    guests: &[(EndpointId, Tick, Instant)],
    now: Instant,
) -> Vec<EndpointId> {
    let back = guests.iter().map(|&(_, next, _)| next).fold(own, Tick::min);
    guests
        .iter()
        .filter(|&&(_, next, since)| next <= back && now.duration_since(since) >= KICK_AFTER)
        .map(|&(id, _, _)| id)
        .collect()
}

/// Starts the links over for the race of `epoch`: nothing sent or received yet, on the
/// connections that are up.
pub(crate) fn start_over(links: &mut HashMap<EndpointId, Link>, epoch: u32) {
    for link in links.values_mut() {
        *link = Link::new(link.connection.clone(), epoch);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn the_furthest_back_silent_player_is_removed() {
        let [a, b] = std::array::from_fn(|_| iroh::SecretKey::generate().public());
        let now = Instant::now();
        let long_ago = now - KICK_AFTER - Duration::from_secs(1);
        // The hung player is at 100; the one waiting for it went on to 160 and is silent as well.
        assert_eq!(
            stalled(160, &[(a, 100, long_ago), (b, 160, long_ago)], now),
            [a]
        );
        // Silent for a moment only.
        assert!(stalled(160, &[(a, 100, now), (b, 160, long_ago)], now).is_empty());
        // Everyone is where the host is.
        assert_eq!(stalled(100, &[(a, 100, long_ago)], now), [a]);
        // The host is the furthest back: it is not blamed, nor are the others.
        assert!(stalled(90, &[(a, 100, long_ago), (b, 160, long_ago)], now).is_empty());
    }
}
