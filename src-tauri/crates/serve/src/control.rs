//! How `rowel approve` and `rowel admin` reach the running broker: an
//! owner-only local socket beside the broker's own (`serve.sock` on Unix, a
//! named pipe on Windows), carrying the broker's length-prefixed frames of
//! JSON. Requests are `{"op":"pending"}`, `{"op":"answer", …}`,
//! `{"op":"lock"}` and `{"op":"status"}`; a reply is `{"ok":true, …}` or
//! `{"ok":false,"error":"…"}`.
//!
//! The trust model is the same user (spec `10` Q9), the same as the
//! terminal the spec names for an approval: the socket is `0600` in a
//! `0700` directory, so whoever can speak to it is already running as the
//! person who unlocked the lockbox, and could read the terminal they would
//! otherwise type at. The master password for a step-up ask travels only
//! here, never over the broker's own socket or the WebSocket, and is wiped
//! once compared.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, Read, Write};
use std::path::Path;
use std::thread;

use interprocess::local_socket::{prelude::*, Listener, ListenerOptions, Name, Stream};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use rowel_broker::frame;

use crate::approvals::PendingAsk;
use crate::host::{AnswerRequest, HeadlessHost, Status};

/// The control socket's file on Unix, beside `browser.sock`.
pub const SOCKET_FILE: &str = "serve.sock";

/// The lock file that makes one broker per data directory.
pub const LOCK_FILE: &str = "serve.lock";

/// Why the data directory cannot be served.
#[derive(Debug)]
pub enum GuardError {
    /// Another `rowel serve` holds the lock: it is running on this data
    /// directory now.
    Held,
    Io(io::Error),
}

impl std::fmt::Display for GuardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Held => f.write_str(
                "another rowel serve is running on this data directory; stop it, or point this one at another --data-dir",
            ),
            Self::Io(e) => write!(f, "could not take the lock: {e}"),
        }
    }
}

impl std::error::Error for GuardError {}

impl From<io::Error> for GuardError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// One broker per data directory: an exclusive advisory lock on `serve.lock`
/// in it, taken before any socket is touched and held for the life of the
/// process (the kernel releases it when the process ends, however it ends).
/// It is what lets `bind` replace a socket file without looking: a second
/// broker never gets that far, so a file that is there is a dead process's.
/// Without it a second `rowel serve` would unlink the first one's control
/// socket on its way to failing to start, and leave the first running and
/// unreachable by `rowel approve`.
pub struct Guard {
    _lock: File,
}

impl Guard {
    pub fn take(root: &Path) -> Result<Self, GuardError> {
        let path = root.join(LOCK_FILE);
        let lock = OpenOptions::new()
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
        match lock.try_lock() {
            Ok(()) => Ok(Self { _lock: lock }),
            Err(TryLockError::WouldBlock) => Err(GuardError::Held),
            Err(TryLockError::Error(e)) => Err(GuardError::Io(e)),
        }
    }
}

/// The control socket in `root`: a file in it on Unix, on Windows a named
/// pipe whose name carries a digest of `root`, as the broker's own is
/// named, under a suffix of its own.
pub fn socket_name(root: &Path) -> io::Result<Name<'static>> {
    #[cfg(unix)]
    {
        use interprocess::local_socket::GenericFilePath;
        root.join(SOCKET_FILE).to_fs_name::<GenericFilePath>()
    }
    #[cfg(windows)]
    {
        use interprocess::local_socket::GenericNamespaced;
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(root.to_string_lossy().as_bytes());
        format!(
            "{}.serve.{}",
            rowel_broker::socket::IDENTIFIER,
            hex::encode(&digest[..8])
        )
        .to_ns_name::<GenericNamespaced>()
    }
}

/// What the CLI sends.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum Request {
    Pending,
    Answer(AnswerRequest),
    Lock,
    Status,
}

/// The listener at `root`'s control socket, owner-only. A file already there
/// is replaced, which is safe only behind `Guard::take`: with the lock held,
/// the file is one a dead process left behind.
pub fn bind(root: &Path) -> io::Result<Listener> {
    let listener = ListenerOptions::new()
        .name(socket_name(root)?)
        .try_overwrite(true)
        .create_sync()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            root.join(SOCKET_FILE),
            std::fs::Permissions::from_mode(0o600),
        )?;
    }
    Ok(listener)
}

/// Serve `listener` for the life of the process, one thread per connection:
/// a request a frame, a reply a frame.
pub fn serve(listener: Listener, host: HeadlessHost) {
    for incoming in listener.incoming() {
        let stream = match incoming {
            Ok(stream) => stream,
            Err(e) => {
                log::warn!("rowel serve: control socket accept failed: {e}");
                continue;
            }
        };
        let host = host.clone();
        thread::spawn(move || serve_stream(stream, &host));
    }
}

fn serve_stream(mut stream: impl Read + Write, host: &HeadlessHost) {
    while let Ok(Some(request)) = frame::read(&mut stream) {
        let reply = match serde_json::from_slice::<Request>(&request) {
            Ok(request) => handle(request, host),
            Err(_) => json!({ "ok": false, "error": "not a control request" }),
        };
        let Ok(bytes) = serde_json::to_vec(&reply) else {
            break;
        };
        if frame::write(&mut stream, &bytes).is_err() {
            break;
        }
    }
}

fn handle(request: Request, host: &HeadlessHost) -> Value {
    match request {
        Request::Pending => json!({ "ok": true, "pending": host.pending() }),
        Request::Answer(answer) => match host.answer(answer) {
            Ok(()) => json!({ "ok": true }),
            Err(error) => json!({ "ok": false, "error": error }),
        },
        Request::Lock => {
            host.seal();
            json!({ "ok": true })
        }
        Request::Status => json!({ "ok": true, "status": host.status() }),
    }
}

/// The CLI's side: one connection, one call per op.
pub struct Client {
    stream: Stream,
}

impl Client {
    /// Connect to the broker serving `root`. Nothing listening is the error
    /// it is: `rowel serve` is not running there.
    pub fn connect(root: &Path) -> io::Result<Self> {
        let stream = Stream::connect(socket_name(root)?)?;
        Ok(Self { stream })
    }

    fn call(&mut self, request: &Request) -> io::Result<Value> {
        let bytes = serde_json::to_vec(request)?;
        frame::write(&mut self.stream, &bytes)?;
        let reply = frame::read(&mut self.stream)?
            .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "the broker hung up"))?;
        let reply: Value = serde_json::from_slice(&reply)?;
        if reply["ok"].as_bool() == Some(true) {
            Ok(reply)
        } else {
            Err(io::Error::other(
                reply["error"]
                    .as_str()
                    .unwrap_or("the broker refused")
                    .to_string(),
            ))
        }
    }

    pub fn pending(&mut self) -> io::Result<Vec<PendingAsk>> {
        let mut reply = self.call(&Request::Pending)?;
        serde_json::from_value(reply["pending"].take()).map_err(io::Error::other)
    }

    pub fn answer(&mut self, answer: AnswerRequest) -> io::Result<()> {
        self.call(&Request::Answer(answer)).map(|_| ())
    }

    pub fn lock(&mut self) -> io::Result<()> {
        self.call(&Request::Lock).map(|_| ())
    }

    pub fn status(&mut self) -> io::Result<Status> {
        let mut reply = self.call(&Request::Status)?;
        serde_json::from_value(reply["status"].take()).map_err(io::Error::other)
    }
}
