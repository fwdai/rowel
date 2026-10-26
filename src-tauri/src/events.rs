//! Every event the backend pushes at the frontend, and the only way to send one.
//!
//! Names and payload shapes live here rather than at the call sites so the
//! catalogue the frontend subscribes to can be read in one place, and so no
//! caller can invent a third spelling of `{ error }`.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::models::EntryMetaDto;
use crate::state::SyncStatus;
use crate::sync::setup::PackInfo;

pub const VAULT_LOCKED: &str = "vault:locked";
pub const VAULT_MERGED: &str = "vault:merged";
pub const SYNC_STATUS: &str = "sync:status";
pub const IMPORT_PROGRESS: &str = "import:progress";
pub const SETUP_ACCOUNT_PENDING: &str = "setup:account:pending";
pub const SETUP_ACCOUNT_PROBED: &str = "setup:account:probed";
pub const SETUP_ACCOUNT_ERROR: &str = "setup:account:error";
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
// Desktop only, like the agent grants the extension host issues (`crate::agents`).
#[cfg(desktop)]
pub const AGENT_GRANT: &str = "agent:grant";
#[cfg(desktop)]
pub const AGENT_GRANT_CLOSED: &str = "agent:grant:closed";
#[cfg(desktop)]
pub const AGENTS_CHANGED: &str = "agents:changed";
// The sign-in to a Rowel Server runs in the browser, so its progress is an
// event like a consent flow's (`org::connect`); desktop only, like that flow.
#[cfg(desktop)]
pub const ORG_CONNECT: &str = "org:connect";
pub const ORG_CHANGED: &str = "org:changed";

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

pub fn setup_account_pending(app: &AppHandle) {
    let _ = app.emit(SETUP_ACCOUNT_PENDING, ());
}

pub fn setup_account_probed(app: &AppHandle, files: Vec<PackInfo>) {
    let _ = app.emit(SETUP_ACCOUNT_PROBED, Probed { files });
}

pub fn setup_account_error(app: &AppHandle, error: &str) {
    let _ = app.emit(SETUP_ACCOUNT_ERROR, ErrorText { error });
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
    kind: rowel_broker::actions::ClientKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    platform: Option<&'a str>,
}

/// A client asks to be let in (`browser::consent`): a browser extension, or
/// an agent saying which `platform` it runs on (`claude-code`, `codex`, …) so
/// the dialog can name it. `key` is its identification public key, for the
/// dialog to show a fingerprint of; the answer comes back through
/// `commands::browser::browser_respond`, naming `id` — this ask's alone, so a
/// dialog left up past it cannot answer the next ask the same client makes.
#[cfg(desktop)]
pub fn browser_associate(
    app: &AppHandle,
    id: &str,
    key: &str,
    kind: rowel_broker::actions::ClientKind,
    platform: Option<&str>,
) {
    let _ = app.emit(
        BROWSER_ASSOCIATE,
        Associate {
            id,
            key,
            kind,
            platform,
        },
    );
}

/// Which client a ceremony or a grant came in through: enough for the
/// dialog to say "Requested by Work Chrome", nothing it could be addressed by.
#[cfg(desktop)]
#[derive(Serialize, Clone)]
struct RequestedBy<'a> {
    kind: rowel_broker::actions::ClientKind,
    name: &'a str,
}

#[cfg(desktop)]
impl<'a> From<&'a rowel_broker::actions::Client> for RequestedBy<'a> {
    fn from(client: &'a rowel_broker::actions::Client) -> Self {
        Self {
            kind: client.kind,
            name: &client.name,
        }
    }
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
    requested_by: RequestedBy<'a>,
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

/// The page at `origin` asks, through the extension `requested_by`, to create
/// a passkey (`register`) or to sign in with one (`get`). The site it is for
/// is `rpId`; the account, on a registration, is the one the site names, and
/// on a sign-in the `accounts` are the ones the vault could sign in as. The
/// answer comes back through `commands::browser::browser_passkey_respond`,
/// naming `id`, so a dialog left up past this ask cannot answer the next.
#[cfg(desktop)]
pub fn browser_passkey(
    app: &AppHandle,
    id: &str,
    origin: &str,
    ceremony: crate::passkey::Ceremony<'_>,
    requested_by: &rowel_broker::actions::Client,
) {
    use crate::passkey::Ceremony;
    let requested_by = RequestedBy::from(requested_by);
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
            requested_by,
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
            requested_by,
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

/// Where a grant ask would let the item be used, as the dialog names it: the
/// site for a fill (its origin, never the page's path or query), the request
/// pattern an agent wrote for a header, the command line for `env` — the
/// one place the value goes, shown verbatim so the person can read what the
/// child process will be.
#[cfg(desktop)]
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AskTarget {
    Origin {
        origin: String,
    },
    Http {
        host: String,
        path: String,
        methods: Vec<String>,
    },
    Command {
        line: String,
    },
}

/// A client asking to use an item somewhere: who asks (the name the user
/// gave it, or its key's two ends, and what kind of client it is), where
/// (`targets`, the request's own), how (`mode`), and which of the items it
/// may be given — the dialog picks one. An agent may say why (`reason`, its
/// own words, bounded and never interpreted); `ends_session` is the sign-out
/// notice: when a fill grant ends, the site's cookies go with it in the
/// browser that filled, whichever client asked for it, and no other mode
/// signs anything out. `verify` says what a step-up ask takes before the
/// answer counts: a biometric prompt, or the master password where the
/// device has none. The answer comes back through
/// `commands::browser::agent_grant_respond`, naming `id`, so a dialog left up
/// past this ask cannot answer the next. `server` names the company whose
/// Rowel Server routed the ask here (`org::approvals`), so the dialog can say
/// the agent is not one of this machine's; absent for the app's own asks.
#[cfg(desktop)]
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AgentGrantAsk<'a> {
    pub id: &'a str,
    pub client_name: &'a str,
    pub client_kind: rowel_broker::actions::ClientKind,
    pub targets: Vec<AskTarget>,
    pub mode: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<&'a str>,
    pub ends_session: bool,
    pub step_up: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verify: Option<&'static str>,
    pub candidates: Vec<rowel_broker::grant::VisibleItem>,
    pub default_standing_days: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server: Option<&'a str>,
}

#[cfg(desktop)]
pub fn agent_grant(app: &AppHandle, ask: AgentGrantAsk<'_>) {
    let _ = app.emit(AGENT_GRANT, ask);
}

/// The grant ask up under `id` was answered without its dialog — refused
/// because the Agents switch, or a lockbox's agent access, changed while it
/// was up (`agents::change_access`), or the vault was sealed under it
/// (`agents::clear`). The dialog closes; an answer it gave anyway would
/// reach no ask, but it must not sit there offering a yes.
#[cfg(desktop)]
#[derive(Serialize, Clone)]
struct AgentGrantClosed<'a> {
    id: &'a str,
}

#[cfg(desktop)]
pub fn agent_grant_closed(app: &AppHandle, id: &str) {
    let _ = app.emit(AGENT_GRANT_CLOSED, AgentGrantClosed { id });
}

/// A rule or a grant came or went behind Settings' back — a dialog answered
/// "for N days", a grant expired, an exposure ended one. No payload: Settings
/// › Browser & agents re-reads `agent_rules` and `agent_grants`.
#[cfg(desktop)]
pub fn agents_changed(app: &AppHandle) {
    let _ = app.emit(AGENTS_CHANGED, ());
}

#[cfg(desktop)]
#[derive(Serialize, Clone)]
struct OrgConnect<'a> {
    url: &'a str,
    status: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'a str>,
}

/// Where the sign-in to the Rowel Server at `url` stands (`org::connect`):
/// `pending` while the browser has it, `done` once the install is signed in
/// and this device enrolled, or `error` with the reason. The command that
/// starts it returns as soon as the browser is open, so this is the one way
/// its outcome reaches the Settings section that asked.
#[cfg(desktop)]
pub fn org_connect(app: &AppHandle, url: &str, status: &str, error: Option<&str>) {
    let _ = app.emit(ORG_CONNECT, OrgConnect { url, status, error });
}

/// Something about the install's Rowel Servers changed behind the Settings
/// section — a sign-in landed, a device was approved, a lockbox was added,
/// a server was forgotten. No payload: the section re-reads
/// `org_connections`, which is the one answer it draws from.
pub fn org_changed(app: &AppHandle) {
    let _ = app.emit(ORG_CHANGED, ());
}
