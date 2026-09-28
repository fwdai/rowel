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
