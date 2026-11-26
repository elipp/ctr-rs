//! The solo leaderboards (TIME-TRIAL-RELIC-LEADERBOARDS.md): runs come in over iroh
//! (`score::proto`) as `race::solo::Run` bytes, are re-simmed on the server's own copy of the
//! level, and ranked by what that replay times. Reads are open; submits are signed by a player
//! key.
//!
//! The same endpoint runs the quick-play matchmaker on an ALPN of its own (`matching.rs`,
//! `matchmaker.rs`, MATCHMAKING.md).
//!
//! Like the realm, the endpoint has no IP transports: it is reached through the relay only, so it
//! never learns a player's address. Its key is kept in a file, so that its id stays the same.
//!
//! `score-server --maps <dir of .ctrmap> [--db <file>] [--key <key file>] [--port <udp port>]`,
//! or `score-server [--db <file>] <operator command>` (`operator.rs`) beside a running one.

mod auth;
mod db;
mod matching;
mod matchmaker;
mod operator;
mod suspect;
mod tracks;

use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use iroh::endpoint::{Connection, presets};
use iroh::{Endpoint, SecretKey};
use race::solo::{Outcome, Run, board_seed};
use score::{
    Action, BoardId, MAX_REQUEST, MAX_TOP, MAX_VERSION, Rejected, Request, Response, Submit, Submitted,
};
use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;
use tracing::{debug, error, info, warn};
use tracing_subscriber::EnvFilter;

use crate::auth::{Limits, Seen};
use crate::db::{Db, NewRun};
use crate::tracks::Tracks;

const DEFAULT_PORT: u16 = 4748;
const DEFAULT_KEY: &str = "score.key";
const DEFAULT_DB: &str = "scores.db";

/// The most connections at once.
const MAX_CONNECTIONS: usize = 1024;

/// How long a request may take to arrive, once its stream is open.
const READ_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a submit waits for a free verifier before it gets [`Response::Busy`].
const VERIFY_QUEUE: Duration = Duration::from_secs(10);

type Error = Box<dyn std::error::Error + Send + Sync>;

struct Args {
    port: u16,
    key: PathBuf,
    db: PathBuf,
    maps: Option<PathBuf>,
    /// An operator command and its arguments; empty to serve.
    command: Vec<String>,
}

const USAGE: &str = "usage: score-server --maps <dir> [--db <file>] [--key <file>] [--port <udp port>], \
    or score-server [--db <file>] <stats | suspects | ban <key> | unban <key> | delete <run id>>";

/// The flags may come anywhere, so that an operator command can follow the image's entrypoint
/// flags (`docker compose run score stats`).
fn args() -> Result<Args, String> {
    let (mut port, mut key, mut db, mut maps) =
        (DEFAULT_PORT, PathBuf::from(DEFAULT_KEY), PathBuf::from(DEFAULT_DB), None);
    let mut command = Vec::new();
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = || it.next().ok_or(format!("{arg} needs a value"));
        match arg.as_str() {
            "--port" => port = value()?.parse().map_err(|e| format!("bad port: {e}"))?,
            "--key" => key = value()?.into(),
            "--db" => db = value()?.into(),
            "--maps" => maps = Some(value()?.into()),
            flag if flag.starts_with("--") => return Err(format!("unknown argument '{flag}'")),
            _ => command.push(arg),
        }
    }
    Ok(Args { port, key, db, maps, command })
}

/// The key in `path`, or a new one, which is then kept there.
fn secret_key(path: &Path) -> io::Result<SecretKey> {
    match fs::read(path) {
        Ok(bytes) => {
            let bytes: [u8; 32] = bytes.try_into().map_err(|_| {
                io::Error::other(format!("{} is not a 32 byte key", path.display()))
            })?;
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
            .alpns(vec![score::ALPN.to_vec(), score::matching::MATCH_ALPN.to_vec()])
            .clear_ip_transports()
    };
    // Both families when the host has them, and IPv4 alone when it does not.
    let both = builder()
        .bind_addr(format!("0.0.0.0:{port}"))?
        .bind_addr(format!("[::]:{port}"))?;
    match both.bind().await {
        Ok(endpoint) => Ok(endpoint),
        Err(e) => {
            warn!("no IPv6 ({e}), IPv4 only");
            Ok(builder().bind_addr(format!("0.0.0.0:{port}"))?.bind().await?)
        }
    }
}

struct App {
    db: Mutex<Db>,
    tracks: Tracks,
    verifiers: Semaphore,
    seen: Seen,
    limits: Limits,
    matchmaker: Mutex<matchmaker::Matchmaker>,
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// A database error: logged, and the client is told no more.
fn failed(e: impl std::fmt::Display) -> Response {
    error!("{e}");
    Response::Failed
}

/// What makes two runs "the same inputs": the pad changes, not the character.
fn pads_hash(run: &Run) -> [u8; 32] {
    let mut hash = Sha256::new();
    for c in &run.demo.changes {
        hash.update(c.tick.to_le_bytes());
        hash.update(c.buttons_held.to_le_bytes());
        hash.update([c.stick_lx, c.stick_ly, c.stick_ry]);
    }
    hash.finalize().into()
}

struct Verified {
    run: Run,
    board: BoardId,
    outcome: Outcome,
    pads_hash: [u8; 32],
}

enum Refusal {
    Rejected(Rejected),
    Busy,
    Internal(String),
}

impl From<Rejected> for Refusal {
    fn from(r: Rejected) -> Refusal {
        Refusal::Rejected(r)
    }
}

fn bad(why: &str) -> Refusal {
    Refusal::Rejected(Rejected::BadFormat { why: why.into() })
}

/// The run of `bytes`, checked against its board and re-simmed. Cheap checks first: the replay
/// is a whole race.
async fn verify(app: &Arc<App>, bytes: &[u8], claimed_ms: i32) -> Result<Verified, Refusal> {
    let run = Run::from_bytes(bytes).map_err(|why| Rejected::BadFormat { why })?;
    if run.sim_version != race::SIM_VERSION {
        return Err(Rejected::WrongSimVersion { ours: race::SIM_VERSION }.into());
    }
    if !(1..=race::MAX_LAPS).contains(&run.laps) {
        return Err(bad("laps"));
    }
    let frames = *race::track::ANIM_NUM_FRAMES
        .get(run.demo.character)
        .ok_or_else(|| bad("character"))?;
    if run.seed != board_seed(run.kind, run.map_hash, run.laps) {
        return Err(Rejected::WrongSeed.into());
    }
    let track = app.tracks.get(run.map_hash).ok_or(Rejected::UnknownMap)?;
    let board = BoardId {
        kind: run.kind,
        map_hash: run.map_hash,
        sim_version: run.sim_version,
        laps: run.laps,
    };
    let pads_hash = pads_hash(&run);
    let same = app.db.lock().unwrap().same_pads(board, &pads_hash);
    match same {
        Ok(Some(of_run)) => return Err(Rejected::Duplicate { of_run }.into()),
        Ok(None) => {}
        Err(e) => return Err(Refusal::Internal(e.to_string())),
    }

    let permit = tokio::time::timeout(VERIFY_QUEUE, app.verifiers.acquire())
        .await
        .map_err(|_| Refusal::Busy)?
        .map_err(|e| Refusal::Internal(e.to_string()))?;
    let (run, outcome) = tokio::task::spawn_blocking(move || {
        let outcome = tracks::replay(&track, &run, frames);
        (run, outcome)
    })
    .await
    .map_err(|e| Refusal::Internal(e.to_string()))?;
    drop(permit);

    let outcome = outcome.ok_or(Rejected::NoFinish)?;
    // `Run::of_finished` cuts the pads right after the finish.
    if outcome.finish_tick + 1 != run.demo.length {
        return Err(bad("pads past the finish"));
    }
    if outcome.final_ms != claimed_ms {
        return Err(Rejected::Mismatch { ours_ms: outcome.final_ms }.into());
    }
    Ok(Verified { run, board, outcome, pads_hash })
}

/// The earlier run of another player's that `v` is much like, if any.
fn suspect_of(app: &App, key: &score::PublicKey, v: &Verified) -> Option<i64> {
    let near = app.db.lock().unwrap().near(
        v.board,
        key,
        v.outcome.final_ms,
        suspect::WITHIN,
        suspect::CANDIDATES,
    );
    let near = near.map_err(|e| error!("{e}")).ok()?;
    near.into_iter()
        .find(|n| Run::from_bytes(&n.run).is_ok_and(|run| suspect::alike(&run, &v.run)))
        .map(|n| n.id)
}

async fn submit(app: &Arc<App>, s: Submit) -> Response {
    // `now` is the signing clock, `Auth::ts`'s; `at` what is stored and shown.
    let now = unix_now();
    let at = now as i64;
    let key = match app.seen.check(&s.auth, Action::Submit, &s.run, now) {
        Ok(key) => key,
        Err(e) => return Response::Unauthorized { why: e.to_string(), now },
    };
    let who = score::fingerprint(&key);
    let nick = score::clean_nick(&s.nick);
    let version: String = s.version.chars().filter(|c| !c.is_control()).take(MAX_VERSION).collect();
    if let Err(e) = app.db.lock().unwrap().seen(&key, &nick, &version, at) {
        return failed(e);
    }
    let reject = |rejected: Rejected| {
        info!("{who} rejected: {rejected}");
        if let Err(e) = app.db.lock().unwrap().rejected(&key, rejected.code(), &version, at) {
            error!("{e}");
        }
        Response::Rejected(rejected)
    };
    let banned = app.db.lock().unwrap().banned(&key);
    match banned {
        Ok(false) => {}
        Ok(true) => return reject(Rejected::Banned),
        Err(e) => return failed(e),
    }
    let Some(_slot) = app.limits.start(&key, now) else {
        return Response::TooMany;
    };
    let v = match verify(app, &s.run, s.claimed_ms).await {
        Ok(v) => v,
        Err(Refusal::Rejected(r)) => return reject(r),
        Err(Refusal::Busy) => return Response::Busy,
        Err(Refusal::Internal(e)) => return failed(e),
    };
    let suspect_of = suspect_of(app, &key, &v);

    let db = app.db.lock().unwrap();
    let inserted = db.insert(&NewRun {
        player: &key,
        board: v.board,
        character: v.run.demo.character as u8,
        final_ms: v.outcome.final_ms,
        perfect: v.outcome.perfect,
        at,
        pads_hash: &v.pads_hash,
        run: &s.run,
        signed_ts: s.auth.ts,
        signature: &s.auth.sig,
        suspect_of,
    });
    let run_id = match inserted {
        Ok(Ok(id)) => id,
        Ok(Err(of_run)) => {
            drop(db);
            return reject(Rejected::Duplicate { of_run });
        }
        Err(e) => return failed(e),
    };
    let (best, top) = match (db.player_best(v.board, &key), db.top(v.board, 1)) {
        (Ok(Some(best)), Ok(top)) => (best, top),
        (Err(e), _) | (_, Err(e)) => return failed(e),
        (Ok(None), _) => return failed("a run just inserted isn't on its board"),
    };
    info!(
        "{who} {:?} {:016x} {} laps: {} ms, run {run_id}, rank {}{}",
        v.board.kind,
        v.board.map_hash,
        v.board.laps,
        v.outcome.final_ms,
        best.rank,
        suspect_of.map_or(String::new(), |of| format!(", suspect: much like run {of}"))
    );
    Response::Submitted(Submitted {
        run_id,
        rank: best.rank,
        pb_ms: best.final_ms,
        top_ms: top.first().map_or(best.final_ms, |t| t.final_ms),
    })
}

fn forget(app: &App, auth: score::Auth) -> Response {
    let now = unix_now();
    let key = match app.seen.check(&auth, Action::Forget, &[], now) {
        Ok(key) => key,
        Err(e) => return Response::Unauthorized { why: e.to_string(), now },
    };
    let forgotten = app.db.lock().unwrap().forget(&key);
    match forgotten {
        Ok(runs) => {
            info!("{} forgotten, with {runs} runs", score::fingerprint(&key));
            Response::Forgotten { runs }
        }
        Err(e) => failed(e),
    }
}

async fn handle(app: &Arc<App>, request: Request) -> Response {
    match request {
        Request::Submit(s) => submit(app, s).await,
        Request::Forget { auth } => forget(app, auth),
        Request::Boards { sim_version } => {
            app.db.lock().unwrap().boards(sim_version).map_or_else(failed, Response::Boards)
        }
        Request::Top { board, n } => {
            app.db.lock().unwrap().top(board, n.min(MAX_TOP)).map_or_else(failed, Response::Top)
        }
        Request::Player { board, key } => {
            let Ok(key) = score::PublicKey::from_bytes(&key) else {
                return Response::Player(None);
            };
            app.db.lock().unwrap().player_best(board, &key).map_or_else(failed, Response::Player)
        }
        Request::Run { id } => app.db.lock().unwrap().run_bytes(id).map_or_else(failed, Response::Run),
    }
}

/// Answers the requests of a connection, until it ends.
async fn serve(app: Arc<App>, connection: Connection) {
    let peer = connection.remote_id().fmt_short();
    debug!(%peer, "connected");
    while let Ok((mut send, mut recv)) = connection.accept_bi().await {
        let request = tokio::time::timeout(READ_TIMEOUT, recv.read_to_end(MAX_REQUEST)).await;
        let request = match request {
            Ok(Ok(bytes)) => Request::from_bytes(&bytes),
            _ => {
                warn!(%peer, "opened a stream and sent no request");
                connection.close(1u32.into(), b"no request");
                break;
            }
        };
        let Ok(request) = request else {
            warn!(%peer, "sent a request that does not parse");
            connection.close(1u32.into(), b"bad request");
            break;
        };
        let response = handle(&app, request).await;
        if send.write_all(&response.to_bytes()).await.is_err() || send.finish().is_err() {
            break;
        }
    }
    debug!(%peer, "disconnected");
}

/// Ctrl-C, or SIGTERM: `docker stop`'s, which PID 1 would otherwise ignore until killed.
async fn shutdown() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let Ok(mut term) = signal(SignalKind::terminate()) else {
            let _ = tokio::signal::ctrl_c().await;
            return;
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
}

async fn run() -> Result<(), Error> {
    let args = args().map_err(|e| format!("{e} ({USAGE})"))?;
    if !args.command.is_empty() {
        let db = Db::open(&args.db).map_err(|e| format!("{}: {e}", args.db.display()))?;
        let at = unix_now() as i64;
        return operator::run(&db, &args.command, at).map_err(|e| format!("{e} ({USAGE})").into());
    }
    let maps = args.maps.ok_or_else(|| format!("--maps is needed ({USAGE})"))?;
    let tracks = Tracks::load(&maps)?;
    let db = Db::open(&args.db).map_err(|e| format!("{}: {e}", args.db.display()))?;
    let endpoint = bind(secret_key(&args.key)?, args.port).await?;
    info!("score id: {} (sim version {})", endpoint.id(), race::SIM_VERSION);

    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    let app = Arc::new(App {
        db: Mutex::new(db),
        tracks,
        verifiers: Semaphore::new(cores.saturating_sub(1).max(1)),
        seen: Seen::default(),
        limits: Limits::default(),
        matchmaker: Mutex::default(),
    });
    {
        let app = app.clone();
        tokio::spawn(async move {
            let mut every = tokio::time::interval(Duration::from_secs(1));
            loop {
                every.tick().await;
                app.matchmaker.lock().unwrap().tick(unix_now());
            }
        });
    }
    let slots = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    let shutdown = shutdown();
    tokio::pin!(shutdown);
    loop {
        let incoming = tokio::select! {
            incoming = endpoint.accept() => incoming,
            _ = &mut shutdown => break,
        };
        let Some(incoming) = incoming else { break };
        let Ok(slot) = slots.clone().try_acquire_owned() else {
            warn!("refused a connection: {MAX_CONNECTIONS} are open");
            incoming.refuse();
            continue;
        };
        let Ok(accepting) = incoming.accept() else {
            continue;
        };
        let app = app.clone();
        tokio::spawn(async move {
            if let Ok(connection) = accepting.await {
                if connection.alpn() == score::matching::MATCH_ALPN {
                    matching::serve(app, connection).await;
                } else {
                    serve(app, connection).await;
                }
            }
            drop(slot);
        });
    }
    info!("shutting down");
    endpoint.close().await;
    Ok(())
}

#[tokio::main]
async fn main() {
    // `RUST_LOG` sets what is shown, like `score_server=debug` for every connection.
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn,score_server=info".into()),
        )
        .with_ansi(std::io::stdout().is_terminal())
        .init();
    if let Err(e) = run().await {
        error!("{e}");
        std::process::exit(1);
    }
}
