//! The wires to one peer: sending, and the task that listens to a connection.

use std::collections::HashSet;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use bytes::Bytes;
use iroh::endpoint::{Connection, ConnectionError};
use iroh::{Endpoint, EndpointAddr, EndpointId};
use tokio::sync::mpsc::UnboundedSender;

use crate::wire::Message;
use crate::{ALPN, Error, KICKED, MAX_BLOB};

pub(crate) async fn send_reliably(connection: &Connection, message: &Message) -> Result<(), Error> {
    let mut stream = connection.open_uni().await?;
    stream.write_all(&message.to_bytes()).await?;
    stream.finish()?;
    Ok(())
}

pub(crate) fn send_datagram(connection: &Connection, message: &Message) {
    // A datagram that does not go is as good as one that is lost.
    let _ = connection.send_datagram(Bytes::from(message.to_bytes()));
}

/// What a player who joins during a race is told.
pub(crate) const RACE_IN_PROGRESS: &str = "Race in progress, you will join the next one";

/// What a connection said, tagged with who.
pub(crate) enum Heard {
    Message(Message),
    /// The round trip of a ping we sent, which came back.
    Ping(Duration),
    /// The connection closed: the number it has (see `Connection::stable_id`), and whether the
    /// host removed us ([`KICKED`]).
    Closed {
        id: usize,
        kicked: bool,
    },
}

/// The clock that the stamps of the pings are read from. Only ours reads them.
pub(crate) fn stamp() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_micros() as u64
}

/// The connections without the repeats: a player can be in the lobby and the race both.
pub(crate) fn distinct<'a>(
    connections: impl Iterator<Item = &'a Connection>,
) -> Vec<&'a Connection> {
    let mut seen = HashSet::new();
    connections.filter(|c| seen.insert(c.stable_id())).collect()
}

/// Reads the datagrams and the reliable messages of a connection until it closes.
pub(crate) fn listen(
    connection: Connection,
    from: EndpointId,
    heard: UnboundedSender<(EndpointId, Heard)>,
) {
    tokio::spawn(async move {
        loop {
            let message = tokio::select! {
                datagram = connection.read_datagram() => datagram.ok().map(|d| Message::from_bytes(&d)),
                stream = connection.accept_uni() => match stream {
                    Ok(mut stream) => match stream.read_to_end(MAX_BLOB + 1024).await {
                        Ok(bytes) => Some(Message::from_bytes(&bytes)),
                        Err(_) => continue,
                    },
                    Err(_) => None ,
                },
            };
            match message {
                // Answered here, where the connection is, and at once: the loop may be busy.
                Some(Ok(Message::Ping { stamp })) => {
                    send_datagram(&connection, &Message::Pong { stamp })
                }
                Some(Ok(Message::Pong { stamp: sent })) => {
                    let rtt = Duration::from_micros(stamp().saturating_sub(sent));
                    if heard.send((from, Heard::Ping(rtt))).is_err() {
                        return;
                    }
                }
                Some(Ok(message)) => {
                    if heard.send((from, Heard::Message(message))).is_err() {
                        return;
                    }
                }
                // Not ours to understand: a stranger's, or a newer build's.
                Some(Err(_)) => {}
                None => {
                    let kicked = matches!(
                        connection.close_reason(),
                        Some(ConnectionError::ApplicationClosed(close))
                            if close.error_code.into_inner() == u64::from(KICKED)
                    );
                    let id = connection.stable_id();
                    let _ = heard.send((from, Heard::Closed { id, kicked }));
                    return;
                }
            }
        }
    });
}

/// Connects to a player of the roster, trying for a while: it may not be listening yet.
pub(crate) fn dial(endpoint: Endpoint, id: EndpointId, dialed: UnboundedSender<Connection>) {
    tokio::spawn(async move {
        for _ in 0..20 {
            if let Ok(connection) = endpoint.connect(EndpointAddr::new(id), ALPN).await {
                let _ = dialed.send(connection);
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    });
}
