//! The listeners the extension connects to — through the proxy for Chrome and
//! Firefox, through the app extension for Safari. Each connection accepted is
//! handed to the broker's loop (`rowel_broker::server`), which serves it and
//! keeps it on the list the lock signals reach; the signals are raised from
//! here under the names the app had for them.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use interprocess::local_socket::{prelude::*, Listener};
use tauri::AppHandle;

use rowel_broker::server::{bind, serve_stream, Lock};
pub use rowel_broker::server::{notify_locked, notify_signals, notify_unlocked};

use super::AppHost;
use crate::{settings, storage};

// One set of listeners per process (the data-directory socket, and on macOS
// the Safari one beside it), for its whole life: once the host is on, turning
// it off in Settings only makes the accept loop refuse what arrives, and the
// manifests are gone, so no browser launches a proxy to arrive anyway.
static STARTED: AtomicBool = AtomicBool::new(false);

// The one-broker lock on the data directory, held from the first bind to the
// end of the process: a `rowel serve` on the same directory is refused while
// the app holds it, and the app refuses to listen while one holds it.
static LOCK: Mutex<Option<Lock>> = Mutex::new(None);

/// Start listening, once. A failure to bind the data-directory socket — the
/// one Chrome and Firefox reach — is logged and leaves the host off; the next
/// enable from Settings tries again. So is a data directory another broker
/// is serving: `rowel serve` holds the same lock, and the app does not take
/// its socket over.
///
/// On macOS a second socket, in the App Group container, serves the Safari
/// extension (`safari_socket_dir`). It is bound best-effort after the first:
/// failing there says only that the Safari extension is unavailable, and
/// Chrome and Firefox are served all the same. The two listeners feed the
/// same connections: each accepted stream is registered for the lock signals
/// and served exactly alike.
pub fn start(app: &AppHandle) {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    let spawned = std::thread::Builder::new()
        .name("browser-host".into())
        .spawn(move || {
            let listener = storage::root_dir(&app)
                .map_err(|e| io::Error::other(e.to_string()))
                .and_then(|root| {
                    let lock = Lock::take(&root).map_err(io::Error::other)?;
                    let listener = bind(&root)?;
                    *LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(lock);
                    Ok(listener)
                });
            let listener = match listener {
                Ok(listener) => listener,
                Err(e) => {
                    log::warn!("browser host: could not listen: {e}");
                    STARTED.store(false, Ordering::SeqCst);
                    return;
                }
            };
            log::info!("browser host: listening");
            #[cfg(target_os = "macos")]
            start_safari(&app);
            accept(listener, &app);
        });
    if let Err(e) = spawned {
        log::warn!("browser host: could not start: {e}");
        STARTED.store(false, Ordering::SeqCst);
    }
}

// The Safari extension's listener, on a thread of its own.
#[cfg(target_os = "macos")]
fn start_safari(app: &AppHandle) {
    let listener = super::safari_socket_dir()
        .map_err(|e| e.to_string())
        .and_then(|dir| bind(&dir).map_err(|e| e.to_string()));
    let listener = match listener {
        Ok(listener) => listener,
        Err(e) => {
            log::warn!("browser host: Safari extension unavailable: {e}");
            return;
        }
    };
    let app = app.clone();
    let spawned = std::thread::Builder::new()
        .name("browser-host-safari".into())
        .spawn(move || {
            log::info!("browser host: listening for Safari");
            accept(listener, &app);
        });
    if let Err(e) = spawned {
        log::warn!("browser host: Safari extension unavailable: {e}");
    }
}

// Serve every connection `listener` accepts, for the life of the process.
fn accept(listener: Listener, app: &AppHandle) {
    for incoming in listener.incoming() {
        let stream = match incoming {
            Ok(stream) => stream,
            Err(e) => {
                log::warn!("browser host: a connection failed: {e}");
                continue;
            }
        };
        // Turned off since: the stream drops here, unanswered.
        if !settings::current(app).browser.enabled {
            continue;
        }
        serve_stream(stream, AppHost(app.clone()));
    }
}
