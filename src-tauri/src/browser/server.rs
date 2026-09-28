//! The listener the extension's proxy connects to, and the loop that serves
//! one connection.

use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use interprocess::local_socket::{prelude::*, Listener, ListenerOptions};
use tauri::AppHandle;

use super::actions::{Connection, Host};
use super::{frame, socket_name, AppHost};
use crate::{settings, storage};

// One listener per process, for its whole life: once the host is on, turning
// it off in Settings only makes the accept loop refuse what arrives, and the
// manifests are gone, so no browser launches a proxy to arrive anyway.
static STARTED: AtomicBool = AtomicBool::new(false);

/// Start listening, once. A failure to bind is logged and leaves the host off;
/// the next enable from Settings tries again.
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
                .and_then(|root| bind(&root));
            let listener = match listener {
                Ok(listener) => listener,
                Err(e) => {
                    log::warn!("browser host: could not listen: {e}");
                    STARTED.store(false, Ordering::SeqCst);
                    return;
                }
            };
            log::info!("browser host: listening");
            for incoming in listener.incoming() {
                let stream = match incoming {
                    Ok(stream) => stream,
                    Err(e) => {
                        log::warn!("browser host: a connection failed: {e}");
                        continue;
                    }
                };
                // Turned off since: the stream drops here, unanswered.
                if !settings::current(&app).browser.enabled {
                    continue;
                }
                let host = AppHost(app.clone());
                std::thread::spawn(move || serve(stream, host));
            }
        });
    if let Err(e) = spawned {
        log::warn!("browser host: could not start: {e}");
        STARTED.store(false, Ordering::SeqCst);
    }
}

/// The listener at `root`'s socket. A socket file a crashed process left
/// behind is replaced rather than refused; the single-instance guard is what
/// keeps two live apps from contending for one.
///
/// The socket file is made owner-only after the bind rather than through the
/// listener's `mode` option, which macOS does not support (the crate answers
/// `Unsupported`, and that is what "could not listen" was). The moment
/// between the two is covered by the data directory, which is `0700` from
/// its creation, so nothing else could reach the file in it anyway.
pub fn bind(root: &Path) -> io::Result<Listener> {
    let listener = ListenerOptions::new()
        .name(socket_name(root)?)
        .try_overwrite(true)
        .create_sync()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            root.join(super::SOCKET_FILE),
            std::fs::Permissions::from_mode(0o600),
        )?;
    }
    Ok(listener)
}

/// Answer requests on one connection until the other side goes away — or the
/// host is switched off, which ends the connection at its next request.
pub fn serve<H: Host>(mut stream: impl Read + Write, host: H) {
    let mut connection = Connection::new(host);
    loop {
        let request = match frame::read(&mut stream) {
            Ok(Some(request)) => request,
            Ok(None) => break,
            Err(e) => {
                log::debug!("browser host: connection ended: {e}");
                break;
            }
        };
        if !connection.host().enabled() {
            break;
        }
        let reply = connection.handle(&request);
        if frame::write(&mut stream, &reply).is_err() {
            break;
        }
    }
}
