use crate::app::APP_NAME;
use crate::crypto::PayloadCipher;
use crate::error::{Error, Result};
use crate::events;
use crate::models::{Entry, EntryMetaDto, VaultData};
use crate::session::{derive_key, list_deleted_metas, list_metas, meta_dto_of, store_err};
use crate::state::AppState;
use crate::store::{migrate, Record, SqliteStore, VaultStore};
use crate::{credential_identities, crypto, save, storage, sync};
use serde::Serialize;
use tauri::{AppHandle, State};
use zeroize::Zeroizing;

// Decrypt one entry on demand (view/edit): fetch its payload and unseal it with
// the session payload key. Nothing is cached in the session.
//
// What goes out is `Entry::redacted`: the passkey private keys stay in the core,
// where the authenticator that needs them lives. Nothing in the webview reads
// one, so nothing there should hold one.
#[tauri::command]
pub fn reveal_entry(id: String, state: State<'_, AppState>) -> Result<Entry> {
    let session = state.session.lock().unwrap();
    let cipher = session.payload_cipher()?;
    let record = session
        .store()?
        .get(&id)
        .map_err(store_err)?
        .ok_or(Error::NotFound)?;
    Ok(cipher.unseal(&record.id, &record.payload)?.redacted())
}

// Persist one entry: seal it into a fresh payload and upsert a single row
// (metadata + payload), stamping updated_at. No whole-vault rewrite.
#[tauri::command]
pub fn save_entry(
    mut entry: Entry,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<EntryMetaDto> {
    let session = state.session.lock().unwrap();
    let cipher = session.payload_cipher()?;
    let store = session.store()?;

    restore_passkey_keys(&mut entry, store, &cipher)?;
    let payload = cipher.seal(&entry)?;
    let record = migrate::build_record(&entry, payload)?;
    store.upsert(&record).map_err(store_err)?;
    // Off this thread, so it reads the vault once this lock is let go.
    credential_identities::publish(&app);

    meta_dto_of(store, &record.id)
}

// The other half of the reveal's redaction: an entry coming back from the
// webview carries its passkeys without their private keys, so every one of them
// is read off the row being replaced and matched by credential id (see
// `Entry::restore_passkey_keys`). The stored row is unsealed only when the
// entry has a passkey at all, so an ordinary save costs nothing extra. The
// unsealed row is handed over whole so the keys move rather than copy, and what
// is left of it is scrubbed and dropped inside `Entry::restore_passkey_keys`.
//
// A save that carries a private key of its own is refused outright. Passkeys
// only ever enter the vault through Rust — import and sync — and a reveal
// redacts, so there is no path by which the webview came to hold one: a key
// here is one it invented, and honouring it would let it overwrite the key the
// core holds for that credential with something it can sign with itself.
fn restore_passkey_keys(
    entry: &mut Entry,
    store: &SqliteStore,
    cipher: &PayloadCipher,
) -> Result<()> {
    if entry.has_supplied_passkey_key() {
        return Err(Error::Unsupported(
            "a passkey's private key cannot be set from here".into(),
        ));
    }
    if !entry.has_passkeys() {
        return Ok(());
    }
    let stored = store
        .get(&entry.id)
        .map_err(store_err)?
        .map(|record| cipher.unseal(&record.id, &record.payload))
        .transpose()?;
    entry.restore_passkey_keys(stored)
}

// Tombstone one entry (retained for sync); it drops out of the list.
#[tauri::command]
pub fn delete_entry(id: String, app: AppHandle, state: State<'_, AppState>) -> Result<()> {
    let session = state.session.lock().unwrap();
    session.store()?.delete(&id).map_err(store_err)?;
    credential_identities::publish(&app);
    Ok(())
}

// The Trash: tombstoned entries' metadata, newest deletion first.
#[tauri::command]
pub fn list_deleted(state: State<'_, AppState>) -> Result<Vec<EntryMetaDto>> {
    let session = state.session.lock().unwrap();
    list_deleted_metas(session.store()?)
}

// Bring a tombstoned entry back; returns its refreshed metadata so the list can
// take it back without a re-read.
#[tauri::command]
pub fn restore_entry(
    id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<EntryMetaDto> {
    let session = state.session.lock().unwrap();
    let store = session.store()?;
    store.restore(&id).map_err(store_err)?;
    credential_identities::publish(&app);
    meta_dto_of(store, &id)
}

// Discard a tombstoned entry's contents for good. See `SqliteStore::purge` for
// why this empties the row rather than deleting it.
#[tauri::command]
pub fn purge_entry(id: String, app: AppHandle, state: State<'_, AppState>) -> Result<()> {
    let session = state.session.lock().unwrap();
    session.store()?.purge(&id).map_err(store_err)?;
    credential_identities::publish(&app);
    Ok(())
}

// Star or unstar one entry. A metadata-only write: no payload is unsealed or
// re-sealed, so the star never risks the secret fields.
#[tauri::command]
pub fn set_favorite(
    id: String,
    favorite: bool,
    state: State<'_, AppState>,
) -> Result<EntryMetaDto> {
    let session = state.session.lock().unwrap();
    let store = session.store()?;
    store.set_favorite(&id, favorite).map_err(store_err)?;
    meta_dto_of(store, &id)
}

/// The backup file's extension, and the one `export_vault` writes: the same
/// pack, and so the same extension, as the vault on Drive (`sync::layout`).
pub const BACKUP_EXTENSION: &str = sync::layout::VAULT_EXTENSION;

/// What an import merged, plus the list it left behind — so the frontend takes
/// the refreshed vault from the same call rather than re-reading it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SwftxReport {
    pub count: usize,
    pub entries: Vec<EntryMetaDto>,
}

// Import a `.swftx` backup into the *currently unlocked* vault. The file is
// independently encrypted and carries its own master password (which may differ
// from the current vault's). Each entry is decrypted under the source key and
// re-sealed under the current session payload key, then upserted (merge/add by
// id). The CPU-bound re-seal loop runs off the UI thread and emits `import:progress`.
#[tauri::command]
pub async fn import_swftx(
    path: String,
    password: Zeroizing<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<SwftxReport> {
    // The cipher and the session it belongs to: the merge below is accepted
    // only by that session (`Session::store_at`), so a password change landing
    // during the re-seal cannot leave rows sealed under a key the vault no
    // longer has.
    let (cur_cipher, epoch) = {
        let session = state.session.lock().unwrap();
        (session.payload_cipher()?, session.epoch())
    };

    // The file read, the source decrypt and the re-seal loop are one hop onto
    // the blocking pool: a backup is the whole vault, and both halves are as
    // expensive as the loop they lead into. Expose under the source key, re-seal
    // under the current payload key — emitting progress as it goes.
    let emitter = app.clone();
    let records = super::blocking(move || -> Result<Vec<Record>> {
        let blob = storage::read_backup(&path)?;
        let src_cryptor = crypto::Cryptor::new(&crypto::hash_secret(&password));
        // Validate the source password before touching the store. The blob goes
        // in by value: the file is freed as soon as it has been decoded, so the
        // loop below runs with the decrypted vault alone, not with the file it
        // came out of still beside it.
        let src: VaultData = src_cryptor
            .decrypt_data_owned(blob)
            .map_err(|_| Error::InvalidPassword)?;

        let total = src.entries.len();
        let mut records = Vec::with_capacity(total);
        for (i, obscured) in src.entries.iter().enumerate() {
            records.push(migrate::reseal_one(obscured, &src_cryptor, &cur_cipher)?);
            events::import_progress(&emitter, i + 1, total);
        }
        Ok(records)
    })
    .await?;

    // Merge into the open store (upsert by id).
    let session = state.session.lock().unwrap();
    let store = session.store_at(epoch)?;
    for record in &records {
        store.upsert(record).map_err(store_err)?;
    }
    credential_identities::publish(&app);
    Ok(SwftxReport {
        count: records.len(),
        entries: list_metas(store)?,
    })
}

// Export the vault to a user-chosen `.rowel` file: the same pack the sync engine
// uploads (KDF descriptor + SQLCipher snapshot, see `sync::pack`), so a backup
// restores through `setup_restore_from_file` exactly as a Drive pack does. The
// snapshot is already sealed under the vault key; `password` is asked for only
// to prove the person exporting can open what they are about to carry away.
// Nothing is purged first — a backup keeps every tombstone the sync pack would
// have reclaimed.
#[tauri::command]
pub async fn export_vault(
    password: Zeroizing<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<String>> {
    let kdf_params_json = storage::read_kdf_sidecar(&app)?.ok_or_else(|| {
        Error::Other("this vault predates the key descriptor and cannot be backed up".into())
    })?;
    let scratch = storage::sync_scratch_dir(&app)?;

    // Argon2id before the lock is taken, not under it: a KDF is hundreds of
    // milliseconds, and the session guard held across one stalls every other
    // command for the duration. Deriving needs only the sidecar and the
    // password, so there is nothing to race with — the result is only *trusted*
    // by the check below.
    let candidate = {
        let app = app.clone();
        super::blocking(move || derive_key(&app, &password)).await?
    };

    let (key, source) = {
        let session = state.session.lock().unwrap();
        // Guard: the export key must match the unlocked vault.
        let key = session.key()?.sqlcipher_key();
        if candidate.sqlcipher_key() != key {
            return Err(Error::InvalidPassword);
        }
        // A connection of its own, so the whole-database copy below does not
        // borrow the session's for its duration.
        let source = crate::session::open_snapshot_source(&app, &*key)?;
        (key, source)
    };

    let bytes = super::blocking(move || {
        Ok(sync::pack::pack_store(
            &source,
            &*key,
            &kdf_params_json,
            &scratch,
        )?)
    })
    .await?;

    let dest = save::save_export(
        &app,
        &format!(
            "{APP_NAME} backup {}.{BACKUP_EXTENSION}",
            chrono::Local::now().format("%Y-%m-%d")
        ),
        &format!("{APP_NAME} backup"),
        bytes,
    )
    .await?;
    Ok(dest.map(|p| p.to_string_lossy().into_owned()))
}

// "Save as file…" on an env entry: hand the revealed `.env` back to disk under
// the name it came in with (or `.env`), owner-readable only. The body arrives
// from the frontend, which already holds it revealed, so nothing is unsealed
// here — and nothing is logged: the body is the entry's one secret. Desktop
// only; the frontend hides the action on mobile, where the picker cannot be
// told what permissions to write with.
#[tauri::command]
pub async fn save_env_file(
    file_name_suggestion: String,
    body: String,
    app: AppHandle,
) -> Result<Option<String>> {
    let suggestion = file_name_suggestion.trim();
    let file_name = if suggestion.is_empty() {
        ".env"
    } else {
        suggestion
    };
    #[cfg(desktop)]
    {
        let dest = save::save_private_text(&app, file_name, body).await?;
        Ok(dest.map(|p| p.to_string_lossy().into_owned()))
    }
    #[cfg(mobile)]
    {
        let _ = (file_name, body, app);
        Err(Error::Unsupported(
            "saving a file is a desktop action".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::VaultKey;
    use crate::models::Passkey;
    use crate::store::SqliteStore;

    fn passkey(credential_id: &str, private_key: &str) -> Passkey {
        Passkey {
            credential_id: credential_id.into(),
            rp_id: "acme.test".into(),
            rp_name: None,
            user_handle: "dWgx".into(),
            user_name: "alice".into(),
            user_display_name: "Alice".into(),
            private_key: private_key.into(),
            counter: 0,
            created_at: None,
        }
    }

    fn login(passkeys: Vec<Passkey>) -> Entry {
        Entry {
            id: "l1".into(),
            kind: "login".into(),
            title: "Site".into(),
            passkeys: Some(passkeys),
            ..Entry::default()
        }
    }

    fn open(dir: &tempfile::TempDir) -> (SqliteStore, PayloadCipher) {
        let key = VaultKey::legacy_from_password("pw");
        let store = SqliteStore::open(&dir.path().join("vault.db"), key.sqlcipher_key().as_slice())
            .unwrap();
        (store, key.payload_cipher())
    }

    fn save(store: &SqliteStore, cipher: &PayloadCipher, entry: &Entry) {
        let payload = cipher.seal(entry).unwrap();
        store
            .upsert(&migrate::build_record(entry, payload).unwrap())
            .unwrap();
    }

    // The round trip the webview makes: what a reveal hands out carries no
    // private key, and the save that comes back is completed from the row it
    // replaces — so the key survives an edit it never travelled through.
    #[test]
    fn a_reveal_hides_the_passkey_key_and_the_save_puts_it_back() {
        let dir = tempfile::tempdir().unwrap();
        let (store, cipher) = open(&dir);
        save(&store, &cipher, &login(vec![passkey("c1", "k1")]));

        let record = store.get("l1").unwrap().unwrap();
        let revealed = cipher
            .unseal(&record.id, &record.payload)
            .unwrap()
            .redacted();
        assert_eq!(revealed.passkeys.as_ref().unwrap()[0].private_key, "");

        let mut incoming = revealed;
        incoming.title = "Renamed".into();
        restore_passkey_keys(&mut incoming, &store, &cipher).unwrap();
        assert_eq!(incoming.passkeys.as_ref().unwrap()[0].private_key, "k1");

        // And it is the completed entry that lands, not the blanked one.
        save(&store, &cipher, &incoming);
        let record = store.get("l1").unwrap().unwrap();
        let stored: Entry = cipher.unseal(&record.id, &record.payload).unwrap();
        assert_eq!(stored.title, "Renamed");
        assert_eq!(stored.passkeys.unwrap()[0].private_key, "k1");
    }

    // A keyless passkey the stored row cannot account for — here a brand new
    // entry, which has no stored row at all — is refused.
    #[test]
    fn a_keyless_passkey_with_nothing_to_merge_from_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (store, cipher) = open(&dir);

        let mut fresh = login(vec![passkey("c1", "")]);
        assert!(matches!(
            restore_passkey_keys(&mut fresh, &store, &cipher),
            Err(Error::NotFound)
        ));
    }

    // No passkeys, nothing to do: an ordinary save never unseals the old row.
    #[test]
    fn a_save_without_passkeys_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let (store, cipher) = open(&dir);

        let mut entry = login(vec![]);
        restore_passkey_keys(&mut entry, &store, &cipher).unwrap();
        assert!(entry.passkeys.unwrap().is_empty());
    }

    // A save carrying a private key is one the webview could only have made up
    // — a reveal never hands one out — so it is refused rather than written
    // over the key the core holds.
    #[test]
    fn a_save_that_supplies_a_private_key_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (store, cipher) = open(&dir);
        save(&store, &cipher, &login(vec![passkey("c1", "k1")]));

        let mut forged = login(vec![passkey("c1", "theirs")]);
        assert!(matches!(
            restore_passkey_keys(&mut forged, &store, &cipher),
            Err(Error::Unsupported(_))
        ));

        // The stored key is untouched.
        let record = store.get("l1").unwrap().unwrap();
        let stored: Entry = cipher.unseal(&record.id, &record.payload).unwrap();
        assert_eq!(stored.passkeys.unwrap()[0].private_key, "k1");
    }

    // One unknown credential refuses the whole save, and the passkey listed
    // before it does not walk off with the stored key on the way out.
    #[test]
    fn an_unknown_credential_refuses_the_save_and_moves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (store, cipher) = open(&dir);
        save(
            &store,
            &cipher,
            &login(vec![passkey("c1", "k1"), passkey("c2", "k2")]),
        );

        let mut incoming = login(vec![passkey("c1", ""), passkey("c9", "")]);
        assert!(matches!(
            restore_passkey_keys(&mut incoming, &store, &cipher),
            Err(Error::NotFound)
        ));
        assert!(incoming
            .passkeys
            .unwrap()
            .iter()
            .all(|p| p.private_key.is_empty()));
    }
}
