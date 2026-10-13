//! Google Drive sync provider. Finds (or creates) `Rowel/Lockboxes/` (read as
//! `Rowel/Vaults/` on accounts from before the rename) and the
//! `<vault-id>.rowel` pack inside it, and reads/writes that one file. Every
//! name it uses comes from [`layout`].
//!
//! Which pack is *this* vault's is settled once per run, before the engine
//! starts: see [`resolve_vault_id`].
//!
//! The id that step settles on reaches the vault's `meta` table in the middle of
//! the run it was settled for (see [`run`]), never at either end: after the
//! remote pack has been fetched, decoded and merged — until then nothing has
//! proved it is this vault's — and before anything is packed or uploaded, so
//! Drive is never changed under an id this vault does not yet answer to.
//!
//! This module is only the *transport*: the sync algorithm lives in [`engine`],
//! behind the [`engine::Remote`] trait that [`DriveRemote`] implements. Drive's
//! REST calls are async and are driven with `block_on` here, which is safe
//! because a run always executes on its own thread (see `commands::sync`) —
//! never on a command thread, and never on a runtime worker.

mod auth;
// Crate-visible: `share::remote` drives the same Drive REST surface.
pub(crate) mod drive;
pub mod engine;
// Every name Drive sees: folders, file names, extensions. Crate-visible so
// sharing and the backup export spell them the same way.
pub mod layout;
pub mod pack;
pub mod restore;
// First-run onboarding's keyless view of the account. Named for the flow, not
// for `setup` below — modules and functions live in separate namespaces.
pub mod setup;

use std::collections::HashSet;
use std::future::Future;
use std::sync::Mutex;
use std::time::Duration;

use reqwest::Client;
use tauri::{async_runtime::block_on, AppHandle, Manager};

use crate::crypto::Cryptor;
use crate::error::{Error, Result};
use crate::state::AppState;
use engine::{LocalVault, Remote, RemoteFile, SessionVault, SyncOutcome};

/// A valid Drive access token for the connected account, refreshed if needed.
/// Crate-visible so `share::remote` can act on the same account.
pub(crate) async fn access_token(
    client: &Client,
    app: &AppHandle,
    cryptor: &Cryptor,
) -> Result<String> {
    auth::access_token(client, app, cryptor).await
}

/// The OAuth tokens for one account, as onboarding holds them: in memory, for
/// the stretch between the user approving consent and a vault key existing to
/// seal them under.
pub(crate) use auth::Tokens;

/// Seal `tokens` under `cryptor` and write them where a later unlock will find
/// them — the step [`setup`] does for itself and onboarding defers.
pub(crate) fn persist_tokens(app: &AppHandle, cryptor: &Cryptor, tokens: &Tokens) -> Result<()> {
    auth::write_tokens(app, cryptor, tokens)
}

/// The open workspace's account, unsealed, with the connection generation it
/// belongs to — `None` when it has none. What a workspace made or restored
/// beside this one is given, so that one account connected once reaches every
/// vault on the device without a second sign-in.
///
/// The two are read as one step under the guard a disconnect bumps and deletes
/// under, for a caller that will refresh the tokens across round trips of its
/// own and write them back with [`persist_tokens_if_current`]. Read apart, a
/// disconnect landing between the reads would hand back the dropped
/// credentials under the generation that replaced them, and the write-back
/// would undo it. The same pairing `auth::access_token` makes for the sync
/// engine's own refresh.
pub(crate) fn current_account(app: &AppHandle, cryptor: &Cryptor) -> Option<(Tokens, u64)> {
    let state = app.state::<AppState>();
    let generation = state.sync_generation.lock().unwrap();
    let tokens = auth::read_tokens(app, cryptor)?;
    Some((tokens, *generation))
}

/// [`persist_tokens`] into the workspace directory `dir` rather than the active
/// workspace's — for a vault added beside the open one while its paths stay put
/// (`commands::autojoin`), and for one re-keyed beside it by a master-password
/// change, whose tokens come out sealed under its new key.
pub(crate) fn persist_tokens_in(
    dir: &std::path::Path,
    cryptor: &Cryptor,
    tokens: &Tokens,
) -> Result<()> {
    auth::write_tokens_in(dir, cryptor, tokens)
}

/// The account sealed in the workspace directory `dir`, unsealed with that
/// workspace's own key — `Ok(None)` when it has no token file at all and
/// therefore does not sync, and an error when it has one that cannot be read.
///
/// No connection generation, unlike [`current_account`]: the caller of this is
/// a delete, which spends the tokens once and never writes them back.
pub(crate) fn read_tokens_in(dir: &std::path::Path, cryptor: &Cryptor) -> Result<Option<Tokens>> {
    auth::read_tokens_in(dir, cryptor)
}

/// [`persist_tokens`], unless a disconnect has ended the connection `generation`
/// names since it was read — in which case nothing is written and `false` comes
/// back. The guard every write-back of refreshed tokens takes (see
/// `AppState::sync_generation`), for a caller that refreshed the open
/// workspace's tokens across a network round trip of its own.
pub(crate) fn persist_tokens_if_current(
    app: &AppHandle,
    cryptor: &Cryptor,
    tokens: &Tokens,
    generation: u64,
) -> Result<bool> {
    auth::persisted_if_current(app, cryptor, tokens, generation)
}

/// Which Drive connection is current — see [`AppState::sync_generation`]. Read
/// before a network round trip whose refreshed tokens may be written back, so
/// [`persist_tokens_if_current`] can reject a result from a disconnected account.
pub(crate) fn connection_generation(app: &AppHandle) -> u64 {
    *app.state::<AppState>().sync_generation.lock().unwrap()
}

/// A valid access token for in-memory `tokens`, refreshed in place if expired.
pub(crate) async fn fresh_access_token(
    client: &Client,
    app: &AppHandle,
    tokens: &mut Tokens,
) -> Result<String> {
    auth::fresh_access_token(client, app, tokens).await
}

/// The desktop consent flow without the persisting: onboarding's half of
/// [`setup`], for an install that has no vault key yet.
#[cfg(desktop)]
pub(crate) fn obtain_tokens(app: &AppHandle) -> Result<Tokens> {
    auth::obtain_tokens(app)
}

/// The mobile twin of [`obtain_tokens`]: redeem a code the deep-link handler
/// accepted, and hand the tokens back rather than writing them.
#[cfg(mobile)]
pub(crate) use auth::exchange_for_tokens;

/// Install the ring rustls provider, once per process.
///
/// reqwest is built with `rustls-no-provider`, and building a `Client` before a
/// provider is installed is a panic, not an error. Our own clients all come
/// through [`http_client`], which installs first — but on iOS and Android Tauri
/// itself builds a reqwest client during launch (the `tauri://` protocol proxies
/// the dev server through one, see `tauri/src/protocol/tauri.rs`), so `run()`
/// has to install before `tauri::Builder` ever runs. Idempotent.
pub(crate) fn install_crypto_provider() {
    static ONCE: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    ONCE.get_or_init(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// How long a connection may take to come up, and how long a response may go
/// silent mid-body, before the request fails. Deadlines on the *stall*, not on
/// the whole request: a pack upload on a slow link legitimately takes minutes,
/// but a peer that stops answering must not hold a run — and with it the
/// `syncing` flag, `sync_now` and every workspace switch — open until the
/// process exits. Callers that know their body is small add a whole-request
/// `timeout` on top (favicons, shares).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const READ_TIMEOUT: Duration = Duration::from_secs(60);

/// The builder every HTTPS client in the app starts from: provider installed
/// (see [`install_crypto_provider`] for why that comes first), deadlines set.
/// For a caller that needs one more setting — a redirect policy, a total
/// timeout — on top of the shared ones.
pub(crate) fn http_client_builder() -> reqwest::ClientBuilder {
    install_crypto_provider();
    Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(READ_TIMEOUT)
}

// Build the shared HTTPS client.
pub(crate) fn http_client() -> Client {
    // Same failure mode as `Client::new`, which also panics: the builder only
    // fails when the TLS backend cannot be set up, and nothing in this app
    // works without it.
    http_client_builder()
        .build()
        .expect("the HTTPS client could not be built")
}

pub fn is_configured(app: &AppHandle, cryptor: &Cryptor) -> bool {
    auth::is_configured(app, cryptor)
}

// The mobile consent flow, cut in two around the browser hand-off. See
// `auth.rs`; the halves are joined by the deep-link handler in `lib.rs`, and
// the second half is `exchange_for_tokens` — every connect is keyless now, so
// the tokens are handed back rather than written.
#[cfg(mobile)]
pub use auth::{begin, parse_redirect, redirect_matches, Redirect};

/// Drop the account locally. An error means the token file is still on disk
/// and the account is therefore still connected.
pub fn disconnect(app: &AppHandle) -> Result<()> {
    auth::disconnect(app)
}

/// Re-seal the stored tokens under a new vault key (a password change). A
/// missing token file is a no-op, not an error.
pub fn reseal_tokens(app: &AppHandle, old: &Cryptor, new: &Cryptor) -> Result<()> {
    auth::reseal_tokens(app, old, new)
}

/// One full sync against Drive. Blocking: call it on a dedicated thread.
pub fn run(app: &AppHandle, cryptor: Cryptor) -> Result<SyncOutcome> {
    if !is_configured(app, &cryptor) {
        return Err(Error::SyncNotConfigured);
    }
    let local = SessionVault::capture(app)?;
    // Which pack on Drive is this vault's, decided before a byte is pulled. A
    // failure here is a sync failure like any other: addressing the wrong pack
    // — or a second one — is the one mistake the merge cannot undo.
    let resolved = block_on(resolve_vault_id(app, &cryptor, &local))?;
    let remote = DriveRemote::new(app.clone(), cryptor, resolved.id.clone());

    // The id is written from inside the run, at the one moment that satisfies
    // both of the things this ordering has to guarantee (see [`engine::sync`],
    // which decides when that moment is):
    //
    // * Nothing before the pull has been fetched, decoded and merged: a run
    //   that cannot reach or read the pack it was pointed at leaves this vault
    //   answering to the id it already had.
    // * Nothing after the first push. Written afterwards, a vault that locked
    //   between the upload and the write would have changed Drive without
    //   changing itself: an id-less vault would have published `<new-id>.rowel`
    //   and would mint a *different* id on its next run.
    //
    // A `settle` that fails therefore aborts the run before anything is
    // uploaded, and the account is left as the run found it.
    //
    // Because the write lands before `pack()`, the first pack a freshly named
    // vault pushes already names its own id inside `meta`. The file name names
    // it too, which is what addresses it on Drive, and a restore still takes
    // the id from there (`restore::adopt_vault_id_from_name`) — belt and braces
    // for packs written before this ordering existed.
    let outcome = engine::sync(&remote, &local, now_ms(), || {
        if resolved.persist {
            local.adopt_vault_id(&resolved.id)?;
        }
        // The registry's copy, for a Drive restore to check against once this
        // workspace is locked (`workspace::record_vault_id`). Best effort: the
        // vault's own record above is the one that addresses the pack, and a
        // registry the next run will write again is not worth failing this one.
        if let Err(e) = crate::workspace::record_vault_id(app, &resolved.id) {
            log::warn!("could not record the vault id in the registry: {e}");
        }
        Ok(())
    })?;

    if outcome.renamed {
        adopt_name(app, &local);
    }
    publish_remote_vaults(app, &resolved.id, resolved.packs);
    sweep_shares(app);
    Ok(outcome)
}

/// The run pulled a newer name for this vault. The vault's own copy is already
/// written (the engine did it); this puts it in the registry too, which is what
/// `app_status` reports and the header and Settings list draw from, and tells
/// the frontend to re-read it.
///
/// Best effort, like [`workspace::record_vault_id`] beside it: a registry the
/// next run will write again is not worth failing this one.
fn adopt_name(app: &AppHandle, local: &impl LocalVault) {
    let Ok((Some(name), _)) = local.name() else {
        return;
    };
    if let Err(e) = crate::workspace::record_vault_name(app, &name) {
        log::warn!("could not record the vault name in the registry: {e}");
        return;
    }
    crate::events::workspace_renamed(app);
}

/// Tell the frontend which of the account's vaults are not on this device, so
/// Settings › Workspaces can offer to add them. Every device connected to an
/// account is meant to hold every vault in it; this is how a vault made on one
/// device shows up on the others.
///
/// Best effort, after a run that has just proved the account reachable: the
/// registry failing to load is not a sync failure, and the next run says again.
fn publish_remote_vaults(app: &AppHandle, own_id: &str, packs: Vec<setup::PackInfo>) {
    let Ok(root) = crate::storage::root_dir(app) else {
        return;
    };
    // Every vault a workspace here holds, plus this one's — which the registry
    // has just been told about, but which the packs were listed before the
    // first push created, so it is named here rather than left to that.
    let held: HashSet<String> = crate::workspace::Registry::load(&root)
        .workspaces
        .into_iter()
        .filter_map(|w| w.vault_id)
        .chain(std::iter::once(own_id.to_string()))
        .collect();
    crate::events::remote_vaults(app, remote_only(packs, &held));
}

/// The packs among `packs` that no workspace on this device holds, in the order
/// the listing gave them (newest first).
pub(crate) fn remote_only(
    packs: Vec<setup::PackInfo>,
    held: &HashSet<String>,
) -> Vec<setup::PackInfo> {
    packs
        .into_iter()
        .filter(|pack| !held.contains(&pack.vault_id))
        .collect()
}

/// Drop expired one-time shares, riding along on a run that has just proved the
/// account reachable. Best effort: a locked vault or a failed listing is not a
/// sync failure, and the next run sweeps again.
fn sweep_shares(app: &AppHandle) {
    // `run` moved its own cryptor into the remote and `Cryptor` is not `Clone`,
    // so take a second from the session — which is also how we notice the vault
    // locked while the sync was in flight.
    let Ok(cryptor) = app.state::<AppState>().session.lock().unwrap().cryptor() else {
        return;
    };
    if let Err(e) = crate::share::sweep_drive(app, cryptor) {
        log::warn!("share sweep failed: {e}");
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

// --- which pack is ours -----------------------------------------------------

/// What the vault-id resolution has to do before a run can address a pack.
///
/// Split out from the Drive calls so the decision — the part with the first-run
/// case in it — is a pure function over two facts and can be read, and tested,
/// on its own.
#[derive(Debug, Clone, PartialEq, Eq)]
enum VaultIdPlan {
    /// Address `<id>.rowel`. Either it is already there, or the first push
    /// creates it.
    Use(String),
    /// Mint an id: the account holds no vault yet, so this one is its first.
    Assign,
    /// A vault that has never synced, facing an account that already holds
    /// vaults. Minting here is what forked a user's data across two packs when
    /// they meant one, and a pack cannot be taken on from here either — it is
    /// sealed under its own key, which only a restore has the password for.
    Refuse,
}

/// Decide which vault id this run addresses.
///
/// `local` is the id in this vault's `meta` table and `live` the ids of the
/// packs found in the packs folder (an absent folder is simply none of them).
///
/// The rule underneath the arms: the account is the source of truth for which
/// vaults exist. A vault that already has a name keeps it; one that has none
/// gets one only when the account has nothing — every device connected to an
/// account syncs the vaults it holds rather than adding to them.
fn plan_vault_id(local: Option<&str>, live: &[&str]) -> VaultIdPlan {
    match (local, live) {
        // Its own name is the answer, whatever else the account holds.
        (Some(id), _) => VaultIdPlan::Use(id.to_string()),
        (None, []) => VaultIdPlan::Assign,
        (None, _) => VaultIdPlan::Refuse,
    }
}

// Surfaced verbatim by the sync indicator, so it has to read as a sentence.
// Ordinarily unreachable: a connect on a vault that has never synced goes
// through the account's vaults first (`commands::sync::sync_connect`) and
// restores one of them instead of syncing this vault to the account.
fn account_has_vaults_error() -> String {
    format!(
        "this Google account already holds a {} vault; disconnect, then connect again to \
         restore it on this device instead of syncing a second vault beside it",
        crate::app::APP_NAME
    )
}

/// The vault id a run addresses, and whether the local vault has yet to be told
/// about it.
struct Resolved {
    /// The id every file name in this run is built from.
    id: String,
    /// Whether `meta` still has to be given `id` — a minted id, as opposed to a
    /// vault that already knew its own name. Deliberately *not* written by
    /// [`resolve_vault_id`]: see [`run`], which hands the write to the engine
    /// to make mid-run, once the pull has proved the id and before any push can
    /// act on it.
    persist: bool,
    /// Every live pack the account held when the run began — the listing the
    /// decision was made from, kept for [`publish_remote_vaults`] so the run
    /// does not list the folder twice.
    packs: Vec<setup::PackInfo>,
}

/// Settle the vault id for this run, doing whatever Drive work [`plan_vault_id`]
/// calls for.
///
/// The *local* vault is not touched here at all — an id this function minted
/// lives in memory until the run's own pull has proved it (see [`run`]), which
/// is what keeps a failed run from leaving a vault pointed at a pack it never
/// read.
async fn resolve_vault_id(
    app: &AppHandle,
    cryptor: &Cryptor,
    local: &impl LocalVault,
) -> Result<Resolved> {
    let client = http_client();
    let token = auth::access_token(&client, app, cryptor).await?;

    // The same listing onboarding's probe makes: a missing `Rowel/` or packs
    // folder is an account nothing has ever synced to, which is the same
    // answer as an empty folder.
    let packs = setup::find_packs(&client, &token).await?;
    let live: Vec<&str> = packs.iter().map(|pack| pack.vault_id.as_str()).collect();

    let (id, persist) = match plan_vault_id(local.vault_id()?.as_deref(), &live) {
        VaultIdPlan::Use(id) => (id, false),
        VaultIdPlan::Assign => (crate::crypto::random_hex_id(), true),
        VaultIdPlan::Refuse => return Err(Error::Other(account_has_vaults_error())),
    };
    Ok(Resolved { id, persist, packs })
}

/// Take this vault out of the account: a marker goes where its pack is, and the
/// pack goes, so the other devices syncing it stop rather than push it back.
///
/// Both halves matter. Deleting the pack alone would leave every other device
/// seeing an account with no pack for a vault it holds — which reads as a first
/// sync, and the next run would upload the vault straight back. The marker is
/// what [`DriveRemote::fetch`] recognises; it is not a `.rowel` name, so no
/// listing counts it as a vault (see [`layout::deleted_marker_name`]).
///
/// The order is the invariant here, and it is the marker first. These are two
/// Drive calls with a window between them, and a run — on this device or
/// another — that looks into that window must never find "no pack, no marker":
/// that is exactly the first-sync reading that puts the pack back. A marker
/// beside a still-live pack is the harmless half-state instead:
/// [`DriveRemote::fetch`] only consults it when the pack is missing, and
/// [`may_create_pack`] stops a run that pulled before the delete from
/// re-creating one after it.
///
/// The folders are created when the account has none ([`ensure_vaults_path`]).
/// A vault whose id is settled has a pack up there or had one, and leaving an
/// unmarked empty account behind — `Rowel/` deleted by hand, say — is leaving
/// another device's ordinary push to recreate the folders and the pack. A pack
/// that is not there needs no deleting, and a marker already there is another
/// device having done this.
pub(crate) async fn delete_pack(
    app: &AppHandle,
    tokens: &mut Tokens,
    vault_id: &str,
) -> Result<()> {
    let client = http_client();
    let token = fresh_access_token(&client, app, tokens).await?;
    let (_, vaults) = ensure_vaults_path(&client, &token).await?;

    let marker = layout::deleted_marker_name(vault_id);
    if drive::find_file(&client, &token, &marker, &vaults)
        .await?
        .is_none()
    {
        drive::create_file(&client, &token, &marker, &vaults, &[]).await?;
    }

    let pack = layout::vault_file_name(vault_id);
    if let Some(file) = drive::find_file(&client, &token, &pack, &vaults).await? {
        drive::delete_file(&client, &token, &file.id).await?;
    }
    Ok(())
}

/// `Rowel/Lockboxes` (or the legacy `Rowel/Vaults`), with either level created
/// if this account has never had it. The root id comes back too, so a caller
/// that caches the pair ([`DriveRemote::ensure_vaults`]) does not list it a
/// second time.
async fn ensure_vaults_path(client: &Client, token: &str) -> Result<(String, String)> {
    let root = match drive::folder_id(client, token, layout::ROOT_FOLDER).await? {
        Some(id) => id,
        None => drive::create_folder(client, token, layout::ROOT_FOLDER).await?,
    };
    let vaults = ensure_packs_folder(
        |name| drive::folder_id_in(client, token, name, &root),
        |name| drive::create_folder_in(client, token, name, Some(&root)),
    )
    .await?;
    Ok((root, vaults))
}

/// The folder inside `Rowel/` (`root`) that holds the packs, if the account has
/// one. Every lookup of the packs folder goes through here, so every device
/// settles on the same one: see [`find_packs_folder`] for the rule.
async fn packs_folder_in(client: &Client, token: &str, root: &str) -> Result<Option<String>> {
    find_packs_folder(|name| drive::folder_id_in(client, token, name, root)).await
}

/// The first of [`layout::PACKS_FOLDERS`] that `lookup` finds: `Lockboxes/`,
/// else the legacy `Vaults/`, else none.
///
/// Reading `Vaults/` is what keeps an account from before the rename syncing
/// where it always has, and where an older client on another device still
/// looks. When both exist `Lockboxes/` wins, deterministically, so two devices
/// never split across them. `lookup` is the Drive call, injected so the rule is
/// testable without one.
async fn find_packs_folder<F, Fut>(mut lookup: F) -> Result<Option<String>>
where
    F: FnMut(&'static str) -> Fut,
    Fut: Future<Output = Result<Option<String>>>,
{
    for name in layout::PACKS_FOLDERS {
        if let Some(id) = lookup(name).await? {
            return Ok(Some(id));
        }
    }
    Ok(None)
}

/// [`find_packs_folder`], creating [`layout::LOCKBOXES_FOLDER`] when the account
/// has neither. Never creates one beside an existing `Vaults/`: that account
/// keeps syncing into it, with no re-upload and nothing for an older client to
/// miss.
async fn ensure_packs_folder<F, Fut, C, CFut>(lookup: F, create: C) -> Result<String>
where
    F: FnMut(&'static str) -> Fut,
    Fut: Future<Output = Result<Option<String>>>,
    C: FnOnce(&'static str) -> CFut,
    CFut: Future<Output = Result<String>>,
{
    match find_packs_folder(lookup).await? {
        Some(id) => Ok(id),
        None => create(layout::LOCKBOXES_FOLDER).await,
    }
}

/// [`Remote`] over one vault's pack, `Rowel/Lockboxes/<vault-id>.rowel` (or
/// under the legacy `Rowel/Vaults/`, see [`find_packs_folder`]).
///
/// Both folder ids are resolved once per instance (one instance per run)
/// because the selection rule is deterministic and re-listing them would only
/// cost round trips. The *file* is re-listed on every call: `head_revision`
/// exists precisely to observe a change another device made, so it must never
/// answer from a cache.
struct DriveRemote {
    app: AppHandle,
    cryptor: Cryptor,
    /// Settled by [`resolve_vault_id`] before the run starts, so every call
    /// below knows the file name it is after without asking again.
    vault_id: String,
    root: Mutex<Option<String>>,
    vaults: Mutex<Option<String>>,
}

impl DriveRemote {
    fn new(app: AppHandle, cryptor: Cryptor, vault_id: String) -> Self {
        Self {
            app,
            cryptor,
            vault_id,
            root: Mutex::new(None),
            vaults: Mutex::new(None),
        }
    }

    // Resolve the Rowel folder, remembering it for the rest of the run.
    // `None` means it does not exist yet — which is also "no remote vault".
    async fn root(&self, client: &Client, token: &str) -> Result<Option<String>> {
        if let Some(id) = self.root.lock().unwrap().clone() {
            return Ok(Some(id));
        }
        let found = drive::folder_id(client, token, layout::ROOT_FOLDER).await?;
        if let Some(id) = &found {
            *self.root.lock().unwrap() = Some(id.clone());
        }
        Ok(found)
    }

    // The packs folder (`Rowel/Lockboxes`, or the legacy `Rowel/Vaults`), the
    // same way.
    async fn vaults(&self, client: &Client, token: &str) -> Result<Option<String>> {
        if let Some(id) = self.vaults.lock().unwrap().clone() {
            return Ok(Some(id));
        }
        let Some(root) = self.root(client, token).await? else {
            return Ok(None);
        };
        let found = packs_folder_in(client, token, &root).await?;
        if let Some(id) = &found {
            *self.vaults.lock().unwrap() = Some(id.clone());
        }
        Ok(found)
    }

    // [`vaults`], for the push: both folders are created when this is the first
    // thing this account has ever had put in it.
    async fn ensure_vaults(&self, client: &Client, token: &str) -> Result<String> {
        if let Some(id) = self.vaults(client, token).await? {
            return Ok(id);
        }
        let (root, vaults) = ensure_vaults_path(client, token).await?;
        *self.root.lock().unwrap() = Some(root);
        *self.vaults.lock().unwrap() = Some(vaults.clone());
        Ok(vaults)
    }

    async fn locate(&self, client: &Client, token: &str) -> Result<Option<drive::DriveFile>> {
        let Some(vaults) = self.vaults(client, token).await? else {
            return Ok(None);
        };
        let name = layout::vault_file_name(&self.vault_id);
        drive::find_file(client, token, &name, &vaults).await
    }

    // Is there a marker where this vault's pack would be? Asked only when the
    // pack is missing, so an ordinary run never pays for it.
    async fn marker(&self, client: &Client, token: &str) -> Result<bool> {
        let Some(vaults) = self.vaults(client, token).await? else {
            return Ok(false);
        };
        let name = layout::deleted_marker_name(&self.vault_id);
        Ok(drive::find_file(client, token, &name, &vaults)
            .await?
            .is_some())
    }

    async fn token(&self, client: &Client) -> Result<String> {
        auth::access_token(client, &self.app, &self.cryptor).await
    }
}

/// What a run makes of finding no pack for its vault.
///
/// With no marker beside where the pack would be, this is simply a vault whose
/// first push has not happened yet, and the engine treats the account as empty.
/// With one, another device deleted this vault everywhere ([`delete_pack`]) —
/// and "the account is empty" would have this run upload the pack again and undo
/// that delete on every device, so the run stops instead.
fn pack_missing(marker: bool) -> Result<Option<RemoteFile>> {
    if marker {
        return Err(Error::VaultDeletedRemotely);
    }
    Ok(None)
}

/// Whether a push may *create* this vault's pack, given whether a deletion
/// marker sits where it would go.
///
/// A run that pulled before a [`delete_pack`] and pushes after it would
/// otherwise put the pack back — this run read an account that still held the
/// vault, so nothing earlier in it has any reason to stop. The marker is the
/// one thing that outlives the pack, so the create path asks for it.
///
/// Only the create path. Finding the pack there means the delete has not
/// reached it (or never will), and an update is the ordinary push every run
/// makes — it pays no extra listing for this.
fn may_create_pack(marker: bool) -> Result<()> {
    if marker {
        return Err(Error::VaultDeletedRemotely);
    }
    Ok(())
}

impl Remote for DriveRemote {
    fn fetch(&self) -> Result<Option<RemoteFile>> {
        block_on(async {
            let client = http_client();
            let token = self.token(&client).await?;
            let Some(file) = self.locate(&client, &token).await? else {
                return pack_missing(self.marker(&client, &token).await?);
            };
            let bytes = drive::read_file(&client, &token, &file.id, pack::MAX_PACK_BYTES).await?;
            Ok(Some(RemoteFile {
                bytes,
                revision: file.head_revision.unwrap_or_default(),
            }))
        })
    }

    fn head_revision(&self) -> Result<Option<String>> {
        block_on(async {
            let client = http_client();
            let token = self.token(&client).await?;
            let Some(file) = self.locate(&client, &token).await? else {
                return Ok(None);
            };
            // Read from the listing, exactly like `fetch` — the pre-flight
            // compares this against `fetch`'s value, so the two must degrade
            // identically. A `files.get` fallback here once made a missing
            // listing field compare `Some(real)` against `fetch`'s `Some("")`,
            // which would burn every retry and fail the run.
            Ok(Some(file.head_revision.unwrap_or_default()))
        })
    }

    fn upload(&self, bytes: &[u8]) -> Result<String> {
        block_on(async {
            let client = http_client();
            let token = self.token(&client).await?;
            let vaults = self.ensure_vaults(&client, &token).await?;
            let name = layout::vault_file_name(&self.vault_id);
            let revision = match drive::find_file(&client, &token, &name, &vaults).await? {
                Some(file) => drive::update_file(&client, &token, &file.id, bytes).await?,
                None => {
                    may_create_pack(self.marker(&client, &token).await?)?;
                    drive::create_file(&client, &token, &name, &vaults, bytes)
                        .await?
                        .head_revision
                }
            };
            Ok(revision.unwrap_or_default())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ensure_packs_folder, find_packs_folder, layout, may_create_pack, pack_missing,
        plan_vault_id, remote_only, setup::PackInfo, Error, VaultIdPlan,
    };
    use std::cell::RefCell;
    use std::collections::HashSet;

    // An account's folders under `Rowel/`, as `folder_id_in` would find them:
    // the folder's own name doubles as its id.
    fn lookup_in(
        folders: &'static [&'static str],
    ) -> impl FnMut(&'static str) -> std::future::Ready<super::Result<Option<String>>> {
        move |name| std::future::ready(Ok(folders.contains(&name).then(|| name.to_string())))
    }

    #[tokio::test]
    async fn a_fresh_layout_account_uses_lockboxes() {
        let found = find_packs_folder(lookup_in(&[layout::LOCKBOXES_FOLDER])).await;
        assert_eq!(found.unwrap().as_deref(), Some("Lockboxes"));
    }

    // An account from before the rename keeps syncing where it always has —
    // and where an older client on another device still looks.
    #[tokio::test]
    async fn an_account_from_before_the_rename_reads_vaults() {
        let found = find_packs_folder(lookup_in(&[layout::LEGACY_VAULTS_FOLDER])).await;
        assert_eq!(found.unwrap().as_deref(), Some("Vaults"));
    }

    #[tokio::test]
    async fn with_both_folders_lockboxes_wins() {
        let both = &[layout::LEGACY_VAULTS_FOLDER, layout::LOCKBOXES_FOLDER];
        let found = find_packs_folder(lookup_in(both)).await;
        assert_eq!(found.unwrap().as_deref(), Some("Lockboxes"));
    }

    #[tokio::test]
    async fn an_account_with_neither_has_no_packs_folder() {
        assert_eq!(find_packs_folder(lookup_in(&[])).await.unwrap(), None);
    }

    #[tokio::test]
    async fn only_an_account_with_neither_folder_gets_one_created_and_it_is_lockboxes() {
        let created = RefCell::new(Vec::new());
        let create = |name: &'static str| {
            created.borrow_mut().push(name);
            std::future::ready(Ok(format!("new-{name}")))
        };
        let id = ensure_packs_folder(lookup_in(&[]), create).await.unwrap();
        assert_eq!(id, "new-Lockboxes");
        assert_eq!(*created.borrow(), ["Lockboxes"]);

        // An existing `Vaults/` is used as is: nothing is created beside it.
        let create = |name: &'static str| {
            created.borrow_mut().push(name);
            std::future::ready(Ok(format!("new-{name}")))
        };
        let id = ensure_packs_folder(lookup_in(&[layout::LEGACY_VAULTS_FOLDER]), create)
            .await
            .unwrap();
        assert_eq!(id, "Vaults");
        assert_eq!(created.borrow().len(), 1);
    }

    const ID: &str = "a1b2c3";
    const OTHER: &str = "dddd";

    fn pack(vault_id: &str) -> PackInfo {
        PackInfo {
            id: format!("file-{vault_id}"),
            name: format!("{vault_id}.rowel"),
            vault_id: vault_id.into(),
            size: 0,
            modified_time: String::new(),
        }
    }

    // The vaults offered to add are exactly the account's packs no workspace
    // here holds — this vault's own included among the held, since its pack
    // may not have existed when the folder was listed.
    #[test]
    fn only_packs_no_workspace_here_holds_are_offered() {
        let held: HashSet<String> = [ID.to_string(), "eeee".to_string()].into();
        let offered = remote_only(
            vec![pack(OTHER), pack(ID), pack("eeee"), pack("ffff")],
            &held,
        );
        let ids: Vec<&str> = offered.iter().map(|p| p.vault_id.as_str()).collect();
        assert_eq!(ids, [OTHER, "ffff"]);
        assert!(remote_only(vec![pack(ID)], &held).is_empty());
    }

    #[test]
    fn a_vault_that_knows_its_id_simply_uses_it() {
        // Its pack is there; and if it is not, the first push creates it.
        assert_eq!(plan_vault_id(Some(ID), &[ID]), VaultIdPlan::Use(ID.into()));
        assert_eq!(plan_vault_id(Some(ID), &[]), VaultIdPlan::Use(ID.into()));
        // Another vault's pack in the folder is none of this one's business.
        assert_eq!(
            plan_vault_id(Some(ID), &[OTHER, "eeee"]),
            VaultIdPlan::Use(ID.into())
        );
    }

    // The first device: nothing up there, so this vault is the account's first.
    #[test]
    fn a_vault_that_has_never_synced_mints_an_id_into_an_empty_account() {
        assert_eq!(plan_vault_id(None, &[]), VaultIdPlan::Assign);
    }

    // Every later device: the account already says which vaults exist, and a
    // vault with no name of its own does not add to them. However many packs
    // are up there, the answer is to restore one, never to mint beside them.
    #[test]
    fn a_vault_that_has_never_synced_never_mints_beside_an_existing_vault() {
        assert_eq!(plan_vault_id(None, &[OTHER]), VaultIdPlan::Refuse);
        assert_eq!(plan_vault_id(None, &[OTHER, "eeee"]), VaultIdPlan::Refuse);
    }

    // No pack is the first push not having happened yet — unless the marker
    // says the vault was deleted from the account, in which case pushing is
    // exactly what must not happen.
    #[test]
    fn a_missing_pack_is_a_delete_only_when_the_marker_is_there() {
        assert!(matches!(pack_missing(false), Ok(None)));
        assert!(matches!(
            pack_missing(true),
            Err(Error::VaultDeletedRemotely)
        ));
    }

    // The other half of the same marker, on the way out: a run that pulled
    // before a "delete everywhere" and pushes after it finds no pack to update,
    // and creating one there would undo the delete on every device. The run
    // that simply has not pushed yet still may.
    #[test]
    fn a_push_may_not_create_a_pack_the_marker_says_was_deleted() {
        assert!(may_create_pack(false).is_ok());
        assert!(matches!(
            may_create_pack(true),
            Err(Error::VaultDeletedRemotely)
        ));
    }
}
