//! Nearby sync (`docs/nearby-sync-design.md` §3 to §5): two Rowel installs
//! on the same network, both unlocked, syncing the lockboxes they share
//! directly. Blocking I/O on threads of its own, like the OAuth loopback
//! listener; no async runtime.
//!
//! - [`identity`]: this install's key pair, its certificate and its device id.
//! - [`exchange`]: one lockbox's revision, pack and merge, as two devices
//!   trade them.
//! - [`channel`]: TLS 1.3 between two identities, whole messages over it, and
//!   the JSON control messages they exchange.
//! - [`trust`]: the proof of a lockbox key, and the tags an advertisement
//!   carries.
//! - [`discovery`]: advertising this device and finding the others.
//! - [`session`]: the manager that drives all of it while the app is
//!   unlocked, against a host that is the app ([`app`]) or a test.
//! - [`app`] and [`commands`]: where the app meets it — the session's start
//!   and end, a change asking for a run, and Settings' four commands.

use std::io;

mod app;
pub mod channel;
pub mod commands;
pub mod discovery;
pub mod exchange;
pub mod identity;
pub mod session;
pub mod trust;

pub(crate) use app::{request_run, session_ended, session_opened};

/// What can go wrong on the nearby channel. Kept to this module and turned
/// into the app's error where a command reports it.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The socket failed or closed.
    #[error("nearby connection: {0}")]
    Io(io::Error),
    /// The TLS layer refused the peer or failed.
    #[error("nearby connection: {0}")]
    Tls(#[from] rustls::Error),
    /// A read or write stalled past the channel's deadline.
    #[error("the nearby device stopped answering")]
    TimedOut,
    /// The device that answered is not the one that was dialled.
    #[error("the device that answered is not the one expected")]
    UnexpectedPeer,
    /// A message longer than the side handling it accepts.
    #[error("a message of {len} bytes is over the {max} accepted")]
    TooLarge { len: usize, max: usize },
    /// The peer broke the protocol: a malformed frame or message, or a step
    /// out of order.
    #[error("the nearby device broke the protocol: {0}")]
    Protocol(String),
    /// This side ended the sync, saying nothing.
    #[error("the sync was ended here: {0}")]
    Ended(&'static str),
    /// This install's identity file could not be read, made or used.
    #[error("nearby identity: {0}")]
    Identity(String),
    /// Text that is not a well-formed device id.
    #[error("{0}")]
    Malformed(&'static str),
    /// Advertising or browsing on the local network failed.
    #[error("nearby discovery: {0}")]
    Discovery(String),
}

pub type Result<T> = std::result::Result<T, Error>;

// A socket's timeout surfaces as `WouldBlock` on Unix and `TimedOut` on
// Windows; rustls hands its own errors back wrapped in an `io::Error`.
impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        if matches!(
            e.kind(),
            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
        ) {
            return Self::TimedOut;
        }
        match e
            .get_ref()
            .and_then(|inner| inner.downcast_ref::<rustls::Error>())
        {
            Some(tls) => Self::Tls(tls.clone()),
            None => Self::Io(e),
        }
    }
}
