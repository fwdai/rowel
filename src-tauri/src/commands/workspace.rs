//! Creating, restoring, switching, renaming and deleting workspaces.
//!
//! Only one workspace's database is ever open, so every command here starts by
//! clearing the session: switching *is* closing what is open and pointing the
//! paths somewhere else. The app, though, is open at its own level
//! (`crate::appkey`): one unlock holds every workspace's key, so a switch to a
//! workspace the ring holds opens the next database itself and hands back an
//! unlock's result. Only a switch to one it does not hold — a workspace that
//! has never been opened with its password on this device — ends as every
//! switch used to, announcing itself as the lock it is (`vault:locked`), which
//! is what re-probes `app_status` on the frontend and lands on that
//! workspace's lock screen. Creating one ends unlocked and has nothing to
//! announce.
//!
//! A workspace can also arrive from Drive rather than be made here. That flow
//! borrows onboarding's keyless connect (`commands::setup`) wholesale — the
//! tokens it leaves pending, the probe, the events — and differs only in where
//! the restored vault lands and in what has to be put back when it fails. Or
//! it arrives from the account the open workspace already syncs to, with no
//! sign-in at all: every device connected to an account is meant to hold every
//! vault in it, so the account's other vaults are offered after each sync
//! (`sync::publish_remote_vaults`) and restored with the open workspace's own
//! tokens. A workspace *made* on a connected device inherits the account the
//! same way, and syncs to it as a vault of its own.

use std::fs;
use std::path::Path;
use std::sync::atomic::Ordering;

use subtle::ConstantTimeEq;
use tauri::{AppHandle, State};
use zeroize::Zeroizing;

use crate::appkey;
use crate::auth::{self, LockoutState};
use crate::crypto::{KdfParams, VaultKey};
use crate::error::{Error, Result};
use crate::models::{EntryMetaDto, UnlockResult};
use crate::secure_store::KeyStore;
use crate::session::{list_metas, store_err, Lease};
use crate::state::AppState;
use crate::storage;
use crate::store::{identity, SqliteStore, StoreError};
use crate::sync::{self, restore};
use crate::workspace::{self, Registry, Workspace};

use super::blocking;
use super::setup::{self, begin_step, create_off_thread};

/// Close whatever is open and make `id` the workspace the app addresses.
///
/// With the target's key in the ring — the app-level unlock — its database is
/// opened here and the result is an unlock's, for the frontend to enter with.
/// Without one, the switch ends on that workspace's lock screen as it always
/// did, and `None` says so; the `vault:locked` it emits is what lands there.
///
/// A relaunch comes back to the workspace chosen: the choice is recorded in
/// the registry, not just in memory.
#[tauri::command]
pub async fn workspace_select(
    id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<UnlockResult>> {
    // The step a create holds while its Argon2 runs, and a master-password
    // change while it rewrites the sidecar: both write through the paths, so
    // neither may see them move underneath. Held to the end, so no second
    // switch lands while the open below is in flight.
    let _step = begin_step(&state)?;
    let root = storage::root_dir(&app)?;

    let material = {
        // Read under the lock too: every writer of the registry holds it, so
        // what is saved below is a change to the current file, never to a
        // stale copy.
        let _paths = state.workspace_lock.lock().unwrap();
        let mut registry = Registry::load(&root);
        if !registry.contains(&id) {
            return Err(Error::NotFound);
        }
        guard_sync_idle(&state)?;

        // Persist first: a save that fails leaves the session open and the
        // paths where they were, so a rejected switch changes nothing. Nothing
        // to record when the choice did not change, which on a single-workspace
        // install is the only case there is — so selecting the primary never
        // conjures a registry file for a user who has no second vault.
        if registry.active != id {
            // The workspace being left keeps its size on the lock screen, as a
            // lock would have recorded it: this switch clears the session
            // itself rather than going through one. Saved with the choice, in
            // the one write. Nothing to count from the lock screen, where the
            // lock already did.
            let leaving = state
                .session
                .lock()
                .unwrap()
                .store()
                .ok()
                .map(|s| s.count_live());
            if let Some(Ok(count)) = leaving {
                let left = registry.active.clone();
                workspace::count_into(&mut registry, &left, count);
            }
            registry.active = id.clone();
            registry.save(&root)?;
        }

        // Together, and in this order: a session outliving the switch would
        // hold one workspace's key against another's database. The ring is
        // untouched — the open workspace's key stays in it, which is what
        // lets a switch back open it without a prompt.
        let material = state.keyring.lock().unwrap().get(&id);
        state.session.lock().unwrap().clear();
        *state.active_workspace.lock().unwrap() = id.clone();
        material
    };
    // Ended like any other session: clipboard cleared, idle timer dropped.
    crate::session::sealed(&app);
    // Sync is per workspace, and what the frontend is holding belongs to the one
    // that just closed. Said now rather than left to the re-probe or the entry
    // below, so no frame shows another vault's "last synced" or another
    // vault's error.
    super::sync::switched(&app);

    let Some(material) = material else {
        return Ok(land_on_lock_screen(&app));
    };
    // The rollback every unlock runs first, for the one open that never passes
    // through an unlock: a password change re-keys every workspace that shares
    // the password, so a crash during one can leave *this* workspace's database
    // and descriptor disagreeing, and the ring's key opens neither of them
    // until the pre-change pair is back. Before the descriptor below is read,
    // since recovery decides whether there is one. Under the step held above,
    // as `auth::unlock` runs it under its own, so a change still in flight is
    // never read as an interrupted one.
    if let Err(e) = auth::recover_interrupted_rekey_in(&workspace::dir_of(&root, &id)) {
        log::warn!("could not roll back an interrupted password change for workspace {id}: {e}");
        return Ok(land_on_lock_screen(&app));
    }
    let key = VaultKey::from_material(material, storage::kdf_sidecar_path(&app)?.exists());
    let (key, store, entries) = match super::auth::open_off_thread(&app, key).await {
        Ok(opened) => opened,
        Err(e) => {
            // The key the ring held does not open this vault — its password
            // was changed somewhere the ring never heard of. It is dropped,
            // and the switch ends where it always used to: on the lock screen,
            // where the password that does open it also puts it back in the
            // ring (`auth::unlock`).
            log::warn!("workspace {id} did not open with the key held for it: {e}");
            state.keyring.lock().unwrap().remove(&id);
            return Ok(land_on_lock_screen(&app));
        }
    };
    let sync_configured = storage::sync_configured(&app);
    // The ring is what authorised this open, so it is asked once more before
    // the vault is handed to the session — under the session lock, as a lock
    // reads and clears it (`session::lock`). A lock that landed while the open
    // was in flight cleared the ring, so a ring that no longer holds this key
    // is a lock that won: the store closes and the vault stays as the lock
    // left it, exactly as a lease `Session::adopt` refuses does for a create.
    // The lock has already said `vault:locked`; there is nothing to add.
    {
        let mut session = state.session.lock().unwrap();
        if !state.keyring.lock().unwrap().has(&id) {
            log::info!("vault locked during the switch; workspace {id} stays locked");
            return Ok(None);
        }
        session.set(key, store, sync_configured);
    }
    // As after any unlock: the session arms its own idle clock, a connected
    // browser extension hears of it (and, re-checking the hash, of the switch),
    // and a vault named before names lived inside it takes the registry's label.
    crate::autolock::touch(&app);
    #[cfg(desktop)]
    crate::browser::server::notify_unlocked();
    super::auth::seed_vault_name(&app, &state);
    Ok(Some(UnlockResult {
        entries,
        sync_configured,
    }))
}

/// The switch that has no key to open its target with: announced like any
/// other lock, so the frontend takes the one path it takes for all of them.
/// After the repoint rather than inside the clear (`session::lock`): the
/// reaction re-probes `app_status`, which must find the new workspace already
/// active.
///
/// The ring stays. The app is still open at its own level — the picker on the
/// lock screen can come straight back to a workspace the ring holds, and a
/// password typed for this one is enough to seal its key under the app key
/// (`auth::unlock`). It goes back on the idle clock for it, since the session
/// that kept it there has ended.
fn land_on_lock_screen(app: &AppHandle) -> Option<UnlockResult> {
    crate::autolock::touch(app);
    crate::events::vault_locked(app);
    None
}

/// Refuse to move the paths while a sync flow is using them. The caller holds
/// `workspace_lock`, which is also what every flow raises its flag under — so
/// what this reads cannot change between the check and the move.
///
/// A consent flow keeps the cryptor of the vault that started it, and a run
/// writes its scratch and tokens through the *active* workspace's paths — so a
/// switch mid-flight would seal one workspace's account under another's
/// directory, and the next sync from there would publish into the wrong pack.
fn guard_sync_idle(state: &AppState) -> Result<()> {
    if state.syncing.load(Ordering::SeqCst) {
        return Err(Error::SyncBusy);
    }
    guard_sync_flows_idle(state)
}

/// The rest of [`guard_sync_idle`], for a caller that took the run claim itself
/// rather than checking it ([`workspace_delete`]): reading `syncing` would only
/// find that caller's own hold, and what is left to refuse are the flows the
/// claim does not cover.
///
/// The active workspace's run state is the only one that can have a flow in it:
/// starting one takes the lock this caller is holding.
fn guard_sync_flows_idle(state: &AppState) -> Result<()> {
    let busy = state.sync_run(|run| run.pending || run.in_progress);
    #[cfg(mobile)]
    let busy = busy || state.pending_auth.lock().unwrap().is_some();
    if busy {
        return Err(Error::SyncBusy);
    }
    Ok(())
}

/// Load, change and save the registry as one step under the workspace lock, so
/// two writers can never each save a copy that lacks the other's change — a
/// rename landing beside a create would otherwise drop the new workspace from
/// the file and leave its vault with no way back to it after a relaunch.
pub(crate) fn update_registry<T>(
    state: &AppState,
    root: &Path,
    change: impl FnOnce(&mut Registry) -> Result<T>,
) -> Result<T> {
    let _paths = state.workspace_lock.lock().unwrap();
    let mut registry = Registry::load(root);
    let out = change(&mut registry)?;
    registry.save(root)?;
    Ok(out)
}

/// Put back what `workspace_create` took out: the session that was open and
/// the workspace the paths pointed at. Under the lock, as one step, for the
/// same reason they came out as one. The session only goes back if nothing
/// locked it while the lease was out (`Session::restore`); the paths go back
/// regardless, since they are what a later unlock resolves through.
fn restore(state: &AppState, active: String, previous: Lease) {
    let _paths = state.workspace_lock.lock().unwrap();
    *state.active_workspace.lock().unwrap() = active;
    state.session.lock().unwrap().restore(previous);
}

/// Create a workspace under the device's master password and leave it unlocked.
///
/// One master password for the device: `password` has to be the primary's, so
/// that the vault opens on another device under the same password
/// (`commands::autojoin`) and so that a password change can carry every
/// workspace along. Proved against the app key the ring holds, which costs the
/// Argon2id run and nothing else; a wrong one is `InvalidPassword`.
///
/// The new vault is empty, so the result needs no listing — it is what the
/// caller of a first unlock expects, with nothing in it yet.
///
/// It inherits the open workspace's Google account, when there is one: the
/// tokens are sealed under the new key, and the vault is given its id here so
/// that its first sync addresses a pack of its own rather than being turned
/// away as a nameless vault beside the account's others (`sync::plan_vault_id`).
/// That first sync — run by the frontend's `enterMain`, as after the first
/// run's create — is what creates the pack, and what puts the new vault in
/// front of every other device on the account. A workspace made on a device
/// that does not sync is exactly what it was: local, sync off.
#[tauri::command]
pub async fn workspace_create(
    name: String,
    password: Zeroizing<String>,
    color: Option<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<UnlockResult> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(Error::WorkspaceNameRequired);
    }
    let color = tile_color(color);
    if password.is_empty() {
        return Err(Error::WorkspacePasswordRequired);
    }
    // The same exclusion first-run setup takes: this writes a KDF sidecar and a
    // database, and two of those interleaving would pair one with the other's.
    // Also what keeps `workspace_select` out until the vault below exists.
    let _step = begin_step(&state)?;

    let root = storage::root_dir(&app)?;
    verify_master_password(&app, &root, &state, password.clone()).await?;
    let id = crate::crypto::random_hex_id();

    // The open session comes out on a lease and the paths move as one step
    // under the lock, so no command sees one workspace's key beside another's
    // directory. The session is kept rather than dropped: a failure puts it
    // back exactly as it was, and the frontend — which only changes screens on
    // success — is still looking at a vault that is still open. A lock that
    // lands while the lease is out wins over both outcomes (`Session::adopt`).
    //
    // The account is read in the same step, before the paths move: the token
    // file resolves through the active workspace, and after the repoint that
    // is the new, empty one.
    let (previous_active, previous, inherited) = {
        let _paths = state.workspace_lock.lock().unwrap();
        guard_sync_idle(&state)?;
        let inherited = open_account(&app, &state)?.map(|(tokens, _)| tokens);
        let lease = state.session.lock().unwrap().take_out()?;
        let active = std::mem::replace(&mut *state.active_workspace.lock().unwrap(), id.clone());
        (active, lease, inherited)
    };

    let created = match create_vault_in(&app, &root, &id, password, inherited.as_ref()).await {
        Ok(created) => created,
        Err(e) => {
            // Leave no trace of a workspace that never opened: the user is put
            // back exactly where they pressed the button, free to try again.
            discard(&root, &id);
            restore(&state, previous_active, previous);
            return Err(e);
        }
    };
    let (key, store, vault_id) = created;

    // Registry last: an entry is recorded only once there is a vault behind it,
    // so a failure anywhere above has nothing on record to undo. If this write
    // fails the vault goes too — a database no registry names would otherwise
    // be one with no way back to it.
    let recorded = update_registry(&state, &root, |registry| {
        registry.workspaces.push(Workspace {
            id: id.clone(),
            name: Some(name),
            // A vault that will sync is recorded by its id now, as a restored
            // one is: its pack exists after the first run, and the record is
            // what turns a restore of that pack away. A local vault has no id
            // and no pack, so there is nothing to record.
            vault_id: vault_id.clone(),
            item_count: None,
            color,
        });
        registry.active = id.clone();
        Ok(())
    });
    if let Err(e) = recorded {
        // The store closes before its files are removed.
        drop(store);
        discard(&root, &id);
        restore(&state, previous_active, previous);
        return Err(e);
    }

    // The new vault continues the session the previous one was taken from. If
    // that session was locked while the vault was being created, it stays
    // locked: the workspace exists and is recorded, and the next unlock opens
    // it, but it does not open itself behind a lock the user asked for.
    let syncing = inherited.is_some();
    let material = Zeroizing::new(key.biometric_material().to_vec());
    let (_, _, _, claim) = previous.split();
    if !state
        .session
        .lock()
        .unwrap()
        .adopt(claim, key, store, syncing)
    {
        return Err(Error::Locked);
    }
    // The new vault is open now, and the Argon2id derive above took a while:
    // put it on the idle clock from here rather than from whenever the webview
    // next reports activity.
    crate::autolock::touch(&app);
    #[cfg(desktop)]
    crate::browser::server::notify_unlocked();
    // And open at the app level: sealed under the app key, which the check
    // above proved is in the ring.
    appkey::adopt(&app, &id, &material);
    Ok(UnlockResult {
        entries: vec![],
        sync_configured: syncing,
    })
}

/// Prove `password` is the master password — the one the primary's key derives
/// from — without opening the primary when that can be avoided. Once the
/// primary has been opened this session its key is in the ring as the app key,
/// and a derive with the primary's own descriptor is compared against it.
/// Before that — another workspace is what is open, unlocked with a password
/// of its own — the primary's own database is asked, exactly as the unlock's
/// detached try asks it, and the key it proves opens the app at its level from
/// here on (`appkey::open_all`), so the next proof is the cheap one.
///
/// Under the primary's own failed-attempt backoff, exactly as an unlock of it
/// and a delete of it run: this proves the primary's password on demand from
/// an open webview, and without the backoff it would be a way to guess that
/// password at full Argon2id speed while the lock screen beside it escalates.
async fn verify_master_password(
    app: &AppHandle,
    root: &Path,
    state: &AppState,
    password: Zeroizing<String>,
) -> Result<()> {
    let lockout = LockoutState::load_in(root)?;
    let now = auth::now_ms();
    if let Some(refusal) = auth::locked_out(lockout, now) {
        return Err(refusal);
    }

    let dir = root.to_path_buf();
    let app_key = state.keyring.lock().unwrap().app_key();
    let proved = match app_key {
        Some(app_key) => {
            let derived = blocking(move || derive_in(&dir, &password)).await?;
            derived.biometric_material().ct_eq(&app_key).into()
        }
        None => match blocking(move || verify_password_in(&dir, &password)).await {
            Ok(key) => {
                appkey::open_all(app, key.biometric_material());
                true
            }
            Err(Error::InvalidPassword) => false,
            Err(e) => return Err(e),
        },
    };
    if proved {
        if lockout != LockoutState::default() {
            if let Err(e) = LockoutState::default().save_in(root) {
                log::warn!("failed to reset lockout sidecar: {e}");
            }
        }
        Ok(())
    } else {
        let (updated, refusal) = auth::penalize(lockout, now);
        if let Err(e) = updated.save_in(root) {
            log::warn!("failed to persist lockout sidecar: {e}");
        }
        Err(refusal)
    }
}

/// The open workspace's Google account, with the connection generation it was
/// read under: `None` when it has none, and an error when it has one that
/// cannot be read. Read under the caller's `workspace_lock`, through the paths
/// as they stand. The generation is read in the same step as the tokens
/// (`sync::current_account`), so a caller that writes refreshed tokens back can
/// never be handed dropped credentials under a generation that still passes.
///
/// The two are told apart on purpose. A workspace whose session says it syncs
/// but whose token file will not unseal or parse is damaged, not local: going
/// ahead would make a workspace that quietly lacks the account the user was
/// promised (or, for a restore, fail later for a less honest reason).
fn open_account(app: &AppHandle, state: &AppState) -> Result<Option<(sync::Tokens, u64)>> {
    let session = state.session.lock().unwrap();
    let cryptor = session.cryptor()?;
    if !session.sync_configured {
        return Ok(None);
    }
    drop(session);
    sync::current_account(app, &cryptor)
        .map(Some)
        .ok_or_else(|| Error::Other(account_unreadable_error()))
}

// Surfaced verbatim in the form, so it has to read as a sentence.
fn account_unreadable_error() -> String {
    "this workspace's Google account could not be read; disconnect and connect it again, then \
     retry"
        .into()
}

// Surfaced verbatim in the form, so it has to read as a sentence.
fn disconnected_mid_restore_error() -> String {
    "Google Drive was disconnected while the vault was downloading; connect again and retry".into()
}

/// Connect a Google account for a workspace that does not exist yet.
///
/// Onboarding's connect, reached from Settings instead of from the first run:
/// the tokens are held in memory rather than sealed, because the vault they
/// belong to is still a pack on Drive. Exactly the same events report it, so
/// the picker the first run draws is the picker this draws.
#[cfg(desktop)]
#[tauri::command]
pub fn workspace_drive_connect(app: AppHandle) -> Result<()> {
    setup::connect_pending(&app)
}

/// The mobile twin of [`workspace_drive_connect`].
#[cfg(mobile)]
#[tauri::command]
pub fn workspace_drive_connect(app: AppHandle, state: State<'_, AppState>) -> Result<()> {
    setup::connect_pending(&app, &state)
}

/// Add a workspace by restoring one of the connected account's vaults into it.
///
/// The second device's way in to everything onboarding could not reach: a fresh
/// install restores exactly one vault, and every other vault on the same
/// account stayed unreachable until this. Step for step it is
/// [`workspace_create`] with the empty vault swapped for a downloaded pack —
/// the same step lock, the same lease and repoint under `workspace_lock`, the
/// same registry-last ordering, the same unwinding — plus the two things
/// onboarding does after a restore of its own (`commands::setup::adopt`): the
/// vault id taken off the file name, and the pending tokens sealed under the
/// key the restore just derived.
///
/// A wrong password costs nothing but the typing. It surfaces as
/// [`Error::InvalidPassword`] from the pack's own SQLCipher open, the new
/// directory is removed, the workspace the user was in comes back exactly as it
/// was, and the account stays pending — so the retry is one press away.
#[tauri::command]
pub async fn workspace_restore_from_drive(
    name: String,
    password: Zeroizing<String>,
    file_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<UnlockResult> {
    let tokens = setup::peek_pending(&state)?;
    restore_workspace(
        &app,
        &state,
        name,
        password,
        file_id,
        tokens,
        Account::Pending,
    )
    .await
}

/// Add a workspace by restoring one of the vaults the open workspace's account
/// holds and this device does not — the ones every sync run reports
/// (`sync::publish_remote_vaults`).
///
/// [`workspace_restore_from_drive`] with the sign-in taken out: the tokens are
/// the open workspace's own, read off its token file, and a copy is sealed
/// under the restored vault's key exactly as a pending account would be. The
/// device ends up with one account in two workspaces, which is what an account
/// connected once is meant to reach. A workspace that does not sync has no
/// account to restore from and is refused before anything is downloaded.
#[tauri::command]
pub async fn workspace_restore_from_account(
    name: String,
    password: Zeroizing<String>,
    file_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<UnlockResult> {
    // Under the lock, as `workspace_create` reads it: the token file resolves
    // through the active workspace, which must not move underneath.
    let (tokens, generation) = {
        let _paths = state.workspace_lock.lock().unwrap();
        open_account(&app, &state)?.ok_or(Error::SyncNotConfigured)?
    };
    restore_workspace(
        &app,
        &state,
        name,
        password,
        file_id,
        tokens,
        Account::Open { generation },
    )
    .await
}

/// Whose tokens a restore runs on, which is the one thing that differs between
/// the two restores: a pending account has to be kept current through the
/// download and dropped once it is sealed inside the new workspace; the open
/// workspace's account is its own to keep, and its token file follows the
/// refresh — if the connection is still the one the tokens were read under,
/// which is what `generation` names.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Account {
    Pending,
    Open { generation: u64 },
}

/// Turn the pack `file_id` into a workspace, unlocked with `password`.
///
/// `name` is optional: left blank, the workspace takes the name the vault
/// carries inside its pack (see [`crate::store::identity`]), which is the name
/// the user gave it on the device they already have it on. A typed one is a
/// rename, and travels back out the same way.
async fn restore_workspace(
    app: &AppHandle,
    state: &AppState,
    name: String,
    password: Zeroizing<String>,
    file_id: String,
    mut tokens: sync::Tokens,
    account: Account,
) -> Result<UnlockResult> {
    let name = name.trim().to_string();
    if password.is_empty() {
        return Err(Error::WorkspacePasswordRequired);
    }
    // The same exclusion `workspace_create` takes, and for the same files.
    let _step = begin_step(state)?;

    let root = storage::root_dir(app)?;
    let id = crate::crypto::random_hex_id();

    // The download runs *before* the paths move. It is the slow half — a
    // listing, a token refresh and a whole vault over the network — and the
    // session of the workspace the user is looking at would otherwise be held
    // out for all of it, taking a vault away that they never asked to leave.
    //
    // The connection generation came with the tokens, read as one step with
    // them, as every refresh that will be written back reads it: a disconnect
    // landing anywhere from that read to the write below must not have its
    // delete undone by the write.
    let (bytes, vault_id) = setup::download(app, &mut tokens, &file_id, |vault_id| {
        guard_other_vault(state, &root, vault_id)
    })
    .await?;
    // Whatever the refresh produced has to be kept, for the reason onboarding
    // keeps it: Google rotates refresh tokens, and a mistyped master password
    // has to leave a retry that works. A pending account lives in memory; the
    // open workspace's lives in its token file, which has to follow too — left
    // on the old copy, its next run could find the refresh token retired while
    // only the restored workspace held the live one. That write happens below,
    // in the same step as the lease.
    if account == Account::Pending {
        setup::replace_pending_tokens(state, tokens.clone())?;
    }

    // As `workspace_create` takes them, and for the same reasons: one step
    // under the lock, the session kept rather than dropped so a failure can put
    // it back, and a lock landing meanwhile winning over both outcomes.
    //
    // For the open workspace's account this step is also where a disconnect is
    // shut out for good. `sync_disconnect` needs the session's key, so once the
    // lease is out none can run until the restore ends — and one that landed
    // before it is caught by the generation check made *after* the lease: the
    // write-back and the check happen with the lock held, and nothing in
    // between can undo them. Sealing tokens the user had just dropped into a
    // new workspace would bring the account back under another name.
    let (previous_active, previous) = {
        let _paths = state.workspace_lock.lock().unwrap();
        guard_sync_idle(state)?;
        if let Account::Open { generation } = account {
            let cryptor = state.session.lock().unwrap().cryptor()?;
            if !sync::persist_tokens_if_current(app, &cryptor, &tokens, generation)? {
                return Err(Error::Other(disconnected_mid_restore_error()));
            }
        }
        let lease = state.session.lock().unwrap().take_out()?;
        if matches!(account, Account::Open { generation } if sync::connection_generation(app) != generation)
        {
            // The paths have not moved yet, so the session goes straight back.
            state.session.lock().unwrap().restore(lease);
            return Err(Error::Other(disconnected_mid_restore_error()));
        }
        let active = std::mem::replace(&mut *state.active_workspace.lock().unwrap(), id.clone());
        (active, lease)
    };

    // Kept past the restore for the account's *other* vaults: the ones this
    // password opens too are added beside this one (`commands::autojoin`).
    let join = password.clone();
    let restored = restore_vault_in(app, &root, &id, bytes, password, &vault_id, &tokens)
        .await
        .and_then(|(key, store, entries)| {
            let name = settle_name(&store, name, &vault_id)?;
            Ok((key, store, entries, name))
        });
    let (key, store, entries, name) = match restored {
        Ok(restored) => restored,
        Err(e) => {
            // Leave no trace of a workspace that never opened — the token file
            // the restore may have written goes with the directory.
            discard(&root, &id);
            restore(state, previous_active, previous);
            return Err(e);
        }
    };

    // Registry last, for the reason `workspace_create` records it last: an
    // entry only once there is a vault behind it.
    let recorded = update_registry(state, &root, |registry| {
        registry.workspaces.push(Workspace {
            id: id.clone(),
            name: Some(name),
            // Known now, and this workspace arrives syncing: the record that
            // turns away a second restore of the same pack is made with the
            // first, not left to the sync that follows.
            vault_id: Some(vault_id.clone()),
            item_count: None,
            color: None,
        });
        registry.active = id.clone();
        Ok(())
    });
    if let Err(e) = recorded {
        // The store closes before its files are removed.
        drop(store);
        discard(&root, &id);
        restore(state, previous_active, previous);
        return Err(e);
    }

    // The account belongs to the new workspace now: its token file is on disk,
    // sealed under the key above, and the registry names the vault it is in.
    // Dropping the in-memory copy is what stops a later restore from silently
    // adopting it — and it goes *before* the adopt below, because a lock that
    // wins there is not a failure of the restore, and must not leave the
    // credentials pending as if it were.
    if account == Account::Pending {
        setup::take_pending(state);
    }

    // The restored vault continues the session the previous one was taken from,
    // and it arrives connected. A lock that landed while the lease was out still
    // wins — the workspace exists and is recorded, and the next unlock opens it.
    let material = Zeroizing::new(key.biometric_material().to_vec());
    let (_, _, _, claim) = previous.split();
    if !state.session.lock().unwrap().adopt(claim, key, store, true) {
        return Err(Error::Locked);
    }
    // As in `workspace_create`: the restored vault is open, so its idle clock
    // starts here — and it is open at the app level, sealed under the app key
    // when the ring holds one.
    crate::autolock::touch(app);
    #[cfg(desktop)]
    crate::browser::server::notify_unlocked();
    appkey::adopt(app, &id, &material);
    super::autojoin::with_password(app, join);
    Ok(UnlockResult {
        entries,
        sync_configured: true,
    })
}

/// Give a workspace a (new) label. Ids never change; only this does.
///
/// The name goes into the vault as well as into the registry, so it travels in
/// the pack and reaches the user's other devices (see [`crate::store::identity`]
/// and `sync::engine`). Only the active workspace is unlocked, though, and only
/// an unlocked vault can be written to: renaming any other one is still a
/// rename — the list draws from the registry — it just does not reach Drive
/// until that vault is next opened, which is where `commands::auth` seeds it.
#[tauri::command]
pub fn workspace_rename(
    id: String,
    name: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<()> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(Error::WorkspaceNameRequired);
    }

    let root = storage::root_dir(&app)?;

    // The vault is the source of truth — it is the copy that travels — and the
    // registry mirrors it, through the very same call a pulled rename mirrors
    // with, so the two paths cannot drift apart. A workspace whose vault is not
    // open has no truth to write: the registry is its only copy, and its
    // failures stay the caller's as they always were.
    if !name_the_open_vault(&state, &root, &id, &name, auth::now_ms())? {
        return update_registry(&state, &root, |registry| {
            let workspace = registry
                .workspaces
                .iter_mut()
                .find(|w| w.id == id)
                .ok_or(Error::NotFound)?;
            workspace.name = Some(name);
            Ok(())
        });
    }

    // The same request an entry save makes: the new name is a change to the
    // vault, and it reaches the account the way every other one does.
    super::sync::request_run_if_ready(&app, &state);
    Ok(())
}

/// Give a workspace a tile colour, or clear it with `None` (or an empty key).
///
/// Registry only, unlike a rename: the colour is this device's to choose (see
/// [`Workspace::color`]), so there is no vault to write and nothing to sync, and
/// a locked workspace is recoloured exactly as the open one is.
#[tauri::command]
pub fn workspace_set_color(
    id: String,
    color: Option<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<()> {
    let root = storage::root_dir(&app)?;
    update_registry(&state, &root, |registry| {
        registry.recolor(&id, tile_color(color))
    })
}

/// A palette key as the frontend sent it: trimmed, and `None` when blank.
fn tile_color(color: Option<String>) -> Option<String> {
    color
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty())
}

/// Write the name into the open vault's `meta` — stamped, so another device can
/// tell which of two renames came last — and mirror it into the registry, as
/// one step. `false` when `id` is not the workspace that is open: a locked one
/// keeps its name in the registry alone, and the caller writes that.
///
/// `workspace_lock` and then the session: together, and in this order, as
/// `workspace_select` takes them. A sync run takes the session lock per
/// operation rather than for its length (`sync::engine::SessionVault`), so any
/// interval between these two writes is one a run can slip into — reading the
/// vault's new name and publishing a rename this command is about to reject, or
/// adopting a newer name from another device for the undo below to clobber.
/// Leaving no interval is what makes the undo safe without a compare: the pair
/// goes back under the very guard that replaced it, before anything else can
/// have read it. The registry's save is one small atomic file write, which is
/// short enough to hold the session across.
fn name_the_open_vault(
    state: &AppState,
    root: &Path,
    id: &str,
    name: &str,
    now_ms: i64,
) -> Result<bool> {
    let _paths = state.workspace_lock.lock().unwrap();
    if *state.active_workspace.lock().unwrap() != id {
        return Ok(false);
    }
    let session = state.session.lock().unwrap();
    let Ok(store) = session.store() else {
        return Ok(false);
    };
    let (previous, previous_ms) = identity::vault_name(store).map_err(store_err)?;
    // Derived from the name it replaces, not from this device's clock alone:
    // see [`identity::rename_stamp`]. The pair was read a line ago and is
    // written back under the same guard, so it is the stamp this rename has to
    // beat.
    let at_ms = identity::rename_stamp(previous_ms, now_ms);
    identity::set_vault_name(store, name, at_ms).map_err(store_err)?;

    // A rename is one action. Telling the user it failed while the vault keeps
    // the new name — and publishes it to every other device on the next run —
    // is the worst of both, so a mirror that will not take it puts the vault
    // back the way it was. Rolling back rather than succeeding quietly because
    // the registry is what the header and the workspace list read: a name only
    // the vault holds is a rename the user cannot see here. Best effort, since
    // the rename is already failing; an empty name is how a vault that had none
    // is put back to having none.
    if let Err(e) = workspace::record_vault_name_locked(root, id, name) {
        let previous = previous.as_deref().unwrap_or("");
        if let Err(e) = identity::set_vault_name(store, previous, previous_ms) {
            log::warn!("could not put the vault's previous name back: {e}");
        }
        return Err(e);
    }
    Ok(true)
}

/// Remove a workspace's vault from this device, and — with `everywhere` — from
/// the Google account it syncs to as well.
///
/// Local by default: whatever the vault has on Drive is left where it is, so
/// the account still holds it and another device — or this one, later — can
/// restore it as a workspace again. `password` is the target's own master
/// password, proved against the target's own files rather than against the
/// session, because the workspace being deleted is usually a locked one whose
/// key is nowhere in memory.
///
/// `everywhere` adds one step in front of all of that: the vault's pack is
/// removed from Drive and a marker left in its place ([`sync::delete_pack`]),
/// which is what has the account's other devices stop syncing it instead of
/// uploading their copy back. It runs *before* anything local is touched, so a
/// network failure leaves the workspace here and the user with something to
/// retry from — the tokens are sealed under this vault's key, and deleting it
/// first would take the only way back to the account with it.
///
/// Deleting the primary is the case that moves files: the root is its
/// directory, so the first surviving workspace is promoted into it and takes
/// `PRIMARY_ID` (see [`Registry::without`]). The biometric enrollment goes with
/// it — the one keychain item held the app key, which was the deleted vault's
/// — and so does every other workspace's copy of its key sealed under that
/// app key: the promoted vault's key is the app key now, and the ring reseals
/// what it holds under it (`crate::appkey`); the rest are stale, and the next
/// unlock of each writes a fresh one.
///
/// Ends like a switch whenever the workspace the paths point at is the one
/// going away or the one being promoted: same guards, same `vault:locked`, so
/// the frontend lands on the survivor's lock screen by the one path it takes
/// for every lock.
#[tauri::command]
pub async fn workspace_delete(
    id: String,
    password: Zeroizing<String>,
    everywhere: bool,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<()> {
    if password.is_empty() {
        return Err(Error::WorkspacePasswordRequired);
    }
    // The same exclusion a create, a restore and a password change take: those
    // write the very files this removes, and a switch may not move the paths
    // while the promotion below is rewriting where they point.
    let _step = begin_step(&state)?;
    let root = storage::root_dir(&app)?;

    // Asked before the derive, so the only refusals the user can do nothing
    // about cost them no Argon2id wait.
    Registry::load(&root).without(&id)?;

    // Under the target's own failed-attempt backoff, exactly as `unlock` runs
    // under the open workspace's. This command proves a password on demand and
    // takes no session, so it is reachable from the lock screen — without this
    // it would be a way to guess a workspace's master password at full speed
    // while the unlock beside it escalates.
    let dir = workspace::dir_of(&root, &id);
    let lockout = LockoutState::load_in(&dir)?;
    let now = auth::now_ms();
    if let Some(refusal) = auth::locked_out(lockout, now) {
        return Err(refusal);
    }

    // Argon2id and a SQLCipher open, so off the command thread — and on the
    // target's own files, whether it is the open workspace or a locked one. The
    // key comes back rather than being thrown away: a "delete everywhere" has
    // to unseal that workspace's own token file with it, and deriving a second
    // time would be another Argon2id wait for nothing.
    let target = dir.clone();
    let key = match blocking(move || verify_password_in(&target, &password)).await {
        Ok(key) => {
            // Reset here rather than leave it to the removal below: the delete
            // can still be refused after the proof (a sync in flight, a Drive
            // failure, a staging failure), and a proven password must not leave
            // attempts standing against a workspace that is still there.
            if lockout != LockoutState::default() {
                if let Err(e) = LockoutState::default().save_in(&dir) {
                    log::warn!("failed to reset lockout sidecar: {e}");
                }
            }
            key
        }
        Err(Error::InvalidPassword) => {
            let (updated, refusal) = auth::penalize(lockout, now);
            if let Err(e) = updated.save_in(&dir) {
                log::warn!("failed to persist lockout sidecar: {e}");
            }
            return Err(refusal);
        }
        Err(e) => return Err(e),
    };

    // One exclusion over the whole delete, network included, and the same one a
    // run takes: a run starting beside the Drive work below could read the pack
    // gone before its marker was written and upload this vault straight back, or
    // push into the middle of the local deletion. Taken after the derive, so an
    // Argon2id wait costs sync nothing, and held to the end of the function —
    // every exit path releases it (`RunClaim`).
    let _claim = super::sync::claim_or_busy(&state)?;

    // The network, before a single local file moves: what it fails to do, the
    // user can try again, and only while the workspace is still here to try it
    // from.
    if everywhere {
        delete_remote_pack(&app, &root, &id, &dir, &key).await?;
    }

    // Everything from here is one step under the lock, as a switch is: the
    // registry is re-read inside it (a rename could have landed while the
    // derive ran), the session ends before its files move, and the paths and
    // the file on disk change together.
    let (ended_session, was_primary, applied) = {
        let _paths = state.workspace_lock.lock().unwrap();
        let registry = Registry::load(&root);
        let deletion = registry.without(&id)?;
        // The claim above is this delete's own, so only the flows beside it are
        // asked about here.
        guard_sync_flows_idle(&state)?;

        // The open workspace is left open unless this delete disturbs it: it is
        // the one going away, or the one whose directory is about to become the
        // root. Either way its store has to close before the files move.
        let active = state.active_workspace.lock().unwrap().clone();
        let ends_session = active == id || deletion.promoted.as_deref() == Some(active.as_str());
        if ends_session {
            state.session.lock().unwrap().clear();
        }

        // Staged, so that whichever step a failure lands on, what is left is
        // either the old layout or a record that can be finished without the
        // password (`workspace::apply_deletion`). Nothing is removed until the
        // registry that no longer names this workspace is on disk — the
        // opposite order to a create, which records the registry last for the
        // same reason: a file set nothing names is one the user has no way to
        // reach, and no way to prove a password against either.
        let applied = workspace::apply_deletion(&root, &id, &deletion);
        if applied.is_ok() {
            *state.active_workspace.lock().unwrap() = deletion.registry.active.clone();
            // The ring: the deleted workspace's key goes, and a promotion
            // makes the promoted vault's key the app key, with every sidecar
            // resealed under it.
            appkey::forget(&app, &id, deletion.promoted.as_deref());
        }
        (ends_session, deletion.promoted.is_some(), applied)
    };

    // Off the lock, because a keychain delete is a blocking call into the OS.
    // The enrollment held the app key, which was the deleted primary's, so the
    // stored bytes open nothing; the marker file went with the primary's other
    // files above. Best effort, like every other unenroll on a path the user
    // cannot retry: an item we could not reach is overwritten by the next
    // enrollment.
    if was_primary && applied.is_ok() {
        let _ = blocking(|| crate::secure_store::Platform.delete()).await;
    }

    // Said exactly as `workspace_select` says it, and in the same order: the
    // session is over, the sync status the frontend holds belongs to a
    // workspace it is no longer looking at, and the lock is announced last so
    // the re-probe it triggers finds the new active workspace already in place.
    // Said for a failed delete too: the session was closed before the files
    // moved, so a vault the user was in is locked whether or not the delete
    // went through, and the frontend has to hear it either way.
    if ended_session {
        crate::session::sealed(&app);
        super::sync::switched(&app);
        crate::events::vault_locked(&app);
    }
    applied
}

// Take the vault of workspace `id` out of the account it syncs to.
//
// Both facts this needs are readable without opening the workspace, which is
// the point: the vault id is the registry's copy (`Workspace::vault_id`, put
// there by the runs that settled it), and the tokens are that workspace's own
// sealed file, unsealed with the key the password just proved.
//
// A workspace with no vault id has no pack up there — it never synced, or never
// finished a first run — and one with no token file at all never connected an
// account: both have nothing to delete and nothing to mark. A token file that
// is there but cannot be read is neither, and fails the delete instead: the
// pack may well be live, and going on would destroy the workspace the retry
// would have to come from (`sync::read_tokens_in`).
async fn delete_remote_pack(
    app: &AppHandle,
    root: &Path,
    id: &str,
    dir: &Path,
    key: &VaultKey,
) -> Result<()> {
    let vault_id = Registry::load(root)
        .workspaces
        .into_iter()
        .find(|w| w.id == id)
        .and_then(|w| w.vault_id);
    let Some(vault_id) = vault_id else {
        return Ok(());
    };
    let Some(mut tokens) = sync::read_tokens_in(dir, &key.cryptor())? else {
        return Ok(());
    };
    sync::delete_pack(app, &mut tokens, &vault_id).await
}

// Prove `password` opens the vault in `dir`, and hand back the key that did it.
//
// The same proof an unlock makes, on a path instead of on the app's: read that
// workspace's own descriptor, derive, and let SQLCipher answer. It has to work
// on a locked workspace — whose key is nowhere in memory — so there is nothing
// to compare against the session the way `change_master_password` does.
//
// Argon2id: run it off the main thread.
pub(crate) fn verify_password_in(dir: &Path, password: &str) -> Result<VaultKey> {
    let key = derive_in(dir, password)?;

    // The same split `session::open_with_key` draws: only the store's own key
    // check is a wrong password. A failing disk must not be reported as one.
    SqliteStore::open(&dir.join(storage::DB_FILE), &*key.sqlcipher_key()).map_err(|e| match e {
        StoreError::WrongKey => Error::InvalidPassword,
        StoreError::SchemaNewer => Error::VaultTooNew,
        e => Error::Other(format!("could not open the vault: {e}")),
    })?;
    Ok(key)
}

// The key `password` derives to for the vault in `dir`, as `session::derive_key`
// derives one for the active workspace: Argon2id under the descriptor beside
// the database, or the legacy scheme for a vault written before the sidecar
// existed. Nothing is opened; whether it is the right key is the caller's
// question.
fn derive_in(dir: &Path, password: &str) -> Result<VaultKey> {
    let sidecar = dir.join(storage::KDF_SIDECAR_FILE);
    if sidecar.exists() {
        let params = KdfParams::from_json(&fs::read_to_string(sidecar)?)?;
        Ok(VaultKey::Argon2 {
            master: crate::crypto::derive(password.as_bytes(), &params)?,
        })
    } else {
        Ok(VaultKey::legacy_from_password(password))
    }
}

// Argon2id + creating the encrypted DB, off the command thread. The directory
// comes first: `create_vault` writes the KDF sidecar before SQLCipher opens
// anything, and the sidecar must land in the new workspace, not beside it.
//
// With an account to inherit, the vault is also named and connected here, so a
// failure in either leaves the caller one `discard` from a clean slate. The id
// is returned for the registry; `None` is a local vault, which has none.
async fn create_vault_in(
    app: &AppHandle,
    root: &Path,
    id: &str,
    password: Zeroizing<String>,
    inherited: Option<&sync::Tokens>,
) -> Result<(VaultKey, SqliteStore, Option<String>)> {
    let dir = workspace::dir_of(root, id);
    crate::store::create_private_dir(&dir)?;
    let (key, store) = create_off_thread(app, password).await?;
    let Some(tokens) = inherited else {
        return Ok((key, store, None));
    };
    // The id before the tokens, as every other stamp-and-seal here orders them:
    // nothing is sealed under a key a failure would then discard.
    let vault_id = crate::crypto::random_hex_id();
    crate::store::identity::adopt_vault_id(&store, &vault_id).map_err(store_err)?;
    sync::persist_tokens(app, &key.cryptor(), tokens)?;
    Ok((key, store, Some(vault_id)))
}

// Everything that turns the chosen pack into a workspace on this device, off
// the command thread: Argon2id, a SQLCipher open and three file writes are all
// blocking work.
//
// `restore_at` is handed the new workspace's paths outright, since its core runs
// without an `AppHandle`. The token file is not: `persist_tokens` resolves
// through whichever workspace is active, which is why the paths are repointed
// before this runs — that is what puts the account inside the new workspace
// rather than beside the old one's.
async fn restore_vault_in(
    app: &AppHandle,
    root: &Path,
    id: &str,
    bytes: Vec<u8>,
    password: Zeroizing<String>,
    vault_id: &str,
    tokens: &sync::Tokens,
) -> Result<(VaultKey, SqliteStore, Vec<EntryMetaDto>)> {
    let dir = workspace::dir_of(root, id);
    crate::store::create_private_dir(&dir)?;

    let app = app.clone();
    let vault_id = vault_id.to_string();
    let tokens = tokens.clone();
    super::blocking(move || {
        let (key, store) = restore::restore_at(
            &dir.join(storage::DB_FILE),
            &dir.join(storage::KDF_SIDECAR_FILE),
            &bytes,
            &password,
        )?;
        // The file name is how the account addresses this vault, and it is
        // authoritative over whatever the snapshot carries — the same stamp
        // onboarding applies, for the same reason (`commands::setup::adopt`).
        crate::store::identity::adopt_vault_id(&store, &vault_id).map_err(store_err)?;
        // The listing before the tokens, as `adopt` orders them: nothing is
        // sealed under a key that a failure would then discard.
        let entries = list_metas(&store)?;
        sync::persist_tokens(&app, &key.cryptor(), &tokens)?;
        Ok((key, store, entries))
    })
    .await
}

// What the restored workspace is called.
//
// A `typed` name is a rename: stamped into the vault's `meta` exactly as
// `workspace_rename` stamps one, so the next sync carries it to the user's
// other devices. A blank one takes the name the pack already carries — the one
// the user gave the vault wherever they made it — and a pack from before names
// travelled carries none, so it falls back to the short-id label every added
// vault starts under.
fn settle_name(store: &SqliteStore, typed: String, vault_id: &str) -> Result<String> {
    let (packed, packed_ms) = identity::vault_name(store).map_err(store_err)?;
    if !typed.is_empty() {
        // Over the stamp the pack carries, not this device's clock alone (see
        // [`identity::rename_stamp`]): the sync that runs straight after the
        // restore would otherwise hand the packed name back and skip the push,
        // and the name the user typed here would be gone before they saw it.
        let at_ms = identity::rename_stamp(packed_ms, auth::now_ms());
        identity::set_vault_name(store, &typed, at_ms).map_err(store_err)?;
        return Ok(typed);
    }
    Ok(packed.unwrap_or_else(|| super::autojoin::label(vault_id)))
}

/// Refuse a pack this device would end up holding twice.
///
/// Restoring a vault beside itself would leave two workspaces syncing one pack,
/// each merging over the other — a duplicate the user has no way to tell apart
/// afterwards, since the two would look identical.
///
/// Two places know which vault a workspace holds. The open one is asked live,
/// from the id in its own database. Every other workspace is locked and its
/// database unreadable, so those are asked through the registry, which records
/// each workspace's vault id as its syncs settle it (`Workspace::vault_id`).
/// A workspace that never synced has no record there — and no pack on Drive to
/// be restored from, so nothing is missed by that.
pub(crate) fn guard_other_vault(state: &AppState, root: &Path, vault_id: &str) -> Result<()> {
    let session = state.session.lock().unwrap();
    // Locked, or held out by another whole-vault operation: nothing to compare
    // against live, and the lease below is what will turn that away.
    if let Ok(store) = session.store() {
        if crate::store::identity::vault_id(store)
            .map_err(store_err)?
            .as_deref()
            == Some(vault_id)
        {
            return Err(Error::VaultAlreadyOpen);
        }
    }
    drop(session);

    let active = state.active_workspace.lock().unwrap().clone();
    if Registry::load(root).holder_of(vault_id, &active).is_some() {
        return Err(Error::VaultAlreadyOpen);
    }
    Ok(())
}

// Undo everything `create_vault_in` or `restore_vault_in` may have written.
// Removing the directory is what takes the restored workspace's token file with
// it — the one file of the set that is not named here.
pub(crate) fn discard(root: &Path, id: &str) {
    let dir = workspace::dir_of(root, id);
    storage::remove_db_files(&dir.join(storage::DB_FILE));
    let _ = fs::remove_file(dir.join(storage::KDF_SIDECAR_FILE));
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::identity;
    use crate::workspace::PRIMARY_ID;

    fn store() -> SqliteStore {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "rowel-workspace-restore-{}-{}.db",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        SqliteStore::open(&path, &[9u8; 32]).unwrap()
    }

    // What `workspace_create` and `workspace_set_color` record: the key the
    // frontend sent, trimmed, and nothing at all for a blank one.
    #[test]
    fn a_tile_color_is_trimmed_and_blank_is_none() {
        assert_eq!(tile_color(Some(" rose ".into())).as_deref(), Some("rose"));
        assert_eq!(tile_color(Some("   ".into())), None);
        assert_eq!(tile_color(None), None);
    }

    // A key of the right shape; nothing here derives or opens anything with it.
    fn key() -> VaultKey {
        VaultKey::Argon2 {
            master: Zeroizing::new(vec![0u8; 32]),
        }
    }

    fn open_with(store: SqliteStore) -> AppState {
        let state = AppState::default();
        state.session.lock().unwrap().set(key(), store, false);
        state
    }

    // A data dir with no registry file: one primary, nothing recorded.
    fn tmp_root() -> std::path::PathBuf {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "rowel-workspace-guard-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    // The collision seen live: the vault the user is looking at as they pick
    // which pack to restore.
    #[test]
    fn the_open_workspaces_own_vault_is_refused() {
        let store = store();
        identity::adopt_vault_id(&store, "a1b2").unwrap();

        assert!(matches!(
            guard_other_vault(&open_with(store), &tmp_root(), "a1b2"),
            Err(Error::VaultAlreadyOpen)
        ));
    }

    // Every other pack on the account is exactly what this flow is for.
    #[test]
    fn another_vault_on_the_same_account_is_allowed() {
        let store = store();
        identity::adopt_vault_id(&store, "a1b2").unwrap();

        assert!(guard_other_vault(&open_with(store), &tmp_root(), "cafe").is_ok());
    }

    // A vault created before ids existed answers to no pack, so it cannot be
    // the one being restored.
    #[test]
    fn a_vault_with_no_id_collides_with_nothing() {
        assert!(guard_other_vault(&open_with(store()), &tmp_root(), "a1b2").is_ok());
    }

    // The collision seen through the registry: a workspace that is locked, and
    // whose vault id a sync recorded — the one the live check cannot reach.
    #[test]
    fn a_locked_workspaces_recorded_vault_is_refused() {
        let root = tmp_root();
        Registry {
            active: PRIMARY_ID.into(),
            workspaces: vec![
                Workspace {
                    id: PRIMARY_ID.into(),
                    name: None,
                    vault_id: None,
                    item_count: None,
                    color: None,
                },
                Workspace {
                    id: "b2c3".into(),
                    name: Some("Work".into()),
                    vault_id: Some("cafe".into()),
                    item_count: None,
                    color: None,
                },
            ],
        }
        .save(&root)
        .unwrap();

        // Asked from the open primary, which holds another vault entirely.
        let store = store();
        identity::adopt_vault_id(&store, "a1b2").unwrap();
        let state = open_with(store);
        assert!(matches!(
            guard_other_vault(&state, &root, "cafe"),
            Err(Error::VaultAlreadyOpen)
        ));
        // A pack nobody here holds is still fine.
        assert!(guard_other_vault(&state, &root, "beef").is_ok());
    }

    // A locked session has nothing to compare against live, and the registry
    // records nothing for this pack. The lease the restore takes next is what
    // turns the locked session away, and it says `Locked` when it does.
    #[test]
    fn a_locked_workspace_is_left_to_the_lease_to_refuse() {
        assert!(guard_other_vault(&AppState::default(), &tmp_root(), "a1b2").is_ok());
    }

    // A "data dir" that is a file: the registry reads as the default single
    // workspace, and saving it cannot create anything beneath.
    fn unwritable_root() -> std::path::PathBuf {
        let path = tmp_root().join("not-a-directory");
        fs::write(&path, "").unwrap();
        path
    }

    // The rename the user asked for, in both copies, in one step.
    #[test]
    fn naming_the_open_vault_writes_the_registry_with_it() {
        let root = tmp_root();
        let state = open_with(store());

        assert!(name_the_open_vault(&state, &root, PRIMARY_ID, "Home", 20).unwrap());
        let session = state.session.lock().unwrap();
        assert_eq!(
            identity::vault_name(session.store().unwrap()).unwrap(),
            (Some("Home".into()), 20)
        );
        assert_eq!(
            Registry::load(&root).workspaces[0].name.as_deref(),
            Some("Home")
        );
    }

    // The vault holds a name stamped by a device whose clock runs ahead of this
    // one. Stamped `now` the rename would read as the older of the two, and the
    // next pull would hand the old name straight back; it has to outrank what it
    // replaces instead.
    #[test]
    fn a_rename_outranks_a_name_stamped_by_a_faster_clock() {
        let root = tmp_root();
        let store = store();
        identity::set_vault_name(&store, "Work", 5_000).unwrap();
        let state = open_with(store);

        assert!(name_the_open_vault(&state, &root, PRIMARY_ID, "Home", 20).unwrap());
        let session = state.session.lock().unwrap();
        let (name, at_ms) = identity::vault_name(session.store().unwrap()).unwrap();
        assert_eq!(name.as_deref(), Some("Home"));
        assert!(identity::name_wins(
            (name.as_deref(), at_ms),
            (Some("Work"), 5_000)
        ));
    }

    // The rollback, which is only safe because the mirror and the undo happen
    // under the one session guard: no sync run can have seen "Home", so putting
    // "Work" back cannot overwrite a name adopted in between.
    #[test]
    fn a_registry_that_will_not_take_the_name_puts_the_vaults_name_back() {
        let store = store();
        identity::set_vault_name(&store, "Work", 10).unwrap();
        let state = open_with(store);

        assert!(name_the_open_vault(&state, &unwritable_root(), PRIMARY_ID, "Home", 20).is_err());
        let session = state.session.lock().unwrap();
        assert_eq!(
            identity::vault_name(session.store().unwrap()).unwrap(),
            (Some("Work".into()), 10)
        );
    }

    // A vault that had no name is put back to having none, not to an empty one.
    #[test]
    fn an_unnamed_vault_is_put_back_unnamed() {
        let state = open_with(store());

        assert!(name_the_open_vault(&state, &unwritable_root(), PRIMARY_ID, "Home", 20).is_err());
        let session = state.session.lock().unwrap();
        assert_eq!(
            identity::vault_name(session.store().unwrap()).unwrap(),
            (None, 0)
        );
    }

    // Any workspace but the open one — and the open one while it is locked —
    // has no vault to write to, and is left to the registry alone.
    #[test]
    fn only_the_open_workspace_has_a_vault_to_name() {
        let root = tmp_root();
        assert!(!name_the_open_vault(&open_with(store()), &root, "b2c3", "Home", 20).unwrap());
        assert!(!name_the_open_vault(&AppState::default(), &root, PRIMARY_ID, "Home", 20).unwrap());
    }

    // The proof a delete asks for, on files rather than on the session — which
    // is what lets it be asked of a locked workspace.
    #[test]
    fn a_password_is_proved_against_the_workspaces_own_files() {
        let root = tmp_root();
        let dir = workspace::dir_of(&root, "a1b2");
        crate::store::create_private_dir(&dir).unwrap();
        let params = crate::crypto::KdfParams::argon2id(b"salt-0123456789012345", 256, 1, 1);
        storage::atomic_write_file(
            &dir.join(storage::KDF_SIDECAR_FILE),
            &params.to_json().unwrap(),
        )
        .unwrap();
        let key = VaultKey::Argon2 {
            master: crate::crypto::derive(b"right-password", &params).unwrap(),
        };
        drop(SqliteStore::open(&dir.join(storage::DB_FILE), &*key.sqlcipher_key()).unwrap());

        assert!(verify_password_in(&dir, "right-password").is_ok());
        assert!(matches!(
            verify_password_in(&dir, "wrong-password"),
            Err(Error::InvalidPassword)
        ));
    }

    // Leaving the field blank is what most restores will do: the vault already
    // has a name, and it is the one the user knows it by.
    #[test]
    fn a_blank_name_takes_the_one_the_pack_carries() {
        let store = store();
        identity::set_vault_name(&store, "Work", 1_700_000_000_000).unwrap();

        assert_eq!(
            settle_name(&store, String::new(), "9f3c1a2b").unwrap(),
            "Work"
        );
        // Untouched: nothing was renamed, so nothing new is owed to the account.
        assert_eq!(
            identity::vault_name(&store).unwrap(),
            (Some("Work".into()), 1_700_000_000_000)
        );
    }

    // A pack written before names travelled carries none.
    #[test]
    fn a_blank_name_over_a_nameless_pack_falls_back_to_the_label() {
        assert_eq!(
            settle_name(&store(), String::new(), "9f3c1a2b").unwrap(),
            "Vault 9f3c1a"
        );
    }

    // A typed name overrides the pack's, and goes into the vault stamped — so
    // the next sync carries it to every other device.
    #[test]
    fn a_typed_name_wins_and_is_written_into_the_vault() {
        let store = store();
        identity::set_vault_name(&store, "Work", 1).unwrap();

        assert_eq!(
            settle_name(&store, "Home".into(), "9f3c1a2b").unwrap(),
            "Home"
        );
        let (name, at_ms) = identity::vault_name(&store).unwrap();
        assert_eq!(name.as_deref(), Some("Home"));
        assert!(at_ms > 1);
    }

    // The pack was named by a device whose clock runs ahead of this one. Stamped
    // `now`, the typed name would read as the older of the two: the sync that
    // follows the restore would put the packed name back and skip the upload,
    // because both sides would then agree — losing the override in silence.
    #[test]
    fn a_typed_name_outranks_a_pack_stamped_in_the_future() {
        let store = store();
        // Far enough ahead that no real clock reaches it during the test.
        let packed_ms = auth::now_ms() + 60 * 60 * 1000;
        identity::set_vault_name(&store, "Work", packed_ms).unwrap();

        assert_eq!(
            settle_name(&store, "Home".into(), "9f3c1a2b").unwrap(),
            "Home"
        );
        let (name, at_ms) = identity::vault_name(&store).unwrap();
        assert_eq!(name.as_deref(), Some("Home"));
        assert!(identity::name_wins(
            (name.as_deref(), at_ms),
            (Some("Work"), packed_ms)
        ));
    }
}
