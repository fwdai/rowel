//! The commands whose work lives outside `commands/`: the file picker, image
//! scanning, and favicon and emblem lookup. Each module owns its logic; this is only where
//! the IPC surface is declared.

use std::path::{Path, PathBuf};

use tauri::{AppHandle, State};
use tauri_plugin_dialog::{DialogExt, FilePath, PickerMode};

use super::vault::{BACKUP_EXTENSION, LEGACY_BACKUP_EXTENSION};
use crate::error::{Error, Result};
use crate::grants::{PathGrants, Purpose};
use crate::picked::Pick;
use crate::scan::ScanResult;
use crate::state::AppState;
use crate::{emblem, favicon, picked, scan, storage};

/// The OS file dialog, for a file the backend will then read: `kind` is
/// `"image"` (filtered to what the scanner opens), `"env"` (any file, since
/// an env file may be named anything), `"attachment"` (any file), `"photo"`
/// (an image to attach — the attachment pick under the image filter),
/// `"import"` (a password manager's export), `"backup"` (a `.rowel` or a
/// legacy `.swftx` to restore on a fresh install), `"importBackup"` (the
/// same two, for Settings › Import) or `"lockboxBackup"` (a `.rowel` alone,
/// to restore as a lockbox beside the open one). What was picked — its path
/// (on Android, the document's URI: see `chosen_path`) and the name it goes by
/// (`picked::name`) — or `None` when the dialog was dismissed. The webview
/// tells anything about the file from the name and hands the path back to read.
///
/// On a phone an image filter makes the dialog plugin show the photo library
/// rather than the file browser (it switches to the media picker when no
/// filter names a non-media type); `browse` asks for the file browser anyway,
/// under the same filter — the "Choose File" row of the phone's source sheet,
/// for a photo kept in Files. The desktop has one dialog and ignores it.
///
/// Run from Rust rather than through the dialog plugin's JS API so the choice
/// is one the backend witnessed: the picked path is granted (see `grants`)
/// before the webview hears of it, and every command that reads a path the
/// webview names reads only granted ones. The grant carries the purpose the dialog was opened
/// for, so a file chosen from the env picker cannot be spent on a scan
/// instead — and a photo picked to attach is only ever attached, never
/// scanned. `label` is the filter's name in the dialog's own chrome,
/// translated by the webview, which owns the catalogue.
#[tauri::command]
pub async fn pick_file(
    app: AppHandle,
    grants: State<'_, PathGrants>,
    kind: String,
    label: Option<String>,
    browse: Option<bool>,
) -> Result<Option<Pick>> {
    let mut dialog = app.dialog().file();
    let purpose = match kind.as_str() {
        "image" => {
            let label = label.unwrap_or_else(|| "Images".into());
            dialog = dialog.add_filter(label, &scan::IMAGE_EXTENSIONS);
            Purpose::Image
        }
        "env" => Purpose::Env,
        "attachment" => Purpose::Attachment,
        "photo" => {
            let label = label.unwrap_or_else(|| "Images".into());
            dialog = dialog.add_filter(label, &scan::IMAGE_EXTENSIONS);
            Purpose::Attachment
        }
        "import" => {
            let label = label.unwrap_or_else(|| "Password exports".into());
            dialog = dialog.add_filter(label, &super::import::EXTENSIONS);
            Purpose::Import
        }
        "backup" => {
            let label = label.unwrap_or_else(|| "Rowel backup".into());
            dialog = dialog.add_filter(label, &[BACKUP_EXTENSION, LEGACY_BACKUP_EXTENSION]);
            Purpose::Backup
        }
        "importBackup" => {
            let label = label.unwrap_or_else(|| "Rowel backup".into());
            dialog = dialog.add_filter(label, &[BACKUP_EXTENSION, LEGACY_BACKUP_EXTENSION]);
            Purpose::Backup
        }
        "lockboxBackup" => {
            let label = label.unwrap_or_else(|| "Rowel backup".into());
            dialog = dialog.add_filter(label, &[BACKUP_EXTENSION]);
            Purpose::Backup
        }
        other => return Err(Error::Unsupported(format!("no {other} picker"))),
    };
    if browse.unwrap_or(false) {
        dialog = dialog.set_picker_mode(PickerMode::Document);
    }
    // The dialog blocks its caller until the user answers, and a document's
    // name is a round trip to its provider, so both run on the blocking pool,
    // not a runtime worker.
    let chosen = super::blocking(move || {
        let Some(answer) = dialog.blocking_pick_file() else {
            return Ok(None);
        };
        let path = chosen_path(answer, picked::DOCUMENTS)?;
        let name = picked::name(&app, &path);
        Ok(Some((path, name)))
    })
    .await?;
    let Some((path, name)) = chosen else {
        return Ok(None);
    };
    grants.grant(&path, purpose);
    Ok(Some(Pick {
        path: path.to_string_lossy().into_owned(),
        name,
    }))
}

/// What a pick is granted and handed to the webview as. The desktop dialog
/// answers with a path and the iOS one with a `file://` URL, which is resolved
/// to its path. The Android picker answers with a document's `content://` URI,
/// which has no path; where the picker hands those out (`documents`) it is
/// kept as it came, for the reader to open through `picked::open`.
fn chosen_path(answer: FilePath, documents: bool) -> Result<PathBuf> {
    match answer {
        FilePath::Url(url) if picked::is_document(Path::new(url.as_str()), documents) => {
            Ok(PathBuf::from(url.as_str()))
        }
        answer => answer
            .into_path()
            .map_err(|_| Error::Unsupported("the picked file has no local path".into())),
    }
}

/// Read a card or an identity document out of the image at `path`.
///
/// Unlocked vaults only. The scanner reads a file the webview named and hands
/// back what it found in it, so a locked app must not run one for anybody: the
/// only surfaces that scan live in the unlocked shell, and the refusal is an
/// error rather than a silent miss because a real scan never asks while locked.
/// The path has to be one the user chose *as an image* — picked through the
/// image `pick_file` or dropped on the window as one — so a file chosen from
/// the unfiltered env dialog is refused here rather than OCRed; and `scan`
/// itself refuses one that is not an image type the pickers offer, before the
/// file is opened. A lock while the scan runs discards what it read: the
/// session that asked is gone.
///
/// An Android document has no path the recognizer could load, so it is read
/// as bytes and scanned as a photo sent as bytes is, under the same cap; the
/// image check is made on the name its provider gives it.
#[tauri::command]
pub async fn scan_image(
    app: AppHandle,
    state: State<'_, AppState>,
    grants: State<'_, PathGrants>,
    path: String,
) -> Result<ScanResult> {
    let epoch = super::unlocked_epoch(&state)?;
    if !grants.take(Path::new(&path), Purpose::Image) {
        return Err(crate::grants::not_chosen());
    }
    let result = if picked::is_document(Path::new(&path), picked::DOCUMENTS) {
        let image = super::blocking(move || {
            let chosen = picked::open(&app, Path::new(&path))?;
            if !scan::is_image(Path::new(&chosen.name)) {
                return Err(Error::Unrecognized);
            }
            storage::read_capped(chosen.contents, scan::MAX_IMAGE_BYTES)
        })
        .await?;
        scan::scan_bytes(image).await?
    } else {
        scan::scan(path).await?
    };
    super::same_session(&state, epoch)?;
    Ok(result)
}

/// Read a card or an identity document out of a photo the webview holds as
/// bytes — the one the phone's system picker handed its file input, from the
/// camera, the photo library or Files, which the webview has whole and which
/// has no path to grant.
///
/// No grant is needed, and none would mean anything: `grants` exists because a
/// *path* is the webview naming a file on disk it never read, whereas these
/// bytes are already the webview's own. Nothing on disk is opened for them.
/// The bytes travel as the request's raw body (a `Uint8Array` argument on the
/// JS side), or as a JSON array of them when Tauri has fallen back to its
/// `postMessage` transport (see `body_bytes`). Unlocked vaults only, and the session must still be the one that asked, as
/// for `scan_image`.
#[tauri::command]
pub async fn scan_image_bytes(
    app: AppHandle,
    state: State<'_, AppState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ScanResult> {
    // The webview has read the camera's photo by now, whatever becomes of it
    // here, so the file wry left it in can go (see `capture`).
    crate::capture::sweep(&app);
    let epoch = super::unlocked_epoch(&state)?;
    // `Request` only lends the body, so the scanner works on a copy: that copy
    // is what gets zeroized (see `scan_bytes`); the body Tauri holds is
    // dropped with the request like any other.
    let image = super::body_bytes(request.body(), scan::MAX_IMAGE_BYTES)?;
    let result = scan::scan_bytes(image).await?;
    super::same_session(&state, epoch)?;
    Ok(result)
}

/// The icon for a host the entry list is showing. Unlocked vaults only: the
/// hosts are vault data, and a locked app should be making no requests about
/// them — a webview that asks anyway gets "no icon", not an error, since the
/// list it was drawing is gone.
///
/// The gate is the *session the request came from*, not "some session is
/// unlocked": a lookup is several round trips, and the vault can lock — or
/// lock and reopen — while one is in flight. `favicon::fetch` asks again
/// before each request, so a lock mid-lookup ends it at the next step, and
/// caches the answer against that session's store or not at all.
#[tauri::command]
pub async fn fetch_favicon(
    app: AppHandle,
    state: State<'_, AppState>,
    host: String,
) -> Result<Option<String>> {
    let Ok(epoch) = super::unlocked_epoch(&state) else {
        return Ok(None);
    };
    favicon::fetch(&app, &host, epoch).await
}

/// The coat of arms a passport from `country` (ISO 3166-1 alpha-3) wears on
/// its cover. Gated like [`fetch_favicon`], for the same reasons: the country
/// comes out of the vault, and a locked app answers "none".
#[tauri::command]
pub async fn fetch_emblem(
    app: AppHandle,
    state: State<'_, AppState>,
    country: String,
) -> Result<Option<String>> {
    let Ok(epoch) = super::unlocked_epoch(&state) else {
        return Ok(None);
    };
    emblem::fetch(&app, &country, epoch).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    const DOCUMENT: &str = "content://com.android.providers.media.documents/document/image%3A1000";

    fn answer(text: &str) -> FilePath {
        FilePath::from_str(text).unwrap()
    }

    // The Android picker's answer is kept as it came, to be granted and read
    // through the provider.
    #[test]
    fn a_document_is_kept_as_its_uri_where_the_picker_hands_them_out() {
        assert_eq!(
            chosen_path(answer(DOCUMENT), true).unwrap(),
            PathBuf::from(DOCUMENT)
        );
    }

    // Anywhere else a `content://` answer is nothing the backend could read.
    #[test]
    fn a_document_has_no_path_anywhere_else() {
        assert!(matches!(
            chosen_path(answer(DOCUMENT), false),
            Err(Error::Unsupported(_))
        ));
    }

    // The desktop and iOS answers resolve to the path on disk, documents or not.
    #[cfg(unix)]
    #[test]
    fn a_path_or_a_file_url_is_the_path_it_names() {
        for documents in [false, true] {
            assert_eq!(
                chosen_path(answer("/Users/me/export.csv"), documents).unwrap(),
                PathBuf::from("/Users/me/export.csv")
            );
            assert_eq!(
                chosen_path(
                    answer("file:///var/mobile/Documents/IMG_0001.HEIC"),
                    documents
                )
                .unwrap(),
                PathBuf::from("/var/mobile/Documents/IMG_0001.HEIC")
            );
        }
    }
}
