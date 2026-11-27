//! A queueing client's connection (`score::matching`): the signed queue frame, then the
//! matchmaker's news one way and the ready check's answers the other, until the match goes or
//! the client leaves. The queueing lasts as long as the stream. A lobby's host's instead starts
//! with `Host`, and its registration lasts as long.

use std::sync::Arc;
use std::time::Duration;

use iroh::endpoint::{Connection, RecvStream, SendStream};
use score::Action;
use score::matching::{QueueEntry, Refused, ToClient, ToServer, frame, frame_length};
use tokio::sync::mpsc::unbounded_channel;
use tracing::{debug, error, info, warn};

use crate::matchmaker::Waiting;
use crate::{App, MAX_VERSION, READ_TIMEOUT, unix_now};

/// How long the client has to see a last frame and close, before the server does.
const LINGER: Duration = Duration::from_secs(10);

/// The most relays a net report may list, and the longest URL kept.
const MAX_RELAYS: usize = 16;
const MAX_URL: usize = 200;

async fn read_frame<T: bitcode::DecodeOwned>(recv: &mut RecvStream) -> Result<T, String> {
    let mut head = [0; 4];
    recv.read_exact(&mut head).await.map_err(|e| e.to_string())?;
    let length = frame_length(head).ok_or("a frame too long")?;
    let mut body = vec![0; length];
    recv.read_exact(&mut body).await.map_err(|e| e.to_string())?;
    bitcode::decode(&body).map_err(|e| e.to_string())
}

async fn write(send: &mut SendStream, message: &ToClient) -> bool {
    send.write_all(&frame(message)).await.is_ok()
}

/// Says `message` as the last word, and lets the client close.
async fn last(connection: &Connection, send: &mut SendStream, message: &ToClient) {
    if write(send, message).await {
        let _ = send.finish();
        let _ = tokio::time::timeout(LINGER, connection.closed()).await;
    }
}

fn clean_url(url: &str) -> Option<String> {
    (url.len() <= MAX_URL && url.chars().all(|c| c.is_ascii_graphic())).then(|| url.to_string())
}

pub async fn serve(app: Arc<App>, connection: Connection) {
    let peer = connection.remote_id();
    let Ok(Ok((mut send, mut recv))) = tokio::time::timeout(READ_TIMEOUT, connection.accept_bi()).await else {
        return;
    };
    let first = tokio::time::timeout(READ_TIMEOUT, read_frame::<ToServer>(&mut recv)).await;
    let (auth, entry) = match first {
        Ok(Ok(ToServer::Queue { auth, entry })) => (auth, entry),
        Ok(Ok(ToServer::Host { lobby })) => return host(app, connection, send, recv, lobby).await,
        _ => {
            warn!(peer = %peer.fmt_short(), "sent no queue frame");
            connection.close(1u32.into(), b"no queue");
            return;
        }
    };
    let now = unix_now();
    let key = match app.seen.check(&auth, Action::Queue, &entry, now) {
        Ok(key) => key,
        Err(e) => {
            return last(&connection, &mut send, &ToClient::Unauthorized { why: e.to_string(), now }).await;
        }
    };
    let refuse = |why: &str| ToClient::Refused(Refused::BadFormat { why: why.into() });
    let Ok(entry) = bitcode::decode::<QueueEntry>(&entry) else {
        return last(&connection, &mut send, &refuse("queue entry")).await;
    };
    // (Signed for this endpoint: one that queues from another is replaying it.)
    if entry.endpoint != *peer.as_bytes() {
        return last(&connection, &mut send, &refuse("not this endpoint's")).await;
    }
    if entry.version.is_empty() || entry.version.len() > MAX_VERSION {
        return last(&connection, &mut send, &refuse("version")).await;
    }
    let banned = app.db.lock().unwrap().banned(&key);
    match banned {
        Ok(false) => {}
        Ok(true) => return last(&connection, &mut send, &ToClient::Refused(Refused::Banned)).await,
        Err(e) => {
            error!("{e}");
            connection.close(2u32.into(), b"failed");
            return;
        }
    }
    let mut net = entry.net;
    net.relays.truncate(MAX_RELAYS);
    net.relays.retain(|(url, _)| clean_url(url).is_some());

    let (to, mut news) = unbounded_channel();
    let waiting = Waiting {
        id: 0,
        key: *key.as_bytes(),
        mode: entry.mode,
        version: entry.version,
        endpoint: entry.endpoint,
        relay: entry.relay.as_deref().and_then(clean_url),
        net,
        nick: score::clean_nick(&entry.nick),
        rejoin: entry.rejoin,
        joined: 0,
        to,
    };
    let who = waiting.who();
    let joined = app.matchmaker.lock().unwrap().join(waiting, now);
    let id = match joined {
        Ok(id) => id,
        Err(refused) => return last(&connection, &mut send, &ToClient::Refused(refused)).await,
    };
    info!("{who} queued ({id})");

    // (A read of its own: `read_exact` loses what it read if a `select!` drops it.)
    let (answers_tx, mut answers) = unbounded_channel();
    let reader = tokio::spawn(async move {
        loop {
            let answer = read_frame::<ToServer>(&mut recv).await;
            let end = answer.is_err();
            if answers_tx.send(answer).is_err() || end {
                break;
            }
        }
    });
    loop {
        tokio::select! {
            message = news.recv() => {
                let Some(message) = message else { break };
                if matches!(message, ToClient::Go { .. } | ToClient::Refused(_)) {
                    last(&connection, &mut send, &message).await;
                    break;
                }
                if !write(&mut send, &message).await {
                    break;
                }
            }
            answer = answers.recv() => {
                let (accept, match_id) = match answer {
                    Some(Ok(ToServer::Accept { id })) => (true, id),
                    Some(Ok(ToServer::Decline { id })) => (false, id),
                    // Closed, or nonsense.
                    _ => break,
                };
                app.matchmaker.lock().unwrap().answer(id, match_id, accept, unix_now());
            }
        }
    }
    reader.abort();
    app.matchmaker.lock().unwrap().leave(id, unix_now());
    info!("{who} left the queue ({id})");
}

/// A lobby's host's registration: its `Open`s and `Admitted`s one way, the `Admit`s the other.
async fn host(app: Arc<App>, connection: Connection, mut send: SendStream, mut recv: RecvStream, lobby: u64) {
    let peer = connection.remote_id();
    let (to, mut news) = unbounded_channel();
    let registered = app.matchmaker.lock().unwrap().host(lobby, *peer.as_bytes(), to, unix_now());
    let id = match registered {
        Ok(id) => id,
        Err(refused) => return last(&connection, &mut send, &ToClient::Refused(refused)).await,
    };
    debug!("{} hosts lobby {lobby} ({id})", peer.fmt_short());
    let (said_tx, mut said) = unbounded_channel();
    let reader = tokio::spawn(async move {
        loop {
            let frame = read_frame::<ToServer>(&mut recv).await;
            let end = frame.is_err();
            if said_tx.send(frame).is_err() || end {
                break;
            }
        }
    });
    loop {
        tokio::select! {
            message = news.recv() => {
                let Some(message) = message else { break };
                if !write(&mut send, &message).await {
                    break;
                }
            }
            frame = said.recv() => match frame {
                Some(Ok(ToServer::Open { open, mut players })) => {
                    players.truncate(score::matching::MAX_MATCH);
                    for p in &mut players {
                        p.relay = p.relay.as_deref().and_then(clean_url);
                        p.nick = score::clean_nick(&p.nick);
                    }
                    app.matchmaker.lock().unwrap().open(id, lobby, open, players);
                }
                Some(Ok(ToServer::Admitted { id: admitted })) => {
                    app.matchmaker.lock().unwrap().admitted(id, lobby, admitted);
                }
                _ => break,
            },
        }
    }
    reader.abort();
    app.matchmaker.lock().unwrap().unhost(id, lobby, unix_now());
    debug!("{} stopped hosting lobby {lobby} ({id})", peer.fmt_short());
}
