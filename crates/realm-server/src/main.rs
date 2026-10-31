//! The realm server: a directory of the public lobbies, for the clients to browse.
//!
//! It listens on an iroh endpoint (QUIC over UDP, one port) and speaks `realm-proto`. The key
//! of the endpoint is kept in a file, so that the id that clients find the realm with stays
//! the same from a start to the next.
//!
//! `realm-server [--port <udp port>] [--key <key file>]`

mod realm;

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use iroh::endpoint::{Connection, presets};
use iroh::{Endpoint, SecretKey};
use realm_proto::{ALPN, MAX_REQUEST, Reject, Request, Response};
use tokio::sync::Semaphore;

use crate::realm::{Bucket, Realm};

const DEFAULT_PORT: u16 = 4747;
const DEFAULT_KEY: &str = "realm.key";

/// The most connections at once.
const MAX_CONNECTIONS: usize = 4096;

/// How long a request may take to arrive, once its stream is open.
const READ_TIMEOUT: Duration = Duration::from_secs(5);

type Error = Box<dyn std::error::Error + Send + Sync>;

struct Args {
    port: u16,
    key: PathBuf,
}

fn args() -> Result<Args, String> {
    let mut args = Args { port: DEFAULT_PORT, key: DEFAULT_KEY.into() };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = || it.next().ok_or(format!("{flag} needs a value"));
        match flag.as_str() {
            "--port" => args.port = value()?.parse().map_err(|e| format!("bad port: {e}"))?,
            "--key" => args.key = value()?.into(),
            _ => return Err(format!("unknown argument '{flag}'")),
        }
    }
    Ok(args)
}

/// The key in `path`, or a new one, which is then kept there.
fn secret_key(path: &Path) -> io::Result<SecretKey> {
    match fs::read(path) {
        Ok(bytes) => {
            let bytes: [u8; 32] = bytes
                .try_into()
                .map_err(|_| io::Error::other(format!("{} is not a 32 byte key", path.display())))?;
            Ok(SecretKey::from_bytes(&bytes))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let key = SecretKey::generate();
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
            options.open(path)?.write_all(&key.to_bytes())?;
            Ok(key)
        }
        Err(e) => Err(e),
    }
}

async fn bind(key: SecretKey, port: u16) -> Result<Endpoint, Error> {
    let builder = || {
        Endpoint::builder(presets::N0)
            .secret_key(key.clone())
            .alpns(vec![ALPN.to_vec()])
            .clear_ip_transports()
    };
    // Both families when the host has them, and IPv4 alone when it does not.
    let both = builder().bind_addr(format!("0.0.0.0:{port}"))?.bind_addr(format!("[::]:{port}"))?;
    match both.bind().await {
        Ok(endpoint) => Ok(endpoint),
        Err(e) => {
            eprintln!("no IPv6 ({e}), IPv4 only");
            Ok(builder().bind_addr(format!("0.0.0.0:{port}"))?.bind().await?)
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Error> {
    let args = args().inspect_err(|e| {
        eprintln!("{e}\nusage: realm-server [--port <udp port>] [--key <key file>]");
    })?;
    let endpoint = bind(secret_key(&args.key)?, args.port).await?;
    println!("realm id: {}", endpoint.id());
    println!("listening on {:?}", endpoint.bound_sockets());

    let realm = Arc::new(Realm::default());
    let slots = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    loop {
        let incoming = tokio::select! {
            incoming = endpoint.accept() => incoming,
            _ = tokio::signal::ctrl_c() => break,
        };
        let Some(incoming) = incoming else { break };
        let Ok(slot) = slots.clone().try_acquire_owned() else {
            incoming.refuse();
            continue;
        };
        let Ok(accepting) = incoming.accept() else { continue };
        let realm = realm.clone();
        tokio::spawn(async move {
            if let Ok(connection) = accepting.await {
                serve(&realm, connection).await;
            }
            drop(slot);
        });
    }
    endpoint.close().await;
    Ok(())
}

/// Answers the requests of a connection, until it ends. Its lobby ends with it.
async fn serve(realm: &Realm, connection: Connection) {
    let id = *connection.remote_id().as_bytes();
    let me = connection.stable_id();
    let mut bucket = Bucket::new(Instant::now());
    while let Ok((mut send, mut recv)) = connection.accept_bi().await {
        let request = tokio::time::timeout(READ_TIMEOUT, recv.read_to_end(MAX_REQUEST)).await;
        let response = match request {
            Ok(Ok(bytes)) => match Request::from_bytes(&bytes) {
                Ok(_) if !bucket.take(Instant::now()) => Response::Rejected(Reject::TooFast),
                Ok(request) => realm.handle(id, me, request),
                Err(_) => {
                    connection.close(1u32.into(), b"bad request");
                    break;
                }
            },
            _ => {
                connection.close(1u32.into(), b"no request");
                break;
            }
        };
        if send.write_all(&response.to_bytes()).await.is_err() || send.finish().is_err() {
            break;
        }
    }
    realm.closed(id, me);
}
