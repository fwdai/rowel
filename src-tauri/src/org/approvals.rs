//! The approvals a company's Rowel Server routes to this member, put to the
//! person in the app's own grant dialog.
//!
//! While the vault is open, one thread per server this install is signed in
//! to with an approved device follows that server's approvals stream
//! (`client::approvals_stream`) and puts every ask it carries through the
//! consent slot (`browser::ask_remote_grant`) — the same slot, the same
//! dialog and the same `agent_grant_respond` as a local agent's ask, with
//! the tenant named so the person can see it is not this machine's. The
//! answer is posted back; an ask left unanswered posts nothing, since the
//! server times it out for itself and must not be told "no". No audit row is
//! written here: the server's log is authoritative for its own grants.
//!
//! The threads start when a session opens (`session::opened`) and stop when
//! it is sealed (`session::sealed`): a stop flag every thread reads between
//! frames, which is why the server has to keep sending keepalive comments —
//! a frame is the moment the flag is looked at, and the stream's read
//! timeout (60 s) is the longest a thread can sit without one. A stream that
//! ends or fails is reconnected with backoff, 5 s doubling to 60 s, for as
//! long as the session is live; a session the server no longer accepts is
//! refreshed once per attempt. A sign-in, an approval or a forgotten server
//! restarts the set ([`restart`]), so the threads always match the
//! connections on disk.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rowel_broker::actions::ClientKind;
use rowel_broker::agents::{now_ms, Verdict};
use tauri::async_runtime::block_on;
use tauri::AppHandle;

use super::client::{session_over, Approval, DeviceState, ServerClient};
use super::connections::{self, Connection, SessionResolver};
use crate::error::{Error, Result};
use crate::{browser, events, settings};

const BACKOFF_MIN: Duration = Duration::from_secs(5);
const BACKOFF_MAX: Duration = Duration::from_secs(60);
/// How long an ask waits for the consent slot while another decision is on
/// screen, between tries.
const BUSY_RETRY: Duration = Duration::from_secs(2);
/// How often a thread waiting on nothing in particular looks at the stop flag.
const STOP_POLL: Duration = Duration::from_millis(250);

/// The stop flag of the set of threads serving the current session, or
/// `None` while no set runs.
static LIVE: Mutex<Option<Arc<AtomicBool>>> = Mutex::new(None);

fn live() -> std::sync::MutexGuard<'static, Option<Arc<AtomicBool>>> {
    LIVE.lock().unwrap_or_else(|e| e.into_inner())
}

/// A session opened: follow every approved connection's approvals until
/// [`stop`]. Nothing to do while a set already runs.
pub(crate) fn start(app: &AppHandle) {
    let mut live = live();
    if live.is_some() {
        return;
    }
    let stop = Arc::new(AtomicBool::new(false));
    *live = Some(stop.clone());
    let app = app.clone();
    let spawned = std::thread::Builder::new()
        .name("org-approvals".into())
        .spawn(move || supervise(app, stop));
    if let Err(e) = spawned {
        log::warn!("org approvals: could not start: {e}");
        *live = None;
    }
}

/// The session was sealed: every thread of the set stands down at its next
/// frame, and the ask one of them has up is refused by the seal itself
/// (`agents::clear`).
pub(crate) fn stop() {
    if let Some(stop) = live().take() {
        stop.store(true, Ordering::SeqCst);
    }
}

/// The connections changed under a live session: the set is rebuilt from
/// what is on disk now.
pub(crate) fn restart(app: &AppHandle) {
    stop();
    start(app);
}

// One thread per approved connection, once the app key is there to read
// them with. The unlock that opened the session puts the key into the ring a
// moment after announcing the session, so the first reads may find the ring
// without it; a session that never gets one is one no server can be reached
// under, and the wait ends with the session.
fn supervise(app: AppHandle, stop: Arc<AtomicBool>) {
    let connections = loop {
        if stop.load(Ordering::SeqCst) {
            return;
        }
        let read = connections::app_root_and_key(&app)
            .and_then(|(root, app_key)| connections::load(&root, &app_key));
        match read {
            Ok(connections) => break connections,
            Err(Error::Locked) => std::thread::sleep(STOP_POLL),
            Err(e) => {
                log::warn!("org approvals: the server connections could not be read: {e}");
                return;
            }
        }
    };
    for connection in connections
        .into_iter()
        .filter(|c| c.device_state == Some(DeviceState::Approved))
    {
        let (app, stop, url) = (app.clone(), stop.clone(), connection.url.clone());
        let spawned = std::thread::Builder::new()
            .name("org-approvals-stream".into())
            .spawn(move || {
                let resolver = SessionResolver::App(app.clone());
                follow(&resolver, &connection.url, &stop, |approval| {
                    ask_person(&app, &stop, &connection.tenant_name, approval)
                });
            });
        if let Err(e) = spawned {
            log::warn!("org approvals: could not follow {url}: {e}");
        }
    }
}

/// Follow the approvals of the server at `url` under `resolver` until
/// `stop`: every approval is put to `ask`, and what it decides is posted
/// back; one it leaves unanswered posts nothing. The stream is reconnected
/// with backoff for as long as this runs, and a session the server no longer
/// accepts is refreshed once per attempt. Blocking: the caller's thread is
/// the one that asks.
pub(crate) fn follow(
    resolver: &SessionResolver,
    url: &str,
    stop: &Arc<AtomicBool>,
    mut ask: impl FnMut(&Approval) -> Verdict,
) {
    let mut backoff = BACKOFF_MIN;
    while !stop.load(Ordering::SeqCst) {
        let started = Instant::now();
        // The session the stream is opened under is kept to name it if the
        // server answers that it is over: a refresh replaces that one, and
        // never a later one another caller has already put in its place.
        let ended = match resolver.connection(url) {
            Ok(session) => {
                attempt(resolver, &session, stop, &mut ask).map_err(|e| (Some(session), e))
            }
            Err(e) => Err((None, e)),
        };
        if stop.load(Ordering::SeqCst) {
            return;
        }
        match ended {
            Ok(()) => log::info!("org approvals: the Rowel Server at {url} ended the stream"),
            Err((Some(session), e)) if session_over(&e) => {
                if let Err(e) = block_on(resolver.refresh(&session)) {
                    log::warn!("org approvals: the session at {url} could not be refreshed: {e}");
                }
            }
            Err((_, e)) => log::warn!("org approvals: the stream from {url} failed: {e}"),
        }
        // A stream that held for a while earned a fresh start; one that
        // failed at once waits longer each time.
        let wait = if started.elapsed() >= BACKOFF_MAX {
            BACKOFF_MIN
        } else {
            backoff
        };
        sleep_unless_stopped(stop, wait);
        backoff = (wait * 2).min(BACKOFF_MAX);
    }
}

enum Frame {
    Approval(Approval),
    Ended(Result<()>),
}

// One stream, from connect to its end. The stream itself runs as a task on
// the async runtime and forwards what it reads; this thread takes the
// approvals, asks, and posts the answers. Split that way so a dialog the
// person takes a minute over holds no network I/O, and so the posting's own
// `block_on` is never nested inside the stream's.
fn attempt(
    resolver: &SessionResolver,
    connection: &Connection,
    stop: &Arc<AtomicBool>,
    ask: &mut impl FnMut(&Approval) -> Verdict,
) -> Result<()> {
    let url = connection.url.as_str();
    let client = ServerClient::with_session(&connection.url, &connection.access_token)?;
    let (tx, rx) = channel::<Frame>();
    let task_stop = stop.clone();
    tauri::async_runtime::spawn(async move {
        let outcome = client
            .approvals_stream(|frame| {
                if task_stop.load(Ordering::SeqCst) {
                    return false;
                }
                match frame {
                    Some(approval) => tx.send(Frame::Approval(approval)).is_ok(),
                    None => true,
                }
            })
            .await;
        let _ = tx.send(Frame::Ended(outcome));
    });
    loop {
        match rx.recv_timeout(STOP_POLL) {
            Ok(Frame::Approval(approval)) => {
                // Decided by the server already; a dialog for it would be
                // answering nothing.
                if approval.expires_at_ms <= now_ms() {
                    continue;
                }
                if let Verdict::Answered(answer) = ask(&approval) {
                    let posted = block_on(resolver.as_member(url, |client| {
                        let id = approval.id.clone();
                        let answer = answer.clone();
                        async move { client.answer(&id, answer).await }
                    }));
                    if let Err(e) = posted {
                        log::warn!("org approvals: the answer could not be posted to {url}: {e}");
                    }
                }
            }
            Ok(Frame::Ended(outcome)) => return outcome,
            Err(RecvTimeoutError::Timeout) => {
                if stop.load(Ordering::SeqCst) {
                    return Ok(());
                }
            }
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        }
    }
}

fn sleep_unless_stopped(stop: &AtomicBool, wait: Duration) {
    let until = Instant::now() + wait;
    while !stop.load(Ordering::SeqCst) {
        let Some(left) = until.checked_duration_since(Instant::now()) else {
            return;
        };
        std::thread::sleep(left.min(STOP_POLL));
    }
}

// An approval put to the person through the consent slot, as the app's own
// grant asks are, with `tenant` named: a fresh ask id, the agent as the
// client, the server's targets and candidates as the dialog draws them, and
// no session end — a remote grant signs nothing out on this machine. Asked
// again every `BUSY_RETRY` while another decision holds the slot, until the
// server's own deadline for it.
fn ask_person(app: &AppHandle, stop: &AtomicBool, tenant: &str, approval: &Approval) -> Verdict {
    let id = crate::crypto::random_hex_id();
    let step_up = approval.step_up;
    let verify = step_up
        .then(browser::StepUp::current)
        .map(browser::StepUp::as_str);
    let default_standing_days = settings::current(app).browser.agents.default_standing_days;
    loop {
        let ask = events::AgentGrantAsk {
            id: &id,
            client_name: &approval.agent_name,
            client_kind: ClientKind::Agent,
            targets: approval.targets.clone(),
            mode: approval.mode.as_str(),
            reason: approval.reason.as_deref(),
            ends_session: false,
            step_up,
            verify,
            candidates: approval.candidates.clone(),
            default_standing_days,
            // *This run* is Tier 1's: a routed ask is not offered it.
            run_hours: None,
            server: Some(tenant),
        };
        match browser::ask_remote_grant(app, &id, step_up, || events::agent_grant(app, ask)) {
            Ok(verdict) => return verdict,
            Err(browser::Busy) => {
                let retry_at = now_ms().saturating_add(BUSY_RETRY.as_millis() as i64);
                if stop.load(Ordering::SeqCst) || retry_at >= approval.expires_at_ms {
                    return Verdict::Unanswered;
                }
                std::thread::sleep(BUSY_RETRY);
            }
        }
    }
}
