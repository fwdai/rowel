use crate::app::APP_NAME;
use crate::crypto::{PayloadCipher, VaultKey};
use crate::error::{Error, Result};
use crate::events;
use crate::grants::{self, PathGrants, Purpose};
use crate::models::{Entry, EntryMetaDto, VaultData};
use crate::org::lockboxes::is_org_lockbox_dir;
use crate::session::{list_deleted_metas, list_metas, meta_dto_of, store_err, Session};
use crate::state::AppState;
use crate::store::{migrate, Record, SqliteStore, VaultStore};
use crate::sync::engine::WorkspaceVault;
use crate::workspace::{self, Registry};
use crate::{auth, credential_identities, crypto, picked, save, storage, sync};
use serde::Serialize;
use std::path::{Path, PathBuf};
use subtle::ConstantTimeEq;
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
    let mut entry = cipher.unseal(&record.id, &record.payload)?;
    // A key saved before this was done on save shows its plate, and a public
    // line that is not its own is not shown as if it were; the next save
    // writes what this settles.
    entry.reconcile_ssh_public(None);
    Ok(entry.redacted())
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

    merge_stored(&mut entry, store, &cipher, &now())?;
    let payload = cipher.seal(&entry)?;
    let record = migrate::build_record(&entry, payload)?;
    store.upsert(&record).map_err(store_err)?;
    // Off this thread, so it reads the vault once this lock is let go.
    credential_identities::publish(&app);

    meta_dto_of(store, &record.id)
}

// This moment, as the stamps on an entry are written.
fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

// Complete an entry coming back from the webview from the row it replaces: the
// stored row is the source of truth for what the webview is never handed or
// never trusted with.
//
// The password history and rotation stamp are carried forward from it, and a
// changed password is recorded there (see `Entry::record_password_change`).
//
// And the other half of the reveal's redaction: the entry carries its passkeys
// without their private keys, so every one of them is read off the stored row
// and matched by credential id (see `Entry::restore_passkey_keys`). The
// unsealed row is handed over whole so the keys move rather than copy, and what
// is left of it is scrubbed and dropped inside `Entry::restore_passkey_keys`.
//
// A save that carries a private key of its own is refused outright. Passkeys
// only ever enter the vault through Rust — import and sync — and a reveal
// redacts, so there is no path by which the webview came to hold one: a key
// here is one it invented, and honouring it would let it overwrite the key the
// core holds for that credential with something it can sign with itself.
fn merge_stored(
    entry: &mut Entry,
    store: &SqliteStore,
    cipher: &PayloadCipher,
    now: &str,
) -> Result<()> {
    if entry.has_supplied_passkey_key() {
        return Err(Error::Unsupported(
            "a passkey's private key cannot be set from here".into(),
        ));
    }
    let mut stored = store
        .get(&entry.id)
        .map_err(store_err)?
        .map(|record| cipher.unseal(&record.id, &record.payload))
        .transpose()?;
    entry.record_password_change(stored.as_mut(), now);
    entry.reconcile_ssh_public(stored.as_ref());
    entry.restore_passkey_keys(stored)
}

// Forget a login's previous passwords. The stored row is unsealed, cleared and
// resealed here rather than sent round the webview, so nothing else on it can
// move. `updatedAt` moves, so the change wins the sync race like any edit.
#[tauri::command]
pub fn clear_password_history(id: String, state: State<'_, AppState>) -> Result<EntryMetaDto> {
    let session = state.session.lock().unwrap();
    let store = session.store()?;
    clear_history_in(store, &session.payload_cipher()?, &id, &now())?;
    meta_dto_of(store, &id)
}

fn clear_history_in(
    store: &SqliteStore,
    cipher: &PayloadCipher,
    id: &str,
    now: &str,
) -> Result<()> {
    let record = store.get(id).map_err(store_err)?.ok_or(Error::NotFound)?;
    let mut entry = cipher.unseal(&record.id, &record.payload)?;
    entry.password_history = None;
    entry.updated_at = Some(now.to_string());
    let payload = cipher.seal(&entry)?;
    store
        .upsert(&migrate::build_record(&entry, payload)?)
        .map_err(store_err)
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

/// The legacy app's backup, which `import_swftx` merges into the open vault.
pub const LEGACY_BACKUP_EXTENSION: &str = "swftx";

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
//
// `path` has to be a backup the user chose — picked, dropped or opened with the
// app (see `grants`). The grant is held across a mistyped password, which reads
// the file again, and spent once the password has opened it.
#[tauri::command]
pub async fn import_swftx(
    path: String,
    password: Zeroizing<String>,
    app: AppHandle,
    state: State<'_, AppState>,
    grants: State<'_, PathGrants>,
) -> Result<SwftxReport> {
    // The cipher and the session it belongs to: the merge below is accepted
    // only by that session (`Session::store_at`), so a password change landing
    // during the re-seal cannot leave rows sealed under a key the vault no
    // longer has.
    let (cur_cipher, epoch) = {
        let session = state.session.lock().unwrap();
        (session.payload_cipher()?, session.epoch())
    };
    let chosen = PathBuf::from(&path);
    if !grants.holds(&chosen, Purpose::Backup) {
        return Err(grants::not_chosen());
    }

    // The file read, the source decrypt and the re-seal loop are one hop onto
    // the blocking pool: a backup is the whole vault, and both halves are as
    // expensive as the loop they lead into. Expose under the source key, re-seal
    // under the current payload key — emitting progress as it goes.
    let emitter = app.clone();
    let records = super::blocking(move || -> Result<Vec<Record>> {
        let chosen = picked::open(&emitter, Path::new(&path))?;
        let (src_cryptor, src) = open_legacy_backup(chosen.file, &password)?;
        reseal_legacy(&emitter, &src_cryptor, &src.entries, &cur_cipher)
    })
    .await?;
    if !grants.take(&chosen, Purpose::Backup) {
        return Err(grants::not_chosen());
    }

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

/// Open the `.swftx` in `file` (as `picked::open` opened it) under `password`:
/// the legacy cryptor it was sealed with, and the vault it holds. A wrong
/// password is [`Error::InvalidPassword`], before anything downstream is
/// touched. The blob goes in by value, so the file is freed as soon as it has
/// been decoded.
///
/// Shared by the merge above and the first run's restore
/// (`commands::setup::setup_restore_from_legacy`).
pub(crate) fn open_legacy_backup(
    file: std::fs::File,
    password: &str,
) -> Result<(crypto::Cryptor, VaultData)> {
    let blob = storage::read_backup(file)?;
    let src_cryptor = crypto::Cryptor::new(&crypto::hash_secret(password));
    let src: VaultData = src_cryptor
        .decrypt_data_owned(blob)
        .map_err(|_| Error::InvalidPassword)?;
    Ok((src_cryptor, src))
}

/// Re-seal the entries of an opened `.swftx` under `cipher`, emitting
/// `import:progress` as it goes. CPU-bound: the caller runs it off the UI
/// thread.
pub(crate) fn reseal_legacy(
    emitter: &AppHandle,
    src: &crypto::Cryptor,
    entries: &[Entry],
    cipher: &PayloadCipher,
) -> Result<Vec<Record>> {
    let total = entries.len();
    let mut records = Vec::with_capacity(total);
    for (i, obscured) in entries.iter().enumerate() {
        records.push(migrate::reseal_one(obscured, src, cipher)?);
        events::import_progress(emitter, i + 1, total);
    }
    Ok(records)
}

/// `InvalidPassword` unless `candidate` is the key the unlocked vault is open
/// under, compared in constant time. The comparison alone: a password the
/// user just typed is proved with
/// [`prove_session_password`](super::vault_password::prove_session_password),
/// which keeps the backoff, and a command that then reads or re-keys the
/// session asks this again under the guard it does so with, so the key it
/// proved is still the session's.
pub(crate) fn prove_password(session: &Session, candidate: &VaultKey) -> Result<()> {
    let key = session.key()?.sqlcipher_key();
    if bool::from(candidate.sqlcipher_key()[..].ct_eq(&key[..])) {
        Ok(())
    } else {
        Err(Error::InvalidPassword)
    }
}

// Export one lockbox to a user-chosen `.rowel` file: the same pack the sync
// engine uploads (KDF descriptor + SQLCipher snapshot, see `sync::pack`), so a
// backup restores through `setup_restore_from_file` exactly as a Drive pack
// does. `workspace_id` names the lockbox, and the open one is saved when it
// names none. The snapshot is already sealed under that lockbox's key;
// `password` is asked for only to prove the person exporting can open what
// they are about to carry away. Nothing is purged first — a backup keeps every
// tombstone the sync pack would have reclaimed.
//
// The password proved is the one that opens the lockbox being saved, not
// whichever lockbox happens to be open. The open lockbox is the session's: its
// own password, proved as every prompt about the open lockbox is
// (`prove_session_password`), and packed through the session. Any other is
// held by the key the app keeps for it, sealed under the app key
// (`crate::appkey`), so what opens it is the vault password
// (`verify_master_password`, under the primary's backoff) — whatever the open
// lockbox is, an org lockbox with no password of its own included. It is
// packed from its own directory and descriptor, opened as a sync of it opens
// it ([`pack_lockbox`]). Both proofs run under the setup step, so nothing
// re-keys, moves or deletes the lockbox while it is proved and read.
//
// The save is recorded once it has happened ([`record_saved`]), against the
// lockbox that was packed, wherever the registry has it by then: a dialog
// dismissed writes nothing and records nothing.
#[tauri::command]
pub async fn export_vault(
    password: Zeroizing<String>,
    workspace_id: Option<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<String>> {
    let root = storage::root_dir(&app)?;
    // From the target on: the open lockbox cannot be switched away between
    // being picked and its password being proved.
    let step = super::setup::begin_step(&state)?;
    let target = {
        let _paths = state.workspace_lock.lock().unwrap();
        let active = workspace::active_id(&app);
        let ring = state.keyring.lock().unwrap();
        backup_target(
            &root,
            &Registry::load(&root),
            &active,
            workspace_id.as_deref(),
            |id| ring.has(id),
        )?
    };

    let bytes = if target.open {
        let kdf_params_json = target.sealed_under.clone();
        let scratch = storage::sync_scratch_dir(&app)?;

        // Proved before the lock is taken, not under it: the Argon2id behind it
        // is hundreds of milliseconds, and the session guard held across one
        // stalls every other command for the duration. The check below is the
        // same comparison again, under the guard the snapshot is read with.
        let candidate =
            super::vault_password::prove_session_password(&app, &step, password).await?;
        drop(step);

        let (key, source) = {
            let session = state.session.lock().unwrap();
            // Guard: the export key must match the unlocked vault.
            prove_password(&session, &candidate)?;
            let key = session.key()?.sqlcipher_key();
            // A connection of its own, so the whole-database copy below does not
            // borrow the session's for its duration.
            let source = crate::session::open_snapshot_source(&app, &*key)?;
            (key, source)
        };

        super::blocking(move || {
            Ok(sync::pack::pack_store(
                &source,
                &*key,
                &kdf_params_json,
                &scratch,
            )?)
        })
        .await?
    } else {
        // Not the session's lockbox, so not the session's password: its key is
        // sealed under the app key, and the password that opens it is the vault
        // password. Proved under the primary's backoff, with the step held as
        // every proof against a backoff is.
        let app_key =
            super::vault_password::verify_master_password(&app, &root, &state, password).await?;
        // The ring read again under the guards, as the session's key is above:
        // a lock that landed meanwhile has emptied it, and a password change
        // has put another app key in it than the one just proved.
        let material = {
            let session = state.session.lock().unwrap();
            session.key()?;
            let ring = state.keyring.lock().unwrap();
            let current = ring
                .app_key()
                .is_some_and(|held| bool::from(held.ct_eq(&app_key)));
            if !current {
                return Err(Error::Locked);
            }
            ring.get(&target.id).ok_or(Error::Locked)?
        };
        let key = VaultKey::from_material(material, true);
        let (dir, id) = (target.dir.clone(), target.id.clone());
        let bytes = super::blocking(move || pack_lockbox(&dir, &id, key)).await?;
        drop(step);
        bytes
    };

    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let dest = save::save_export(
        &app,
        &backup_file_name(target.name.as_deref(), &today),
        &format!("{APP_NAME} backup"),
        bytes,
    )
    .await?;
    let saved = {
        let _paths = state.workspace_lock.lock().unwrap();
        record_saved(&root, &target.sealed_under, dest, &now())
    };
    Ok(saved)
}

fn no_descriptor() -> Error {
    Error::Other("this vault predates the key descriptor and cannot be backed up".into())
}

/// The lockbox [`export_vault`] saves.
#[derive(Debug)]
struct BackupTarget {
    id: String,
    /// The registry's name for it, for the file name: `None` for a primary
    /// nobody has named.
    name: Option<String>,
    dir: PathBuf,
    /// Its key descriptor, as its sidecar had it when it was picked: the pack's
    /// header, and what the lockbox is known by once the step is released. The
    /// registry's id is a slot, not a lockbox — `default` names whichever
    /// lockbox is the primary, and a delete of the primary moves a survivor
    /// into it — while the descriptor carries the lockbox's own random salt
    /// and moves with its files.
    sealed_under: String,
    /// The session's own, packed through the session rather than its files.
    open: bool,
}

/// The lockbox `requested` names — `active`, the open one, when it names none
/// — refused when it is not one this device can save: an id the registry does
/// not name; an org lockbox, whose key its server holds and whose pack, under
/// the server's descriptor, no password opens; and one the app holds no key
/// for (`held`, asked of the ring), whose files nothing here opens without its
/// own password. Settled before the password is asked about, so a refusal
/// costs no Argon2id wait and no attempt against the backoff.
///
/// The caller holds the setup step: a lockbox that is not open is first put
/// back as an interrupted password change left it (see below).
fn backup_target(
    root: &Path,
    registry: &Registry,
    active: &str,
    requested: Option<&str>,
    held: impl FnOnce(&str) -> bool,
) -> Result<BackupTarget> {
    let id = requested.unwrap_or(active);
    let lockbox = registry
        .workspaces
        .iter()
        .find(|w| w.id == id)
        .ok_or(Error::NotFound)?;
    let dir = workspace::dir_of(root, id);
    let open = id == active;
    // The rollback every unlock runs first, for a lockbox this save reaches
    // without one: a password change re-keys every lockbox that shares the
    // password, and a crash during it can leave this one's entries resealed
    // under a key its database and descriptor do not name yet. The key the
    // ring holds would still open that database, and the pack would carry
    // entries nothing decrypts. Before anything here reads the descriptor,
    // since the rollback decides what it says; under the caller's setup step,
    // as every rollback runs, so a change still in flight is never taken for
    // an interrupted one. The open lockbox passed through this when it was
    // opened, and its store is in use.
    if !open {
        auth::recover_interrupted_rekey_in(&dir)?;
    }
    // Told by its descriptor, as a delete and nearby sync tell one: the
    // registry's `server` is a note beside it.
    if is_org_lockbox_dir(&dir) {
        return Err(Error::Unsupported(
            "an org lockbox is kept by its server and cannot be saved as a backup".into(),
        ));
    }
    let sealed_under = match std::fs::read_to_string(dir.join(storage::KDF_SIDECAR_FILE)) {
        Ok(descriptor) => descriptor,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(no_descriptor()),
        Err(e) => return Err(e.into()),
    };
    if !open && !held(id) {
        return Err(Error::Unsupported(
            "this lockbox is not unlocked on this device; switch to it once to unlock it, \
             then save its backup"
                .into(),
        ));
    }
    Ok(BackupTarget {
        id: id.to_string(),
        name: lockbox.name.clone(),
        dir,
        sealed_under,
        open,
    })
}

/// A lockbox the session does not have open, packed from its own directory and
/// descriptor with `key`: opened as a sync of it is (`WorkspaceVault::open`)
/// and copied whole, nothing reclaimed. Blocking — a SQLCipher open and a copy
/// of the whole database.
pub(crate) fn pack_lockbox(dir: &Path, workspace_id: &str, key: VaultKey) -> Result<Vec<u8>> {
    // The provider is named only by a foreign-pack error, which packing never
    // meets; no audit writer, since nothing is pruned.
    WorkspaceVault::open(dir, workspace_id, key, sync::Provider::default(), None)?.backup()
}

/// What the save dialog came back with, for the webview — and, when it put the
/// file somewhere, a note of when it did against the lockbox that was packed
/// (`None` is a dialog dismissed).
///
/// The lockbox is found by `sealed_under`, the descriptor it was packed with
/// ([`workspace::sealed_under`]), not by the id it had then: the dialog stays
/// open with no step held, and a delete of the primary meanwhile leaves that
/// id naming another lockbox. One that has gone is not noted anywhere; one
/// that was promoted is noted where it lives now. A lockbox whose password
/// changed while the dialog was open has a new descriptor and goes unnoted,
/// which leaves the page showing an older time for a backup that is under the
/// old password anyway.
///
/// Best effort: the backup is on disk either way. For a caller holding
/// `workspace_lock` (see [`workspace::record_backup_in`]).
fn record_saved(
    root: &Path,
    sealed_under: &str,
    dest: Option<PathBuf>,
    at: &str,
) -> Option<String> {
    let dest = dest?;
    match workspace::sealed_under(root, sealed_under) {
        Some(id) => {
            if let Err(e) = workspace::record_backup_in(root, &id, at) {
                log::warn!("could not record lockbox {id}'s backup: {e}");
            }
        }
        None => log::info!("the lockbox a backup was saved of is no longer on this device"),
    }
    Some(dest.to_string_lossy().into_owned())
}

/// The name the save dialog offers: the app's, the lockbox's and the day's, so
/// two lockboxes saved the same day are two files rather than one offered to
/// overwrite the other. A primary nobody has named has no name to give, and is
/// the one lockbox without one, so it still reads apart from the rest.
fn backup_file_name(lockbox: Option<&str>, date: &str) -> String {
    match lockbox.map(file_safe).filter(|name| !name.is_empty()) {
        Some(name) => format!("{APP_NAME} backup {name} {date}.{BACKUP_EXTENSION}"),
        None => format!("{APP_NAME} backup {date}.{BACKUP_EXTENSION}"),
    }
}

// The most of a lockbox's name a file name carries: well inside the 255 bytes a
// name may take on every platform, at four bytes a character.
const FILE_NAME_CHARS: usize = 48;

// A name as it can stand inside a file name anywhere the app runs: separators,
// the characters Windows reserves and control characters become spaces, a run
// of spaces one, and a long name is cut short.
fn file_safe(name: &str) -> String {
    let spaced: String = name
        .chars()
        .map(|c| {
            if c.is_control() || r#"/\:*?"<>|"#.contains(c) {
                ' '
            } else {
                c
            }
        })
        .collect();
    let words = spaced.split_whitespace().collect::<Vec<_>>().join(" ");
    let cut: String = words.chars().take(FILE_NAME_CHARS).collect();
    cut.trim_end().to_string()
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
    use crate::crypto::{KdfParams, VaultKey};
    use crate::models::{Passkey, PasswordHistoryItem};
    use crate::store::SqliteStore;
    use crate::workspace::{dir_of, statuses, PRIMARY_ID};

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

    // A `.swftx` as the legacy app wrote it: the whole vault, obscured entry by
    // entry and then sealed as one blob, both under `hash_secret(password)`.
    fn swftx(dir: &tempfile::TempDir, password: &str, entries: Vec<Entry>) -> String {
        let cryptor = crypto::Cryptor::new(&crypto::hash_secret(password));
        let obscured = entries
            .iter()
            .map(|e| cryptor.obscure(e).unwrap())
            .collect();
        let blob = cryptor
            .encrypt_data(&VaultData { entries: obscured })
            .unwrap();
        let path = dir.path().join("old-vault.swftx");
        std::fs::write(&path, blob).unwrap();
        path.to_string_lossy().into_owned()
    }

    // The file on disk, opened as the command opens it.
    fn opened(path: &str) -> std::fs::File {
        picked::open_local(Path::new(path)).unwrap().file
    }

    #[test]
    fn a_legacy_backup_opens_under_its_own_password_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = swftx(&dir, "old-password", vec![login(vec![])]);

        assert!(matches!(
            open_legacy_backup(opened(&path), "not-it"),
            Err(Error::InvalidPassword)
        ));

        let (src, data) = open_legacy_backup(opened(&path), "old-password").unwrap();
        assert_eq!(data.entries.len(), 1);
        // What came out is still obscured: the entries are exposed one at a
        // time by the re-seal, under the cryptor handed back with them.
        let plain = src.expose(&data.entries[0]).unwrap();
        assert_eq!(plain.id, "l1");
        assert_eq!(plain.title, "Site");
    }

    fn save(store: &SqliteStore, cipher: &PayloadCipher, entry: &Entry) {
        let payload = cipher.seal(entry).unwrap();
        store
            .upsert(&migrate::build_record(entry, payload).unwrap())
            .unwrap();
    }

    fn unsealed(store: &SqliteStore, cipher: &PayloadCipher) -> Entry {
        let record = store.get("l1").unwrap().unwrap();
        cipher.unseal(&record.id, &record.payload).unwrap()
    }

    const THEN: &str = "2026-01-01T00:00:00.000Z";
    const NOW: &str = "2026-09-26T12:00:00.000Z";

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
        merge_stored(&mut incoming, &store, &cipher, NOW).unwrap();
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
            merge_stored(&mut fresh, &store, &cipher, NOW),
            Err(Error::NotFound)
        ));
    }

    // No passkeys, nothing to put back.
    #[test]
    fn a_save_without_passkeys_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let (store, cipher) = open(&dir);

        let mut entry = login(vec![]);
        merge_stored(&mut entry, &store, &cipher, NOW).unwrap();
        assert!(entry.passkeys.unwrap().is_empty());
    }

    // The webview's round trip for a rotation: the revealed entry comes back
    // with a new password and a history of its own making. What lands is the
    // stored history with the old password in front — the planted one is gone.
    // Clearing then empties it on the row, and the next ordinary save keeps
    // it empty.
    #[test]
    fn a_save_records_the_replaced_password_and_a_clear_forgets_it() {
        let dir = tempfile::tempdir().unwrap();
        let (store, cipher) = open(&dir);
        save(
            &store,
            &cipher,
            &Entry {
                password: Some("old".into()),
                ..login(vec![])
            },
        );

        let mut incoming = Entry {
            password: Some("new".into()),
            password_history: Some(vec![PasswordHistoryItem {
                password: "planted".into(),
                replaced_at: THEN.into(),
            }]),
            ..login(vec![])
        };
        merge_stored(&mut incoming, &store, &cipher, NOW).unwrap();
        save(&store, &cipher, &incoming);

        let stored = unsealed(&store, &cipher);
        let history = stored.password_history.unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].password, "old");
        assert_eq!(history[0].replaced_at, NOW);
        assert_eq!(stored.password_updated_at.as_deref(), Some(NOW));

        clear_history_in(&store, &cipher, "l1", NOW).unwrap();
        let cleared = unsealed(&store, &cipher);
        assert_eq!(cleared.password_history, None);
        assert_eq!(cleared.password.as_deref(), Some("new"));
        assert_eq!(cleared.updated_at.as_deref(), Some(NOW));

        // A reveal hands the cleared row out; saving it unchanged adds nothing.
        let mut edit = cleared.redacted();
        merge_stored(&mut edit, &store, &cipher, NOW).unwrap();
        assert_eq!(edit.password_history, None);
    }

    // Clearing keeps everything else on the row, passkey keys included — the
    // row never left the core.
    #[test]
    fn clearing_the_history_keeps_the_passkey_keys() {
        let dir = tempfile::tempdir().unwrap();
        let (store, cipher) = open(&dir);
        save(&store, &cipher, &login(vec![passkey("c1", "k1")]));

        clear_history_in(&store, &cipher, "l1", NOW).unwrap();
        assert_eq!(
            unsealed(&store, &cipher).passkeys.unwrap()[0].private_key,
            "k1"
        );
        assert!(matches!(
            clear_history_in(&store, &cipher, "missing", NOW),
            Err(Error::NotFound)
        ));
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
            merge_stored(&mut forged, &store, &cipher, NOW),
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
            merge_stored(&mut incoming, &store, &cipher, NOW),
            Err(Error::NotFound)
        ));
        assert!(incoming
            .passkeys
            .unwrap()
            .iter()
            .all(|p| p.private_key.is_empty()));
    }

    fn ssh_entry(pair: &crate::models::SshKeyPair) -> Entry {
        Entry {
            id: "l1".into(),
            kind: "ssh".into(),
            title: "Server".into(),
            private_key: Some(pair.private_key.clone()),
            public_key: Some(pair.public_key.clone()),
            fingerprint: Some(pair.fingerprint.clone()),
            ..Entry::default()
        }
    }

    // The edit the editor makes when one key is pasted over another: the
    // private key changes and the old public line and fingerprint ride back
    // as they were revealed. What lands has to describe the new key, or the
    // line copied to a server is one the stored key cannot sign for.
    #[test]
    fn pasting_a_new_ssh_key_over_an_old_one_replaces_its_public_half() {
        use crate::commands::generator::generate_ssh_key;
        let dir = tempfile::tempdir().unwrap();
        let (store, cipher) = open(&dir);
        let old = generate_ssh_key(Some("old".into())).unwrap();
        let new = generate_ssh_key(Some("new".into())).unwrap();
        save(&store, &cipher, &ssh_entry(&old));

        let mut edit = unsealed(&store, &cipher).redacted();
        edit.private_key = Some(new.private_key.clone());
        merge_stored(&mut edit, &store, &cipher, NOW).unwrap();
        save(&store, &cipher, &edit);

        let stored = unsealed(&store, &cipher);
        assert_eq!(stored.public_key.as_deref(), Some(new.public_key.as_str()));
        assert_eq!(
            stored.fingerprint.as_deref(),
            Some(new.fingerprint.as_str())
        );
    }

    // A replacement that cannot be read, here a paste that lost its middle,
    // cannot be checked against anything, so the old key's line and
    // fingerprint go rather than stay beside it as if they were its own.
    #[test]
    fn an_unreadable_replacement_ssh_key_keeps_nothing_of_the_old_one() {
        use crate::commands::generator::generate_ssh_key;
        let dir = tempfile::tempdir().unwrap();
        let (store, cipher) = open(&dir);
        let old = generate_ssh_key(None).unwrap();
        save(&store, &cipher, &ssh_entry(&old));

        let new = generate_ssh_key(None).unwrap();
        let lines: Vec<&str> = new.private_key.lines().collect();
        let truncated = format!("{}\n{}\n{}\n", lines[0], lines[1], lines[lines.len() - 1]);

        let mut edit = unsealed(&store, &cipher).redacted();
        edit.private_key = Some(truncated);
        merge_stored(&mut edit, &store, &cipher, NOW).unwrap();
        save(&store, &cipher, &edit);

        let stored = unsealed(&store, &cipher);
        assert_eq!(stored.public_key, None);
        assert_eq!(stored.fingerprint, None);
    }

    // --- the backup ---------------------------------------------------------

    const VAULT_PASSWORD: &str = "vault password";

    // A lockbox in `dir` under `password`, closed again once it holds one
    // login per title, the first of them deleted: what sits beside the open
    // lockbox on disk.
    fn closed_lockbox(dir: &Path, password: &str, titles: &[&str]) -> VaultKey {
        std::fs::create_dir_all(dir).unwrap();
        let params = KdfParams::argon2id(b"salt-0123456789012345", 256, 1, 1);
        storage::atomic_write_file(
            &dir.join(storage::KDF_SIDECAR_FILE),
            &params.to_json().unwrap(),
        )
        .unwrap();
        let key = VaultKey::Argon2 {
            master: crypto::derive(password.as_bytes(), &params).unwrap(),
        };
        let store = SqliteStore::open(&dir.join(storage::DB_FILE), &*key.sqlcipher_key()).unwrap();
        for (i, title) in titles.iter().enumerate() {
            let entry = Entry {
                id: format!("e{i}"),
                kind: "login".into(),
                title: (*title).into(),
                ..Entry::default()
            };
            save(&store, &key.payload_cipher(), &entry);
        }
        store.delete("e0").unwrap();
        key
    }

    // The lockbox beside the open one, saved with the key the app holds for
    // it, from its own directory and descriptor: the file opens with the vault
    // password, holds that lockbox's entries, and keeps the deletion a sync
    // pack would in time have let go.
    #[test]
    fn a_lockbox_that_is_not_open_is_saved_and_restores_with_the_vault_password() {
        let root = tempfile::tempdir().unwrap();
        let dir = dir_of(root.path(), "a1b2");
        let key = closed_lockbox(&dir, VAULT_PASSWORD, &["Gone", "Bank", "Mail"]);

        let bytes = pack_lockbox(&dir, "a1b2", key).unwrap();

        let fresh = tempfile::tempdir().unwrap();
        let db = fresh.path().join(storage::DB_FILE);
        let kdf = fresh.path().join(storage::KDF_SIDECAR_FILE);
        assert!(matches!(
            sync::restore::restore_at(&db, &kdf, &bytes, "not it"),
            Err(Error::InvalidPassword)
        ));
        let (_, store) = sync::restore::restore_at(&db, &kdf, &bytes, VAULT_PASSWORD).unwrap();
        let mut titles: Vec<String> = list_metas(&store)
            .unwrap()
            .into_iter()
            .map(|m| m.title)
            .collect();
        titles.sort();
        assert_eq!(titles, ["Bank", "Mail"]);
        let deleted = list_deleted_metas(&store).unwrap();
        assert_eq!(deleted.len(), 1);
        assert_eq!(deleted[0].title, "Gone");
    }

    // A descriptor of its own for lockbox `id`, as every lockbox has: `salt`
    // is what tells one from another. Returned as its sidecar holds it.
    fn sealed(root: &Path, id: &str, salt: &str) -> String {
        let dir = dir_of(root, id);
        std::fs::create_dir_all(&dir).unwrap();
        let descriptor = KdfParams::argon2id(format!("{salt:-<21}").as_bytes(), 256, 1, 1)
            .to_json()
            .unwrap();
        storage::atomic_write_file(&dir.join(storage::KDF_SIDECAR_FILE), &descriptor).unwrap();
        descriptor
    }

    // The primary, open; a lockbox beside it; and an org lockbox, told by its
    // descriptor. Read as JSON rather than built, so a field added to the
    // registry record does not have to be added here.
    fn lockboxes(root: &Path) -> Registry {
        sealed(root, PRIMARY_ID, "primary");
        closed_lockbox(&dir_of(root, "a1b2"), VAULT_PASSWORD, &["x"]);
        let org = dir_of(root, "0r61");
        std::fs::create_dir_all(&org).unwrap();
        storage::atomic_write_file(
            &org.join(storage::KDF_SIDECAR_FILE),
            &KdfParams::external("kid", 1).to_json().unwrap(),
        )
        .unwrap();
        serde_json::from_str(
            r#"{"active":"default","workspaces":[
                {"id":"default","name":null},
                {"id":"a1b2","name":"Work"},
                {"id":"0r61","name":"Deploy","server":"https://rowel.acme.example"}
            ]}"#,
        )
        .unwrap()
    }

    // Named or not, the open lockbox is the session's to pack: the ring is not
    // asked about it.
    #[test]
    fn the_open_lockbox_is_saved_when_none_is_named() {
        let root = tempfile::tempdir().unwrap();
        let registry = lockboxes(root.path());
        let unasked = |_: &str| -> bool { panic!("the open lockbox's key is the session's") };

        for requested in [None, Some(PRIMARY_ID)] {
            let target =
                backup_target(root.path(), &registry, PRIMARY_ID, requested, unasked).unwrap();
            assert!(target.open);
            assert_eq!(target.id, PRIMARY_ID);
            assert_eq!(target.name, None);
        }
    }

    #[test]
    fn a_lockbox_beside_the_open_one_is_saved_from_its_own_directory() {
        let root = tempfile::tempdir().unwrap();
        let registry = lockboxes(root.path());

        let target = backup_target(root.path(), &registry, PRIMARY_ID, Some("a1b2"), |id| {
            id == "a1b2"
        })
        .unwrap();
        assert!(!target.open);
        assert_eq!(target.dir, dir_of(root.path(), "a1b2"));
        assert_eq!(target.name.as_deref(), Some("Work"));
    }

    // A password change that crashed on a lockbox nobody had open left its
    // database and descriptor on the way to a new key. The save rolls that back
    // before it reads either, as a switch to the lockbox would, so the file it
    // writes is the lockbox as the held key opens it, entries and all.
    #[test]
    fn an_interrupted_password_change_is_rolled_back_before_a_lockbox_is_saved() {
        let root = tempfile::tempdir().unwrap();
        let registry = lockboxes(root.path());
        let dir = dir_of(root.path(), "a1b2");
        let key = closed_lockbox(&dir, VAULT_PASSWORD, &["Gone", "Bank"]);
        let sealed_under = std::fs::read_to_string(dir.join(storage::KDF_SIDECAR_FILE)).unwrap();

        // The crash: the pre-change pair snapshotted, the live pair half moved.
        std::fs::copy(
            dir.join(storage::DB_FILE),
            dir.join(storage::DB_REKEY_BACKUP_FILE),
        )
        .unwrap();
        std::fs::copy(
            dir.join(storage::KDF_SIDECAR_FILE),
            dir.join(storage::KDF_SIDECAR_REKEY_BACKUP_FILE),
        )
        .unwrap();
        std::fs::write(dir.join(storage::DB_FILE), "half re-keyed").unwrap();
        std::fs::write(dir.join(storage::KDF_SIDECAR_FILE), "new-params").unwrap();

        let target =
            backup_target(root.path(), &registry, PRIMARY_ID, Some("a1b2"), |_| true).unwrap();
        assert_eq!(target.sealed_under, sealed_under);
        assert!(!dir.join(storage::DB_REKEY_BACKUP_FILE).exists());

        let bytes = pack_lockbox(&target.dir, &target.id, key).unwrap();
        let fresh = tempfile::tempdir().unwrap();
        let (_, store) = sync::restore::restore_at(
            &fresh.path().join(storage::DB_FILE),
            &fresh.path().join(storage::KDF_SIDECAR_FILE),
            &bytes,
            VAULT_PASSWORD,
        )
        .unwrap();
        let titles: Vec<String> = list_metas(&store)
            .unwrap()
            .into_iter()
            .map(|m| m.title)
            .collect();
        assert_eq!(titles, ["Bank"]);
    }

    // What this device cannot save is refused by what it is: an id nobody
    // made, a lockbox the app holds no key for, a lockbox with no descriptor
    // to put in the pack, and an org lockbox — open or not, held or not.
    #[test]
    fn a_lockbox_this_device_cannot_save_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let registry = lockboxes(root.path());
        let target = |active: &str, requested: Option<&str>, held: bool| {
            backup_target(root.path(), &registry, active, requested, |_| held)
        };

        assert!(matches!(
            target(PRIMARY_ID, Some("gone"), true),
            Err(Error::NotFound)
        ));
        assert!(matches!(
            target(PRIMARY_ID, Some("a1b2"), false),
            Err(Error::Unsupported(_))
        ));
        assert!(matches!(
            target(PRIMARY_ID, Some("0r61"), true),
            Err(Error::Unsupported(_))
        ));
        assert!(matches!(
            target("0r61", None, true),
            Err(Error::Unsupported(_))
        ));

        std::fs::remove_file(dir_of(root.path(), "a1b2").join(storage::KDF_SIDECAR_FILE)).unwrap();
        assert!(matches!(
            target(PRIMARY_ID, Some("a1b2"), true),
            Err(Error::Other(_))
        ));
    }

    // Only a save that happened is noted: a dismissed dialog leaves the
    // lockbox never saved, a chosen destination stamps it, and the stamp is
    // what the lockbox list says. An install with one lockbox keeps its stamp
    // without being given a registry file for it.
    #[test]
    fn a_backup_is_recorded_once_saved_and_not_when_the_dialog_is_dismissed() {
        let root = tempfile::tempdir().unwrap();
        let registry = Registry::default();
        let primary = sealed(root.path(), PRIMARY_ID, "primary");

        assert_eq!(record_saved(root.path(), &primary, None, NOW), None);
        assert_eq!(statuses(&registry, root.path())[0].backed_up_at, None);

        let dest = PathBuf::from("/chosen/Rowel backup 2026-09-26.rowel");
        assert_eq!(
            record_saved(root.path(), &primary, Some(dest.clone()), NOW),
            Some(dest.to_string_lossy().into_owned())
        );
        let listed = statuses(&registry, root.path());
        assert_eq!(listed[0].backed_up_at.as_deref(), Some(NOW));
        assert_eq!(serde_json::to_value(&listed[0]).unwrap()["backedUpAt"], NOW);
        assert!(!root
            .path()
            .join(rowel_core::workspace::REGISTRY_FILE)
            .exists());
    }

    // Per lockbox, and only for one that is still here: a lockbox deleted
    // while its save dialog was open is not given a directory back.
    #[test]
    fn a_backup_is_recorded_against_its_own_lockbox_only() {
        let root = tempfile::tempdir().unwrap();
        let registry = lockboxes(root.path());
        registry.save(root.path()).unwrap();
        let work = sealed(root.path(), "a1b2", "work");
        let dest = || Some(PathBuf::from("/chosen/backup.rowel"));

        record_saved(root.path(), &work, dest(), NOW);
        let listed = statuses(&registry, root.path());
        assert_eq!(listed[0].backed_up_at, None);
        assert_eq!(listed[1].backed_up_at.as_deref(), Some(NOW));
        assert!(serde_json::to_value(&listed[0])
            .unwrap()
            .get("backedUpAt")
            .is_none());

        let gone = KdfParams::argon2id(b"gone-0123456789012345", 256, 1, 1)
            .to_json()
            .unwrap();
        assert!(record_saved(root.path(), &gone, dest(), NOW).is_some());
        let listed = statuses(&registry, root.path());
        assert_eq!(listed[0].backed_up_at, None);
        assert_eq!(listed[1].backed_up_at.as_deref(), Some(NOW));
    }

    // The registry's id is a slot. While the save dialog was open the primary
    // was deleted and the lockbox beside it promoted into `default`: a save of
    // the deleted primary must not mark the survivor as backed up, and a save
    // of the survivor is noted where it lives now.
    #[test]
    fn a_backup_follows_its_lockbox_when_the_primary_is_deleted_under_the_dialog() {
        let root = tempfile::tempdir().unwrap();
        let primary = sealed(root.path(), PRIMARY_ID, "primary");
        let work = sealed(root.path(), "a1b2", "work");
        let dest = || Some(PathBuf::from("/chosen/backup.rowel"));

        // The promotion, as far as a save can see it: the survivor's files are
        // the primary's now, and its own slot is gone.
        std::fs::copy(
            dir_of(root.path(), "a1b2").join(storage::KDF_SIDECAR_FILE),
            dir_of(root.path(), PRIMARY_ID).join(storage::KDF_SIDECAR_FILE),
        )
        .unwrap();
        std::fs::remove_dir_all(dir_of(root.path(), "a1b2")).unwrap();
        let registry = Registry::default();

        assert!(record_saved(root.path(), &primary, dest(), NOW).is_some());
        assert_eq!(statuses(&registry, root.path())[0].backed_up_at, None);

        assert!(record_saved(root.path(), &work, dest(), NOW).is_some());
        assert_eq!(
            statuses(&registry, root.path())[0].backed_up_at.as_deref(),
            Some(NOW)
        );
    }

    // The name the save dialog offers says which lockbox is in it, and can be
    // saved under on any platform whatever the lockbox is called.
    #[test]
    fn the_offered_file_name_tells_lockboxes_apart_and_is_safe_to_save() {
        let name = |lockbox| backup_file_name(lockbox, "2026-11-23");

        assert_eq!(
            name(Some("Work")),
            format!("{APP_NAME} backup Work 2026-11-23.rowel")
        );
        assert_eq!(name(None), format!("{APP_NAME} backup 2026-11-23.rowel"));
        assert_eq!(
            name(Some("Clients: A/B\\C \"new\"\t*?<>|")),
            format!("{APP_NAME} backup Clients A B C new 2026-11-23.rowel")
        );
        assert_eq!(
            name(Some("Семья")),
            format!("{APP_NAME} backup Семья 2026-11-23.rowel")
        );
        // Nothing left to name it by is no name.
        assert_eq!(name(Some(" / ")), name(None));
        let long = "x".repeat(300);
        assert_eq!(
            name(Some(&long)),
            format!(
                "{APP_NAME} backup {} 2026-11-23.rowel",
                "x".repeat(FILE_NAME_CHARS)
            )
        );
    }
}
