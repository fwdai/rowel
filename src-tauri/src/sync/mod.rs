//! Sync: one engine and one set of rules about vault identity, over whichever
//! cloud provider the workspace is connected to. Finds (or creates)
//! `Rowel/Vaults/` — read as `Rowel/Lockboxes/` once a later release has
//! renamed it, see [`store::pick_packs_folder`] — and the `<vault-id>.rowel`
//! pack inside it, and reads/writes that one file. Every name it uses comes
//! from [`layout`].
//!
//! Which pack is *this* vault's is settled once per run, before the engine
//! starts: see [`resolve_vault_id`].
//!
//! The id that step settles on reaches the vault's `meta` table in the middle of
//! the run it was settled for (see [`run`]), never at either end: after the
//! remote pack has been fetched, decoded and merged — until then nothing has
//! proved it is this vault's — and before anything is packed or uploaded, so
//! the account is never changed under an id this vault does not yet answer to.
//!
//! The sync algorithm lives in [`engine`], behind the [`engine::Remote`] trait
//! that [`store::PackRemote`] implements over any [`VaultStore`]. A provider is
//! an adapter under [`providers`] and nothing else; its REST calls are async
//! and are driven with `block_on`, which is safe because a run always executes
//! on its own thread (see `commands::sync`) — never on a command thread, and
//! never on a runtime worker.

// The OAuth flow and the sealed token file, generic over the provider.
pub(crate) mod account;
pub mod engine;
// Every name the account sees: folders, file names, extensions. Crate-visible
// so sharing and the backup export spell them the same way.
pub mod layout;
pub mod pack;
mod provider;
// One directory per provider. Crate-visible: `share` reaches Drive's share
// transport through it.
pub(crate) mod providers;
pub mod restore;
// First-run onboarding's keyless view of the account. Named for the flow, not
// for `setup` below — modules and functions live in separate namespaces.
pub mod setup;
pub(crate) mod store;

use std::collections::HashSet;
use std::time::Duration;

use reqwest::Client;
use tauri::{async_runtime::block_on, AppHandle, Manager};

use crate::crypto::Cryptor;
use crate::error::{Error, Result};
use crate::state::AppState;
pub(crate) use account::{sealed_provider, Auth};
use engine::{LocalVault, SessionVault, SyncOutcome};
pub(crate) use provider::Provider;
pub(crate) use store::{store_for, AnyStore, PackRemote, VaultStore};

/// A valid access token for the open workspace's account, refreshed if needed.
/// Crate-visible so a provider's share transport can act on the same account.
pub(crate) async fn access_token(
    client: &Client,
    app: &AppHandle,
    cryptor: &Cryptor,
) -> Result<String> {
    account::access_token(client, app, cryptor).await
}

/// The OAuth tokens for one account, as onboarding holds them: in memory, for
/// the stretch between the user approving consent and a vault key existing to
/// seal them under.
pub(crate) use account::Tokens;

/// Seal `tokens` under `cryptor` and write them where a later unlock will find
/// them — the step [`setup`] does for itself and onboarding defers.
pub(crate) fn persist_tokens(app: &AppHandle, cryptor: &Cryptor, tokens: &Tokens) -> Result<()> {
    account::write_tokens(app, cryptor, tokens)
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
/// would undo it. The same pairing `account::access_token` makes for the sync
/// engine's own refresh.
pub(crate) fn current_account(app: &AppHandle, cryptor: &Cryptor) -> Option<(Tokens, u64)> {
    let state = app.state::<AppState>();
    let generation = state.sync_generation.lock().unwrap();
    let tokens = account::read_tokens(app, cryptor)?;
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
    account::write_tokens_in(dir, cryptor, tokens)
}

/// The account sealed in the workspace directory `dir`, unsealed with that
/// workspace's own key — `Ok(None)` when it has no token file at all and
/// therefore does not sync, and an error when it has one that cannot be read.
///
/// No connection generation, unlike [`current_account`]: the caller of this is
/// a delete, which spends the tokens once and never writes them back.
pub(crate) fn read_tokens_in(dir: &std::path::Path, cryptor: &Cryptor) -> Result<Option<Tokens>> {
    account::read_tokens_in(dir, cryptor)
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
    account::persisted_if_current(app, cryptor, tokens, generation)
}

/// Which account connection is current — see [`AppState::sync_generation`].
/// Read before a network round trip whose refreshed tokens may be written back,
/// so [`persist_tokens_if_current`] can reject a result from a disconnected
/// account.
pub(crate) fn connection_generation(app: &AppHandle) -> u64 {
    *app.state::<AppState>().sync_generation.lock().unwrap()
}

/// A store over in-memory `tokens` — consent's result, or an account a caller
/// unsealed for itself — whose refreshes stay in memory. Hand the store to
/// [`AnyStore::into_tokens`] afterwards to keep them.
pub(crate) fn pending_store(app: &AppHandle, tokens: Tokens) -> AnyStore {
    store_for(Auth::pending(app, tokens))
}

/// The desktop consent flow without the persisting: onboarding's half of
/// [`setup`], for an install that has no vault key yet.
#[cfg(desktop)]
pub(crate) fn obtain_tokens(app: &AppHandle, provider: Provider) -> Result<Tokens> {
    account::obtain_tokens(app, provider)
}

/// The mobile twin of [`obtain_tokens`]: redeem a code the deep-link handler
/// accepted, and hand the tokens back rather than writing them.
#[cfg(mobile)]
pub(crate) use account::exchange_for_tokens;

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

/// Read a response body, failing with [`Error::FileTooLarge`] the moment it
/// exceeds `max_bytes` — before the excess is ever held in memory. The check
/// is on what arrives, not on `Content-Length`, so a body that lies about its
/// size (or sends none) is bounded the same. Crate-visible: every download in
/// the app has a cap, and this is the one loop that enforces one.
pub(crate) async fn read_capped(resp: &mut reqwest::Response, max_bytes: usize) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| Error::Other(e.to_string()))?
    {
        if body.len() + chunk.len() > max_bytes {
            return Err(Error::FileTooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
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
    account::is_configured(app, cryptor)
}

// The mobile consent flow, cut in two around the browser hand-off. See
// `account.rs`; the halves are joined by the deep-link handler in `lib.rs`, and
// the second half is `exchange_for_tokens` — every connect is keyless now, so
// the tokens are handed back rather than written.
#[cfg(mobile)]
pub use account::{begin, cancel_matches, parse_redirect, redirect_matches, Redirect};

/// Drop the account locally. An error means the token file is still on disk
/// and the account is therefore still connected.
pub fn disconnect(app: &AppHandle) -> Result<()> {
    account::disconnect(app)
}

/// Re-seal the stored tokens under a new vault key (a password change). A
/// missing token file is a no-op, not an error.
pub fn reseal_tokens(app: &AppHandle, old: &Cryptor, new: &Cryptor) -> Result<()> {
    account::reseal_tokens(app, old, new)
}

/// One full sync against the workspace's account. Blocking: call it on a
/// dedicated thread.
pub fn run(app: &AppHandle, cryptor: Cryptor) -> Result<SyncOutcome> {
    if !is_configured(app, &cryptor) {
        return Err(Error::SyncNotConfigured);
    }
    // One store for the whole run: the resolution below and every pull and push
    // after it share its folder lookups.
    let store = store_for(Auth::Sealed {
        app: app.clone(),
        cryptor,
    });
    let local = SessionVault::capture(app, store.provider())?;
    // Which pack in the account is this vault's, decided before a byte is
    // pulled. A failure here is a sync failure like any other: addressing the
    // wrong pack — or a second one — is the one mistake the merge cannot undo.
    let resolved = block_on(resolve_vault_id(&store, &local))?;
    let remote = PackRemote::new(store, resolved.id.clone());

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
/// sync failure, and the next run sweeps again. A provider without shares has
/// nothing to sweep.
fn sweep_shares(app: &AppHandle) {
    // `run` moved its own cryptor into the store and `Cryptor` is not `Clone`,
    // so take a second from the session — which is also how we notice the vault
    // locked while the sync was in flight.
    let Ok(cryptor) = app.state::<AppState>().session.lock().unwrap().cryptor() else {
        return;
    };
    if let Err(e) = crate::share::sweep_account(app, cryptor) {
        log::warn!("share sweep failed: {e}");
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

// --- which pack is ours -----------------------------------------------------

/// What the vault-id resolution has to do before a run can address a pack.
///
/// Split out from the account calls so the decision — the part with the first-run
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
fn account_has_vaults_error(provider: Provider) -> String {
    format!(
        "this {} account already holds a {} vault; disconnect, then connect again to \
         restore it on this device instead of syncing a second vault beside it",
        provider.account_name(),
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

/// Settle the vault id for this run, doing whatever account work
/// [`plan_vault_id`] calls for.
///
/// The *local* vault is not touched here at all — an id this function minted
/// lives in memory until the run's own pull has proved it (see [`run`]), which
/// is what keeps a failed run from leaving a vault pointed at a pack it never
/// read.
async fn resolve_vault_id(store: &impl VaultStore, local: &impl LocalVault) -> Result<Resolved> {
    // The same listing onboarding's probe makes: a missing `Rowel/` or packs
    // folder is an account nothing has ever synced to, which is the same
    // answer as an empty folder.
    let packs = setup::find_packs(store).await?;
    let live: Vec<&str> = packs.iter().map(|pack| pack.vault_id.as_str()).collect();

    let (id, persist) = match plan_vault_id(local.vault_id()?.as_deref(), &live) {
        VaultIdPlan::Use(id) => (id, false),
        VaultIdPlan::Assign => (crate::crypto::random_hex_id(), true),
        VaultIdPlan::Refuse => {
            return Err(Error::Other(account_has_vaults_error(store.provider())))
        }
    };
    Ok(Resolved { id, persist, packs })
}

/// Take this vault out of the account: a marker goes where its pack is, and the
/// pack goes, so the other devices syncing it stop rather than push it back.
///
/// Both halves matter. Deleting the pack alone would leave every other device
/// seeing an account with no pack for a vault it holds — which reads as a first
/// sync, and the next run would upload the vault straight back. The marker is
/// what [`PackRemote::fetch`](engine::Remote::fetch) recognises; it is not a
/// `.rowel` name, so no listing counts it as a vault (see
/// [`layout::deleted_marker_name`]).
///
/// The order is the invariant here, and it is the marker first. These are two
/// calls to the account with a window between them, and a run — on this device
/// or another — that looks into that window must never find "no pack, no
/// marker": that is exactly the first-sync reading that puts the pack back. A
/// marker beside a still-live pack is the harmless half-state instead:
/// [`PackRemote`]'s fetch only consults it when the pack is missing, and its
/// create path (`store::may_create_pack`) stops a run that pulled before the
/// delete from re-creating one after it.
///
/// The folders are created when the account has none (the marker's create
/// makes them). A vault whose id is settled has a pack up there or had one, and
/// leaving an unmarked empty account behind — `Rowel/` deleted by hand, say — is
/// leaving another device's ordinary push to recreate the folders and the pack.
/// A pack that is not there needs no deleting, and a marker already there is
/// another device having done this.
pub(crate) async fn delete_pack(store: &impl VaultStore, vault_id: &str) -> Result<()> {
    let marker = layout::deleted_marker_name(vault_id);
    if store.stat(&marker).await?.is_none() {
        store.create(&marker, &[]).await?;
    }

    let pack = layout::vault_file_name(vault_id);
    if let Some(entry) = store.stat(&pack).await? {
        store.delete(&entry.handle).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        account_has_vaults_error, delete_pack, layout, plan_vault_id, remote_only, setup::PackInfo,
        store::fake::FakeStore, Provider, VaultIdPlan,
    };
    use std::collections::HashSet;

    const ID: &str = "a1b2c3";
    const OTHER: &str = "dddd";

    fn pack(vault_id: &str) -> PackInfo {
        PackInfo {
            id: format!("file-{vault_id}"),
            name: format!("{vault_id}.rowel"),
            vault_id: vault_id.into(),
            size: 0,
            modified_time: String::new(),
            provider: Provider::GoogleDrive,
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

    // "Delete everywhere" leaves the marker before it takes the pack: a run
    // looking in between must never find neither.
    #[tokio::test]
    async fn a_delete_marks_the_vault_before_it_removes_the_pack() {
        let store = FakeStore::default();
        store.put(&layout::vault_file_name(ID), b"pack");

        delete_pack(&store, ID).await.unwrap();

        assert_eq!(
            *store.log.lock().unwrap(),
            [
                format!("create {}", layout::deleted_marker_name(ID)),
                format!("delete {}", layout::vault_file_name(ID)),
            ]
        );
        assert!(!store.has(&layout::vault_file_name(ID)));
        assert!(store.has(&layout::deleted_marker_name(ID)));
    }

    // Another device got there first, or the pack never existed: nothing is
    // written twice and nothing fails.
    #[tokio::test]
    async fn a_delete_someone_else_already_made_changes_nothing() {
        let store = FakeStore::default();
        store.put(&layout::deleted_marker_name(ID), &[]);

        delete_pack(&store, ID).await.unwrap();

        assert!(store.log.lock().unwrap().is_empty());
    }

    // The sentence the sync indicator shows names the provider's account in the
    // words the Google copy always used.
    #[test]
    fn the_account_has_vaults_error_reads_as_it_always_did_for_google() {
        assert_eq!(
            account_has_vaults_error(Provider::GoogleDrive),
            format!(
                "this Google account already holds a {} vault; disconnect, then connect again \
                 to restore it on this device instead of syncing a second vault beside it",
                crate::app::APP_NAME
            )
        );
        assert!(account_has_vaults_error(Provider::Dropbox)
            .starts_with("this Dropbox account already holds a "));
    }
}
