//! Files attached to entries. A file never travels inside an entry: the entry
//! is sealed and revealed whole, so its bytes would reach the webview on every
//! reveal. Each file is a row of its own beside the entry (`store::Attachment`),
//! sealed by `crypto::AttachmentCipher`, read off disk and written back to disk
//! here in Rust. The webview gets names and sizes, never the bytes.

use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{AppHandle, State};
use zeroize::Zeroizing;

use crate::crypto::AttachmentCipher;
use crate::error::{Error, Result};
use crate::grants::{PathGrants, Purpose};
use crate::save;
use crate::session::{store_err, Epoch, Session};
use crate::state::AppState;
use crate::storage::read_regular_file_capped;
use crate::store::{migrate, Attachment, AttachmentMeta, SqliteStore, VaultStore};

/// The largest file one attachment may be: a small share of the vault's
/// budget below.
pub const MAX_ATTACHMENT_BYTES: u64 = 10 * 1024 * 1024;

/// The most one vault's files may add up to, archived ones included (see
/// `VaultStore::attachment_bytes`). Every byte rides the sync pack, which a
/// pull refuses past `sync::pack::MAX_PACK_BYTES` (256 MiB), and so does a
/// backup restore. The pack is the whole SQLCipher snapshot — entries, indexes
/// and WAL besides the files — so the files get half of it and the rest is
/// headroom.
pub const MAX_VAULT_ATTACHMENT_BYTES: u64 = 128 * 1024 * 1024;

/// How much of the vault's attachment budget its files take, in bytes.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct AttachmentUsage {
    pub used: u64,
    pub limit: u64,
}

/// One attachment as the entry's list shows it: never the file itself.
#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentDto {
    pub id: String,
    pub entry_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
    pub size: i64,
    pub created_at: Option<String>,
}

impl From<AttachmentMeta> for AttachmentDto {
    fn from(m: AttachmentMeta) -> Self {
        Self {
            id: m.id,
            entry_id: m.entry_id,
            name: m.name,
            mime: m.mime,
            size: m.size,
            created_at: chrono::DateTime::from_timestamp_millis(m.created_at)
                .map(|d| d.to_rfc3339()),
        }
    }
}

/// Attach the file at `path` to the entry `entry_id`. The path has to be one
/// the user chose for this: picked through the attachment `pick_file`, or
/// dropped on the window (see `grants`). The read runs off the IPC thread and
/// is refused past [`MAX_ATTACHMENT_BYTES`], or past what is left of
/// [`MAX_VAULT_ATTACHMENT_BYTES`]; a lock or a workspace switch while
/// it runs discards what was read rather than writing it into whatever vault
/// is open by then.
#[tauri::command]
pub async fn attachment_add(
    entry_id: String,
    path: String,
    state: State<'_, AppState>,
    grants: State<'_, PathGrants>,
) -> Result<AttachmentDto> {
    let path = PathBuf::from(path);
    let (epoch, cipher, room) = begin_add(&state, &grants, &entry_id, &path)?;
    let attachment = {
        let entry_id = entry_id.clone();
        super::blocking(move || seal_file(&cipher, &entry_id, &path, room)).await?
    };
    finish_add(&state, epoch, &attachment)
}

/// The live attachments of one entry, oldest first.
#[tauri::command]
pub fn attachment_list(entry_id: String, state: State<'_, AppState>) -> Result<Vec<AttachmentDto>> {
    let session = state.session.lock().unwrap();
    Ok(session
        .store()?
        .list_attachments(&entry_id)
        .map_err(store_err)?
        .into_iter()
        .map(AttachmentDto::from)
        .collect())
}

/// Write an attachment back to disk where the user says, under its own name.
/// Unsealed here and handed straight to the save dialog's writer — owner-only
/// on desktop, staged in the app's sandbox for the iOS exporter — so the file
/// goes from the vault to disk without passing through the webview. The path
/// it landed at, or `None` when the dialog was dismissed.
#[tauri::command]
pub async fn attachment_save(
    id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<String>> {
    let (epoch, cipher, attachment) = begin_save(&state, &id)?;
    let name = file_name(&attachment.meta.name);
    let mut bytes =
        super::blocking(move || cipher.unseal(&attachment.meta.id, &attachment.blob)).await?;
    // Locked while the file was unsealed: the session that asked is gone, and
    // the bytes are scrubbed as they drop.
    super::same_session(&state, epoch)?;
    let dest = save::save_export(&app, &name, "", std::mem::take(&mut *bytes)).await?;
    Ok(dest.map(|p| p.to_string_lossy().into_owned()))
}

/// How full the vault's attachment budget is, for the entry's list to say so
/// once it is nearly spent.
#[tauri::command]
pub fn attachment_usage(state: State<'_, AppState>) -> Result<AttachmentUsage> {
    usage(&state)
}

/// Remove an attachment for good (see `VaultStore::remove_attachment`).
#[tauri::command]
pub fn attachment_delete(id: String, state: State<'_, AppState>) -> Result<()> {
    let session = state.session.lock().unwrap();
    session.store()?.remove_attachment(&id).map_err(store_err)
}

// --- the steps, split out so the guards can be tested without Tauri ----------

/// Everything `attachment_add` checks before it reads: an open vault that can
/// hold attachments, a live entry of a kind that takes them, and a path the
/// user chose. Hands back the session and the cipher the read is for. The
/// grant is spent last, so a refusal above leaves the user's choice standing.
/// Hands back, too, the room left in the vault's budget.
fn begin_add(
    state: &AppState,
    grants: &PathGrants,
    entry_id: &str,
    path: &Path,
) -> Result<(Epoch, AttachmentCipher, u64)> {
    let session = state.session.lock().unwrap();
    let cipher = cipher_of(&session)?;
    let store = session.store()?;
    attachable(store, entry_id)?;
    let used = store.attachment_bytes().map_err(store_err)?;
    if !grants.take(path, Purpose::Attachment) {
        return Err(Error::Unsupported(
            "this file was not chosen in the app".into(),
        ));
    }
    Ok((
        session.epoch(),
        cipher,
        MAX_VAULT_ATTACHMENT_BYTES.saturating_sub(used),
    ))
}

/// Read the file at `path` whole, refusing it past the cap or past the `room`
/// left in the vault's budget, and seal it under a fresh id. The plaintext is
/// scrubbed on the way out.
fn seal_file(
    cipher: &AttachmentCipher,
    entry_id: &str,
    path: &Path,
    room: u64,
) -> Result<Attachment> {
    let bytes = Zeroizing::new(read_regular_file_capped(path, MAX_ATTACHMENT_BYTES)?);
    // Checked here, where the size is known and nothing is sealed yet. Two adds
    // racing can each fit on their own and overshoot together, by one file at
    // most — which the headroom under the pack cap absorbs.
    if bytes.len() as u64 > room {
        return Err(Error::VaultFull);
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let id = migrate::new_entry_id();
    let blob = cipher.seal(&id, &bytes)?;
    Ok(Attachment {
        meta: AttachmentMeta {
            id,
            entry_id: entry_id.into(),
            mime: mime_of(&name),
            name,
            size: bytes.len() as i64,
            created_at: 0,
            updated_at: 0,
            deleted_at: None,
        },
        blob,
    })
}

/// Write the sealed file into the session the read was for, and only that
/// one. The entry is asked about again: it may have been archived meanwhile.
fn finish_add(state: &AppState, epoch: Epoch, attachment: &Attachment) -> Result<AttachmentDto> {
    let session = state.session.lock().unwrap();
    if !session.is_unlocked() || session.epoch() != epoch {
        return Err(Error::Locked);
    }
    let store = session.store()?;
    let entry_id = &attachment.meta.entry_id;
    attachable(store, entry_id)?;
    store.insert_attachment(attachment).map_err(store_err)?;
    store
        .list_attachments(entry_id)
        .map_err(store_err)?
        .into_iter()
        .find(|m| m.id == attachment.meta.id)
        .map(AttachmentDto::from)
        .ok_or(Error::NotFound)
}

fn usage(state: &AppState) -> Result<AttachmentUsage> {
    let session = state.session.lock().unwrap();
    Ok(AttachmentUsage {
        used: session.store()?.attachment_bytes().map_err(store_err)?,
        limit: MAX_VAULT_ATTACHMENT_BYTES,
    })
}

fn begin_save(state: &AppState, id: &str) -> Result<(Epoch, AttachmentCipher, Attachment)> {
    let session = state.session.lock().unwrap();
    let cipher = cipher_of(&session)?;
    let attachment = session
        .store()?
        .get_attachment(id)
        .map_err(store_err)?
        .ok_or(Error::NotFound)?;
    Ok((session.epoch(), cipher, attachment))
}

/// The open vault's attachment cipher. `Locked` when there is no vault, and a
/// refusal for one still on the legacy key, which has no scheme for files.
fn cipher_of(session: &Session) -> Result<AttachmentCipher> {
    session.key()?.attachment_cipher().ok_or_else(|| {
        Error::Unsupported(
            "this vault uses an older key scheme that cannot hold attachments".into(),
        )
    })
}

/// A live entry that takes attachments: every kind but `env`, whose file is
/// the entry itself.
fn attachable(store: &SqliteStore, entry_id: &str) -> Result<()> {
    let entry = store
        .row_meta(entry_id)
        .map_err(store_err)?
        .filter(|m| m.deleted_at.is_none())
        .ok_or(Error::NotFound)?;
    if entry.kind == "env" {
        return Err(Error::Unsupported(
            "an env file entry cannot hold attachments".into(),
        ));
    }
    Ok(())
}

/// The name a save offers: the last component of the stored one, so a name
/// that arrived through sync cannot point the mobile staging anywhere else.
fn file_name(name: &str) -> String {
    Path::new(name)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "attachment".into())
}

/// A best guess at the type from the name, for the list's glyph. `None` is
/// simply "a file".
fn mime_of(name: &str) -> Option<String> {
    let ext = Path::new(name).extension()?.to_str()?.to_ascii_lowercase();
    let mime = match ext.as_str() {
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "heic" => "image/heic",
        "txt" | "md" => "text/plain",
        "csv" => "text/csv",
        "json" => "application/json",
        "zip" => "application/zip",
        _ => return None,
    };
    Some(mime.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::VaultKey;
    use crate::models::Entry;
    use std::fs;

    fn argon2_key() -> VaultKey {
        VaultKey::Argon2 {
            master: Zeroizing::new(vec![3u8; 32]),
        }
    }

    // A vault on disk holding one entry of `kind`, and its path for reopening.
    fn vault(dir: &tempfile::TempDir, key: &VaultKey, kind: &str) -> (SqliteStore, PathBuf) {
        let path = dir.path().join("vault.db");
        let store = SqliteStore::open(&path, key.sqlcipher_key().as_slice()).unwrap();
        let entry = Entry {
            id: "e1".into(),
            kind: kind.into(),
            title: "Site".into(),
            ..Entry::default()
        };
        store
            .upsert(&migrate::build_record(&entry, b"sealed".to_vec()).unwrap())
            .unwrap();
        (store, path)
    }

    fn open_with(key: VaultKey, store: SqliteStore) -> AppState {
        let state = AppState::default();
        state.session.lock().unwrap().set(key, store, false);
        state
    }

    fn file(dir: &tempfile::TempDir, name: &str, bytes: &[u8]) -> PathBuf {
        let path = dir.path().join(name);
        fs::write(&path, bytes).unwrap();
        path
    }

    fn granted(path: &Path) -> PathGrants {
        let grants = PathGrants::default();
        grants.grant(path, Purpose::Attachment);
        grants
    }

    #[test]
    fn a_picked_file_is_sealed_listed_and_unsealed_back() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = vault(&dir, &argon2_key(), "login");
        let state = open_with(argon2_key(), store);
        let path = file(&dir, "scan.pdf", b"%PDF-1.7 the file");
        let grants = granted(&path);

        let (epoch, cipher, room) = begin_add(&state, &grants, "e1", &path).unwrap();
        let attachment = seal_file(&cipher, "e1", &path, room).unwrap();
        assert!(!attachment.blob.windows(8).any(|w| w == b"the file"));
        let dto = finish_add(&state, epoch, &attachment).unwrap();
        assert_eq!(
            (dto.name.as_str(), dto.size, dto.mime.as_deref()),
            ("scan.pdf", 17, Some("application/pdf"))
        );

        let (_, cipher, stored) = begin_save(&state, &dto.id).unwrap();
        assert_eq!(
            &**cipher.unseal(&stored.meta.id, &stored.blob).unwrap(),
            b"%PDF-1.7 the file"
        );
    }

    #[test]
    fn a_file_past_the_cap_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let cipher = argon2_key().attachment_cipher().unwrap();
        let room = MAX_VAULT_ATTACHMENT_BYTES;
        let at_cap = file(
            &dir,
            "at-cap.bin",
            &vec![0u8; MAX_ATTACHMENT_BYTES as usize],
        );
        assert!(seal_file(&cipher, "e1", &at_cap, room).is_ok());

        let over = file(
            &dir,
            "over.bin",
            &vec![0u8; MAX_ATTACHMENT_BYTES as usize + 1],
        );
        assert!(matches!(
            seal_file(&cipher, "e1", &over, room),
            Err(Error::FileTooLarge)
        ));
    }

    // The budget is the vault's, not the file's: a file well under the per-file
    // cap is refused once it would take the vault's files past theirs.
    #[test]
    fn a_file_past_the_vault_budget_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = vault(&dir, &argon2_key(), "login");
        // The budget counts sizes, so the row need not hold the bytes.
        store
            .import_attachments(&[Attachment {
                meta: AttachmentMeta {
                    id: "big".into(),
                    entry_id: "e1".into(),
                    name: "big.bin".into(),
                    mime: None,
                    size: MAX_VAULT_ATTACHMENT_BYTES as i64 - 4,
                    created_at: 1,
                    updated_at: 1,
                    deleted_at: None,
                },
                blob: b"sealed".to_vec(),
            }])
            .unwrap();
        let state = open_with(argon2_key(), store);

        let fits = file(&dir, "fits.txt", b"1234");
        let (_, cipher, room) = begin_add(&state, &granted(&fits), "e1", &fits).unwrap();
        assert!(seal_file(&cipher, "e1", &fits, room).is_ok());

        let over = file(&dir, "over.txt", b"12345");
        let (_, cipher, room) = begin_add(&state, &granted(&over), "e1", &over).unwrap();
        assert!(matches!(
            seal_file(&cipher, "e1", &over, room),
            Err(Error::VaultFull)
        ));
        assert_eq!(
            usage(&state).unwrap(),
            AttachmentUsage {
                used: MAX_VAULT_ATTACHMENT_BYTES - 4,
                limit: MAX_VAULT_ATTACHMENT_BYTES
            }
        );
    }

    #[test]
    fn a_locked_vault_reads_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = file(&dir, "a.txt", b"x");
        let grants = granted(&path);

        let state = AppState::default();
        assert!(matches!(
            begin_add(&state, &grants, "e1", &path),
            Err(Error::Locked)
        ));
        assert!(matches!(begin_save(&state, "a1"), Err(Error::Locked)));
        // The refusal spent nothing: the user's choice is still there.
        assert!(grants.take(&path, Purpose::Attachment));
    }

    #[test]
    fn a_path_nobody_chose_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = vault(&dir, &argon2_key(), "login");
        let state = open_with(argon2_key(), store);
        let path = file(&dir, "a.txt", b"x");

        let picked_as_env = PathGrants::default();
        picked_as_env.grant(&path, Purpose::Env);
        for grants in [PathGrants::default(), picked_as_env] {
            assert!(matches!(
                begin_add(&state, &grants, "e1", &path),
                Err(Error::Unsupported(_))
            ));
        }

        // A drop, whatever its kind, is the user handing the file over.
        let dropped = PathGrants::default();
        dropped.grant_drop(&path, Purpose::Env);
        assert!(begin_add(&state, &dropped, "e1", &path).is_ok());
    }

    #[test]
    fn a_legacy_vault_an_env_entry_and_a_missing_entry_take_no_attachments() {
        let dir = tempfile::tempdir().unwrap();
        let path = file(&dir, "a.txt", b"x");

        let legacy = VaultKey::legacy_from_password("pw");
        let (store, _) = vault(&dir, &legacy, "login");
        let state = open_with(legacy, store);
        assert!(matches!(
            begin_add(&state, &granted(&path), "e1", &path),
            Err(Error::Unsupported(_))
        ));

        let dir = tempfile::tempdir().unwrap();
        let (store, _) = vault(&dir, &argon2_key(), "env");
        let state = open_with(argon2_key(), store);
        assert!(matches!(
            begin_add(&state, &granted(&path), "e1", &path),
            Err(Error::Unsupported(_))
        ));
        assert!(matches!(
            begin_add(&state, &granted(&path), "nope", &path),
            Err(Error::NotFound)
        ));
    }

    // The vault locked — or locked and opened again — while the file was being
    // read: what was read belongs to a session that is gone, and is not
    // written into either.
    #[test]
    fn a_lock_mid_read_discards_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let (store, db) = vault(&dir, &argon2_key(), "login");
        let state = open_with(argon2_key(), store);
        let path = file(&dir, "a.txt", b"secret");
        let (epoch, cipher, room) = begin_add(&state, &granted(&path), "e1", &path).unwrap();
        let attachment = seal_file(&cipher, "e1", &path, room).unwrap();

        state.session.lock().unwrap().clear();
        assert!(matches!(
            finish_add(&state, epoch, &attachment),
            Err(Error::Locked)
        ));

        let reopened = SqliteStore::open(&db, argon2_key().sqlcipher_key().as_slice()).unwrap();
        state
            .session
            .lock()
            .unwrap()
            .set(argon2_key(), reopened, false);
        assert!(matches!(
            finish_add(&state, epoch, &attachment),
            Err(Error::Locked)
        ));
        let session = state.session.lock().unwrap();
        assert!(session
            .store()
            .unwrap()
            .export_attachments_for_sync()
            .unwrap()
            .is_empty());
    }

    // Archived while the file was read: nothing is attached to the tombstone.
    #[test]
    fn an_entry_archived_mid_read_is_not_attached_to() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = vault(&dir, &argon2_key(), "login");
        let state = open_with(argon2_key(), store);
        let path = file(&dir, "a.txt", b"secret");
        let (epoch, cipher, room) = begin_add(&state, &granted(&path), "e1", &path).unwrap();
        let attachment = seal_file(&cipher, "e1", &path, room).unwrap();

        state
            .session
            .lock()
            .unwrap()
            .store()
            .unwrap()
            .delete("e1")
            .unwrap();
        assert!(matches!(
            finish_add(&state, epoch, &attachment),
            Err(Error::NotFound)
        ));
    }

    #[test]
    fn a_removed_attachment_cannot_be_saved() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = vault(&dir, &argon2_key(), "login");
        let state = open_with(argon2_key(), store);
        let path = file(&dir, "a.txt", b"x");
        let (epoch, cipher, room) = begin_add(&state, &granted(&path), "e1", &path).unwrap();
        let dto = finish_add(
            &state,
            epoch,
            &seal_file(&cipher, "e1", &path, room).unwrap(),
        )
        .unwrap();

        state
            .session
            .lock()
            .unwrap()
            .store()
            .unwrap()
            .remove_attachment(&dto.id)
            .unwrap();
        assert!(matches!(begin_save(&state, &dto.id), Err(Error::NotFound)));
    }

    #[test]
    fn a_saved_name_is_only_ever_a_file_name() {
        assert_eq!(file_name("scan.pdf"), "scan.pdf");
        assert_eq!(file_name("../../etc/passwd"), "passwd");
        assert_eq!(file_name(""), "attachment");
        assert_eq!(file_name(".."), "attachment");
    }
}
