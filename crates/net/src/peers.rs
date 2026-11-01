//! What the session keeps of the other players: the lobby's guests, the race's roster, the
//! players that left it, and the link of pads to each player in it.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use iroh::endpoint::Connection;
use iroh::{Endpoint, EndpointId};
use n0_future::time::Instant;
use tokio::sync::mpsc::UnboundedSender;

use crate::conn::{dial, is_direct, send_datagram};
use crate::realm::Publisher;
use crate::transfers::Transfers;
use crate::wire::{InputReceiver, InputSender, Message};
use crate::{KICK_AFTER, Public, RateLimit, Ticket};
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
    /// The track, its hash and the laps of a start that waits for the Hellos.
    pub(crate) start_wanted: Option<(u32, u64, u8)>,
    /// The maps being sent.
    pub(crate) transfers: Transfers,
    /// Who is ready, before tick 0.
    pub(crate) ready_from: HashSet<EndpointId>,
    /// A race has been started and the game has not said it is over.
    pub(crate) racing: bool,
    /// The players that left the race, whose pads are still being settled, by slot.
    pub(crate) departures: HashMap<usize, Departure>,
    /// The listing of the lobby in the realm, if it is public.
    pub(crate) publisher: Option<Publisher>,
    /// How it is listed (what the guests are told, for one of them to carry on with).
    pub(crate) public: Option<Public>,
}

/// What a guest keeps of the lobby to carry it on if the host goes: the players in the order
/// they succeed in (the first one that is left hosts), and what the host would be keeping.
#[derive(Default)]
pub(crate) struct Heirs {
    /// The players of the lobby that can host, the host first, as the latest
    /// [`Message::Lobby`] said.
    pub(crate) order: Vec<EndpointId>,
    /// How the lobby is listed, if it is.
    pub(crate) public: Option<Public>,
    /// The hosts that are gone, as far as we have found out (the one that closed, and every heir
    /// that could not be reached or closed in turn).
    pub(crate) dead: HashSet<EndpointId>,
    /// Looking for the next host: pick one and dial it.
    pub(crate) searching: bool,
    /// The heir being dialed.
    pub(crate) dialing: Option<EndpointId>,
    /// What players that took us for the host said (they dialed us before we knew we were), for
    /// when we are.
    pub(crate) heard: Vec<(EndpointId, Message)>,
}

impl Heirs {
    /// The one to carry the lobby on: the first of the order that is not known to be gone.
    pub(crate) fn next_host(&self) -> Option<EndpointId> {
        self.order
            .iter()
            .copied()
            .find(|id| !self.dead.contains(id))
    }
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
    /// The tries of dialing it ourselves (see [`Guest::wants_swap`]) so far, and the earliest time
    /// for the next.
    pub(crate) swaps: u32,
    pub(crate) next_swap: Instant,
}

impl Guest {
    pub(crate) fn new(id: EndpointId, connection: Connection) -> Guest {
        Guest {
            id,
            connection,
            hello: None,
            nick: (0, String::new()),
            spectator: false,
            swaps: 0,
            next_swap: Instant::now() + RELAY_PATIENCE,
        }
    }

    /// Whether to dial the guest ourselves, for a lobby connection the other way round from the one
    /// it made: the way to it is not direct after the patience or the last try.
    pub(crate) fn wants_swap(&self, now: Instant) -> bool {
        redial_due(true, self.swaps, now >= self.next_swap, is_direct(&self.connection))
    }

    /// A swap is tried.
    pub(crate) fn swapped(&mut self, now: Instant) {
        self.swaps += 1;
        self.next_swap = now + REDIAL_EVERY;
    }
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

/// How long a link may be on a relay before the other side is tried as the one that dials, and
/// how long between the tries after that, and how many there are. (Only the side that dials
/// makes holes, so a link that is stuck on a relay may get on with the other one dialing.)
pub(crate) const RELAY_PATIENCE: Duration = Duration::from_secs(10);
pub(crate) const REDIAL_EVERY: Duration = Duration::from_secs(20);
pub(crate) const REDIALS: u32 = 4;

/// The connection to another player in the race, and the pads that go over it.
pub(crate) struct Link {
    /// The first connection made to the player.
    pub(crate) connection: Connection,
    /// The one made later by the side that was dialed, when the first stayed on a relay (see
    /// [`Link::wants_redial`]). Pads go over whichever of the two is direct.
    pub(crate) alt: Option<Connection>,
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
    /// The tries of dialing the other way so far, and the earliest time for the next.
    pub(crate) redials: u32,
    pub(crate) next_redial: Instant,
    /// When the link began, and what was last said of the way it goes (see [`Link::changed`]).
    pub(crate) born: Instant,
    said: Option<(bool, Option<bool>)>,
}

impl Link {
    pub(crate) fn new(connection: Connection, epoch: u32) -> Link {
        Link {
            epoch,
            connection,
            alt: None,
            sender: InputSender::default(),
            receiver: InputReceiver::default(),
            peer_has: 0,
            advanced: Instant::now(),
            redials: 0,
            next_redial: Instant::now() + RELAY_PATIENCE,
            born: Instant::now(),
            said: None,
        }
    }

    /// Starts the pads over for the race of `epoch`, on the connections it has.
    pub(crate) fn restart(&mut self, epoch: u32) {
        self.epoch = epoch;
        self.sender = InputSender::default();
        self.receiver = InputReceiver::default();
        self.peer_has = 0;
        self.advanced = Instant::now();
    }

    /// The connection that the pads go over: the second one when only that is direct, or the
    /// only one that is up.
    pub(crate) fn best(&self) -> &Connection {
        match &self.alt {
            Some(alt)
                if self.connection.close_reason().is_some()
                    || (is_direct(alt) == Some(true) && is_direct(&self.connection) != Some(true)) =>
            {
                alt
            }
            _ => &self.connection,
        }
    }

    /// Whether to dial the player again, the other way round from the first time: we were
    /// dialed, and the way to it still is not direct, after the patience or the last try.
    pub(crate) fn wants_redial(&self, now: Instant) -> bool {
        redial_due(
            self.connection.side().is_server(),
            self.redials,
            now >= self.next_redial,
            is_direct(self.best()),
        )
    }

    /// A redial is made: the one that did not come to anything is dropped. Its number is
    /// returned, for its close not to be taken for the player leaving.
    pub(crate) fn redialed(&mut self, now: Instant) -> Option<usize> {
        self.redials += 1;
        self.next_redial = now + REDIAL_EVERY;
        self.alt.take().map(|alt| {
            alt.close(0u32.into(), b"redial");
            alt.stable_id()
        })
    }

    /// Seconds since the link began, for the log.
    pub(crate) fn age(&self) -> f32 {
        self.born.elapsed().as_secs_f32()
    }

    /// What to log if the way the pads go changed since the last time: over which connection, and
    /// whether that one is direct, on a relay, or has no path yet.
    pub(crate) fn changed(&mut self) -> Option<String> {
        let best = self.best();
        let now = (
            self.alt.as_ref().is_some_and(|alt| alt.stable_id() == best.stable_id()),
            is_direct(best),
        );
        if self.said == Some(now) {
            return None;
        }
        self.said = Some(now);
        let (second, direct) = now;
        Some(format!(
            "{} connection is {}",
            if second { "second" } else { "first" },
            match direct {
                Some(true) => "direct",
                Some(false) => "on a relay",
                None => "without a path yet",
            }
        ))
    }

    /// Sends the pads it lacks of ours, and what we have of its.
    pub(crate) fn send(&mut self) {
        let mut packet = self.sender.packet(self.peer_has, self.receiver.next());
        packet.epoch = self.epoch;
        send_datagram(self.best(), &Message::Inputs(packet));
    }
}

/// Whether a link is due to be dialed the other way: we were dialed (`dialed`), the tries are not
/// used up, the wait is over (`waited`), and the way to the player is not direct (`direct`).
fn redial_due(dialed: bool, redials: u32, waited: bool, direct: Option<bool>) -> bool {
    dialed && redials < REDIALS && waited && direct != Some(true)
}

/// The players of `roster` (the host first) that we dial for their links, those we are not
/// `linked` to. A guest dials the host, as it did the lobby: that way the pair has one dialing
/// side and the NAT of the guest has already sent out to the host. Of two guests the one with the
/// lower id dials.
pub(crate) fn to_dial(
    me: EndpointId,
    roster: &[EndpointId],
    linked: impl Fn(&EndpointId) -> bool,
) -> Vec<EndpointId> {
    let host = roster.first().copied();
    roster
        .iter()
        .copied()
        .filter(|&id| {
            let dials = if Some(me) == host {
                false
            } else if Some(id) == host {
                true
            } else {
                me.as_bytes() < id.as_bytes()
            };
            id != me && dials && !linked(&id)
        })
        .collect()
}

/// How long a dial in `pending` is waited for before the player is dialed again (a dial itself
/// tries for 10 seconds).
const DIAL_WAIT: Duration = Duration::from_secs(15);

/// Dials the players that [`to_dial`] says, except those already being dialed (`pending`: when it
/// was begun). Called from the lobby on, so that the links are up (and direct, or have been
/// dialed the other way) by the time a race starts.
pub(crate) fn dial_links(
    endpoint: &Endpoint,
    roster: &[EndpointId],
    links: &HashMap<EndpointId, Link>,
    pending: &mut HashMap<EndpointId, Instant>,
    dialed: &UnboundedSender<Connection>,
) {
    let now = Instant::now();
    pending.retain(|_, since| now.duration_since(*since) < DIAL_WAIT);
    let waiting = pending.clone();
    for id in to_dial(endpoint.id(), roster, |id| links.contains_key(id) || waiting.contains_key(id)) {
        pending.insert(id, now);
        dial(endpoint.clone(), id, dialed.clone());
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
        link.restart(epoch);
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

    #[test]
    fn of_each_pair_one_dials_whoever_the_host_is() {
        let ids: [EndpointId; 4] = std::array::from_fn(|_| iroh::SecretKey::generate().public());
        // Any of them can be first in the roster, which is the host.
        for host in 0..ids.len() {
            let mut roster = ids.to_vec();
            roster.swap(0, host);
            let dials: Vec<(EndpointId, EndpointId)> = roster
                .iter()
                .flat_map(|&me| {
                    to_dial(me, &roster, |_| false)
                        .into_iter()
                        .map(move |id| (me, id))
                })
                .collect();
            // Six pairs, each dialed from one side only, and never from oneself.
            assert_eq!(dials.len(), 6);
            for &(from, to) in &dials {
                assert_ne!(from, to);
                assert!(!dials.contains(&(to, from)));
            }
        }
    }

    #[test]
    fn a_link_stuck_on_a_relay_is_dialed_the_other_way_a_few_times() {
        // Dialed, waited, on a relay or with no path yet.
        assert!(redial_due(true, 0, true, Some(false)));
        assert!(redial_due(true, REDIALS - 1, true, None));
        // Not yet, not by the side that dialed, not when direct, not for ever.
        assert!(!redial_due(true, 0, false, Some(false)));
        assert!(!redial_due(false, 0, true, Some(false)));
        assert!(!redial_due(true, 0, true, Some(true)));
        assert!(!redial_due(true, REDIALS, true, Some(false)));
    }

    #[test]
    fn a_player_we_are_linked_to_is_not_dialed() {
        let [a, b] = std::array::from_fn(|_| iroh::SecretKey::generate().public());
        // The guest dials the host, whichever id is lower.
        for roster in [[a, b], [b, a]] {
            let [host, guest] = roster;
            assert_eq!(to_dial(guest, &roster, |_| false), [host]);
            assert!(to_dial(guest, &roster, |id| *id == host).is_empty());
            assert!(to_dial(host, &roster, |_| false).is_empty());
        }
    }

    #[test]
    fn guests_dial_the_host_and_the_lower_of_two_guests_dials() {
        let ids: [EndpointId; 3] = std::array::from_fn(|_| iroh::SecretKey::generate().public());
        let roster = ids.to_vec();
        assert!(to_dial(ids[0], &roster, |_| false).is_empty());
        for guest in &ids[1..] {
            assert!(to_dial(*guest, &roster, |_| false).contains(&ids[0]));
        }
        let (low, high) = if ids[1].as_bytes() < ids[2].as_bytes() { (ids[1], ids[2]) } else { (ids[2], ids[1]) };
        assert_eq!(to_dial(low, &roster, |_| false).len(), 2);
        assert_eq!(to_dial(high, &roster, |_| false), [ids[0]]);
    }

    #[test]
    fn the_first_player_left_in_the_order_hosts_next() {
        let [host, a, b] = std::array::from_fn(|_| iroh::SecretKey::generate().public());
        let mut heirs = Heirs {
            order: vec![host, a, b],
            ..Heirs::default()
        };
        heirs.dead.insert(host);
        assert_eq!(heirs.next_host(), Some(a));
        // The heir is gone too, before or after it took over.
        heirs.dead.insert(a);
        assert_eq!(heirs.next_host(), Some(b));
        heirs.dead.insert(b);
        assert_eq!(heirs.next_host(), None);
    }
}
