pub mod app;
pub mod audit;
pub mod auth;
// Adding the account's other vaults with the password that just opened one.
pub mod autojoin;
// The browser extension host's switches. Declared only: the Settings section
// and the consent dialog that reach them are a later PR, and a command the
// webview never invokes is refused by the frontend's contract test, so the
// registration in lib.rs waits for them too.
#[cfg(desktop)]
#[allow(dead_code)]
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
