//! The broker without the GUI. `rowel serve` runs the same engine the desktop
//! app runs — the sealed channel, the grant engine, the lockbox-side
//! functions and the audit chain of `rowel-broker` — over a device-held
//! lockbox on a box with no desktop: a developer's VM, a CI runner, a Linux
//! server. The lockbox is unlocked from a terminal or a key file (`unlock`),
//! agents and sidecars connect over the data directory's socket or a
//! WebSocket (`listen`), standing rules from a config file serve them
//! (`config`), and an ask waits for `rowel approve` on the same machine or
//! times out (`approvals`, `control`). [`HeadlessHost`] is the `Host` all of
//! it runs through; [`run`] is the process.
//!
//! What is deliberately not here: a GUI, sync (the lockbox is served as it
//! was copied), a password generator, and biometrics — a step-up ask takes
//! the master password at `rowel approve`.

// Desktop only, as the broker's `socket` and the CLI are: a phone has no
// terminal to serve from, so there the crate is empty and the workspace still
// builds for one.
#![cfg(not(any(target_os = "android", target_os = "ios")))]

pub mod approvals;
pub mod config;
pub mod control;
pub mod host;
pub mod listen;
pub mod run;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;
#[cfg(test)]
mod tests;
pub mod unlock;

use std::path::Path;

pub use host::HeadlessHost;
pub use run::{run, PasswordPrompt, ServeOptions};
pub use unlock::Unlock;

/// This install's own id, on the root of the data directory: the audit
/// log's writer id, as the app keeps it under the same name
/// (`storage::INSTALL_ID_FILE`), so a data directory copied from a laptop
/// keeps writing the one chain that install started.
pub const INSTALL_ID_FILE: &str = "install_id";

/// The id kept at `root`, or a fresh one — 16 random bytes, hex — written
/// there owner-only; when the write fails, just minted, and the caller's
/// one copy per process is what makes it stick ([`HeadlessHost`] reads it
/// once). A one-function duplicate of the app's `storage::install_id_in`,
/// which is behind Tauri: the two must agree on the file and its shape, and
/// do.
pub fn install_id_in(root: &Path) -> String {
    let path = root.join(INSTALL_ID_FILE);
    if let Ok(id) = std::fs::read_to_string(&path) {
        let id = id.trim();
        if !id.is_empty() {
            return id.to_string();
        }
    }
    let id = rowel_core::crypto::random_hex_id();
    if let Err(e) = rowel_core::atomic::atomic_write_private(&path, id.as_bytes()) {
        log::warn!("rowel serve: could not keep the install id: {e}");
    }
    id
}
