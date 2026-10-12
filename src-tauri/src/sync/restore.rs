//! Fresh-install restore: adopt a remote `.rowel` pack as *the* local vault.
//!
//! This is the one path that installs a vault it did not create, and it only
//! ever runs on an install that has none. A device that already holds a vault
//! merges instead (the sync engine); replacing an existing DB with a remote
//! snapshot would silently discard whatever had not been pushed yet.
//!
//! A pack carries its vault id inside the snapshot, and a restore keeps it. A
//! Drive restore then stamps the id the pack was *filed under* on top
//! (`commands::setup`): the file name is where the account addresses the
//! vault, and the snapshot can lag it — the engine commits an adopted id only
//! after the run that pushed the pack has succeeded.

// Reached from onboarding's `setup_restore_from_account`, which is what supplies
// the two things this path needs and no other caller can: the master password
// (asked for on a screen that runs before a vault exists) and account credentials
// held in memory, since the token file is sealed under a vault key that does not
// exist until this function has produced one.

use std::fs;
use std::path::Path;

use tauri::AppHandle;

use super::pack;
use crate::crypto::{self, KdfParams, VaultKey};
use crate::error::{Error, Result};
use crate::storage;
use crate::store::{SqliteStore, StoreError, SYNC_META_PREFIX};

/// Restore the vault carried by `bytes` into this install's data dir.
pub fn restore_from_pack(
    app: &AppHandle,
    bytes: &[u8],
    password: &str,
) -> Result<(VaultKey, SqliteStore)> {
    restore_at(
        &storage::db_path(app)?,
        &storage::kdf_sidecar_path(app)?,
        bytes,
        password,
    )
}

/// The path-level core of [`restore_from_pack`].
///
/// Sequence: refuse if a vault is already here, unpack, write the KDF sidecar,
/// derive, write the snapshot, open. Opening is what validates the password —
/// SQLCipher cannot read a byte under the wrong key — and what surfaces a
/// snapshot from a future build as [`Error::VaultTooNew`] rather than as a
/// wrong password.
pub fn restore_at(
    db_path: &Path,
    sidecar_path: &Path,
    bytes: &[u8],
    password: &str,
) -> Result<(VaultKey, SqliteStore)> {
    // Hard guard, before anything is read or written. Not `db_exists` (which
    // ignores a zero-length file): if anything at all sits at the vault path,
    // this install is not the fresh install this path is for.
    if db_path.exists() {
        // The same refusal `setup::guard_no_vault` gives, reached from the
        // other side: a vault is here, so this install is not a fresh one.
        return Err(Error::AlreadySetUp);
    }

    // Validated before the first write, so a bad file never touches the disk.
    let unpacked = pack::unpack(bytes)?;

    match install(db_path, sidecar_path, &unpacked, password) {
        Ok(restored) => Ok(restored),
        Err(e) => {
            // Everything this function wrote goes away. A half-restored install
            // is the worst outcome available: a sidecar whose params do not
            // match the DB, or a DB that never finished, reads as "wrong master
            // password" forever after, and the user has no way to tell that
            // from actually mistyping it.
            storage::remove_db_files(db_path);
            let _ = fs::remove_file(sidecar_path);
            Err(e)
        }
    }
}

// The write half, split out so `restore_at` has exactly one cleanup site.
fn install(
    db_path: &Path,
    sidecar_path: &Path,
    unpacked: &pack::Unpacked,
    password: &str,
) -> Result<(VaultKey, SqliteStore)> {
    let params = KdfParams::from_json(&unpacked.kdf_params_json)?;

    // Sidecar first, same invariant as `create_vault`: a DB must never exist
    // without the descriptor needed to open it. Written verbatim from the
    // header so the restored install derives byte-identically to the source.
    storage::atomic_write_file(sidecar_path, &unpacked.kdf_params_json)?;

    let key = VaultKey::Argon2 {
        master: crypto::derive(password.as_bytes(), &params)?,
    };

    write_private(db_path, &unpacked.snapshot)?;

    // Same split `session::open_with_key` draws: only the key verification is
    // a wrong password. A snapshot this build cannot write or read for any
    // other reason is a restore failure, and says so.
    let store = SqliteStore::open(db_path, &*key.sqlcipher_key()).map_err(|e| match e {
        StoreError::WrongKey => Error::InvalidPassword,
        StoreError::SchemaNewer => Error::VaultTooNew,
        e => Error::Other(format!("could not open the restored vault: {e}")),
    })?;

    // The snapshot carries the *source* device's sync bookkeeping. Keeping it
    // would have this install believe it had already synced state it has never
    // seen; scrubbing by prefix keeps that true for keys the engine has not
    // invented yet.
    store
        .meta_delete_prefix(SYNC_META_PREFIX)
        .map_err(crate::session::store_err)?;

    Ok((key, store))
}

// Create the DB file 0600 from the outset — `SqliteStore::open` chmods it too,
// but only after the bytes are already on disk and readable.
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }

    let mut file = opts.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{identity, Record, VaultStore};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    const PASSWORD: &str = "correct horse battery staple";

    // Low-cost Argon2id keeps these tests fast; production uses the 64 MiB defaults.
    fn params() -> KdfParams {
        KdfParams::argon2id(b"salt-restore-0123456789012345678", 256, 1, 1)
    }

    fn key_for(password: &str) -> VaultKey {
        VaultKey::Argon2 {
            master: crypto::derive(password.as_bytes(), &params()).unwrap(),
        }
    }

    fn tmp_dir() -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "rowel-restore-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn record(id: &str) -> Record {
        Record {
            id: id.into(),
            kind: "login".into(),
            title: "Example".into(),
            tags: "[]".into(),
            url_host: "example.com".into(),
            created_at: 100,
            updated_at: 200,
            deleted_at: None,
            payload: b"sealed".to_vec(),
            card_brand: None,
            favorite: false,
            has_passkey: false,
            file_name: None,
            var_count: None,
            username: None,
        }
    }

    // A source vault plus its packed bytes — what the remote drive would hold.
    fn packed_source() -> (SqliteStore, Vec<u8>) {
        let dir = tmp_dir();
        let key = key_for(PASSWORD);
        let store = SqliteStore::open(&dir.join("vault.db"), &*key.sqlcipher_key()).unwrap();
        store.import(&[record("1"), record("2")]).unwrap();
        // Bookkeeping the restore must not inherit.
        store.meta_set("sync_last_digest", "deadbeef").unwrap();
        // …and app meta that it must keep.
        store.meta_set("kdf", "argon2id").unwrap();
        identity::assign_vault_id(&store).unwrap();

        let bytes = pack::pack_store(
            &store,
            &*key.sqlcipher_key(),
            &params().to_json().unwrap(),
            &dir.join("scratch"),
        )
        .unwrap();
        (store, bytes)
    }

    // Where a fresh install's vault would go: an empty data dir.
    fn fresh_target() -> (PathBuf, PathBuf) {
        let dir = tmp_dir();
        (dir.join("vault.db"), dir.join("vault.kdf.json"))
    }

    fn is_empty(dir: &Path) -> bool {
        fs::read_dir(dir).unwrap().next().is_none()
    }

    #[test]
    fn restores_a_packed_vault_onto_a_fresh_install() {
        let (source, bytes) = packed_source();
        let (db, sidecar) = fresh_target();

        let (_, store) = restore_at(&db, &sidecar, &bytes, PASSWORD).unwrap();

        assert_eq!(
            store.state_digest().unwrap(),
            source.state_digest().unwrap()
        );
        assert_eq!(store.list().unwrap().len(), 2);
        assert_eq!(store.get("1").unwrap().unwrap().payload, b"sealed");
        // The sidecar is the source's descriptor verbatim, so the next unlock
        // derives the same key this restore just did.
        assert_eq!(
            fs::read_to_string(&sidecar).unwrap(),
            params().to_json().unwrap()
        );
    }

    #[test]
    fn restore_scrubs_the_source_devices_sync_bookkeeping() {
        let (_, bytes) = packed_source();
        let (db, sidecar) = fresh_target();

        let (_, store) = restore_at(&db, &sidecar, &bytes, PASSWORD).unwrap();

        assert_eq!(store.meta_get("sync_last_digest").unwrap(), None);
        // Scrubbing is scoped to the prefix: app meta is untouched.
        assert_eq!(store.meta_get("kdf").unwrap().as_deref(), Some("argon2id"));
    }

    // The restored vault *is* the source vault, so it answers to the same name
    // on Drive. The id is deliberately not under `SYNC_META_PREFIX`: scrubbing
    // it would make the next sync mint a second id, and the account would end
    // up holding the same vault twice under two names.
    #[test]
    fn restore_keeps_the_vault_id_the_pack_carries() {
        let (source, bytes) = packed_source();
        let (db, sidecar) = fresh_target();

        let (_, store) = restore_at(&db, &sidecar, &bytes, PASSWORD).unwrap();

        let restored = identity::vault_id(&store).unwrap();
        assert!(restored.is_some());
        assert_eq!(restored, identity::vault_id(&source).unwrap());
    }

    #[test]
    fn restore_refuses_when_a_vault_is_already_installed() {
        let (_, bytes) = packed_source();
        let (db, sidecar) = fresh_target();
        fs::write(&db, b"an existing vault").unwrap();

        assert!(restore_at(&db, &sidecar, &bytes, PASSWORD).is_err());
        // Untouched: the local vault is authoritative, and no sidecar appeared.
        assert_eq!(fs::read(&db).unwrap(), b"an existing vault");
        assert!(!sidecar.exists());
    }

    #[test]
    fn a_wrong_password_leaves_no_remnants_behind() {
        let (_, bytes) = packed_source();
        let (db, sidecar) = fresh_target();

        match restore_at(&db, &sidecar, &bytes, "wrong password").map(|_| ()) {
            Err(Error::InvalidPassword) => {}
            other => panic!("expected InvalidPassword, got {other:?}"),
        }
        // Exactly as the install started — nothing to masquerade as a broken vault.
        assert!(is_empty(db.parent().unwrap()));
    }

    #[test]
    fn a_snapshot_from_a_future_build_says_so_and_still_cleans_up() {
        let dir = tmp_dir();
        let key = key_for(PASSWORD);
        let store = SqliteStore::open(&dir.join("vault.db"), &*key.sqlcipher_key()).unwrap();
        store.import(&[record("1")]).unwrap();
        store.set_user_version(99).unwrap();
        let bytes = pack::pack_store(
            &store,
            &*key.sqlcipher_key(),
            &params().to_json().unwrap(),
            &dir.join("scratch"),
        )
        .unwrap();

        let (db, sidecar) = fresh_target();
        match restore_at(&db, &sidecar, &bytes, PASSWORD).map(|_| ()) {
            Err(Error::VaultTooNew) => {}
            other => panic!("expected VaultTooNew, got {other:?}"),
        }
        assert!(is_empty(db.parent().unwrap()));
    }

    #[test]
    fn a_corrupt_pack_is_rejected_before_anything_is_written() {
        let (_, bytes) = packed_source();
        let (db, sidecar) = fresh_target();

        assert!(restore_at(&db, &sidecar, &bytes[..bytes.len() / 2], PASSWORD).is_err());
        assert!(is_empty(db.parent().unwrap()));
    }
}
