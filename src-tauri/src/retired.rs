//! The key epochs a lockbox has been re-keyed away from, kept on the device.
//!
//! A password change seals the account's pack under the new key on the next
//! run — but a slower device may push under the old one first, and an old
//! revision can be put back. Either way the pack's header names an epoch this
//! device has retired, and without the old key it could neither open nor merge
//! it. So every re-key keeps the epoch it leaves behind: the KDF descriptor
//! verbatim and the key material, newest first, at most [`MAX_RETIRED`].
//!
//! The list is sealed under a subkey of the *current* key
//! ([`VaultKey::retired_keys_key`]) and bound to the workspace id, so it is
//! only readable by someone who could already open the vault, and a copy moved
//! under another workspace does not open there. It is never written to the
//! account. Losing it only costs the merge of an old-epoch pack: every read
//! failure is an empty list, never an error.

use std::path::Path;

use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::crypto::{seal_aead, unseal_aead, VaultKey};
use crate::error::{Error, Result};
use crate::storage;

/// How many retired epochs are kept. Enough for a device that missed a couple
/// of changes in a row; beyond that it re-joins through the prompt or the
/// replace action, which is what a device that never held the key does too.
pub const MAX_RETIRED: usize = 3;

/// One retired epoch: the descriptor a pack header carries for it, and the key
/// material that derived from it.
pub struct RetiredKey {
    pub kdf: String,
    pub material: Zeroizing<Vec<u8>>,
    pub retired_at: i64,
}

impl RetiredKey {
    /// The epoch's key. A retired epoch always has a descriptor, so the
    /// material is always an Argon2id master.
    pub fn key(&self) -> VaultKey {
        VaultKey::from_material(self.material.clone(), true)
    }
}

#[derive(Serialize, Deserialize)]
struct Plain {
    entries: Vec<PlainEntry>,
}

#[derive(Serialize, Deserialize)]
struct PlainEntry {
    kdf: String,
    material: Zeroizing<String>,
    retired_at: i64,
}

/// The retired epochs of the workspace in `dir`, opened under its current
/// `key`. Empty — with a warning — when the file is missing, does not open, or
/// does not parse.
pub fn load(dir: &Path, key: &VaultKey, workspace_id: &str) -> Vec<RetiredKey> {
    load_file(&dir.join(storage::RETIRED_KEYS_FILE), key, workspace_id)
}

pub(crate) fn load_file(path: &Path, key: &VaultKey, workspace_id: &str) -> Vec<RetiredKey> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => {
            log::warn!("the retired-keys list could not be read: {e}");
            return Vec::new();
        }
    };
    open(&text, key, workspace_id).unwrap_or_else(|e| {
        log::warn!("the retired-keys list does not open under this key; ignoring it: {e}");
        Vec::new()
    })
}

/// Write the list a re-key leaves behind: what `old_key` could open, with the
/// epoch being left (`old_kdf`, when the vault had a descriptor) on the front,
/// pruned to [`MAX_RETIRED`], sealed under `new_key`. Called by the re-key saga
/// in the step that writes the new sidecar, so the two commit and roll back
/// together. An empty list removes the file.
pub(crate) fn retire(
    path: &Path,
    workspace_id: &str,
    old_key: &VaultKey,
    old_kdf: Option<&str>,
    new_key: &VaultKey,
    new_kdf: &str,
) -> Result<()> {
    let mut entries = load_file(path, old_key, workspace_id);
    if let Some(old_kdf) = old_kdf {
        entries.retain(|e| e.kdf != old_kdf);
        entries.insert(
            0,
            RetiredKey {
                kdf: old_kdf.to_string(),
                material: Zeroizing::new(old_key.biometric_material().to_vec()),
                retired_at: crate::auth::now_ms(),
            },
        );
    }
    // An epoch the vault is moving (back) onto is current, not retired.
    entries.retain(|e| e.kdf != new_kdf);
    entries.truncate(MAX_RETIRED);
    if entries.is_empty() {
        return match std::fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        };
    }
    let sealed = seal(&entries, new_key, workspace_id)?;
    storage::atomic_write_private(path, sealed.as_bytes())
}

fn seal(entries: &[RetiredKey], key: &VaultKey, workspace_id: &str) -> Result<String> {
    let plain = Plain {
        entries: entries
            .iter()
            .map(|e| PlainEntry {
                kdf: e.kdf.clone(),
                material: Zeroizing::new(STANDARD.encode(&*e.material)),
                retired_at: e.retired_at,
            })
            .collect(),
    };
    let json = Zeroizing::new(serde_json::to_vec(&plain)?);
    let sealed = seal_aead(&*key.retired_keys_key(), workspace_id.as_bytes(), &json)?;
    Ok(STANDARD.encode(sealed))
}

fn open(text: &str, key: &VaultKey, workspace_id: &str) -> Result<Vec<RetiredKey>> {
    let blob = STANDARD
        .decode(text.trim())
        .map_err(|e| Error::Crypto(e.to_string()))?;
    let json = Zeroizing::new(unseal_aead(
        &*key.retired_keys_key(),
        workspace_id.as_bytes(),
        &blob,
    )?);
    let plain: Plain = serde_json::from_slice(&json)?;
    plain
        .entries
        .into_iter()
        .map(|e| {
            let material = STANDARD
                .decode(e.material.as_bytes())
                .map_err(|e| Error::Crypto(e.to_string()))?;
            Ok(RetiredKey {
                kdf: e.kdf,
                material: Zeroizing::new(material),
                retired_at: e.retired_at,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(byte: u8) -> VaultKey {
        VaultKey::Argon2 {
            master: Zeroizing::new(vec![byte; 32]),
        }
    }

    fn kdfs(list: &[RetiredKey]) -> Vec<&str> {
        list.iter().map(|e| e.kdf.as_str()).collect()
    }

    #[test]
    fn a_retired_epoch_round_trips_under_the_new_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(storage::RETIRED_KEYS_FILE);
        retire(&path, "ws", &key(1), Some("kdf-1"), &key(2), "kdf-2").unwrap();

        let list = load(dir.path(), &key(2), "ws");
        assert_eq!(kdfs(&list), ["kdf-1"]);
        assert_eq!(&*list[0].material, &[1u8; 32][..]);
        assert_eq!(
            list[0].key().sqlcipher_key(),
            key(1).sqlcipher_key(),
            "the material gives back the old epoch's key"
        );
    }

    #[test]
    fn a_wrong_key_or_another_workspace_reads_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(storage::RETIRED_KEYS_FILE);
        retire(&path, "ws", &key(1), Some("kdf-1"), &key(2), "kdf-2").unwrap();

        assert!(load(dir.path(), &key(1), "ws").is_empty());
        assert!(load(dir.path(), &key(2), "other").is_empty());
        std::fs::write(&path, "not sealed").unwrap();
        assert!(load(dir.path(), &key(2), "ws").is_empty());
        assert!(load(tempfile::tempdir().unwrap().path(), &key(2), "ws").is_empty());
    }

    #[test]
    fn the_list_keeps_the_newest_three() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(storage::RETIRED_KEYS_FILE);
        for n in 1..=5u8 {
            retire(
                &path,
                "ws",
                &key(n),
                Some(&format!("kdf-{n}")),
                &key(n + 1),
                &format!("kdf-{}", n + 1),
            )
            .unwrap();
        }
        let list = load(dir.path(), &key(6), "ws");
        assert_eq!(kdfs(&list), ["kdf-5", "kdf-4", "kdf-3"]);
        assert_eq!(&*list[2].material, &[3u8; 32][..]);
    }

    // Adopting back onto an epoch the list holds (a change undone elsewhere)
    // makes it current again, so it leaves the list.
    #[test]
    fn the_epoch_moved_onto_is_not_kept_as_retired() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(storage::RETIRED_KEYS_FILE);
        retire(&path, "ws", &key(1), Some("kdf-1"), &key(2), "kdf-2").unwrap();
        retire(&path, "ws", &key(2), Some("kdf-2"), &key(1), "kdf-1").unwrap();
        assert_eq!(kdfs(&load(dir.path(), &key(1), "ws")), ["kdf-2"]);
    }

    // A vault with no descriptor had no epoch to keep.
    #[test]
    fn a_sidecar_less_vault_leaves_no_list() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(storage::RETIRED_KEYS_FILE);
        retire(&path, "ws", &key(1), None, &key(2), "kdf-2").unwrap();
        assert!(!path.exists());
    }
}
