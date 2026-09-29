//! The browser extension host's switches: on or off, the extensions let in,
//! and the answer to one asking.

use serde::Serialize;
use serde_json::json;
use tauri::AppHandle;

use crate::browser::{self, manifest, Client};
use crate::error::Result;
use crate::settings::{self, BrowserSettings};
use crate::storage;

/// The host as Settings shows it. `clients` are the open vault's — the list
/// is kept inside each vault — so it is empty while locked.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BrowserStatus {
    pub enabled: bool,
    pub keepassxc_compat: bool,
    pub browsers: Vec<manifest::Status>,
    pub clients: Vec<Client>,
}

fn status(app: &AppHandle) -> Result<BrowserStatus> {
    let root = storage::root_dir(app)?;
    let host = settings::current(app).browser;
    Ok(BrowserStatus {
        enabled: host.enabled,
        keepassxc_compat: host.keepassxc_compat,
        browsers: manifest::status(&root, host.keepassxc_compat),
        clients: browser::clients(app),
    })
}

// The group goes back whole: a settings patch replaces a top-level key, so
// sending one field would reset the other to its default. Settings sends one
// change at a time, so nothing lands between the read and the write.
fn save(app: &AppHandle, change: impl FnOnce(&mut BrowserSettings)) -> Result<BrowserSettings> {
    let mut host = settings::current(app).browser;
    change(&mut host);
    Ok(settings::set(app, &json!({ "browser": host }))?.browser)
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
    let host = save(&app, |host| host.enabled = enabled)?;
    if enabled {
        manifest::install(&root, host.keepassxc_compat);
        browser::server::start(&app);
    } else {
        manifest::remove(&root);
    }
    status(&app)
}

/// Also register under KeePassXC's host name, for the stock KeePassXC-Browser
/// extension — or stop, taking that manifest back and leaving Rowel's own.
/// With the host off there is nothing to write; the choice waits for it.
#[tauri::command]
pub fn browser_set_keepassxc_compat(enabled: bool, app: AppHandle) -> Result<BrowserStatus> {
    let root = storage::root_dir(&app)?;
    let host = save(&app, |host| host.keepassxc_compat = enabled)?;
    if host.enabled {
        manifest::install(&root, host.keepassxc_compat);
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
