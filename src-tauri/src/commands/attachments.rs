//! Files attached to entries. A file never travels inside an entry: the entry
//! is sealed and revealed whole, so its bytes would reach the webview on every
//! reveal. Each file is a row of its own beside the entry (`store::Attachment`),
//! sealed by `crypto::AttachmentCipher`, read off disk and written back to disk
//! here in Rust. The webview lists names and sizes; it gets a file's bytes only
//! when it asks for that one file (`attachment_read`), to show it — a preview,
//! an image's thumbnail — and a save never passes through it at all. A picture
//! the webview's engine cannot decode is re-encoded here as a PNG it can
//! (`attachment_render`), so what an image shows does not depend on the OS.

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

/// The IPC headers `attachment_add_bytes` reads its arguments from: the body
/// is the file, so everything else rides beside it. The name is
/// percent-encoded by the webview, since a header carries ASCII and a file is
/// named in anything.
const ENTRY_ID_HEADER: &str = "x-entry-id";
const FILE_NAME_HEADER: &str = "x-file-name";

/// Attach a file the webview holds as bytes — the one the phone's system
/// picker handed its file input, from the camera, the photo library or Files —
/// to the entry named in the request's headers. The bytes are the request's
/// raw body, so a photo is not spelled out as an array of numbers.
///
/// No grant is involved, and none would mean anything: `grants` exists because
/// a *path* is the webview naming a file on disk it never read, whereas these
/// bytes are already the webview's own. Nothing on disk is opened for them.
/// Everything else is as for [`attachment_add`]: the per-file cap, the vault's
/// budget, and a lock or a workspace switch while the file is sealed discards
/// it rather than writing it into whatever vault is open by then.
#[tauri::command]
pub async fn attachment_add_bytes(
    state: State<'_, AppState>,
    request: tauri::ipc::Request<'_>,
) -> Result<AttachmentDto> {
    let entry_id = header(&request, ENTRY_ID_HEADER)?;
    let name = header(&request, FILE_NAME_HEADER)?;
    let tauri::ipc::InvokeBody::Raw(body) = request.body() else {
        return Err(Error::Unsupported(
            "the file has to be sent as bytes".into(),
        ));
    };
    // Sized before it is copied: a body past the cap is refused without a
    // second copy of it being made. The copy that is made is scrubbed on the
    // way out; the body Tauri holds is dropped with the request.
    if body.len() as u64 > MAX_ATTACHMENT_BYTES {
        return Err(Error::FileTooLarge);
    }
    let bytes = Zeroizing::new(body.clone());
    let (epoch, cipher, room) = begin_add_bytes(&state, &entry_id)?;
    let attachment =
        super::blocking(move || seal_bytes(&cipher, &entry_id, &name, &bytes, room)).await?;
    finish_add(&state, epoch, &attachment)
}

/// One of the request's headers, percent-decoded.
fn header(request: &tauri::ipc::Request<'_>, name: &str) -> Result<String> {
    let value = request
        .headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| Error::Unsupported(format!("the request carries no {name}")))?;
    Ok(percent_encoding::percent_decode_str(value)
        .decode_utf8_lossy()
        .into_owned())
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
    let (name, mut bytes) = unseal(&state, &id).await?;
    let dest = save::save_export(&app, &name, "", std::mem::take(&mut *bytes)).await?;
    Ok(dest.map(|p| p.to_string_lossy().into_owned()))
}

/// One attachment's bytes, for the webview to show: the preview, and an image
/// row's thumbnail. Raw over the IPC rather than JSON — a 10 MB image is not
/// something to base64 through `serde` — and only ever one file, on request,
/// which is what keeps the list itself free of them. The copy the webview holds
/// lives as long as the view that asked (see `useAttachmentUrl`).
#[tauri::command]
pub async fn attachment_read(
    id: String,
    state: State<'_, AppState>,
) -> Result<tauri::ipc::Response> {
    let (_, mut bytes) = unseal(&state, &id).await?;
    Ok(tauri::ipc::Response::new(std::mem::take(&mut *bytes)))
}

/// The attachment as a PNG, for a picture the webview could not decode itself:
/// WebKit on Apple platforms reads a TIFF or a BMP, Chromium on Windows and
/// WebKitGTK on Linux do not. Decoded by the `image` crate from the codecs it
/// is built with ([`render_png`]); a file it cannot read is `Unsupported`, and
/// the webview says there is no preview. Asked for only after the plain bytes
/// failed, so the common case costs nothing extra.
#[tauri::command]
pub async fn attachment_render(
    id: String,
    state: State<'_, AppState>,
) -> Result<tauri::ipc::Response> {
    let (_, bytes) = unseal(&state, &id).await?;
    let png = super::blocking(move || render_png(&bytes, None)).await?;
    Ok(tauri::ipc::Response::new(png))
}

/// A picture for the list's row: the image scaled to fit [`THUMBNAIL_SIDE`],
/// as a PNG. A list of images used to hand the webview every file whole just
/// to draw it at 40px, and hold them all — up to the vault's whole budget,
/// plus the decoded bitmaps; now a row costs kilobytes, the file itself is
/// read only when it is opened, and the rows ask one at a time as they come
/// on screen (`useThumbnail`) while [`render_png`] decodes one picture at a
/// time whoever asks. `Unsupported` for a picture the codecs here cannot read
/// (HEIC), which the row answers with the kind's glyph.
#[tauri::command]
pub async fn attachment_thumbnail(
    id: String,
    state: State<'_, AppState>,
) -> Result<tauri::ipc::Response> {
    let (_, bytes) = unseal(&state, &id).await?;
    let png = super::blocking(move || render_png(&bytes, Some(THUMBNAIL_SIDE))).await?;
    Ok(tauri::ipc::Response::new(png))
}

/// What the three commands that open a file share: find it in the open vault,
/// unseal it off the IPC thread, and refuse the result if the vault locked —
/// or locked and reopened — while that ran, since the session that asked is
/// gone and the bytes are scrubbed as they drop. Hands back the name a save
/// offers along with the plaintext.
async fn unseal(state: &AppState, id: &str) -> Result<(String, Zeroizing<Vec<u8>>)> {
    let (epoch, cipher, attachment) = begin_unseal(state, id)?;
    let name = file_name(&attachment.meta.name);
    let bytes =
        super::blocking(move || cipher.unseal(&attachment.meta.id, &attachment.blob)).await?;
    super::same_session(state, epoch)?;
    Ok((name, bytes))
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
    let (epoch, cipher, room) = begin_add_in(&session, entry_id)?;
    if !grants.take(path, Purpose::Attachment) {
        return Err(Error::Unsupported(
            "this file was not chosen in the app".into(),
        ));
    }
    Ok((epoch, cipher, room))
}

/// The same checks for a file that arrives as bytes, which has no path and so
/// no grant to spend.
fn begin_add_bytes(state: &AppState, entry_id: &str) -> Result<(Epoch, AttachmentCipher, u64)> {
    let session = state.session.lock().unwrap();
    begin_add_in(&session, entry_id)
}

fn begin_add_in(session: &Session, entry_id: &str) -> Result<(Epoch, AttachmentCipher, u64)> {
    let cipher = cipher_of(session)?;
    let store = session.store()?;
    attachable(store, entry_id)?;
    let used = store.attachment_bytes().map_err(store_err)?;
    Ok((
        session.epoch(),
        cipher,
        MAX_VAULT_ATTACHMENT_BYTES.saturating_sub(used),
    ))
}

/// Read the file at `path` whole, refusing it past the cap, and seal it under
/// its own name (see `seal_bytes`). The plaintext is scrubbed on the way out.
fn seal_file(
    cipher: &AttachmentCipher,
    entry_id: &str,
    path: &Path,
    room: u64,
) -> Result<Attachment> {
    let bytes = Zeroizing::new(read_regular_file_capped(path, MAX_ATTACHMENT_BYTES)?);
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    seal_bytes(cipher, entry_id, &name, &bytes, room)
}

/// Seal `bytes` as a file called `name`, refusing them past the `room` left in
/// the vault's budget, under a fresh id. The name is kept to its last path
/// component: for a file read off disk that is all `seal_file` passes, and for
/// one the webview named it is what keeps the name a name.
fn seal_bytes(
    cipher: &AttachmentCipher,
    entry_id: &str,
    name: &str,
    bytes: &[u8],
    room: u64,
) -> Result<Attachment> {
    // Checked here, where the size is known and nothing is sealed yet. Two adds
    // racing can each fit on their own and overshoot together, by one file at
    // most — which the headroom under the pack cap absorbs.
    if bytes.len() as u64 > room {
        return Err(Error::VaultFull);
    }
    let name = file_name(name);
    let id = migrate::new_entry_id();
    let blob = cipher.seal(&id, bytes)?;
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

fn begin_unseal(state: &AppState, id: &str) -> Result<(Epoch, AttachmentCipher, Attachment)> {
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

/// A name kept to its last path component: the one a save offers, so a name
/// that arrived through sync cannot point the mobile staging anywhere else,
/// and the one a file sent as bytes is stored under, for the same reason.
fn file_name(name: &str) -> String {
    Path::new(name)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "attachment".into())
}

/// Decode a picture and write it back as a PNG — turned the way its EXIF
/// orientation says (a phone stores a photo as the sensor saw it and notes
/// which way is up; a browser honours the note, the decoder here does not), and
/// scaled down to fit `fit` a side first, when asked, keeping its proportions
/// and never scaling up. The decoder is told how big an image it may build,
/// since a small TIFF can describe an enormous one: a side past
/// [`MAX_RENDER_SIDE`], or a decode past [`MAX_RENDER_BYTES`] of memory, is
/// refused rather than attempted. One picture at a time, whoever asks: a list
/// of photos opening at once used to decode them all at once, and the bitmaps
/// together could take what a phone has. Pure, so it is tested without a vault.
fn render_png(bytes: &[u8], fit: Option<u32>) -> Result<Vec<u8>> {
    use image::ImageDecoder;

    let _one_at_a_time = ONE_PICTURE.lock().unwrap_or_else(|e| e.into_inner());
    let unsupported = |e: image::ImageError| Error::Unsupported(e.to_string());
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_RENDER_SIDE);
    limits.max_image_height = Some(MAX_RENDER_SIDE);
    limits.max_alloc = Some(MAX_RENDER_BYTES);
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| Error::Unsupported(e.to_string()))?;
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(unsupported)?;
    let orientation = decoder.orientation().map_err(unsupported)?;
    let mut decoded = image::DynamicImage::from_decoder(decoder).map_err(unsupported)?;
    decoded.apply_orientation(orientation);
    if let Some(side) = fit {
        if decoded.width() > side || decoded.height() > side {
            decoded = decoded.thumbnail(side, side);
        }
    }
    let mut png = std::io::Cursor::new(Vec::new());
    decoded
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(|e| Error::Other(e.to_string()))?;
    Ok(png.into_inner())
}

/// What makes [`render_png`] decode one picture at a time.
static ONE_PICTURE: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The largest picture a rendition will decode: 12k a side, a quarter gigabyte
/// of pixels — well past any photo, well short of what a crafted header asks.
const MAX_RENDER_SIDE: u32 = 12_000;
const MAX_RENDER_BYTES: u64 = 256 * 1024 * 1024;

/// What a row's thumbnail is scaled to fit: a 40px tile at the sharpest phone
/// density is 120px, with room for the crop `object-cover` makes.
const THUMBNAIL_SIDE: u32 = 160;

/// A best guess at the type from the name, for the list's glyph and for what
/// the webview will try to show. `None` is simply "a file" — and a name the
/// table does not know is guessed again from its extension in the webview
/// (`Attachments/kind.ts`), which is what covers files attached before a
/// format was added here.
fn mime_of(name: &str) -> Option<String> {
    let ext = Path::new(name).extension()?.to_str()?.to_ascii_lowercase();
    let mime = match ext.as_str() {
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "heic" => "image/heic",
        "heif" => "image/heif",
        "tiff" | "tif" => "image/tiff",
        "bmp" => "image/bmp",
        "avif" => "image/avif",
        "svg" => "image/svg+xml",
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

        let (_, cipher, stored) = begin_unseal(&state, &dto.id).unwrap();
        assert_eq!(
            &**cipher.unseal(&stored.meta.id, &stored.blob).unwrap(),
            b"%PDF-1.7 the file"
        );
    }

    // The phone's route: no path, no grant, the bytes themselves — sealed under
    // the name the webview gave, kept to a file name.
    #[test]
    fn bytes_are_sealed_under_their_file_name() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = vault(&dir, &argon2_key(), "login");
        let state = open_with(argon2_key(), store);

        let (epoch, cipher, room) = begin_add_bytes(&state, "e1").unwrap();
        let attachment =
            seal_bytes(&cipher, "e1", "../../IMG_0042.heic", b"the photo", room).unwrap();
        assert!(!attachment.blob.windows(9).any(|w| w == b"the photo"));
        let dto = finish_add(&state, epoch, &attachment).unwrap();
        assert_eq!(
            (dto.name.as_str(), dto.size, dto.mime.as_deref()),
            ("IMG_0042.heic", 9, Some("image/heic"))
        );

        let (_, cipher, stored) = begin_unseal(&state, &dto.id).unwrap();
        assert_eq!(
            &**cipher.unseal(&stored.meta.id, &stored.blob).unwrap(),
            b"the photo"
        );
    }

    // Bytes answer to the same guards as a path: the vault's state and budget.
    #[test]
    fn bytes_are_held_to_the_same_guards() {
        assert!(matches!(
            begin_add_bytes(&AppState::default(), "e1"),
            Err(Error::Locked)
        ));

        let dir = tempfile::tempdir().unwrap();
        let (store, _) = vault(&dir, &argon2_key(), "env");
        let state = open_with(argon2_key(), store);
        assert!(matches!(
            begin_add_bytes(&state, "e1"),
            Err(Error::Unsupported(_))
        ));
        assert!(matches!(
            begin_add_bytes(&state, "nope"),
            Err(Error::NotFound)
        ));

        let cipher = argon2_key().attachment_cipher().unwrap();
        assert!(seal_bytes(&cipher, "e1", "a.bin", b"1234", 4).is_ok());
        assert!(matches!(
            seal_bytes(&cipher, "e1", "a.bin", b"12345", 4),
            Err(Error::VaultFull)
        ));
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
        assert!(matches!(begin_unseal(&state, "a1"), Err(Error::Locked)));
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
        assert!(matches!(
            begin_unseal(&state, &dto.id),
            Err(Error::NotFound)
        ));
    }

    // What the preview falls back on: a format the webview may not decode comes
    // back as a PNG of the same picture, and a file that is no picture at all is
    // refused as unsupported rather than failing some other way.
    #[test]
    fn a_picture_renders_as_a_png_and_a_non_picture_is_refused() {
        let mut bmp = std::io::Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(3, 2, image::Rgba([10, 20, 30, 255]))
            .write_to(&mut bmp, image::ImageFormat::Bmp)
            .unwrap();

        let png = render_png(bmp.get_ref(), None).unwrap();
        assert!(png.starts_with(b"\x89PNG"));
        let back = image::load_from_memory(&png).unwrap().to_rgba8();
        assert_eq!(back.dimensions(), (3, 2));
        assert_eq!(back.get_pixel(2, 1).0, [10, 20, 30, 255]);

        assert!(matches!(
            render_png(b"%PDF-1.7 not a picture", None),
            Err(Error::Unsupported(_))
        ));
    }

    // A phone's JPEG says which way is up in its EXIF, and the rendition has to
    // turn the pixels that way: a 16×8 picture, red on the left and blue on the
    // right, tagged "rotate 90° clockwise" (orientation 6) comes out 8×16 with
    // the red on top. The tag is spliced into the JPEG by hand — an APP1 segment
    // holding the smallest TIFF that carries one Orientation entry — since the
    // encoder here writes none.
    #[test]
    fn a_rendition_is_turned_the_way_its_exif_says() {
        let mut picture = image::RgbImage::new(16, 8);
        for (x, _, pixel) in picture.enumerate_pixels_mut() {
            *pixel = if x < 8 {
                image::Rgb([255, 0, 0])
            } else {
                image::Rgb([0, 0, 255])
            };
        }
        let mut jpeg = std::io::Cursor::new(Vec::new());
        picture
            .write_to(&mut jpeg, image::ImageFormat::Jpeg)
            .unwrap();
        let jpeg = jpeg.into_inner();

        let tiff: Vec<u8> = [
            &b"II"[..],
            &42u16.to_le_bytes(),
            &8u32.to_le_bytes(),      // the IFD follows the header
            &1u16.to_le_bytes(),      // one entry:
            &0x0112u16.to_le_bytes(), // Orientation,
            &3u16.to_le_bytes(),      // a SHORT,
            &1u32.to_le_bytes(),      // one of them,
            &6u32.to_le_bytes(),      // = 6, rotate 90° clockwise
            &0u32.to_le_bytes(),      // no next IFD
        ]
        .concat();
        let payload = [&b"Exif\0\0"[..], &tiff].concat();
        let mut app1 = vec![0xFF, 0xE1];
        app1.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
        app1.extend_from_slice(&payload);
        // Right after SOI, before the encoder's own segments.
        let tagged = [&jpeg[..2], &app1, &jpeg[2..]].concat();

        let turned = image::load_from_memory(&render_png(&tagged, None).unwrap())
            .unwrap()
            .to_rgb8();
        assert_eq!(turned.dimensions(), (8, 16));
        let [r, _, b] = turned.get_pixel(4, 2).0;
        assert!(
            r > 200 && b < 60,
            "top should be red, got {:?}",
            turned.get_pixel(4, 2)
        );
        let [r, _, b] = turned.get_pixel(4, 13).0;
        assert!(
            b > 200 && r < 60,
            "bottom should be blue, got {:?}",
            turned.get_pixel(4, 13)
        );

        // Untagged, the same picture keeps its shape.
        let plain = image::load_from_memory(&render_png(&jpeg, None).unwrap()).unwrap();
        assert_eq!((plain.width(), plain.height()), (16, 8));
    }

    // A thumbnail is bounded by its longer side and keeps the picture's shape;
    // one already small enough is left at its own size rather than blown up.
    #[test]
    fn a_thumbnail_fits_its_side_and_never_scales_up() {
        let encode = |w, h| {
            let mut png = std::io::Cursor::new(Vec::new());
            image::RgbaImage::from_pixel(w, h, image::Rgba([1, 2, 3, 255]))
                .write_to(&mut png, image::ImageFormat::Png)
                .unwrap();
            png.into_inner()
        };
        let size = |png: &[u8]| {
            image::load_from_memory(png)
                .unwrap()
                .to_rgba8()
                .dimensions()
        };

        assert_eq!(
            size(&render_png(&encode(800, 200), Some(160)).unwrap()),
            (160, 40)
        );
        assert_eq!(
            size(&render_png(&encode(200, 800), Some(160)).unwrap()),
            (40, 160)
        );
        assert_eq!(
            size(&render_png(&encode(30, 20), Some(160)).unwrap()),
            (30, 20)
        );
    }

    #[test]
    fn a_saved_name_is_only_ever_a_file_name() {
        assert_eq!(file_name("scan.pdf"), "scan.pdf");
        assert_eq!(file_name("../../etc/passwd"), "passwd");
        assert_eq!(file_name(""), "attachment");
        assert_eq!(file_name(".."), "attachment");
    }
}
