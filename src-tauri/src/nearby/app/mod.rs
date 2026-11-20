//! Nearby sync in the app's life: the manager runs while a session is open
//! and nearby sync is on — started when a session opens, stopped when it ends
//! (a lock, the auto-lock, a switch), and started or stopped by the switch in
//! Settings ([`set_enabled`]). The book, what nearby sync keeps per install,
//! lives for the whole process.
//!
//! The app reaches this module at three places only: the session's start and
//! end (`session::announce_opened`, `announce_ended`), and a change that asks
//! for a run (`commands::sync`'s `sync_now`, and its report of a cloud run
//! that merged rows). [`host`] is the app as the manager's host.

mod host;

use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use tauri::{AppHandle, Manager as _};

use self::host::AppHost;
use super::discovery::mdns::Mdns;
use super::identity::Identity;
use super::session::{Book, Config, Manager, Stored};
use crate::error::{Error, Result};
use crate::state::AppState;
use crate::{events, storage};

/// Nearby sync's own directory under the app's data root: this install's key
/// ([`IDENTITY_FILE`]) and what it keeps ([`STATE_FILE`]).
const DIR: &str = "nearby";
const IDENTITY_FILE: &str = "identity.key";
/// On or off, the devices paused, removed and seen. Not secret, like
/// `settings.json`, and kept the same way: one JSON file with defaults,
/// written atomically, a missing or unreadable one read as the defaults.
const STATE_FILE: &str = "state.json";

/// The book, for the process's whole life, and the manager while one runs.
pub(super) struct NearbyState {
    pub book: Arc<Book>,
    running: Mutex<Running>,
}

#[derive(Default)]
struct Running {
    /// Moved by every start and stop, so a start that finishes after a stop
    /// (or a later start) finds it moved and stands its manager down.
    generation: u64,
    manager: Option<Manager<AppHost>>,
}

static STATE: OnceLock<NearbyState> = OnceLock::new();

/// The state, read from disk the first time anything asks.
pub(super) fn state(app: &AppHandle) -> &'static NearbyState {
    STATE.get_or_init(|| {
        let path = storage::root_dir(app).map(|root| root.join(DIR).join(STATE_FILE));
        let stored = path.as_deref().map(load).unwrap_or_default();
        let emitter = app.clone();
        let book = Book::new(
            stored,
            crate::org::connect::device_name(),
            Box::new(move |stored| match &path {
                Ok(path) => save(path, stored),
                Err(e) => log::warn!("nearby: what it keeps has nowhere to go: {e}"),
            }),
            Box::new(move |status| events::nearby_status(&emitter, status)),
        );
        NearbyState {
            book: Arc::new(book),
            running: Mutex::default(),
        }
    })
}

fn load(path: &Path) -> Stored {
    let Ok(json) = std::fs::read_to_string(path) else {
        return Stored::default();
    };
    serde_json::from_str(&json).unwrap_or_else(|e| {
        log::warn!(
            "nearby: {} is not readable, starting from nothing: {e}",
            path.display()
        );
        Stored::default()
    })
}

fn save(path: &Path, stored: &Stored) {
    let written = path
        .parent()
        .map_or(Ok(()), |dir| {
            crate::store::create_private_dir(dir).map_err(Error::Io)
        })
        .and_then(|()| Ok(serde_json::to_string_pretty(stored)?))
        .and_then(|json| storage::atomic_write_file(path, &json));
    if let Err(e) = written {
        log::warn!("nearby: could not keep its state: {e}");
    }
}

/// A session opened: start the manager if nearby sync is on. Under the
/// session guard, so nothing here takes the session lock; the start itself is
/// on a thread of its own.
pub(crate) fn session_opened(app: &AppHandle) {
    let state = state(app);
    if state.book.enabled() {
        start(app, state);
    }
}

/// A session ended: the manager stops, and with it the listener, the
/// advertisement and every connection. Under the session guard; returns at
/// once.
pub(crate) fn session_ended(app: &AppHandle) {
    stop(state(app));
}

/// This device's lockboxes changed: every connected device runs.
pub(crate) fn request_run(app: &AppHandle) {
    if let Some(manager) = &state(app).running.lock().unwrap().manager {
        manager.request_runs();
    }
}

/// Turn nearby sync on or off: kept, then acted on under the session guard,
/// so a lock cannot land between the look at the session and the start.
pub(super) fn set_enabled(app: &AppHandle, enabled: bool) {
    let state = state(app);
    state.book.set_enabled(enabled);
    let app_state = app.state::<AppState>();
    let session = app_state.session.lock().unwrap();
    if enabled && session.is_unlocked() {
        start(app, state);
    } else if !enabled {
        stop(state);
    }
}

/// Pause or resume `device_id`: through the manager when one runs, so a
/// resume runs at once.
pub(super) fn set_paused(app: &AppHandle, device_id: &str, paused: bool) {
    let state = state(app);
    match &state.running.lock().unwrap().manager {
        Some(manager) => manager.set_paused(device_id, paused),
        None => state.book.set_paused(device_id, paused),
    }
}

/// Remove `device_id`: through the manager when one runs; otherwise kept
/// for the next one.
pub(super) fn remove(app: &AppHandle, device_id: &str) {
    let state = state(app);
    match &state.running.lock().unwrap().manager {
        Some(manager) => manager.remove(device_id),
        None => drop(state.book.remove(&[device_id.to_string()])),
    }
}

fn start(app: &AppHandle, state: &'static NearbyState) {
    let generation = stop(state);
    let app = app.clone();
    let spawned = std::thread::Builder::new()
        .name("nearby-start".into())
        .spawn(move || {
            let started = start_manager(&app, &state.book);
            let mut running = state.running.lock().unwrap();
            match started {
                Ok(manager) if running.generation == generation => running.manager = Some(manager),
                // Stopped, or started again, while this one was starting.
                Ok(manager) => manager.stop(),
                Err(e) => log::warn!("nearby: not started: {e}"),
            }
        });
    if let Err(e) = spawned {
        log::warn!("nearby: not started: {e}");
    }
}

/// Stop the manager if one runs. The generation that comes back is the one a
/// start begun now may install its manager under.
fn stop(state: &NearbyState) -> u64 {
    let mut running = state.running.lock().unwrap();
    running.generation += 1;
    if let Some(manager) = running.manager.take() {
        manager.stop();
    }
    running.generation
}

fn start_manager(app: &AppHandle, book: &Arc<Book>) -> Result<Manager<AppHost>> {
    let nearby = |e: super::Error| Error::Other(e.to_string());
    let identity = Identity::load_or_create(&identity_path(app)?).map_err(nearby)?;
    let discovery = Mdns::new().map_err(nearby)?;
    Manager::start(Config {
        host: Arc::new(AppHost::new(app)),
        identity,
        discovery: Box::new(discovery),
        book: book.clone(),
        listen: Ipv4Addr::UNSPECIFIED.into(),
    })
    .map_err(nearby)
}

fn identity_path(app: &AppHandle) -> Result<PathBuf> {
    let dir = storage::root_dir(app)?.join(DIR);
    crate::store::create_private_dir(&dir).map_err(Error::Io)?;
    Ok(dir.join(IDENTITY_FILE))
}
