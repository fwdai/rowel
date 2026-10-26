//! The process: `rowel serve --server`. Refuse a data directory `rowel admin
//! server init` never made into a server, take the broker lock, open the
//! state, listen — TLS from PEM files or plain behind a reverse proxy — for
//! every API and the agent WebSocket on one address, run the agents' timer,
//! serve until SIGTERM or SIGINT, seal every open lockbox so no master
//! outlives the process in its memory, exit. The shape and the voice are
//! `rowel_serve::run`'s; what is served is this crate's.

use std::future::IntoFuture;
use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::routing::get;
use axum::serve::Listener;
use axum::{Json, Router};
use serde_json::{json, Value};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, watch, Semaphore};
use tokio_rustls::server::TlsStream;
use tokio_rustls::TlsAcceptor;

use rowel_broker::server::{Lock, LockError};

use crate::config::{self, Config};
use crate::state::AppState;
use crate::store::DB_FILE;
use crate::{admin, agent, approvals, members, packs};

pub struct RunOptions {
    /// The data directory: `server.json`, `server.db`, `lockboxes/`.
    pub data_dir: PathBuf,
    /// The config file; `<data dir>/server.json` when `None`.
    pub config: Option<PathBuf>,
}

/// Run the server until it is told to stop. The exit code: 0 after a clean
/// stop, 1 when it could not start. A usage mistake, 2, is the command
/// line's to catch before this is called.
pub fn run(options: RunOptions) -> i32 {
    match serve(options) {
        Ok(()) => 0,
        Err(message) => {
            eprintln!("rowel serve --server: {message}");
            1
        }
    }
}

/// Every route the server answers, over `state`.
pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/v1/health", get(health))
        .merge(members::router())
        .merge(admin::router())
        .merge(approvals::router())
        .merge(agent::router())
        .merge(packs::router())
        .with_state(state)
}

// No auth and nothing about the tenant: whoever can reach the port may ask
// whether the process is up, and that is all they learn.
async fn health() -> Json<Value> {
    Json(json!({ "ok": true }))
}

/// How long a TLS handshake may take. An idle TCP connection must not hold
/// a handshake slot for longer.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// How many TLS handshakes may be under way at once; past it a connection is
/// closed before the handshake and the client retries. A handshake costs a
/// task and its buffers, and the port may face anyone.
const MAX_HANDSHAKES: usize = rowel_serve::listen::MAX_CONNECTIONS;

/// How long the connections still open at a stop signal have to finish
/// before the server seals and exits anyway. An approvals stream never
/// finishes on its own, and `systemctl stop` must not wait for it until the
/// unit's kill timeout.
const STOP_GRACE: Duration = Duration::from_secs(5);

fn serve(options: RunOptions) -> Result<(), String> {
    let data_dir = options.data_dir;
    // Opening the store creates it; a mistyped --data-dir must not become a
    // second, empty tenant, nor leave a directory behind.
    let db = data_dir.join(DB_FILE);
    if !db.exists() {
        return Err(format!(
            "no server database at {}; run rowel admin server init first",
            db.display()
        ));
    }

    // One broker per data directory, settled before the root key is read
    // and before anything listens, so two servers (or a server and a
    // headless broker) never write the same lockboxes under one writer id.
    let _lock = Lock::take(&data_dir).map_err(|e| match e {
        LockError::Held => {
            format!("{e}; stop it, or point rowel serve --server at another --data-dir")
        }
        other => other.to_string(),
    })?;

    let config_path = options.config.unwrap_or_else(|| config::path_in(&data_dir));
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let state = runtime
        .block_on(AppState::open(&config_path, &data_dir))
        .map_err(|e| e.to_string())?;

    let served = runtime.block_on(listen_and_serve(state.clone()));
    seal_all(&state);
    served?;
    eprintln!("rowel serve --server: sealed, exiting");
    Ok(())
}

async fn listen_and_serve(state: Arc<AppState>) -> Result<(), String> {
    let config = &state.config;
    let address: SocketAddr = config
        .listen
        .http
        .parse()
        .map_err(|e| format!("listen.http: {e}"))?;
    let tls = config
        .listen
        .tls
        .as_ref()
        .map(|tls| {
            rowel_serve::listen::tls_acceptor(&rowel_serve::config::Tls {
                cert: tls.cert.clone(),
                key: tls.key.clone(),
            })
        })
        .transpose()
        .map_err(|e| e.to_string())?;
    let tcp = TcpListener::bind(address)
        .await
        .map_err(|e| format!("cannot listen on {address}: {e}"))?;
    let bound = tcp.local_addr().map_err(|e| e.to_string())?;
    if tls.is_none() {
        warn_in_the_clear(config, bound);
    }

    let ticker = tokio::spawn(agent::run_ticker(state.clone()));
    let app = router(state);
    let outcome = match tls {
        Some(acceptor) => {
            eprintln!("rowel serve --server: listening on https://{bound}");
            serve_until_stopped(TlsListener::new(tcp, acceptor, bound), app).await
        }
        None => {
            eprintln!("rowel serve --server: listening on http://{bound}");
            serve_until_stopped(tcp, app).await
        }
    };
    ticker.abort();
    outcome
}

/// Serve on `listener` until SIGTERM or SIGINT, then let the open
/// connections finish for [`STOP_GRACE`] at most.
async fn serve_until_stopped<L>(listener: L, app: Router) -> Result<(), String>
where
    L: Listener<Addr = SocketAddr>,
{
    let (stop, mut stopped) = watch::channel(());
    let serving = axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = stopped.changed().await;
        })
        .into_future();
    tokio::pin!(serving);
    tokio::select! {
        ended = &mut serving => {
            return Err(match ended {
                Ok(()) => "the listener stopped".into(),
                Err(e) => format!("the listener failed: {e}"),
            });
        }
        () = rowel_serve::run::stop_signal() => {}
    }
    let _ = stop.send(());
    if tokio::time::timeout(STOP_GRACE, serving).await.is_err() {
        log::info!(
            "rowel serve --server: connections still open after {}s were cut",
            STOP_GRACE.as_secs()
        );
    }
    Ok(())
}

/// Plain HTTP is for a reverse proxy on this machine that terminates TLS in
/// front. Say so when the config suggests otherwise, since what crosses
/// this listener is bearer tokens, wrapped masters and lockbox packs.
fn warn_in_the_clear(config: &Config, bound: SocketAddr) {
    if !bound.ip().is_loopback() {
        eprintln!(
            "rowel serve --server: warning: listening in the clear on {bound}, which other \
             machines can reach; set listen.tls, or listen on 127.0.0.1 behind a reverse proxy"
        );
    }
    if config.public_url.starts_with("https://") {
        eprintln!(
            "rowel serve --server: publicUrl is {} and this listener is plain http: the reverse \
             proxy in front must terminate TLS",
            config.public_url
        );
    }
}

/// Close every open lockbox and wipe its master, with the rows of its last
/// session mirrored. The vaults are reached by the lockbox rows, since an
/// agent can only ever have opened a lockbox the store lists.
fn seal_all(state: &AppState) {
    match state.store.list_lockboxes() {
        Ok(lockboxes) => {
            for lockbox in lockboxes {
                state.vaults.seal(state, &lockbox.id);
            }
        }
        Err(e) => eprintln!(
            "rowel serve --server: the lockboxes are unreadable, nothing sealed explicitly: {e}"
        ),
    }
}

/// A TCP listener whose connections reach axum only once their TLS handshake
/// is done. The handshakes run on tasks of their own, each under
/// [`HANDSHAKE_TIMEOUT`], so a client that opens a connection and says
/// nothing holds up nobody else's; the finished streams come through a
/// channel that `accept` reads.
struct TlsListener {
    accepted: mpsc::Receiver<(TlsStream<TcpStream>, SocketAddr)>,
    bound: SocketAddr,
}

impl TlsListener {
    fn new(tcp: TcpListener, acceptor: TlsAcceptor, bound: SocketAddr) -> Self {
        let (handed, accepted) = mpsc::channel(MAX_HANDSHAKES);
        tokio::spawn(handshakes(tcp, acceptor, handed));
        Self { accepted, bound }
    }
}

async fn handshakes(
    tcp: TcpListener,
    acceptor: TlsAcceptor,
    handed: mpsc::Sender<(TlsStream<TcpStream>, SocketAddr)>,
) {
    let slots = Arc::new(Semaphore::new(MAX_HANDSHAKES));
    while !handed.is_closed() {
        let (stream, peer) = match tcp.accept().await {
            Ok(accepted) => accepted,
            Err(e) => {
                // Out of descriptors, most likely: a pause rather than a
                // loop that spins on the same error.
                log::warn!("rowel serve --server: accept failed: {e}");
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let Ok(slot) = slots.clone().try_acquire_owned() else {
            log::warn!(
                "rowel serve --server: {MAX_HANDSHAKES} TLS handshakes are under way; {peer} was turned away"
            );
            continue;
        };
        let acceptor = acceptor.clone();
        let handed = handed.clone();
        tokio::spawn(async move {
            let _slot = slot;
            match tokio::time::timeout(HANDSHAKE_TIMEOUT, acceptor.accept(stream)).await {
                Ok(Ok(tls)) => {
                    let _ = handed.send((tls, peer)).await;
                }
                Ok(Err(e)) => {
                    log::debug!("rowel serve --server: TLS handshake with {peer} failed: {e}")
                }
                Err(_) => log::debug!("rowel serve --server: TLS handshake with {peer} timed out"),
            }
        });
    }
}

impl Listener for TlsListener {
    type Io = TlsStream<TcpStream>;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        match self.accepted.recv().await {
            Some(accepted) => accepted,
            // The handshake task only ends once this receiver is gone, so
            // this is never reached while the listener is served.
            None => std::future::pending().await,
        }
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        Ok(self.bound)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_data_directory_never_initialised_is_refused_and_left_alone() {
        let temp = tempfile::tempdir().unwrap();
        let data_dir = temp.path().join("typo");
        assert_eq!(
            run(RunOptions {
                data_dir: data_dir.clone(),
                config: None
            }),
            1
        );
        assert!(!data_dir.exists());
    }
}
