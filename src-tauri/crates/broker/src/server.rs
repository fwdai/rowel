//! The loop that serves one connection — the extension's, through the proxy
//! for Chrome and Firefox and through the app extension for Safari, or an
//! agent's — and the listener it arrives on. A host brings its own accept
//! loop (the app's `browser::server`, a headless broker's) and hands each
//! stream to [`serve_stream`], which registers it on the host's own
//! [`Hub`](crate::hub::Hub) for the lock signals.

use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use interprocess::local_socket::{prelude::*, Listener, ListenerOptions, RecvHalf, Stream};

use crate::actions::{Connection, Host};
use crate::frame;
use crate::socket::socket_name;

/// A connection's writing end, shared by the thread serving it and the lock
/// signals, so a signal never lands inside a reply.
pub type Writer = Arc<Mutex<dyn Write + Send>>;

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
/// a thread of its own: its writing half is registered for `host`'s lock
/// signals first, so none raised while it is up is missed, and forgotten
/// once [`serve`] returns.
pub fn serve_stream<H: Host + Send + 'static>(stream: Stream, host: H) {
    let (reader, writer) = stream.split();
    let writer: Writer = Arc::new(Mutex::new(writer));
    let shared = writer.clone();
    let hub = host.hub().clone();
    let id = hub.register(Box::new(move |message| {
        frame::write(&mut *hold(&shared), message)
    }));
    std::thread::spawn(move || {
        serve(Connected { reader, writer }, host);
        hub.forget(id);
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
