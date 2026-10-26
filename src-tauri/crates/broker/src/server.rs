//! The loop that serves one connection — the extension's, through the proxy
//! for Chrome and Firefox and through the app extension for Safari, or an
//! agent's — the listener it arrives on, and the lock signals every
//! connection is sent. A host brings its own accept loop (the app's
//! `browser::server`, a headless broker's) and hands each stream to
//! [`serve_stream`].

use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex, MutexGuard};

use interprocess::local_socket::{prelude::*, Listener, ListenerOptions, RecvHalf, Stream};

use crate::actions::{Connection, Host};
use crate::frame;
use crate::signal::ROWEL_SIGNAL;
use crate::socket::socket_name;

/// A connection's writing end, shared by the thread serving it and the lock
/// signals, so a signal never lands inside a reply.
pub type Writer = Arc<Mutex<dyn Write + Send>>;

/// Where a connection's signals go: one whole message per call, framed
/// however that connection's transport frames one. The socket's is a
/// `frame::write` through its shared [`Writer`]; a transport that is not a
/// length-prefixed stream joins the registry with a sink of its own.
pub type Sink = Box<dyn FnMut(&[u8]) -> io::Result<()> + Send>;

// A connection's signal queue, by the id `register` handed out.
type Queue = (u64, Sender<&'static [u8]>);

// Every connection being served, for the lock signals to reach. Fed by the
// accept loop rather than by `serve`, which stays a loop over any stream.
static CLIENTS: Mutex<Vec<Queue>> = Mutex::new(Vec::new());
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// The lock file that makes one broker per data directory.
pub const LOCK_FILE: &str = "broker.lock";

/// Why a data directory cannot be served.
#[derive(Debug)]
pub enum LockError {
    /// Another broker holds the lock: the app or a `rowel serve` is serving
    /// this data directory now.
    Held,
    Io(io::Error),
}

impl std::fmt::Display for LockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Held => f.write_str(
                "another broker (the Rowel app or a rowel serve) is serving this data directory",
            ),
            Self::Io(e) => write!(f, "could not take the broker lock: {e}"),
        }
    }
}

impl std::error::Error for LockError {}

impl From<io::Error> for LockError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// One broker per data directory, whatever kind and whatever it listens on:
/// an exclusive advisory lock on `broker.lock` in `root`, held for the life
/// of the process and released by the kernel however the process ends. The
/// app takes it before it binds the extension's socket; `rowel serve` takes
/// it before it asks for a password or binds anything. A second process of
/// either kind is refused here, so neither can replace the other's live
/// socket on its way to failing, and no two brokers write the same lockbox
/// under the same install id. It is also what lets [`bind`] replace a socket
/// file without looking: with the lock held, a file that is there is a dead
/// process's.
pub struct Lock {
    _file: std::fs::File,
}

impl Lock {
    pub fn take(root: &Path) -> Result<Self, LockError> {
        make_root(root)?;
        let path = root.join(LOCK_FILE);
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }
        match file.try_lock() {
            Ok(()) => Ok(Self { _file: file }),
            Err(std::fs::TryLockError::WouldBlock) => Err(LockError::Held),
            Err(std::fs::TryLockError::Error(e)) => Err(LockError::Io(e)),
        }
    }
}

/// `root`, made owner-only if it is missing: the Safari extension's is in
/// the App Group container (the app's `safari_socket_dir`), which nothing
/// may have created yet — the system makes it when a signed, entitled
/// process first asks for it, and an unsigned `tauri dev` build never does.
/// A plain directory made there serves the same, for the app and for the
/// extension alike.
fn make_root(root: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(root)
    }
    #[cfg(not(unix))]
    std::fs::create_dir_all(root)
}

/// The listener at `root`'s socket. A socket file already there is replaced
/// rather than refused, which is safe behind [`Lock::take`]: with the lock
/// held, the file is one a crashed process left behind.
///
/// The socket file is made owner-only after the bind rather than through the
/// listener's `mode` option, which macOS does not support (the crate answers
/// `Unsupported`, and that is what "could not listen" was). The moment
/// between the two is covered by the directory, which is `0700` from its
/// creation — the data directory, or the group container (the system makes
/// it `0700` as well) — so nothing else could reach the file in it anyway.
pub fn bind(root: &Path) -> io::Result<Listener> {
    make_root(root)?;
    let listener = ListenerOptions::new()
        .name(socket_name(root)?)
        .try_overwrite(true)
        .create_sync()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            root.join(crate::socket::SOCKET_FILE),
            std::fs::Permissions::from_mode(0o600),
        )?;
    }
    Ok(listener)
}

/// One accepted local socket connection, served for as long as it lasts, on
/// a thread of its own: its writing half is registered for the lock signals
/// first, so none raised while it is up is missed, and forgotten once
/// [`serve`] returns.
pub fn serve_stream<H: Host + Send + 'static>(stream: Stream, host: H) {
    let (reader, writer) = stream.split();
    let writer: Writer = Arc::new(Mutex::new(writer));
    let shared = writer.clone();
    let id = register(Box::new(move |message| {
        frame::write(&mut *hold(&shared), message)
    }));
    std::thread::spawn(move || {
        serve(Connected { reader, writer }, host);
        forget(id);
    });
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

/// Hand a connection's `sink` to the lock signals, until [`forget`] is
/// called with the id this returns.
///
/// Each connection gets a queue and a thread of its own to drain it, so the
/// signals reach it in the order they were raised — a workspace switch
/// raises a lock and an unlock back to back, and an extension that heard
/// them the other way round would show the vault just opened as sealed — and
/// so a peer that has stopped reading, its buffer full and the write to it
/// blocking, stalls its own thread and nothing else: not the lock that raised
/// the signal, not the accept loop, and not the signals to the other
/// browsers. A write that fails ends the thread, and with it the sink.
///
/// A thread blocked in such a write cannot see its queue dropped, so it
/// outlives [`forget`] — for as long as the peer neither reads nor goes
/// away. That is the proxy process the browser launched, and it is blocked
/// on the browser's stdio in turn: the moment the browser exits, the proxy
/// does, the socket closes, and the write fails. One thread and one socket
/// per hung browser, for the life of that browser, is the whole of it.
pub fn register(mut sink: Sink) -> u64 {
    let (sender, receiver) = channel::<&'static [u8]>();
    std::thread::spawn(move || {
        for message in receiver {
            if sink(message).is_err() {
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

/// The host has something queued for an extension (`AgentState::signals`):
/// a fill grant an agent obtained, a grant that ended. What it is stays
/// inside the sealed `get-signals` the extension answers this with; the
/// wake-up itself says nothing, like the lock signals beside it. Called with
/// every lock let go — a queue is unbounded and a send never waits, but the
/// order of things is that a signal follows the change it reports.
pub fn notify_signals() {
    signal(ROWEL_SIGNAL);
}

// Queue `message` for every connection, in the clear and unsolicited, as
// KeePassXC sends its signals: no nonce to seal under, and nothing in them
// the extension would not learn from its next ask. Never waits: a queue is
// unbounded, and one whose thread has ended — its write failed, the peer is
// gone — refuses the send and goes from the list with it.
fn signal(message: &'static [u8]) {
    clients().retain(|(_, queue)| queue.send(message).is_ok());
}
