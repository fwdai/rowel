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
//! The webview may not be listening when it arrives, and the vault may be
//! locked. So the document is parked here as well as announced as
//! `exchange:received`, and the frontend asks once it has subscribed
//! (`commands::import::exchange_pending`). It does not outlive the session it
//! was offered to: a lock drops it, and the user exports again.

use std::sync::{Mutex, OnceLock};

use tauri::AppHandle;
use zeroize::Zeroizing;

use crate::error::{Error, Result};

/// As large as an export file may be (`commands::import`).
const MAX_BYTES: usize = 25 * 1024 * 1024;

/// What the native side delivered: the document, or why there is none.
enum Held {
    Document(Zeroizing<Vec<u8>>),
    Failed(String),
}

// Process-wide rather than in `AppState`: the native side calls in with no
// handle, and on a launch may do so before the app has one.
static HELD: Mutex<Option<Held>> = Mutex::new(None);
static APP: OnceLock<AppHandle> = OnceLock::new();

/// Let an arrival be announced. Before this, one is only parked.
pub fn init(app: &AppHandle) {
    let _ = APP.set(app.clone());
}

/// Park a document and announce it. A newer one replaces an older one that
/// was never imported.
#[cfg_attr(not(any(target_os = "ios", test)), allow(dead_code))]
fn accept(document: &[u8]) {
    if document.len() > MAX_BYTES {
        return failed("the other app sent more than Rowel can import at once");
    }
    hold(Held::Document(Zeroizing::new(document.to_vec())));
}

/// Park the reason a transfer produced nothing, to be reported where the
/// import would have been previewed.
#[cfg_attr(not(any(target_os = "ios", test)), allow(dead_code))]
fn failed(reason: &str) {
    hold(Held::Failed(reason.to_owned()));
}

fn hold(held: Held) {
    *HELD.lock().unwrap() = Some(held);
    if let Some(app) = APP.get() {
        crate::events::exchange_received(app);
    }
}

/// Whether a transfer is waiting to be looked at.
pub fn pending() -> bool {
    HELD.lock().unwrap().is_some()
}

/// The held document, which stays held: a preview reads it and the import
/// that follows reads it again. A failure is reported once and forgotten.
pub fn document() -> Result<Zeroizing<Vec<u8>>> {
    let mut held = HELD.lock().unwrap();
    match held.take() {
        Some(Held::Document(document)) => {
            let copy = document.clone();
            *held = Some(Held::Document(document));
            Ok(copy)
        }
        Some(Held::Failed(reason)) => Err(Error::Other(reason)),
        None => Err(Error::Other(
            "nothing is waiting to be imported; export from the other app again".into(),
        )),
    }
}

/// Forget what is held: it was imported, or the session it was offered to
/// ended.
pub fn clear() {
    *HELD.lock().unwrap() = None;
}

/// Whether this device can receive a transfer at all: iOS 26 or later.
pub fn supported() -> bool {
    #[cfg(target_os = "ios")]
    {
        // SAFETY: a Swift function of no arguments that reads the OS version.
        unsafe { native::rowel_credential_exchange_supported() }
    }
    #[cfg(not(target_os = "ios"))]
    {
        false
    }
}

/// The two ends of `CredentialExchange.swift`.
#[cfg(target_os = "ios")]
mod native {
    use std::ffi::{c_char, CStr};

    extern "C" {
        pub fn rowel_credential_exchange_supported() -> bool;
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

    // The state is one per process, so the cases share a test rather than
    // race each other across threads.
    #[test]
    fn a_document_is_held_until_cleared_and_a_failure_is_told_once() {
        clear();
        assert!(!pending());
        assert!(document().is_err());

        accept(b"{\"accounts\":[]}");
        assert!(pending());
        // A preview and the import after it read the same document.
        assert_eq!(document().unwrap().as_slice(), b"{\"accounts\":[]}");
        assert_eq!(document().unwrap().as_slice(), b"{\"accounts\":[]}");
        clear();
        assert!(!pending());

        failed("the user cancelled");
        assert!(pending());
        let Err(Error::Other(reason)) = document() else {
            panic!("a failed transfer has no document");
        };
        assert_eq!(reason, "the user cancelled");
        assert!(!pending());

        // A newer arrival replaces an older one.
        accept(b"old");
        accept(b"new");
        assert_eq!(document().unwrap().as_slice(), b"new");

        accept(&vec![0u8; MAX_BYTES + 1]);
        assert!(document().is_err());
        clear();
    }
}
