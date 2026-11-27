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
//! This module is the one place that knows what is held. Each arrival has a
//! number; the webview is told the number (`exchange:received`), asks for it
//! whenever it may have changed (`commands::import::exchange_held`), and names
//! it in every preview and import — so what is imported is what was previewed,
//! and a transfer that has been replaced or let go of is refused rather than
//! answered with another one's rows.
//!
//! A newer arrival replaces an older one. Otherwise a transfer is held until
//! it is imported or until the open session it was offered to ends — a lock,
//! or that lockbox being left or deleted: it does not outlive that session,
//! and is never carried into another lockbox. One that arrives with the vault
//! locked was offered to none, and waits for the unlock, through a choice of
//! lockbox on the lock screen if need be.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use tauri::AppHandle;
use zeroize::Zeroizing;

use crate::error::{Error, Result};

/// As large as an export file may be (`commands::import`).
const MAX_BYTES: usize = 25 * 1024 * 1024;

/// Which transfer: the count of arrivals when it came, so never reused within
/// a run of the app.
pub type Arrival = u64;

/// What the native side delivered: the document, or why there is none.
enum Delivered {
    Document(Zeroizing<Vec<u8>>),
    Failed(String),
}

struct Slot {
    held: Option<(Arrival, Delivered)>,
    arrivals: Arrival,
}

// Process-wide rather than in `AppState`: the native side calls in with no
// handle, and on a launch may do so before the app has one.
static SLOT: Mutex<Slot> = Mutex::new(Slot {
    held: None,
    arrivals: 0,
});
static APP: OnceLock<AppHandle> = OnceLock::new();

/// Let an arrival be announced. Before this, one is only parked.
pub fn init(app: &AppHandle) {
    let _ = APP.set(app.clone());
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
    let arrival = {
        let mut slot = SLOT.lock().unwrap();
        slot.arrivals += 1;
        slot.held = Some((slot.arrivals, delivered));
        slot.arrivals
    };
    if let Some(app) = APP.get() {
        crate::events::exchange_received(app, arrival);
    }
}

/// The transfer that is held, if one is.
pub fn held() -> Option<Arrival> {
    SLOT.lock()
        .unwrap()
        .held
        .as_ref()
        .map(|(arrival, _)| *arrival)
}

/// The document of `arrival`, which stays held: a preview reads it and the
/// import that follows reads it again. A transfer that failed is told once,
/// as its reason, and forgotten. Any other arrival is refused: it has been
/// replaced, imported or let go of.
pub fn document(arrival: Arrival) -> Result<Zeroizing<Vec<u8>>> {
    let mut slot = SLOT.lock().unwrap();
    match slot.held.take() {
        Some((held, Delivered::Document(document))) if held == arrival => {
            let copy = document.clone();
            slot.held = Some((held, Delivered::Document(document)));
            Ok(copy)
        }
        Some((held, Delivered::Failed(reason))) if held == arrival => Err(Error::Other(reason)),
        other => {
            slot.held = other;
            Err(Error::Other(
                "that transfer is no longer waiting; export from the other app again".into(),
            ))
        }
    }
}

/// Let go of `arrival`, which was imported. Whatever arrived after it, while
/// the import ran, is a different transfer and stays.
pub fn imported(arrival: Arrival) {
    let mut slot = SLOT.lock().unwrap();
    if matches!(slot.held, Some((held, _)) if held == arrival) {
        slot.held = None;
    }
}

/// Forget whatever is held: the session it was offered to ended.
pub fn clear() {
    SLOT.lock().unwrap().held = None;
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

    fn read(arrival: Arrival) -> Vec<u8> {
        document(arrival).unwrap().to_vec()
    }

    // The state is one per process, so the cases share a test rather than
    // race each other across threads.
    #[test]
    fn a_transfer_is_held_by_number_until_its_import_or_a_lock() {
        clear();
        assert_eq!(held(), None);

        accept(b"{\"accounts\":[]}");
        let first = held().unwrap();
        // A preview and the import after it read the same document.
        assert_eq!(read(first), b"{\"accounts\":[]}");
        assert_eq!(read(first), b"{\"accounts\":[]}");
        imported(first);
        assert_eq!(held(), None);
        // Imported is gone: it cannot be written twice.
        assert!(document(first).is_err());

        failed("the user cancelled");
        let cancelled = held().unwrap();
        assert_ne!(cancelled, first);
        let Err(Error::Other(reason)) = document(cancelled) else {
            panic!("a failed transfer has no document");
        };
        assert_eq!(reason, "the user cancelled");
        // Told once.
        assert_eq!(held(), None);

        // A newer arrival replaces an older one. The older is then refused
        // rather than answered with the newer one's document, and its import
        // finishing late lets go of nothing.
        accept(b"old");
        let old = held().unwrap();
        accept(b"new");
        let new = held().unwrap();
        assert!(document(old).is_err());
        imported(old);
        assert_eq!(held(), Some(new));
        assert_eq!(read(new), b"new");

        // A lock lets go of whatever is there.
        clear();
        assert_eq!(held(), None);
        assert!(document(new).is_err());

        accept(&vec![0u8; MAX_BYTES + 1]);
        assert!(document(held().unwrap()).is_err());
        clear();
    }
}
