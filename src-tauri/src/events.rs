//! Every event the backend pushes at the frontend, and the only way to send one.
//!
//! Names and payload shapes live here rather than at the call sites so the
//! catalogue the frontend subscribes to can be read in one place, and so no
//! caller can invent a third spelling of `{ error }`.

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::models::EntryMetaDto;
use crate::state::SyncStatus;
use crate::sync::setup::PackInfo;

pub const VAULT_LOCKED: &str = "vault:locked";
pub const VAULT_MERGED: &str = "vault:merged";
pub const SYNC_STATUS: &str = "sync:status";
pub const IMPORT_PROGRESS: &str = "import:progress";
pub const SETUP_DRIVE_PENDING: &str = "setup:drive:pending";
pub const SETUP_DRIVE_PROBED: &str = "setup:drive:probed";
pub const SETUP_DRIVE_ERROR: &str = "setup:drive:error";
pub const WORKSPACES_REMOTE: &str = "workspaces:remote";
pub const WORKSPACES_ADDED: &str = "workspaces:added";
pub const WORKSPACES_RENAMED: &str = "workspaces:renamed";
// Desktop only, like the file associations that produce it (`crate::opened`).
#[cfg(desktop)]
pub const FILE_OPENED: &str = "file:opened";
// Desktop only, like the extension host that raises it (`crate::browser`).
#[cfg(desktop)]
pub const BROWSER_ASSOCIATE: &str = "browser:associate";
#[cfg(desktop)]
pub const BROWSER_PASSKEY: &str = "browser:passkey";
#[cfg(desktop)]
pub const BROWSER_CLIENTS: &str = "browser:clients";

#[derive(Serialize, Clone)]
struct Entries {
    entries: Vec<EntryMetaDto>,
}

#[cfg(desktop)]
#[derive(Serialize, Clone)]
struct Opened<'a> {
    path: &'a str,
}

#[derive(Serialize, Clone)]
struct ErrorText<'a> {
    error: &'a str,
}

#[derive(Serialize, Clone)]
struct Progress {
    done: usize,
    total: usize,
}

/// Every vault the account holds, so the first run can offer a choice rather
/// than one picked for it. An empty list is an answer — the account holds no
/// vault — not an absence.
#[derive(Serialize, Clone)]
struct Probed {
    files: Vec<PackInfo>,
}

pub fn vault_locked(app: &AppHandle) {
    let _ = app.emit(VAULT_LOCKED, ());
}

pub fn vault_merged(app: &AppHandle, entries: Vec<EntryMetaDto>) {
    let _ = app.emit(VAULT_MERGED, Entries { entries });
}

/// The whole of sync, every time anything about it changes. One snapshot
/// rather than one event per transition: the frontend stores it as-is, and
/// neither side has to agree on an order of events.
pub fn sync_status(app: &AppHandle, status: SyncStatus) {
    let _ = app.emit(SYNC_STATUS, status);
}

pub fn import_progress(app: &AppHandle, done: usize, total: usize) {
    let _ = app.emit(IMPORT_PROGRESS, Progress { done, total });
}

pub fn setup_drive_pending(app: &AppHandle) {
    let _ = app.emit(SETUP_DRIVE_PENDING, ());
}

pub fn setup_drive_probed(app: &AppHandle, files: Vec<PackInfo>) {
    let _ = app.emit(SETUP_DRIVE_PROBED, Probed { files });
}

pub fn setup_drive_error(app: &AppHandle, error: &str) {
    let _ = app.emit(SETUP_DRIVE_ERROR, ErrorText { error });
}

/// The account's vaults that no workspace on this device holds, after every
/// sync run — the same shape as the probe's answer, because Settings ›
/// Workspaces offers them through the same picker. Empty means every vault in
/// the account is here.
pub fn remote_vaults(app: &AppHandle, files: Vec<PackInfo>) {
    let _ = app.emit(WORKSPACES_REMOTE, Probed { files });
}

#[derive(Serialize, Clone)]
struct Added<'a> {
    name: &'a str,
}

/// A vault from the account was added as a workspace without being asked for
/// — the password that just opened a vault opened it too (`commands::autojoin`).
/// The frontend re-probes for the new list and says so; nothing switches.
pub fn workspace_added(app: &AppHandle, name: &str) {
    let _ = app.emit(WORKSPACES_ADDED, Added { name });
}

/// A sync pulled a newer name for the open workspace. No payload: the names
/// live on `app_status`, which the frontend re-reads, so the header and the
/// Workspaces list redraw from the one answer rather than from two.
pub fn workspace_renamed(app: &AppHandle) {
    let _ = app.emit(WORKSPACES_RENAMED, ());
}

/// The OS asked the app to open a backup (see `crate::opened`). Also parked for
/// a webview that is not listening yet; this is the live half.
#[cfg(desktop)]
pub fn file_opened(app: &AppHandle, path: &str) {
    let _ = app.emit(FILE_OPENED, Opened { path });
}

#[cfg(desktop)]
#[derive(Serialize, Clone)]
struct Associate<'a> {
    id: &'a str,
    key: &'a str,
}

/// A browser extension asks to be let in (`browser::consent`). `key` is its
/// identification public key, for the dialog to show a fingerprint of; the
/// answer comes back through `commands::browser::browser_respond`, naming
/// `id` — this ask's alone, so a dialog left up past it cannot answer the
/// next ask the same extension makes.
#[cfg(desktop)]
pub fn browser_associate(app: &AppHandle, id: &str, key: &str) {
    let _ = app.emit(BROWSER_ASSOCIATE, Associate { id, key });
}

#[cfg(desktop)]
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct PasskeyAsk<'a> {
    id: &'a str,
    kind: &'static str,
    rp_id: &'a str,
    origin: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    user_name: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    user_display_name: Option<&'a str>,
    /// A sign-in's accounts, as the vault names them; the answer picks one by
    /// its place here. Not sent for a registration.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    accounts: Vec<PasskeyAccount<'a>>,
}

#[cfg(desktop)]
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct PasskeyAccount<'a> {
    user_name: &'a str,
    user_display_name: &'a str,
    /// RFC3339, when known: what tells two passkeys named alike apart.
    #[serde(skip_serializing_if = "Option::is_none")]
    created_at: Option<&'a str>,
}

/// The page at `origin` asks, through the extension, to create a passkey
/// (`register`) or to sign in with one (`get`). The site it is for is
/// `rpId`; the account, on a registration, is the one the site names, and on
/// a sign-in the `accounts` are the ones the vault could sign in as. The
/// answer comes back through `commands::browser::browser_passkey_respond`,
/// naming `id`, so a dialog left up past this ask cannot answer the next.
#[cfg(desktop)]
pub fn browser_passkey(
    app: &AppHandle,
    id: &str,
    origin: &str,
    ceremony: crate::passkey::Ceremony<'_>,
) {
    use crate::passkey::Ceremony;
    let ask = match ceremony {
        Ceremony::Register {
            rp_id,
            user_name,
            user_display_name,
        } => PasskeyAsk {
            id,
            kind: "register",
            rp_id,
            origin,
            user_name,
            user_display_name,
            accounts: Vec::new(),
        },
        Ceremony::SignIn { rp_id, accounts } => PasskeyAsk {
            id,
            kind: "get",
            rp_id,
            origin,
            user_name: None,
            user_display_name: None,
            accounts: accounts
                .iter()
                .map(|account| PasskeyAccount {
                    user_name: &account.user_name,
                    user_display_name: &account.user_display_name,
                    created_at: account.created_at.as_deref(),
                })
                .collect(),
        },
    };
    let _ = app.emit(BROWSER_PASSKEY, ask);
}

/// An extension was let into the open vault by the consent dialog. No
/// payload: Settings › Browser extension re-reads `browser_status`, which is
/// the one answer its list is drawn from.
#[cfg(desktop)]
pub fn browser_clients(app: &AppHandle) {
    let _ = app.emit(BROWSER_CLIENTS, ());
}
