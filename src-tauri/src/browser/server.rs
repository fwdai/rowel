//! The listener the extension's proxy connects to, the loop that serves one
//! connection, and the lock signals every connection is sent.

use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex, MutexGuard};

use interprocess::local_socket::{prelude::*, Listener, ListenerOptions, RecvHalf};
use tauri::AppHandle;

use super::actions::{Connection, Host};
use super::{frame, socket_name, AppHost};
use crate::{settings, storage};

// One listener per process, for its whole life: once the host is on, turning
// it off in Settings only makes the accept loop refuse what arrives, and the
// manifests are gone, so no browser launches a proxy to arrive anyway.
static STARTED: AtomicBool = AtomicBool::new(false);

/// A connection's writing end, shared by the thread serving it and the lock
/// signals, so a signal never lands inside a reply.
pub type Writer = Arc<Mutex<dyn Write + Send>>;

// A connection's signal queue, by the id `register` handed out.
type Queue = (u64, Sender<&'static [u8]>);

// Every connection being served, for the lock signals to reach. Fed by the
// accept loop rather than by `serve`, which stays a loop over any stream.
static CLIENTS: Mutex<Vec<Queue>> = Mutex::new(Vec::new());
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

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
                let (reader, writer) = stream.split();
                let writer: Writer = Arc::new(Mutex::new(writer));
                let id = register(writer.clone());
                let host = AppHost(app.clone());
                std::thread::spawn(move || {
                    serve(Connected { reader, writer }, host);
                    forget(id);
                });
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

/// A local socket connection as `serve` sees it: read from its own half,
/// written through the half the lock signals share. `frame::write` is one
/// `write_all`, and one `write_all` is one hold of the lock.
struct Connected {
    reader: RecvHalf,
    writer: Writer,
}

impl Read for Connected {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.reader.read(buf)
    }
}

impl Write for Connected {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        hold(&self.writer).write(buf)
    }
    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        hold(&self.writer).write_all(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        hold(&self.writer).flush()
    }
}

fn hold(writer: &Writer) -> MutexGuard<'_, dyn Write + Send + 'static> {
    writer.lock().unwrap_or_else(|e| e.into_inner())
}

fn clients() -> MutexGuard<'static, Vec<Queue>> {
    CLIENTS.lock().unwrap_or_else(|e| e.into_inner())
}

/// Hand a connection's writer to the lock signals, until [`forget`] is
/// called with the id this returns.
///
/// Each connection gets a queue and a thread of its own to drain it, so the
/// signals reach it in the order they were raised — a workspace switch
/// raises a lock and an unlock back to back, and an extension that heard
/// them the other way round would show the vault just opened as sealed — and
/// so a peer that has stopped reading, its buffer full and the write to it
/// blocking, stalls its own thread and nothing else: not the lock that raised
/// the signal, not the accept loop, and not the signals to the other
/// browsers. A write that fails ends the thread, and with it the writer.
///
/// A thread blocked in such a write cannot see its queue dropped, so it
/// outlives [`forget`] — for as long as the peer neither reads nor goes
/// away. That is the proxy process the browser launched, and it is blocked
/// on the browser's stdio in turn: the moment the browser exits, the proxy
/// does, the socket closes, and the write fails. One thread and one socket
/// per hung browser, for the life of that browser, is the whole of it.
pub fn register(writer: Writer) -> u64 {
    let (sender, receiver) = channel::<&'static [u8]>();
    std::thread::spawn(move || {
        for message in receiver {
            if frame::write(&mut *hold(&writer), message).is_err() {
                break;
            }
        }
    });
    let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
    clients().push((id, sender));
    id
}

/// The connection `id` has ended: its queue is dropped, which ends its thread.
pub fn forget(id: u64) {
    clients().retain(|(known, _)| *known != id);
}

pub const LOCKED: &[u8] = br#"{"action":"database-locked"}"#;
pub const UNLOCKED: &[u8] = br#"{"action":"database-unlocked"}"#;

/// The vault just locked: every connected extension flips its icon, and asks
/// again before it fills anything.
pub fn notify_locked() {
    signal(LOCKED);
}

/// The vault just opened, or another workspace did: every connected extension
/// re-checks the hash, which says which one it is talking to now.
pub fn notify_unlocked() {
    signal(UNLOCKED);
}

// Queue `message` for every connection, in the clear and unsolicited, as
// KeePassXC sends its signals: no nonce to seal under, and nothing in them
// the extension would not learn from its next ask. Never waits: a queue is
// unbounded, and one whose thread has ended — its write failed, the peer is
// gone — refuses the send and goes from the list with it.
fn signal(message: &'static [u8]) {
    clients().retain(|(_, queue)| queue.send(message).is_ok());
}
