//! Keeping at iroh to find a direct path to each peer, rather than leave it on the relay.
//!
//! iroh holepunches to a peer when a connection to it comes up, when an address of either side
//! changes, and otherwise once a minute; and only from the side that has a client connection to
//! the peer. A round probes for some 4 seconds: one that comes too early (the other side's NAT
//! has not been opened yet) or loses its probes leaves the pair on the relay for up to a minute,
//! and the side that only accepted connections never tries at all.
//!
//! So we hold a connection of our own (with [`PUNCH_ALPN`]) to every peer we race with, or will:
//! opening it starts a round, from our side, whichever way the other connections go. While the
//! peer is still reached through the relay, it is opened again every few seconds. A direct path
//! found on any connection to a peer is taken by all of them, so the lobby connection and the
//! links of the race get it too; and a link dialed later goes the direct way from the start.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use iroh::endpoint::Connection;
use iroh::{Endpoint, EndpointAddr, EndpointId};
use n0_future::task::AbortOnDropHandle;
use n0_future::time::{sleep, timeout};

use crate::PUNCH_ALPN;

/// How long a connection is given to come up.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// How often we look at the way a peer is reached.
const CHECK: Duration = Duration::from_secs(1);

/// How long the `attempt`th try (from 0) waits for a direct path before it opens a new
/// connection, which starts a new round. iroh starts no more than one round in 5 seconds unless
/// an address changed, so the tries are a bit further apart than that; after a minute of them,
/// they are let down to twice a minute (iroh's own is once a minute).
fn patience(attempt: u32) -> Duration {
    if attempt < 10 {
        Duration::from_secs(6)
    } else {
        Duration::from_secs(30)
    }
}

/// How long the side that accepted a connection keeps it, at most: the side that opened it
/// closes it as soon as it is done with it.
pub(crate) const ACCEPTED_FOR: Duration = Duration::from_secs(5 * 60);

/// Whether the data on `connection` goes straight to the peer: false while it goes through a
/// relay, or there is no path yet.
pub(crate) fn is_direct(connection: &Connection) -> bool {
    connection
        .paths()
        .iter()
        .any(|path| path.is_selected() && !path.is_relay())
}

/// The peers being punched through to, each with its task.
pub(crate) struct Puncher {
    endpoint: Endpoint,
    /// Without sockets of our own (the relay only, or a browser) there is nothing to punch.
    enabled: bool,
    tasks: HashMap<EndpointId, AbortOnDropHandle<()>>,
}

impl Puncher {
    pub(crate) fn new(endpoint: Endpoint) -> Puncher {
        #[cfg(not(target_arch = "wasm32"))]
        let enabled = !endpoint.bound_sockets().is_empty();
        #[cfg(target_arch = "wasm32")]
        let enabled = false;
        Puncher {
            endpoint,
            enabled,
            tasks: HashMap::new(),
        }
    }

    /// Punches through to `peers` (us left out) and to no others: the tasks of new ones start,
    /// those of the ones not in it stop.
    pub(crate) fn set(&mut self, peers: impl IntoIterator<Item = EndpointId>) {
        if !self.enabled {
            return;
        }
        let me = self.endpoint.id();
        let wanted: HashSet<EndpointId> = peers.into_iter().filter(|&id| id != me).collect();
        self.tasks.retain(|id, _| wanted.contains(id));
        for id in wanted {
            self.tasks.entry(id).or_insert_with(|| {
                let endpoint = self.endpoint.clone();
                AbortOnDropHandle::new(n0_future::task::spawn(punch(endpoint, id)))
            });
        }
    }
}

/// Keeps a connection to `id` and starts rounds of holepunching on it until the path is direct;
/// and again if it goes back to the relay.
async fn punch(endpoint: Endpoint, id: EndpointId) {
    let mut attempt = 0;
    loop {
        let connection = timeout(
            CONNECT_TIMEOUT,
            endpoint.connect(EndpointAddr::new(id), PUNCH_ALPN),
        )
        .await;
        let Ok(Ok(connection)) = connection else {
            // Not reachable for now (or an older build, which does not take these): no round.
            sleep(patience(attempt)).await;
            attempt += 1;
            continue;
        };
        // The new connection started a round: give it the time to find a path.
        let mut waited = Duration::ZERO;
        while waited < patience(attempt)
            && !is_direct(&connection)
            && connection.close_reason().is_none()
        {
            sleep(CHECK).await;
            waited += CHECK;
        }
        if is_direct(&connection) {
            // Held as long as it is direct; if it falls back, we try from the start.
            while is_direct(&connection) && connection.close_reason().is_none() {
                sleep(CHECK).await;
            }
            attempt = 0;
        } else {
            attempt += 1;
        }
        connection.close(0u32.into(), b"punched");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tries_are_further_apart_than_iroh_starts_rounds() {
        for attempt in 0..20 {
            assert!(patience(attempt) > Duration::from_secs(5));
        }
        assert!(patience(0) < patience(100));
    }
}
