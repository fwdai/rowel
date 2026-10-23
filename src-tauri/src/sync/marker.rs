//! The body of a "deleted everywhere" marker (`<vault-id>.deleted`, see
//! [`super::layout::deleted_marker_name`]).
//!
//! The marker stops every device syncing the vault, so it cannot be taken on
//! its existence alone: anyone who can write the account's folder — a Dropbox
//! app folder, a compromised account — could otherwise halt sync everywhere
//! with an empty file. Its body is sealed (AES-256-GCM) under a subkey of the
//! vault's own key ([`VaultKey::delete_marker_key`]) with the vault id as
//! associated data, so only a device that holds this vault's key can write one
//! that the others honour, and a marker copied from another vault does not
//! open here. A marker that does not open — the empty one older builds wrote
//! included — is treated as absent.
//!
//! A sealed marker can still be replayed (a copy of a genuine one put back
//! after the user re-uploaded), which halts sync but destroys nothing: no
//! device deletes a pack because it found a marker, beyond a first push
//! taking back the one object its own create just wrote, by the handle that
//! create returned (`PackRemote::create_pack`) — never a pack another device
//! wrote.
//!
//! [`VaultKey::delete_marker_key`]: crate::crypto::VaultKey::delete_marker_key

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use super::store::{RemoteEntry, VaultStore};
use crate::crypto::{seal_aead, unseal_aead, KEY_LEN};
use crate::error::{Error, Result};

/// The subkey a marker is sealed under, as a run carries it.
pub(crate) type MarkerKey = Zeroizing<[u8; KEY_LEN]>;

/// The most a marker body may be when read. A real one is well under 100
/// bytes; the cap keeps a planted file from being buffered whole.
const MAX_MARKER_BYTES: usize = 4 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Body {
    /// When the delete was made, in ms since the epoch. For the log only.
    deleted_at: i64,
}

/// A marker body for `vault_id`, sealed under `key`.
pub(crate) fn seal(key: &[u8], vault_id: &str, deleted_at: i64) -> Result<Vec<u8>> {
    let body = serde_json::to_vec(&Body { deleted_at })?;
    seal_aead(key, vault_id.as_bytes(), &body)
}

/// Whether `bytes` is a marker for `vault_id` sealed under `key`.
pub(crate) fn opens(key: &[u8], vault_id: &str, bytes: &[u8]) -> bool {
    unseal_aead(key, vault_id.as_bytes(), bytes)
        .ok()
        .and_then(|plain| serde_json::from_slice::<Body>(&plain).ok())
        .is_some()
}

/// Whether the marker `entry` (found by a `stat` of the marker name) is a real
/// one for `vault_id`. A marker that is not — empty, foreign, tampered with,
/// or too big to be one — is logged and answers `false`, so the caller acts as
/// though there were none. A read that fails for any other reason (the
/// network) is an error rather than a verdict.
///
/// `keys` is the current marker key first, then those of the key epochs this
/// device has retired (`crate::retired`): a delete written on another device
/// just before or after a password change is sealed under the epoch that
/// device held, and is honoured all the same.
pub(crate) async fn verified(
    store: &impl VaultStore,
    entry: &RemoteEntry,
    keys: &[&[u8]],
    vault_id: &str,
) -> Result<bool> {
    // The empty marker every build before this one wrote. Nothing to read.
    if entry.size == Some(0) {
        log::warn!(
            "ignoring an unverified deletion marker for vault {vault_id}: it is empty \
             (written by an older version, or by someone else)"
        );
        return Ok(false);
    }
    let bytes = match store.read(&entry.handle, MAX_MARKER_BYTES).await {
        Ok(bytes) => bytes,
        Err(Error::FileTooLarge) => {
            log::warn!("ignoring an unverified deletion marker for vault {vault_id}: too large");
            return Ok(false);
        }
        Err(e) => return Err(e),
    };
    if keys.iter().any(|key| opens(key, vault_id, &bytes)) {
        return Ok(true);
    }
    log::warn!(
        "ignoring an unverified deletion marker for vault {vault_id}: it was not sealed \
         under this vault's key"
    );
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; KEY_LEN] = [7; KEY_LEN];

    #[test]
    fn a_marker_opens_only_for_its_own_vault_and_key() {
        let sealed = seal(&KEY, "a1b2", 1).unwrap();
        assert!(opens(&KEY, "a1b2", &sealed));
        assert!(!opens(&KEY, "c3d4", &sealed));
        assert!(!opens(&[8; KEY_LEN], "a1b2", &sealed));
        assert!(!opens(&KEY, "a1b2", &[]));
        assert!(!opens(&KEY, "a1b2", b"deleted"));
    }
}
