//! Credentials another password manager hands over through the system: iOS 26
//! credential exchange, which Passwords offers as "Export Data to Another App".
//!
//! The transfer starts in the other app. The system launches this one with a
//! token, `CredentialExchange.swift` trades it for the credentials, and they
//! arrive here as a FIDO Credential Exchange Format document — the same JSON
//! the CXF importer reads from a file. It is held in memory, never on disk and
//! never in the webview, until `commands::import::import_exchange` previews it
//! and writes it into the vault.
//!
//! This module is the one place that knows what is held, and it says so every
//! time that changes (`exchange:held`): an arrival, an import, a failure told,
//! the end of a session. The webview keeps a copy and asks once, when it
//! starts listening (`commands::import::exchange_held`). Each transfer has a
//! number, which every preview and import names — so what is imported is what
//! was previewed, and a transfer that has been replaced or let go of is
//! refused rather than answered with another one's rows.
//!
//! A newer arrival replaces an older one. Otherwise a transfer is held until
//! it is imported or until the open session it was offered to ends — a lock,
//! or that lockbox being left or deleted: it does not outlive that session,
//! and is never carried into another lockbox. One that arrives with the vault
//! locked was offered to none, and waits for the unlock, through a choice of
//! lockbox on the lock screen if need be.
//!
//! Which session a transfer belongs to is decided here, under this module's
//! own lock, and not by looking at the session: the session says when one
//! opens and when one ends (`session_opened`, `session_ended`), an arrival
//! takes the same lock, and so each arrival is wholly before or wholly after
//! each end. Nothing reads what is held at one moment to act on it at another.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use serde::Serialize;
use tauri::AppHandle;
use zeroize::Zeroizing;

use crate::error::{Error, Result};

/// As large as an export file may be (`commands::import`).
const MAX_BYTES: usize = 25 * 1024 * 1024;

/// Which transfer: the count of changes when it came, so never reused within
/// a run of the app.
pub type Arrival = u64;

/// What is held, as the webview is told: which transfer, if any, and the
/// count of changes that answer has been through. The count is what puts two
/// tellings in order when they reach the webview out of it — an event sent
/// from another thread, the answer to a question asked before the news.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Held {
    pub arrival: Option<Arrival>,
    pub change: u64,
}

/// What the native side delivered: the document, or why there is none.
enum Delivered {
    Document(Zeroizing<Vec<u8>>),
    Failed(String),
}

struct Slot {
    held: Option<(Arrival, Delivered)>,
    changes: u64,
    /// Whether a session is open: what is held has been offered to it.
    offered: bool,
}

impl Slot {
    fn told(&self) -> Held {
        Held {
            arrival: self.held.as_ref().map(|(arrival, _)| *arrival),
            change: self.changes,
        }
    }

    /// Put `held` in the slot, and say what the webview is to be told of it.
    fn set(&mut self, held: Option<Delivered>) -> Held {
        self.changes += 1;
        self.held = held.map(|delivered| (self.changes, delivered));
        self.told()
    }
}

// Process-wide rather than in `AppState`: the native side calls in with no
// handle, and on a launch may do so before the app has one.
static SLOT: Mutex<Slot> = Mutex::new(Slot {
    held: None,
    changes: 0,
    offered: false,
});
static APP: OnceLock<AppHandle> = OnceLock::new();

/// Let changes be announced. Before this, a transfer is only parked, and the
/// webview learns of it by asking.
pub fn init(app: &AppHandle) {
    let _ = APP.set(app.clone());
}

// Told outside the slot's lock, so in no particular order across threads:
// `Held::change` is what orders it on the other side.
fn announce(held: Held) {
    if let Some(app) = APP.get() {
        crate::events::exchange_held(app, held);
    }
}

/// Park a document and announce it.
#[cfg_attr(not(any(target_os = "ios", test)), allow(dead_code))]
fn accept(document: &[u8]) {
    if document.len() > MAX_BYTES {
        return failed("the other app sent more than Rowel can import at once");
    }
    hold(Delivered::Document(Zeroizing::new(document.to_vec())));
}

/// Park the reason a transfer produced nothing, to be reported where the
/// import would have been previewed.
#[cfg_attr(not(any(target_os = "ios", test)), allow(dead_code))]
fn failed(reason: &str) {
    hold(Delivered::Failed(reason.to_owned()));
}

fn hold(delivered: Delivered) {
    let held = SLOT.lock().unwrap().set(Some(delivered));
    announce(held);
}

/// What is held now.
pub fn held() -> Held {
    SLOT.lock().unwrap().told()
}

/// The document of `arrival`, which stays held: a preview reads it and the
/// import that follows reads it again. A transfer that failed is told once,
/// as its reason, and forgotten. Any other arrival is refused: it has been
/// replaced, imported or let go of.
pub fn document(arrival: Arrival) -> Result<Zeroizing<Vec<u8>>> {
    let mut slot = SLOT.lock().unwrap();
    match &slot.held {
        Some((held, Delivered::Document(document))) if *held == arrival => Ok(document.clone()),
        Some((held, Delivered::Failed(reason))) if *held == arrival => {
            let reason = reason.clone();
            let gone = slot.set(None);
            drop(slot);
            announce(gone);
            Err(Error::Other(reason))
        }
        _ => Err(Error::Other(
            "that transfer is no longer waiting; export from the other app again".into(),
        )),
    }
}

/// A session opened: what is held, and what arrives while it is open, is
/// offered to it.
pub fn session_opened() {
    SLOT.lock().unwrap().offered = true;
}

/// A session ended, or the vault was sent to the lock screen with none open.
/// What was offered to an open session goes with it; with none open nothing
/// was offered, and a transfer that arrived over the lock screen stays.
pub fn session_ended() {
    let gone = {
        let mut slot = SLOT.lock().unwrap();
        if !std::mem::take(&mut slot.offered) || slot.held.is_none() {
            return;
        }
        slot.set(None)
    };
    announce(gone);
}

/// Let go of `arrival`, which was imported. Whatever arrived after it, while
/// the import ran, is a different transfer and stays.
pub fn release(arrival: Arrival) {
    let gone = {
        let mut slot = SLOT.lock().unwrap();
        if !matches!(slot.held, Some((held, _)) if held == arrival) {
            return;
        }
        slot.set(None)
    };
    announce(gone);
}

/// Whether this device can receive a transfer at all: iOS 26 or later. Only
/// the native side can tell, and it says so as the app starts; nowhere else
/// does anything say it.
static SUPPORTED: AtomicBool = AtomicBool::new(false);

pub fn supported() -> bool {
    SUPPORTED.load(Ordering::Relaxed)
}

/// What `CredentialExchange.swift` calls. Every call across that boundary
/// goes this way, Swift into Rust, and none the other: this crate is also
/// linked on its own, as a dynamic library, before the app's Swift exists to
/// link against, so a function of Swift's would be an undefined symbol there.
#[cfg(target_os = "ios")]
mod native {
    use std::ffi::{c_char, CStr};
    use std::sync::atomic::Ordering;

    /// Said once, as the app starts, by a system that has credential exchange.
    #[no_mangle]
    pub extern "C" fn rowel_credential_exchange_available() {
        super::SUPPORTED.store(true, Ordering::Relaxed);
    }

    /// The document of a transfer that succeeded. The bytes are copied; the
    /// caller keeps and wipes its own.
    ///
    /// # Safety
    /// `bytes` points to `len` readable bytes.
    #[no_mangle]
    pub unsafe extern "C" fn rowel_credential_exchange_received(bytes: *const u8, len: usize) {
        if bytes.is_null() {
            return super::failed("the other app sent nothing");
        }
        super::accept(std::slice::from_raw_parts(bytes, len));
    }

    /// Why a transfer the system began produced no document.
    ///
    /// # Safety
    /// `reason` is null or a NUL-terminated string.
    #[no_mangle]
    pub unsafe extern "C" fn rowel_credential_exchange_failed(reason: *const c_char) {
        let reason = if reason.is_null() {
            "the transfer did not complete".into()
        } else {
            CStr::from_ptr(reason).to_string_lossy()
        };
        super::failed(&reason);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arrival() -> Arrival {
        held().arrival.unwrap()
    }

    fn read(arrival: Arrival) -> Vec<u8> {
        document(arrival).unwrap().to_vec()
    }

    #[test]
    fn a_transfer_is_held_by_number_until_its_import() {
        let _serial = SERIAL.lock().unwrap();
        // Whatever an earlier test left: no session, nothing held.
        session_opened();
        session_ended();
        let before = held();
        assert_eq!(before.arrival, None);

        accept(b"{\"accounts\":[]}");
        let first = arrival();
        // Every change to what is held counts, so a telling of it can be put
        // in order against any other.
        assert!(held().change > before.change);
        // A preview and the import after it read the same document, and
        // reading changes nothing.
        let unread = held();
        assert_eq!(read(first), b"{\"accounts\":[]}");
        assert_eq!(read(first), b"{\"accounts\":[]}");
        assert_eq!(held(), unread);
        release(first);
        assert_eq!(held().arrival, None);
        assert!(held().change > unread.change);
        // Released is gone: it cannot be written twice.
        assert!(document(first).is_err());

        failed("the user cancelled");
        let cancelled = arrival();
        assert_ne!(cancelled, first);
        let Err(Error::Other(reason)) = document(cancelled) else {
            panic!("a failed transfer has no document");
        };
        assert_eq!(reason, "the user cancelled");
        // Told once.
        assert_eq!(held().arrival, None);

        // A newer arrival replaces an older one. The older is then refused
        // rather than answered with the newer one's document, and its import
        // finishing late takes nothing of the newer.
        accept(b"old");
        let old = arrival();
        accept(b"new");
        let new = arrival();
        assert!(document(old).is_err());
        let untouched = held();
        release(old);
        assert_eq!(held(), untouched);
        assert_eq!(read(new), b"new");
        release(new);

        accept(&vec![0u8; MAX_BYTES + 1]);
        assert!(document(arrival()).is_err());
        assert_eq!(held().arrival, None);
    }

    // The two tests share the one slot, so they do not run side by side.
    static SERIAL: Mutex<()> = Mutex::new(());

    #[test]
    fn a_transfer_goes_with_the_session_it_was_offered_to() {
        let _serial = SERIAL.lock().unwrap();
        session_opened();
        session_ended();

        // Over the lock screen: offered to no session. An end with none open
        // (a lockbox chosen or deleted there) leaves it for the unlock.
        accept(b"waiting");
        let waiting = arrival();
        session_ended();
        assert_eq!(held().arrival, Some(waiting));

        // The unlock offers it, and that session's end takes it — a lock, or
        // the lockbox being left: it is not carried into another.
        session_opened();
        session_ended();
        assert_eq!(held().arrival, None);

        // One that arrives while a session is open is offered to it at once.
        session_opened();
        accept(b"live");
        session_ended();
        assert_eq!(held().arrival, None);

        // And one that arrives after the end belongs to the next session, not
        // the one just ended: a second end takes nothing.
        accept(b"after");
        let after = arrival();
        session_ended();
        assert_eq!(held().arrival, Some(after));
        release(after);
    }
}
