//! The manager's life in the app: it runs while a session is open and nearby sync is on.

mod host;

use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use tauri::{AppHandle, Manager as _};

use self::host::AppHost;
use super::discovery::mdns::Mdns;
use super::identity::Identity;
use super::session::{Book, Config, Kept, Keys, Manager, Stored};
use crate::crypto::VaultKey;
use crate::error::{Error, Result};
use crate::state::AppState;
use crate::{events, storage};

const DIR: &str = "nearby";
const IDENTITY_FILE: &str = "identity.key";
const STATE_FILE: &str = "state.json";
const PERIOD: Duration = Duration::from_secs(30);

pub(super) struct NearbyState {
    pub book: Arc<Book>,
    running: Mutex<Running>,
    // One start at a time: a manager attaches to the book as it starts, and one
    // no longer wanted that attached after the wanted one would leave it unheard.
    starting: Mutex<()>,
    kept: Kept,
}

#[derive(Default)]
struct Running {
    generation: u64,
    manager: Option<Manager<AppHost>>,
}

static STATE: OnceLock<NearbyState> = OnceLock::new();

pub(super) fn state(app: &AppHandle) -> &'static NearbyState {
    STATE.get_or_init(|| {
        let path = dir(app).map(|dir| dir.join(STATE_FILE));
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
            starting: Mutex::default(),
            kept: Kept::default(),
        }
    })
}

fn dir(app: &AppHandle) -> Result<PathBuf> {
    Ok(storage::root_dir(app)?.join(DIR))
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

// Under the session guard: nothing here may take the session lock.
pub(crate) fn session_opened(app: &AppHandle) {
    let state = state(app);
    if state.book.enabled() {
        start(app, state);
    }
}

pub(crate) fn session_ended(app: &AppHandle) {
    let state = state(app);
    stop(state);
    state.kept.clear();
}

pub(crate) fn request_run(app: &AppHandle) {
    through_manager(app, Manager::request_runs, |_| {});
}

pub(crate) fn kept_packs(app: &AppHandle, key: &VaultKey) -> Vec<Vec<u8>> {
    state(app).kept.packs(&Keys::of(key).id)
}

pub(crate) fn forget_packs(app: &AppHandle) {
    state(app).kept.clear();
}

pub(super) fn set_enabled(app: &AppHandle, enabled: bool) {
    let state = state(app);
    state.book.set_enabled(enabled);
    let app_state = app.state::<AppState>();
    // Held across the start, so a lock cannot land between the look and it.
    let session = app_state.session.lock().unwrap();
    if enabled && session.is_unlocked() {
        start(app, state);
    } else if !enabled {
        stop(state);
    }
}

pub(super) fn set_paused(app: &AppHandle, device_id: &str, paused: bool) {
    through_manager(
        app,
        |manager| manager.set_paused(device_id, paused),
        |book| book.set_paused(device_id, paused),
    );
}

fn through_manager(
    app: &AppHandle,
    running: impl FnOnce(&Manager<AppHost>),
    stopped: impl FnOnce(&Book),
) {
    let state = state(app);
    match &state.running.lock().unwrap().manager {
        Some(manager) => running(manager),
        None => stopped(&state.book),
    }
}

fn start(app: &AppHandle, state: &'static NearbyState) {
    let generation = stop(state);
    let app = app.clone();
    let spawned = std::thread::Builder::new()
        .name("nearby-start".into())
        .spawn(move || {
            let _starting = state.starting.lock().unwrap();
            if state.running.lock().unwrap().generation != generation {
                return;
            }
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
        period: PERIOD,
    })
    .map_err(nearby)
}

fn identity_path(app: &AppHandle) -> Result<PathBuf> {
    let dir = dir(app)?;
    crate::store::create_private_dir(&dir).map_err(Error::Io)?;
    Ok(dir.join(IDENTITY_FILE))
}
