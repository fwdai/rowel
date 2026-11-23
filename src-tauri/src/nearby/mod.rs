//! Nearby sync (`docs/nearby-sync-design.md`): unlocked installs on one network sync the lockboxes they share.

use std::io;

mod app;
pub mod channel;
pub mod commands;
pub mod discovery;
pub mod exchange;
pub mod identity;
pub mod session;
pub mod trust;

pub(crate) use app::{
    forget_packs, kept_packs, password_changed, request_run, session_ended, session_opened,
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("nearby connection: {0}")]
    Io(io::Error),
    #[error("nearby connection: {0}")]
    Tls(#[from] rustls::Error),
    #[error("the nearby device stopped answering")]
    TimedOut,
    #[error("a message of {len} bytes is over the {max} accepted")]
    TooLarge { len: usize, max: usize },
    #[error("the nearby device broke the protocol: {0}")]
    Protocol(String),
    #[error("the sync was ended here: {0}")]
    Ended(&'static str),
    #[error("nearby identity: {0}")]
    Identity(String),
    #[error("{0}")]
    Malformed(&'static str),
    #[error("nearby discovery: {0}")]
    Discovery(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        // A socket's deadline surfaces as `WouldBlock` on Unix and `TimedOut` on Windows.
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
