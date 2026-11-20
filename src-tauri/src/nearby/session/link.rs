//! The running side's [`PeerLink`] over a channel: every call one request and
//! its answer, with whatever else the other side says meanwhile kept for the
//! connection's loop.

use std::cell::RefCell;

use crate::error::{Error, Result};
use crate::nearby::channel::{Channel, Control, Failure};
use crate::nearby::exchange::{PeerHead, PeerLink};
use crate::sync::engine::RemoteFile;
use crate::sync::pack::MAX_PACK_BYTES;

/// The longest error sentence kept from the other side, in bytes.
const MAX_MESSAGE: usize = 300;

/// What a run over the link heard besides its answers.
#[derive(Default)]
pub(super) struct Heard {
    /// The other side answered `sync/busy`: ask again later.
    pub busy: bool,
    /// The channel failed: the connection is over, whatever the run says.
    pub broken: Option<crate::nearby::Error>,
    /// The other side asked for a run meanwhile.
    pub poked: bool,
    /// Removed ids it sent meanwhile.
    pub removed: Vec<String>,
}

pub(super) struct ChannelLink<'a> {
    channel: RefCell<&'a mut Channel>,
    vault: &'a str,
    now_ms: i64,
    heard: RefCell<Heard>,
}

impl<'a> ChannelLink<'a> {
    /// A link for one run of `vault`, whose clock reads `now_ms`.
    pub fn new(channel: &'a mut Channel, vault: &'a str, now_ms: i64) -> Self {
        Self {
            channel: RefCell::new(channel),
            vault,
            now_ms,
            heard: RefCell::default(),
        }
    }

    pub fn into_heard(self) -> Heard {
        self.heard.into_inner()
    }

    /// Send `request` (and `body` after it), then read up to its answer.
    fn request(&self, request: &Control, body: Option<&[u8]>) -> Result<Control> {
        let mut channel = self.channel.borrow_mut();
        let sent = channel
            .send_control(request)
            .and_then(|()| body.map_or(Ok(()), |body| channel.send(body)));
        if let Err(e) = sent {
            return Err(self.broke(e));
        }
        loop {
            let mut heard = self.heard.borrow_mut();
            match channel.recv_control() {
                Err(e) => {
                    drop(heard);
                    return Err(self.broke(e));
                }
                Ok(Control::Unsupported) => {}
                Ok(Control::Poke) => heard.poked = true,
                Ok(Control::Removed { ids }) => heard.removed.extend(ids),
                Ok(Control::Busy) => {
                    heard.busy = true;
                    return Err(Error::SyncBusy);
                }
                Ok(Control::Failed(failure)) => return Err(peer_error(failure)),
                Ok(answer) => return Ok(answer),
            }
        }
    }

    // The channel failed or the other side broke step: kept as the reason the
    // connection ends (the first failure; the rest follow from it), and an
    // error for the engine to stop on.
    fn broke(&self, e: crate::nearby::Error) -> Error {
        let message = e.to_string();
        self.heard.borrow_mut().broken.get_or_insert(e);
        Error::Other(message)
    }

    fn out_of_step(&self, answer: Control) -> Error {
        self.broke(crate::nearby::Error::Protocol(format!(
            "{answer:?} is not the answer asked for"
        )))
    }
}

impl PeerLink for ChannelLink<'_> {
    fn head(&self) -> Result<PeerHead> {
        let request = Control::Head {
            vault: self.vault.to_string(),
            now_ms: self.now_ms,
        };
        match self.request(&request, None)? {
            Control::HeadReply { revision, now_ms } => Ok(PeerHead { revision, now_ms }),
            other => Err(self.out_of_step(other)),
        }
    }

    fn fetch(&self) -> Result<RemoteFile> {
        let request = Control::Fetch {
            vault: self.vault.to_string(),
        };
        match self.request(&request, None)? {
            Control::Pack { revision } => {
                let bytes = self
                    .channel
                    .borrow_mut()
                    .recv(MAX_PACK_BYTES)
                    .map_err(|e| self.broke(e))?;
                Ok(RemoteFile { bytes, revision })
            }
            other => Err(self.out_of_step(other)),
        }
    }

    fn absorb(&self, pack: &[u8]) -> Result<String> {
        let request = Control::Absorb {
            vault: self.vault.to_string(),
        };
        match self.request(&request, Some(pack))? {
            Control::Absorbed { revision } => Ok(revision),
            other => Err(self.out_of_step(other)),
        }
    }
}

/// An error as it crosses the channel.
pub(super) fn failure_of(error: &Error) -> Failure {
    Failure {
        kind: error.kind().to_string(),
        message: error.to_string(),
    }
}

/// An error the other side raised, as this side's: the kinds that mean the
/// same thing here as there keep their variant, so they are told apart as
/// they would be for a cloud run, and every other is its sentence, bounded.
fn peer_error(failure: Failure) -> Error {
    match failure.kind.as_str() {
        "passwordChangedRemotely" => Error::PasswordChangedRemotely,
        "vaultUnderRetiredPassword" => Error::VaultUnderRetiredPassword,
        "vaultTooNew" => Error::VaultTooNew,
        _ => Error::Other(truncated(&failure.message, MAX_MESSAGE).to_string()),
    }
}

/// A failure the other side reported about a run, as this side keeps it:
/// as [`peer_error`] reads it, except a clock skew, whose sentence speaks of
/// both devices and so reads the same on either.
pub(super) fn bounded(failure: Failure) -> Failure {
    if failure.kind == "clockSkew" {
        return Failure {
            message: truncated(&failure.message, MAX_MESSAGE).to_string(),
            kind: failure.kind,
        };
    }
    failure_of(&peer_error(failure))
}

/// `text` cut to at most `max` bytes, at a character boundary.
fn truncated(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}
