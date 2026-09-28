//! The browser extension host's switches: on or off, the extensions let in,
//! and the answer to one asking.

use serde::Serialize;
use serde_json::json;
use tauri::AppHandle;

use crate::browser::{self, manifest, Client};
use crate::error::Result;
use crate::settings;
use crate::storage;

/// The host as Settings shows it. `clients` are the open vault's — the list
/// is kept inside each vault — so it is empty while locked.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BrowserStatus {
    pub enabled: bool,
    pub browsers: Vec<manifest::Status>,
    pub clients: Vec<Client>,
}

fn status(app: &AppHandle) -> Result<BrowserStatus> {
    let root = storage::root_dir(app)?;
    Ok(BrowserStatus {
        enabled: settings::current(app).browser.enabled,
        browsers: manifest::status(&root),
        clients: browser::clients(app),
    })
}

#[tauri::command]
pub fn browser_status(app: AppHandle) -> Result<BrowserStatus> {
    status(&app)
}

/// Turn the host on — writing the manifests every browser here finds it by,
/// and listening — or off, taking the manifests back. The listener stays up
/// once started (see `browser::server`); off means every connection, the
/// ones up included, is refused at its next request.
#[tauri::command]
pub fn browser_set_enabled(enabled: bool, app: AppHandle) -> Result<BrowserStatus> {
    let root = storage::root_dir(&app)?;
    settings::set(&app, &json!({ "browser": { "enabled": enabled } }))?;
    if enabled {
        manifest::install(&root);
        browser::server::start(&app);
    } else {
        manifest::remove(&root);
    }
    status(&app)
}

/// The user's answer to the extension asking to connect under `id`: the name
/// they gave it, or `null` to refuse. Nothing happens when no ask is up under
/// that id — a dialog answering late, after its ask timed out and another
/// took its place, approves nothing, not even the same extension's retry.
#[tauri::command]
pub fn browser_respond(id: String, name: Option<String>) -> Result<()> {
    browser::respond(&id, name);
    Ok(())
}

/// The user's answer to the passkey ceremony asked under `id`: `allow`, and
/// for a sign-in which of the ask's accounts (`account`, the newest when
/// unsaid). Nothing happens when no ask is up under it — a dialog answering
/// late, after its ask timed out and another took its place, approves nothing.
#[tauri::command]
pub fn browser_passkey_respond(id: String, allow: bool, account: Option<usize>) -> Result<()> {
    browser::respond_passkey(&id, allow.then(|| account.unwrap_or(0)));
    Ok(())
}

/// Take an extension's access to the open vault back. Its next request — on
/// a connection that is up as much as on a new one — fails the association
/// check, and it has to ask, and be let in, again.
#[tauri::command]
pub fn browser_forget_client(key: String, app: AppHandle) -> Result<BrowserStatus> {
    browser::forget(&app, &key)?;
    status(&app)
}
