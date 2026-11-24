//! The commands whose work lives outside `commands/`: the file picker, image
//! scanning, and favicon and emblem lookup. Each module owns its logic; this is only where
//! the IPC surface is declared.

use std::path::Path;

use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::{DialogExt, PickerMode};

use super::vault::{BACKUP_EXTENSION, LEGACY_BACKUP_EXTENSION};
use crate::error::{Error, Result};
use crate::grants::{PathGrants, Purpose};
use crate::scan::ScanResult;
use crate::state::AppState;
use crate::{emblem, favicon, picked, scan};

/// The OS file dialog, for a file the backend will then read: `kind` is
/// `"image"` (filtered to what the scanner opens), `"env"` (any file, since
/// an env file may be named anything), `"attachment"` (any file), `"photo"`
/// (an image to attach — the attachment pick under the image filter),
/// `"import"` (a password manager's export), `"backup"` (a `.rowel` or a
/// legacy `.swftx` to restore on a fresh install), `"importBackup"` (the
/// same two, for Settings › Import) or `"lockboxBackup"` (a `.rowel` alone,
/// to restore as a lockbox beside the open one). The path picked, or `None`
/// when the dialog was dismissed or outlived the session it was opened in
/// (see `grants`). On Android that is the path of the copy
/// made of the document picked (`picked`), as on iOS it is the copy its
/// picker made.
///
/// On iOS an image filter makes the dialog plugin show the photo library
/// rather than the file browser (it switches to the media picker when no
/// filter names a non-media type); `browse` asks for the file browser anyway,
/// under the same filter — the "Choose File" row of the phone's source sheet,
/// for a photo kept in Files. The desktop has one dialog and ignores it. What
/// Android's picker is asked to filter on is [`offered`].
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
) -> Result<Option<String>> {
    let mut dialog = app.dialog().file();
    // What the pick is for, and the extensions its filter offers, with the
    // filter's name where the webview gave none.
    let (purpose, filter): (Purpose, Option<(&str, &'static [&'static str])>) = match kind.as_str()
    {
        "image" => (Purpose::Image, Some(("Images", &scan::IMAGE_EXTENSIONS))),
        "env" => (Purpose::Env, None),
        "attachment" => (Purpose::Attachment, None),
        "photo" => (
            Purpose::Attachment,
            Some(("Images", &scan::IMAGE_EXTENSIONS)),
        ),
        "import" => (
            Purpose::Import,
            Some(("Password exports", &super::import::EXTENSIONS)),
        ),
        "backup" | "importBackup" => (
            Purpose::Backup,
            Some(("Rowel backup", &[BACKUP_EXTENSION, LEGACY_BACKUP_EXTENSION])),
        ),
        "lockboxBackup" => (Purpose::Backup, Some(("Rowel backup", &[BACKUP_EXTENSION]))),
        other => return Err(Error::Unsupported(format!("no {other} picker"))),
    };
    if let Some((name, extensions)) = filter {
        if let Some(offered) = offered(extensions, cfg!(target_os = "android")) {
            dialog = dialog.add_filter(label.unwrap_or_else(|| name.into()), offered);
        }
    }
    if browse.unwrap_or(false) {
        dialog = dialog.set_picker_mode(PickerMode::Document);
    }
    // A session that ends while the dialog is open takes its answer with it.
    let generation = grants.generation();
    // The dialog blocks its caller until the user answers, and an Android
    // document is copied before it has a path (`picked`), so both run on the
    // blocking pool, not a runtime worker.
    super::blocking(move || {
        let Some(answer) = dialog.blocking_pick_file() else {
            return Ok(None);
        };
        // No copy for a session that ended while the dialog was open, and none
        // kept for one that ended while the copy was made.
        let grants = app.state::<PathGrants>();
        if grants.generation() != generation {
            return Ok(None);
        }
        let path = picked::into_path(&app, answer)?;
        if !grants.grant_picked(generation, &path, purpose) {
            picked::discard(&app, &path);
            return Ok(None);
        }
        Ok(Some(path.to_string_lossy().into_owned()))
    })
    .await
}

/// What the dialog is asked to filter a pick for `extensions` on: the
/// extensions themselves, on desktop and iOS. Android's picker filters on MIME
/// types, which the dialog plugin maps each extension to through `MimeTypeMap`,
/// dropping any it has none for (`tauri-plugin-dialog` 2.8.4, `DialogPlugin.kt`:
/// `parseFiltersOption`, `showFilePicker`) — `1pux`, `rowel` and `swftx` among
/// them — so the files they name are greyed out under the types that are left,
/// or every file is offered once none are. There (`android`) a pick is
/// filtered only by `image/*` when it is for images, a type the plugin passes
/// as it is and every image carries, and otherwise not at all: what was picked
/// is held to its kind by its name afterwards, by the reader it is for.
fn offered(extensions: &'static [&'static str], android: bool) -> Option<&'static [&'static str]> {
    if !android {
        return Some(extensions);
    }
    let images = extensions
        .iter()
        .all(|e| scan::IMAGE_EXTENSIONS.contains(e));
    images.then_some(&["image/*"])
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
#[tauri::command]
pub async fn scan_image(
    state: State<'_, AppState>,
    grants: State<'_, PathGrants>,
    path: String,
) -> Result<ScanResult> {
    let epoch = super::unlocked_epoch(&state)?;
    if !grants.take(Path::new(&path), Purpose::Image) {
        return Err(crate::grants::not_chosen());
    }
    let result = scan::scan(path).await?;
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
    state: State<'_, AppState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ScanResult> {
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

    // Every filter a pick is offered under, as `pick_file` builds them.
    const FILTERS: [&[&str]; 3] = [
        &scan::IMAGE_EXTENSIONS,
        &super::super::import::EXTENSIONS,
        &[BACKUP_EXTENSION, LEGACY_BACKUP_EXTENSION],
    ];

    // Desktop and iOS keep the filter exactly as it was.
    #[test]
    fn a_filter_is_its_extensions_where_the_picker_takes_them() {
        for extensions in FILTERS.into_iter().chain([&[BACKUP_EXTENSION][..]]) {
            assert_eq!(offered(extensions, false), Some(extensions));
        }
    }

    // Android: an image pick is filtered by the one type every image carries,
    // and no pick by an extension, which the picker may have no type for —
    // `.1pux`, `.rowel` and `.swftx` would be greyed out.
    #[test]
    fn an_android_pick_is_filtered_by_no_extension() {
        assert_eq!(
            offered(&scan::IMAGE_EXTENSIONS, true),
            Some(&["image/*"][..])
        );
        assert_eq!(offered(&super::super::import::EXTENSIONS, true), None);
        assert_eq!(
            offered(&[BACKUP_EXTENSION, LEGACY_BACKUP_EXTENSION], true),
            None
        );
        assert_eq!(offered(&[BACKUP_EXTENSION], true), None);
    }
}
