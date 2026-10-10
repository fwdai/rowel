//! The commands whose work lives outside `commands/`: the file picker, image
//! scanning and favicon lookup. Each module owns its logic; this is only where
//! the IPC surface is declared.

use std::path::Path;

use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::error::{Error, Result};
use crate::grants::{PathGrants, Purpose};
use crate::scan::ScanResult;
use crate::state::AppState;
use crate::{favicon, scan};

/// The OS file dialog, for a file the backend will then read: `kind` is
/// `"image"` (filtered to what the scanner opens), `"env"` (any file, since
/// an env file may be named anything) or `"attachment"` (any file). The path
/// picked, or `None` when the dialog was dismissed.
///
/// Run from Rust rather than through the dialog plugin's JS API so the choice
/// is one the backend witnessed: the picked path is granted (see `grants`)
/// before the webview hears of it, and `scan_image` / `read_env_file` read
/// only granted paths. The grant carries the `kind` the dialog was opened as,
/// so a file chosen from the env picker cannot be spent on a scan instead.
/// `label` is the filter's name in the dialog's own chrome, translated by the
/// webview, which owns the catalogue.
#[tauri::command]
pub async fn pick_file(
    app: AppHandle,
    grants: State<'_, PathGrants>,
    kind: String,
    label: Option<String>,
) -> Result<Option<String>> {
    let mut dialog = app.dialog().file();
    let purpose = match kind.as_str() {
        "image" => {
            let label = label.unwrap_or_else(|| "Images".into());
            dialog = dialog.add_filter(label, &scan::IMAGE_EXTENSIONS);
            Purpose::Image
        }
        "env" => Purpose::Env,
        "attachment" => Purpose::Attachment,
        other => return Err(Error::Unsupported(format!("no {other} picker"))),
    };
    // The dialog blocks its caller until the user answers, so it runs on the
    // blocking pool, not a runtime worker.
    let Some(picked) = super::blocking(move || Ok(dialog.blocking_pick_file())).await? else {
        return Ok(None);
    };
    let path = picked
        .into_path()
        .map_err(|_| Error::Unsupported("the picked file has no local path".into()))?;
    grants.grant(&path, purpose);
    Ok(Some(path.to_string_lossy().into_owned()))
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
        return Err(Error::Unsupported(
            "this file was not chosen in the app".into(),
        ));
    }
    let result = scan::scan(path).await?;
    super::same_session(&state, epoch)?;
    Ok(result)
}

/// Read a card or an identity document out of a photo the webview holds as
/// bytes — the one the phone's camera just took through the capture input,
/// which exists nowhere on disk and so has no path to grant.
///
/// No grant is needed, and none would mean anything: `grants` exists because a
/// *path* is the webview naming a file on disk it never read, whereas these
/// bytes are already the webview's own. Nothing on disk is opened for them.
/// The bytes travel as the request's raw body (a `Uint8Array` argument on the
/// JS side), not as JSON, so a photo is not spelled out as an array of numbers.
/// Unlocked vaults only, and the session must still be the one that asked, as
/// for `scan_image`.
#[tauri::command]
pub async fn scan_image_bytes(
    state: State<'_, AppState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ScanResult> {
    let epoch = super::unlocked_epoch(&state)?;
    let tauri::ipc::InvokeBody::Raw(image) = request.body() else {
        return Err(Error::Unsupported(
            "the photo has to be sent as bytes".into(),
        ));
    };
    // `Request` only lends the body, so the scanner works on a clone: that
    // copy is what gets zeroized (see `scan_bytes`); the body Tauri holds is
    // dropped with the request like any other.
    let result = scan::scan_bytes(image.clone()).await?;
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
