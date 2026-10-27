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

/// The broker's audit writer id for the data directory at `root`, minted
/// the first time and kept owner-only in its own file
/// (`layout::SERVE_WRITER_ID_FILE`), never the app's `install_id`: the data
/// directory is a copy of a device's, and the device keeps writing its own
/// chain. [`HeadlessHost`] reads it once per process.
pub fn writer_id_in(root: &Path) -> String {
    rowel_core::layout::kept_id(&root.join(rowel_core::layout::SERVE_WRITER_ID_FILE))
}
