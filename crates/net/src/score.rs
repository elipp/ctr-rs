//! Talking to `score-server` (TIME-TRIAL-RELIC-LEADERBOARDS.md). Every job binds an endpoint of
//! its own with a random key, so the relay sees a throwaway key, never the player key, which only
//! shows inside a signed request.

use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use iroh::endpoint::Connection;
use iroh::{EndpointAddr, EndpointId, SecretKey};
use score::{ALPN, Action, Auth, MAX_RESPONSE, Request, Response, Submit};

use crate::Error;
use crate::pending::Pending;

/// The score server's id, built in, unless `CTR_SCORE` names another. None until one is deployed.
pub const DEFAULT_SCORE: Option<&str> = None;

/// The score server: the id in `CTR_SCORE` if there is one (to try a server of one's own), else
/// [`DEFAULT_SCORE`].
pub fn default_score() -> Option<EndpointId> {
    std::env::var("CTR_SCORE")
        .ok()
        .and_then(|id| id.trim().parse().ok())
        .or_else(|| {
            DEFAULT_SCORE.map(|id| id.parse().expect("the default score server is an endpoint id"))
        })
}

/// A request on its way, or its answer.
pub type Answer = Pending<Result<Response, String>>;

/// How long a read may take, connecting included.
const READ_TIMEOUT: Duration = Duration::from_secs(15);

/// How long a signed request may take: a submit may be queued for a free verifier (10 s) and
/// then replay a whole race.
const SUBMIT_TIMEOUT: Duration = Duration::from_secs(40);

/// What the server's clock is ahead of ours by, in seconds, as an `Unauthorized` said: kept for
/// the session, as some browsers run on wrong clocks.
static CLOCK_OFFSET: AtomicI64 = AtomicI64::new(0);

/// Asks `server` an unsigned request (a read).
pub fn ask(server: EndpointId, request: Request) -> Answer {
    Pending::spawn("score", move || async move {
        let job = with_connection(server, |connection| async move {
            exchange(&connection, &request).await
        });
        n0_future::time::timeout(READ_TIMEOUT, job)
            .await
            .unwrap_or_else(|_| Err("the score server took too long".into()))
    })
}

/// Submits `run` (`Run::to_bytes`) signed by `key`. `nick` and `version` go on the player's row.
pub fn submit(
    server: EndpointId,
    key: SecretKey,
    nick: String,
    version: String,
    claimed_ms: i32,
    run: Vec<u8>,
) -> Answer {
    signed(server, move |ts| {
        Request::Submit(Submit {
            auth: Auth::sign(&key, Action::Submit, ts, &run),
            nick: nick.clone(),
            version: version.clone(),
            claimed_ms,
            run: run.clone(),
        })
    })
}

/// Deletes `key`'s player and runs from the boards.
pub fn forget(server: EndpointId, key: SecretKey) -> Answer {
    signed(server, move |ts| Request::Forget {
        auth: Auth::sign(&key, Action::Forget, ts, &[]),
    })
}

/// Sends the request `signed_at` makes for a `ts`, and if the server's answer says our clock is
/// off, once more with its clock.
fn signed(server: EndpointId, signed_at: impl Fn(u64) -> Request + Send + 'static) -> Answer {
    Pending::spawn("score", move || async move {
        let job = with_connection(server, |connection| async move {
            let ts = signing_time();
            match exchange(&connection, &signed_at(ts)).await? {
                // Only a clock that's off is worth another try: anything else would be told the
                // same.
                Response::Unauthorized { now, .. } if ts.abs_diff(now) > score::MAX_SKEW => {
                    let offset = now as i64 - unix_now() as i64;
                    CLOCK_OFFSET.store(offset, Ordering::Relaxed);
                    exchange(&connection, &signed_at(signing_time())).await
                }
                response => Ok(response),
            }
        });
        n0_future::time::timeout(SUBMIT_TIMEOUT, job)
            .await
            .unwrap_or_else(|_| Err("the score server took too long".into()))
    })
}

fn unix_now() -> u64 {
    n0_future::time::SystemTime::now()
        .duration_since(n0_future::time::SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn signing_time() -> u64 {
    unix_now().saturating_add_signed(CLOCK_OFFSET.load(Ordering::Relaxed))
}

/// Binds a throwaway endpoint, connects to `server`, and runs `job` on the connection.
async fn with_connection<F, Fut>(server: EndpointId, job: F) -> Result<Response, String>
where
    F: FnOnce(Connection) -> Fut,
    Fut: Future<Output = Result<Response, Error>>,
{
    let endpoint = crate::endpoint_builder()
        .bind()
        .await
        .map_err(|e| e.to_string())?;
    let result = match endpoint.connect(EndpointAddr::new(server), ALPN).await {
        Ok(connection) => {
            let result = job(connection.clone()).await;
            connection.close(0u32.into(), b"bye");
            result.map_err(|e| e.to_string())
        }
        Err(e) if crate::realm::is_alpn_mismatch(&e) => Err(crate::realm::OUTDATED.into()),
        Err(e) => Err(e.to_string()),
    };
    endpoint.close().await;
    result
}

async fn exchange(connection: &Connection, request: &Request) -> Result<Response, Error> {
    let (mut send, mut recv) = connection.open_bi().await?;
    send.write_all(&request.to_bytes()).await?;
    send.finish()?;
    let bytes = recv.read_to_end(MAX_RESPONSE).await?;
    Ok(Response::from_bytes(&bytes)?)
}
