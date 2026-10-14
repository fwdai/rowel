pub mod app;
// Files attached to entries, sealed and kept beside them.
pub mod attachments;
pub mod audit;
pub mod auth;
// Adding the account's other vaults with the password that just opened one.
pub mod autojoin;
// The browser extension host's switches, for Settings and the consent dialog.
// Desktop only, like the host; the registration in lib.rs carries the same cfg.
#[cfg(desktop)]
pub mod browser;
pub mod clipboard;
// E2E-only reset seam. Compiled out of release builds entirely (see e2e.rs for
// the full gating rationale); the registration in lib.rs carries the same cfg.
#[cfg(debug_assertions)]
pub mod e2e;
pub mod env;
pub mod generator;
pub mod import;
// First-run onboarding (connect Drive before a vault exists).
pub mod setup;
pub mod share;
pub mod sync;
pub mod tools;
pub mod vault;
// Creating, switching and renaming the optional additional vaults.
pub mod workspace;

use crate::error::{Error, Result};
use crate::session::Epoch;
use crate::state::AppState;

// --- the session a read belongs to ------------------------------------------
//
// A command that reads on the vault's behalf and awaits in the middle can have
// the vault lock — or lock and reopen — under it. The result was asked for by
// the session that is gone, so it is not handed to the one that took its
// place: the epoch is taken before the work and checked again after it.

/// The open session's epoch, or `Locked`.
pub(crate) fn unlocked_epoch(state: &AppState) -> Result<Epoch> {
    let session = state.session.lock().unwrap();
    if !session.is_unlocked() {
        return Err(Error::Locked);
    }
    Ok(session.epoch())
}

/// `Locked` unless the session `epoch` was taken from is still the open one.
pub(crate) fn same_session(state: &AppState, epoch: Epoch) -> Result<()> {
    let session = state.session.lock().unwrap();
    if session.is_unlocked() && session.epoch() == epoch {
        Ok(())
    } else {
        Err(Error::Locked)
    }
}

// --- a file sent as the request's body --------------------------------------
//
// `scan_image_bytes` and `attachment_add_bytes` take a file the webview holds,
// sent as a `Uint8Array`. Over Tauri's custom-protocol IPC that is the raw
// body. But a call that fails on that transport is retried — and every later
// call made — over `postMessage`, which carries the same `Uint8Array` as a JSON
// array of numbers. On iOS that happens around the camera: the full-screen
// capture takes the webview off screen, a request in flight fails, and the
// photo it hands back then travels as JSON. Both are the same bytes.

/// The file in `body`, raw or as a JSON array of bytes, refused past `max`
/// bytes before any copy of it is made.
pub(crate) fn body_bytes(body: &tauri::ipc::InvokeBody, max: u64) -> Result<Vec<u8>> {
    use tauri::ipc::InvokeBody;
    let not_bytes = || Error::Other("the request body is not a file's bytes".into());
    match body {
        InvokeBody::Raw(bytes) => {
            if bytes.len() as u64 > max {
                return Err(Error::FileTooLarge);
            }
            Ok(bytes.clone())
        }
        InvokeBody::Json(serde_json::Value::Array(items)) => {
            if items.len() as u64 > max {
                return Err(Error::FileTooLarge);
            }
            items
                .iter()
                .map(|item| item.as_u64().and_then(|n| u8::try_from(n).ok()))
                .collect::<Option<Vec<u8>>>()
                .ok_or_else(not_bytes)
        }
        InvokeBody::Json(_) => Err(not_bytes()),
    }
}

// --- getting off the async runtime ------------------------------------------
//
// A `#[tauri::command] fn` without `async` runs inline on the IPC thread, which
// is the main (UI) thread on macOS. `async` moves it to a runtime worker — but a
// worker is not a place to block either, so every command here that does real
// work hands it to the blocking pool through one of the two helpers below.
//
// Two kinds of caller need them. CPU- and disk-bound work (Argon2id, a
// whole-vault re-seal, a SQLCipher snapshot) must not sit on a worker. And the
// Drive transport drives its async calls with `block_on`: that is illegal *on a
// runtime worker*, which is what it would deadlock, but perfectly legal on a
// blocking-pool thread — those are not workers, they exist precisely to be
// blocked.

/// Run `work` on the blocking pool and await its result.
///
/// A join error means the thread panicked or was cancelled; there is no inner
/// result to report, so the join failure itself is surfaced.
pub(crate) async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|e| Error::Other(e.to_string()))?
}

/// The fire-and-forget form: same pool, nothing to await.
///
/// Used by the flows that report through events rather than through their return
/// value — a consent round trip, a sync run — so the command can return the
/// moment its guards have passed.
pub(crate) fn detached(work: impl FnOnce() + Send + 'static) {
    tauri::async_runtime::spawn_blocking(work);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tauri::ipc::InvokeBody;

    #[test]
    fn body_bytes_takes_a_raw_body() {
        let body = InvokeBody::Raw(vec![0xff, 0xd8, 0xff]);
        assert_eq!(body_bytes(&body, 3).unwrap(), vec![0xff, 0xd8, 0xff]);
    }

    #[test]
    fn body_bytes_takes_the_postmessage_fallbacks_json_array() {
        let body = InvokeBody::Json(json!([255, 216, 255, 0]));
        assert_eq!(body_bytes(&body, 4).unwrap(), vec![0xff, 0xd8, 0xff, 0]);
    }

    #[test]
    fn body_bytes_refuses_a_body_past_the_cap_in_either_shape() {
        for body in [
            InvokeBody::Raw(vec![0; 5]),
            InvokeBody::Json(json!([0, 0, 0, 0, 0])),
        ] {
            assert!(matches!(body_bytes(&body, 4), Err(Error::FileTooLarge)));
        }
    }

    #[test]
    fn body_bytes_refuses_json_that_is_not_bytes() {
        for value in [
            json!({ "image": [1, 2] }),
            json!([1, 256]),
            json!([1, -1]),
            json!(["1"]),
        ] {
            let body = InvokeBody::Json(value);
            assert!(matches!(body_bytes(&body, 16), Err(Error::Other(_))));
        }
    }
}
