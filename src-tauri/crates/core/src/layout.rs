//! Where the data dir is and what the files in it are called — the part of the
//! app's on-disk layout another process has to agree on to open the same
//! vault: the iOS AutoFill extension, which finds it in the App Group
//! container. The rest of the layout stays with the app (`storage.rs`).

use std::path::{Path, PathBuf};

use crate::app::APP_GROUP_DATA_DIR;

pub const DB_FILE: &str = "vault.db";
// Plaintext KDF descriptor stored next to the DB. It holds the Argon2id params +
// salt (public by design) and is read *before* deriving the key — the salt/params
// cannot live inside the encrypted DB, since deriving the key is what opens it.
pub const KDF_SIDECAR_FILE: &str = "vault.kdf.json";
// Marker for "biometric unlock is enabled". The key itself lives in the OS
// secure store; this flag lets us report availability without a biometric prompt.
// Its contents name the gate the key was enrolled behind (`keychain::GateMode`)
// — not a secret: it says *how* the key is gated, never anything about the key.
pub const BIOMETRIC_FILE: &str = "biometric.enabled";
// A non-primary workspace's own vault key, sealed under the primary's (the app
// key) so that one unlock opens every workspace on the device — see
// `crypto::seal_workspace_key`. Ciphertext, but held to `0600` all the same;
// the primary has none, since its key *is* the app key.
pub const WRAPPED_KEY_FILE: &str = "vault.key.sealed";

/// Pre-change recovery snapshot of the encrypted DB, written next to it before
/// the destructive change-master-password sequence. Its presence is the marker
/// that a change is in flight: the app's next unlock rolls the vault back to
/// it, and another process (the iOS AutoFill extension) must not open the
/// vault while it is there — the database and the sealed payloads may be on
/// different keys until the app has.
pub const DB_REKEY_BACKUP_FILE: &str = "vault.db.rekey-backup";
/// Pre-change recovery snapshot of the KDF sidecar, taken alongside the DB one.
/// Rolling the DB back to its old key is only half a rollback: the descriptor
/// that says how to derive that key has to roll back with it, or nothing opens.
pub const KDF_SIDECAR_REKEY_BACKUP_FILE: &str = "vault.kdf.json.rekey-backup";
/// The key epochs this device has re-keyed away from, sealed under the current
/// key: what lets it still open a sync pack a slower device pushed under a
/// password this one has already replaced. Never leaves the device.
pub const RETIRED_KEYS_FILE: &str = "vault.retired.sealed";
/// Pre-change recovery snapshot of [`RETIRED_KEYS_FILE`], taken with the other
/// two: a rollback that restored the old key but kept the new list would hold
/// a list nothing on this device can open.
pub const RETIRED_KEYS_REKEY_BACKUP_FILE: &str = "vault.retired.sealed.rekey-backup";

/// The app's own id on the data directory's root: its audit log's writer id
/// (`storage::install_id`), which a workspace id cannot be — every install's
/// primary lockbox is `default`.
pub const INSTALL_ID_FILE: &str = "install_id";
/// `rowel serve`'s writer id, beside the app's and never the same file: a
/// data directory copied from a device brings the device's `install_id` with
/// it, and a broker writing under that id would extend the device's chain
/// from a second place — a fork no verifier can tell from tampering.
pub const SERVE_WRITER_ID_FILE: &str = "serve_writer_id";

/// The id kept at `path`, or a fresh one — 16 random bytes, hex — written
/// there owner-only; an empty file, which a torn write leaves, mints afresh.
/// When the write fails the id is just minted and the failure logged: the
/// caller's one copy per process is what makes it stick, so every row of the
/// process still chains under one writer.
pub fn kept_id(path: &Path) -> String {
    if let Ok(id) = std::fs::read_to_string(path) {
        let id = id.trim();
        if !id.is_empty() {
            return id.to_string();
        }
    }
    let id = crate::crypto::random_hex_id();
    if let Err(e) = crate::atomic::atomic_write_private(path, id.as_bytes()) {
        log::warn!("could not keep the writer id at {}: {e}", path.display());
    }
    id
}

/// Dev builds share the prod identifier, so isolate their data in a subdir
/// to avoid mutating the real vault while iterating.
pub fn dev_subdir(dir: PathBuf) -> PathBuf {
    if cfg!(debug_assertions) {
        dir.join("dev")
    } else {
        dir
    }
}

/// The data dir inside the iOS App Group container at `container`. A debug
/// build's is the `dev` subdir of it, as everywhere else ([`dev_subdir`]) — so
/// the extension finds the vault of the app built the same way it was.
pub fn app_group_root(container: &Path) -> PathBuf {
    dev_subdir(container.join(APP_GROUP_DATA_DIR))
}
