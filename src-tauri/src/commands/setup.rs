//! First-run onboarding: connecting Google Drive *before* a vault exists.
//!
//! Every other path through `sync` needs an unlocked session, because the OAuth
//! tokens live in a file sealed under the vault key. On a fresh install there is
//! no key and no vault, so the order has to be inverted: consent first, tokens
//! held in memory ([`AppState::pending_drive`]), and the account only written
//! down once the user's choice — restore what is up there, or start over — has
//! produced a key to seal it with.
//!
//! The connect step reports through events rather than its return value, so the
//! frontend can render "waiting for the browser" while the flow is out. Exactly
//! one of `setup:drive:probed` / `setup:drive:error` follows every
//! `setup:drive:pending` (see [`crate::events`]).
//!
//! That first half is not onboarding's alone. Adding a workspace by restoring
//! one of the account's other vaults has the same problem — a vault that does
//! not exist yet, so no key to seal an account under — and so borrows
//! [`connect_pending`], [`peek_pending`], [`download`] and
//! [`setup_drive_disconnect`] from here rather than copying them (see
//! `commands::workspace`). What that flow does *not* share is
//! [`guard_no_vault`], which is exactly the condition it inverts.

use std::sync::atomic::Ordering;

use tauri::{AppHandle, Manager, State};
use zeroize::Zeroizing;

use crate::crypto::VaultKey;
use crate::error::{Error, Result};
use crate::events;
use crate::models::UnlockResult;
use crate::session::{create_vault, list_metas, store_err};
use crate::state::{AppState, PendingDrive};
use crate::storage;
use crate::store::SqliteStore;
use crate::sync::{self, restore, setup::PackInfo};

// --- connect ---------------------------------------------------------------

/// Connect an account and report what it holds. Onboarding only.
#[cfg(desktop)]
#[tauri::command]
pub fn setup_drive_connect(app: AppHandle) -> Result<()> {
    guard_no_vault(&app)?;
    connect_pending(&app)
}

/// The mobile twin of [`setup_drive_connect`].
#[cfg(mobile)]
#[tauri::command]
pub fn setup_drive_connect(app: AppHandle, state: State<'_, AppState>) -> Result<()> {
    guard_no_vault(&app)?;
    connect_pending(&app, &state)
}

/// Consent, probe, report — the whole of connecting an account whose tokens
/// have nowhere to be written yet.
///
/// Shared with `commands::workspace`, which offers the same flow from Settings
/// to add a workspace by restoring one of the account's other vaults. The
/// precondition is the only difference between the two callers, and it is the
/// opposite one: onboarding refuses to run once a vault exists, and a workspace
/// restore is the case where one already does. Everything after it is the same
/// — the tokens wait in [`AppState::pending_drive`] until whichever flow
/// follows produces a key to seal them under, and the same probe answers on the
/// same `setup:drive:*` events, so one screen's picker serves both.
///
/// Desktop: the consent flow waits on a loopback listener and the probe is a
/// network round trip, so both go to the blocking pool — the command returns as
/// soon as its guards have passed, and the frontend listens for the events.
#[cfg(desktop)]
pub(crate) fn connect_pending(app: &AppHandle) -> Result<()> {
    ensure_idle(&app.state::<AppState>())?;
    let attempt = begin_attempt(&app.state::<AppState>());
    events::setup_drive_pending(app);

    let app = app.clone();
    super::detached(move || {
        let probed = sync::obtain_tokens(&app).and_then(|mut tokens| {
            // `block_on` is legal here because a blocking-pool thread is not one
            // of the async runtime's workers. Same rule as a sync run.
            let files = tauri::async_runtime::block_on(probe(&app, &mut tokens))?;
            Ok((tokens, files))
        });
        report(&app, attempt, probed);
    });
    Ok(())
}

/// The mobile twin: start consent and return — iOS suspends the app behind
/// Safari, so there is no result to wait for. [`on_consent`] finishes it.
///
/// One purpose covers both callers. [`crate::state::AuthPurpose`] says what the
/// redirect does when it comes back, and that is identical here: redeem the
/// code, probe, and leave the tokens pending. Which flow asked is the
/// frontend's to remember, not the deep-link handler's.
#[cfg(mobile)]
pub(crate) fn connect_pending(app: &AppHandle, state: &AppState) -> Result<()> {
    ensure_idle(state)?;
    begin_attempt(state);
    crate::commands::sync::start_consent(app, state, crate::state::AuthPurpose::Setup)
}

/// Mobile, second half: redeem the code the deep-link handler accepted, then
/// probe — reported through the same two events as desktop. The redirect was
/// matched against `pending_auth`, which a disconnect clears, so an attempt
/// that gets this far is still the current one.
#[cfg(mobile)]
pub(crate) async fn on_consent(app: &AppHandle, code: &str, verifier: &str) {
    let attempt = current_attempt(&app.state::<AppState>());
    let probed = match sync::exchange_for_tokens(app, code, verifier).await {
        Ok(mut tokens) => probe(app, &mut tokens).await.map(|files| (tokens, files)),
        Err(e) => Err(e),
    };
    report(app, attempt, probed);
}

/// Forget the connected account. "Go back" and "switch account" are the same
/// thing to the backend: nothing was written, so nothing needs undoing — but a
/// consent still out with the browser has to be disowned too, or its tokens
/// would land in `pending_drive` after the user had already moved on.
///
/// Not onboarding's alone: cancelling the Settings dialog that restores a
/// workspace from Drive means exactly this, so it calls the same command rather
/// than a second spelling of it. There is no vault to guard here — the whole
/// point is that nothing has been written yet.
#[tauri::command]
pub fn setup_drive_disconnect(state: State<'_, AppState>) -> Result<()> {
    ensure_idle(&state)?;
    // Invalidate first. An adoption may be holding the pending-token lock while
    // it seals credentials; advancing the attempt lets its post-write check see
    // this cancellation and remove the file again before it reports connected.
    abandon_attempt(&state);
    take_pending(&state);
    // On mobile the half-finished consent is a record, not a thread: dropping
    // it makes the redirect, if it ever comes, a stranger the handler ignores.
    #[cfg(mobile)]
    {
        let mut pending = state.pending_auth.lock().unwrap();
        if pending
            .as_ref()
            .is_some_and(|p| p.purpose == crate::state::AuthPurpose::Setup)
        {
            *pending = None;
        }
    }
    Ok(())
}

/// Which vaults does the account hold? Refreshes `tokens` in place if the
/// access token consent just minted has somehow already expired.
///
/// Every one of them, not the pick of them: an account can hold a pack per
/// vault (two installs, two primaries, two ids), and it is the user who has to
/// say which of those is theirs.
///
/// Crate-visible: `commands::sync::sync_adopt_pending` asks the same question
/// of the pending account before deciding whether the open vault may join it.
pub(crate) async fn probe(app: &AppHandle, tokens: &mut sync::Tokens) -> Result<Vec<PackInfo>> {
    let client = sync::http_client();
    let token = sync::fresh_access_token(&client, app, tokens).await?;
    sync::setup::find_packs(&client, &token).await
}

/// The one ending for a connect attempt, so the frontend always hears exactly
/// one answer and the pending tokens only ever survive a success.
///
/// An attempt the user has since backed out of (or replaced) hears nothing
/// and keeps nothing: adopting its account now would silently connect a
/// fresh vault to a sign-in the user thought they had cancelled.
fn report(app: &AppHandle, attempt: u64, probed: Result<(sync::Tokens, Vec<PackInfo>)>) {
    let state = app.state::<AppState>();
    // `begin_attempt` waits for this guard after advancing the generation. That
    // makes publication and the corresponding event one ordered operation: an
    // old callback can finish before a replacement announces "pending", or be
    // discarded after it starts, but can never publish over the newer attempt.
    let mut pending = state.pending_drive.lock().unwrap();
    if current_attempt(&state) != attempt {
        log::info!("drive setup: dropping a consent the user already abandoned");
        return;
    }
    match probed {
        Ok((tokens, files)) => {
            *pending = Some(PendingDrive { attempt, tokens });
            events::setup_drive_probed(app, files);
        }
        Err(e) => {
            // Half a connection is worse than none — the next attempt starts
            // from consent rather than from credentials that failed once.
            if pending
                .as_ref()
                .is_some_and(|pending| pending.attempt == attempt)
            {
                *pending = None;
            }
            log::warn!("drive setup failed: {e}");
            events::setup_drive_error(app, &e.to_string());
        }
    }
}

// --- restore ---------------------------------------------------------------

/// Adopt one of the connected account's vaults as this device's, with the
/// master password it was created under. `file_id` is the Drive file the user
/// chose from what the probe listed.
///
/// A wrong password leaves the install exactly as it was (see
/// [`restore::restore_from_pack`]) *and* leaves the account connected, so the
/// retry costs the user nothing but the typing. No lockout bookkeeping: there
/// is no local vault to throttle guesses against yet, and the pack is Argon2id
/// + SQLCipher on its own terms.
#[tauri::command]
pub async fn setup_restore_from_drive(
    password: Zeroizing<String>,
    file_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<UnlockResult> {
    guard_no_vault(&app)?;
    let _step = begin_step(&state)?;
    let mut tokens = peek_pending(&state)?;

    // Nothing to refuse on a fresh install: there is no vault here for the
    // chosen one to collide with, which is what `guard_no_vault` just said.
    let (bytes, vault_id) = download(&app, &mut tokens, &file_id, |_| Ok(())).await?;
    // Whatever the download's refresh produced has to be kept: Google rotates
    // refresh tokens, and a retry after a mistyped password uses these again.
    replace_pending_tokens(&state, tokens.clone())?;

    // Kept past the restore for the account's *other* vaults: the ones this
    // password opens too are added beside this one (`commands::autojoin`).
    let join = password.clone();
    let (key, store) = restore_off_thread(&app, bytes, password).await?;
    let result = adopt(&app, &state, key, store, Some(&tokens), Some(&vault_id))?;
    super::autojoin::with_password(&app, join);
    Ok(result)
}

/// Restore from a `.rowel` backup on disk. Onboarding only.
///
/// The same pack the sync engine uploads, written by `export_vault` instead of
/// pulled from Drive — so this is [`setup_restore_from_drive`] with the
/// download swapped for a file read, and no account to keep afterwards. The
/// read shares the blocking thread with the restore: a backup is the whole
/// vault, and a large one on a slow disk must not stall the async runtime. No
/// size cap, for the same reason the Drive download has none — the pack's own
/// structural checks (`pack::unpack`) are what reject a file that is not one.
#[tauri::command]
pub async fn setup_restore_from_file(
    path: String,
    password: Zeroizing<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<UnlockResult> {
    guard_no_vault(&app)?;
    let _step = begin_step(&state)?;

    let handle = app.clone();
    let (key, store) = super::blocking(move || {
        let bytes = std::fs::read(&path)?;
        restore::restore_from_pack(&handle, &bytes, &password)
    })
    .await?;
    adopt(&app, &state, key, store, None, None)
}

/// Fetch the chosen pack, and say which vault its name says it is.
///
/// The listing is taken again rather than the probe's answer trusted: the two
/// are minutes apart, and an id that is no longer there fails as
/// [`Error::NoRemoteVault`] — "no vault up there any more", which is a clear
/// failure where a stale id would be a confusing one.
///
/// `accept` is asked about the vault id the listing resolved, before a byte of
/// the pack is pulled: a restore the caller is going to refuse should not cost
/// the user the wait for a whole vault over the network. Crate-visible for
/// `commands::workspace`, which fetches the same way and refuses the vault it
/// already has open.
pub(crate) async fn download(
    app: &AppHandle,
    tokens: &mut sync::Tokens,
    file_id: &str,
    accept: impl FnOnce(&str) -> Result<()>,
) -> Result<(Vec<u8>, String)> {
    let client = sync::http_client();
    let token = sync::fresh_access_token(&client, app, tokens).await?;
    let file = sync::setup::find_pack_by_id(&client, &token, file_id).await?;
    accept(&file.vault_id)?;
    let bytes = sync::setup::download_pack(&client, &token, &file.id).await?;
    Ok((bytes, file.vault_id))
}

// Argon2id + a SQLCipher open, both CPU-bound: off the command thread, as unlock does.
async fn restore_off_thread(
    app: &AppHandle,
    bytes: Vec<u8>,
    password: Zeroizing<String>,
) -> Result<(VaultKey, SqliteStore)> {
    let app = app.clone();
    super::blocking(move || restore::restore_from_pack(&app, &bytes, &password)).await
}

// --- create ----------------------------------------------------------------

/// Create a new vault, turning sync on if an account was connected first.
///
/// With no pending account this is exactly `setup`: a fresh vault, sync off.
/// With one, the credentials are sealed under the new key and the session is
/// marked connected — the frontend's `enterMain` runs the first sync, which is
/// what creates the folder and the pack on Drive.
///
/// The account is expected to be empty when tokens are pending: the first run
/// offers no way past a probe that found a vault except restoring it. A create
/// that reached here beside one anyway would not fork the account — the first
/// sync refuses to mint an id next to an existing vault (`sync::plan_vault_id`).
#[tauri::command]
pub async fn setup_create(
    password: Zeroizing<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<UnlockResult> {
    // `create_vault` writes a fresh KDF sidecar before it opens the database,
    // so running it over an existing vault would leave that vault unopenable.
    guard_no_vault(&app)?;
    let _step = begin_step(&state)?;
    // Creating without Drive is valid; a stale result from a cancelled or
    // replaced attempt is not.
    let tokens = peek_pending(&state).ok();

    let (key, store) = create_off_thread(&app, password).await?;
    adopt(&app, &state, key, store, tokens.as_ref(), None)
}

// Argon2id + creating the encrypted DB: CPU-bound, same as the restore path.
// Shared with `commands::workspace`, which creates a vault the same way.
pub(crate) async fn create_off_thread(
    app: &AppHandle,
    password: Zeroizing<String>,
) -> Result<(VaultKey, SqliteStore)> {
    let app = app.clone();
    super::blocking(move || create_vault(&app, &password)).await
}

// --- shared ----------------------------------------------------------------

/// Make a vault that has just landed on disk this device's: seal the pending
/// account under its key, open the session, forget the pending copy.
///
/// All or nothing. The vault is already committed when this runs, so a failure
/// here — the token file not writing, the store not listing — would otherwise
/// leave a vault on disk with no session behind it and `guard_no_vault`
/// refusing every retry: an install the user can neither finish nor start
/// over. Removing what was just written puts them back on a fresh install,
/// where the retry is one press away.
///
/// `vault_id` is the id the restored pack's own file name gave it, when it came
/// from `Vaults/<id>.rowel`. Stamped here, inside the all-or-nothing, rather
/// than after the fact.
fn adopt(
    app: &AppHandle,
    state: &AppState,
    key: VaultKey,
    store: SqliteStore,
    tokens: Option<&sync::Tokens>,
    vault_id: Option<&str>,
) -> Result<UnlockResult> {
    // Id first, then metadata, tokens last: nothing else is written until the
    // one read that could fail has succeeded, so a failure here leaves no token
    // file sealed under a key that is about to be discarded.
    let stamped = match vault_id {
        Some(id) => stamp_vault_id(&store, id),
        // A vault created from scratch, or restored from a backup file that no
        // account addresses: it mints its own id on its first sync.
        None => Ok(()),
    };
    let installed = stamped.and_then(|()| {
        let entries = list_metas(&store)?;
        if let Some(tokens) = tokens {
            sync::persist_tokens(app, &key.cryptor(), tokens)?;
        }
        Ok(entries)
    });
    match installed {
        Ok(entries) => {
            // A restored pack carries the vault's name, and the header reads the
            // registry — so the primary arrives under the name the user's other
            // devices show it by rather than under the default label. A pack
            // with no name leaves the entry as it was.
            if let Ok((Some(name), _)) = crate::store::identity::vault_name(&store) {
                if let Err(e) = crate::workspace::record_vault_name(app, &name) {
                    log::warn!("cannot record the restored vault's name: {e}");
                }
            }
            let sync_configured = tokens.is_some();
            // Copied before the key goes into the session: this is the app
            // key, and the ring is what every later workspace is sealed under.
            let material = Zeroizing::new(key.biometric_material().to_vec());
            state
                .session
                .lock()
                .unwrap()
                .set(key, store, sync_configured);
            // A vault created or restored at first run is an open vault, and
            // it goes on the idle clock here rather than waiting for the
            // frontend's first activity ping.
            crate::autolock::touch(app);
            #[cfg(desktop)]
            crate::browser::server::notify_unlocked();
            crate::appkey::adopt(app, &crate::workspace::active_id(app), &material);
            take_pending(state);
            Ok(UnlockResult {
                entries,
                sync_configured,
            })
        }
        Err(e) => {
            // The connection has to close before its files can go.
            drop(store);
            discard_fresh_vault(app);
            Err(e)
        }
    }
}

/// Make the restored vault answer to the name the account addresses it by.
///
/// The file name is authoritative, not whatever the pack carries inside: a pack
/// written before ids were stamped into `meta` carries none at all, and this
/// device has to reach `Vaults/<id>.rowel` on its very first sync — under a
/// different id it would upload a second pack beside the one it just restored.
fn stamp_vault_id(store: &SqliteStore, vault_id: &str) -> Result<()> {
    crate::store::identity::adopt_vault_id(store, vault_id).map_err(store_err)
}

/// Undo `create_vault` / `restore_from_pack`: remove the database (with its
/// WAL siblings) and the KDF sidecar, in that order — a sidecar without a
/// database is harmless, a database without its sidecar reads as "wrong
/// password" forever. The token file goes too: the write that failed may have
/// left part of one behind, sealed under a key that no longer exists, and a
/// non-empty file there is what `sync_configured` reads as "syncing".
fn discard_fresh_vault(app: &AppHandle) {
    if let Ok(path) = storage::db_path(app) {
        storage::remove_db_files(&path);
    }
    if let Ok(path) = storage::kdf_sidecar_path(app) {
        let _ = std::fs::remove_file(path);
    }
    // Best effort, unlike the disconnect path: this is already the cleanup of a
    // failed setup, and there is no vault left for a surviving token file to
    // sync — the next create or restore overwrites it before anything reads it.
    let _ = storage::remove_gdrive(app);
}

/// Refuse to touch the pending account while a create or restore is using it.
///
/// A restore clones the tokens before its network and Argon2 work, so a
/// disconnect (or a fresh connect) landing mid-way could not stop it — it
/// would finish and seal the vault to the account the user had just left.
/// The step already excludes other steps; this excludes the account changing
/// under one.
fn ensure_idle(state: &AppState) -> Result<()> {
    if state.setup_busy.load(Ordering::SeqCst) {
        return Err(Error::SetupBusy);
    }
    Ok(())
}

/// Hold `setup_busy` for the length of one create or restore.
///
/// `guard_no_vault` looks before the write; this makes sure nothing else
/// writes in between. Two overlapping requests would each write their own
/// KDF sidecar and database, and whichever finished second would pair a
/// database with the other's descriptor — a vault nobody's password opens.
pub(crate) struct SetupStep<'a>(&'a AppState);

// Also held by `workspace_create`, which writes the same two files.
pub(crate) fn begin_step(state: &AppState) -> Result<SetupStep<'_>> {
    state
        .setup_busy
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .map_err(|_| Error::SetupBusy)?;
    Ok(SetupStep(state))
}

impl Drop for SetupStep<'_> {
    fn drop(&mut self) {
        self.0.setup_busy.store(false, Ordering::SeqCst);
    }
}

/// Start a connect attempt; the number identifies it to [`report`].
fn begin_attempt(state: &AppState) -> u64 {
    // Advance first so an adoption holding `pending_drive` can observe the
    // replacement and roll back its write. Waiting for that same lock before
    // returning also orders the old callback's event before the new caller's
    // "pending" event, and retires credentials from the previous attempt.
    let attempt = state.setup_attempt.fetch_add(1, Ordering::SeqCst) + 1;
    *state.pending_drive.lock().unwrap() = None;
    attempt
}

/// Disown whatever attempt is in flight: its `report` will find itself stale.
fn abandon_attempt(state: &AppState) {
    state.setup_attempt.fetch_add(1, Ordering::SeqCst);
}

pub(crate) fn current_attempt(state: &AppState) -> u64 {
    state.setup_attempt.load(Ordering::SeqCst)
}

/// Atomically make one consent attempt the completed connection. Cancellation
/// and replacement advance the same counter, so exactly one side can win: a
/// loser must roll back anything it wrote before trying this transition.
pub(crate) fn complete_attempt(state: &AppState, attempt: u64) -> bool {
    state
        .setup_attempt
        .compare_exchange(
            attempt,
            attempt.wrapping_add(1),
            Ordering::SeqCst,
            Ordering::SeqCst,
        )
        .is_ok()
}

/// Onboarding only. A device that already holds a vault has other routes to
/// Drive (`sync_connect`, `workspace_restore_from_drive`), and every one of
/// them merges or adds a workspace rather than replaces — which is the point:
/// nothing here may be reachable in a way that could overwrite a vault this
/// device already has.
fn guard_no_vault(app: &AppHandle) -> Result<()> {
    if storage::db_exists(app) {
        return Err(Error::AlreadySetUp);
    }
    Ok(())
}

/// The connected account's tokens, left in place for a retry. Crate-visible
/// because a workspace restore reads them the same way.
pub(crate) fn peek_pending(state: &AppState) -> Result<sync::Tokens> {
    peek_pending_attempt(state).map(|(_, tokens)| tokens)
}

/// The pending account together with the consent attempt that produced it.
/// Consumers that await must carry the attempt through the await and compare it
/// again before writing, so a cancel or replacement account cannot be mistaken
/// for the account they originally probed.
pub(crate) fn peek_pending_attempt(state: &AppState) -> Result<(u64, sync::Tokens)> {
    let pending = state.pending_drive.lock().unwrap();
    let pending = pending.as_ref().ok_or(Error::DriveNotConnected)?;
    if current_attempt(state) != pending.attempt {
        return Err(Error::DriveNotConnected);
    }
    Ok((pending.attempt, pending.tokens.clone()))
}

/// Keep a refresh-token rotation only for the account that is still pending.
pub(crate) fn replace_pending_tokens(state: &AppState, tokens: sync::Tokens) -> Result<()> {
    let mut pending = state.pending_drive.lock().unwrap();
    let pending = pending.as_mut().ok_or(Error::DriveNotConnected)?;
    if current_attempt(state) != pending.attempt {
        return Err(Error::DriveNotConnected);
    }
    pending.tokens = tokens;
    Ok(())
}

/// Forget the connected account, if there is one.
pub(crate) fn take_pending(state: &AppState) -> Option<sync::Tokens> {
    state
        .pending_drive
        .lock()
        .unwrap()
        .take()
        .map(|pending| pending.tokens)
}

/// Forget one particular consent result without disturbing a newer account.
pub(crate) fn take_pending_attempt(state: &AppState, attempt: u64) -> Option<sync::Tokens> {
    let mut pending = state.pending_drive.lock().unwrap();
    if pending
        .as_ref()
        .is_some_and(|pending| pending.attempt == attempt)
    {
        pending.take().map(|pending| pending.tokens)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(access: &str) -> sync::Tokens {
        sync::Tokens {
            access_token: Some(access.into()),
            refresh_token: Some("refresh".into()),
            expires_at: Some(4_102_444_800),
        }
    }

    fn set_pending(state: &AppState, access: &str) -> u64 {
        let attempt = begin_attempt(state);
        *state.pending_drive.lock().unwrap() = Some(PendingDrive {
            attempt,
            tokens: tokens(access),
        });
        attempt
    }

    // A fresh install has connected nothing, so restore has nothing to ask for
    // — and says so in words the onboarding screen can show.
    #[test]
    fn nothing_is_pending_until_an_account_is_connected() {
        let state = AppState::default();
        assert!(matches!(
            peek_pending(&state),
            Err(Error::DriveNotConnected)
        ));
        assert!(take_pending(&state).is_none());
    }

    // Peek leaves the account connected (a mistyped master password must be
    // retryable), take ends it (that is what "go back" means).
    #[test]
    fn peeking_keeps_the_account_and_taking_ends_it() {
        let state = AppState::default();
        set_pending(&state, "first");

        assert_eq!(
            peek_pending(&state).unwrap().access_token.as_deref(),
            Some("first")
        );
        assert_eq!(
            peek_pending(&state).unwrap().access_token.as_deref(),
            Some("first")
        );

        assert_eq!(
            take_pending(&state).unwrap().access_token.as_deref(),
            Some("first")
        );
        assert!(peek_pending(&state).is_err());
    }

    // "Switch account" is disconnect + connect: the second consent replaces the
    // first outright, so no stale credentials can be used by mistake.
    #[test]
    fn connecting_again_replaces_the_account() {
        let state = AppState::default();
        set_pending(&state, "first");
        set_pending(&state, "second");
        assert_eq!(
            take_pending(&state).unwrap().access_token.as_deref(),
            Some("second")
        );
    }

    #[test]
    fn starting_a_replacement_retires_the_previous_account() {
        let state = AppState::default();
        set_pending(&state, "first");

        begin_attempt(&state);

        assert!(matches!(
            peek_pending_attempt(&state),
            Err(Error::DriveNotConnected)
        ));
    }

    // Two creates (or a create and a restore) cannot both be writing the vault:
    // the second is turned away, and the way is clear again once the first is
    // done — however it ended.
    #[test]
    fn only_one_setup_step_writes_at_a_time() {
        let state = AppState::default();
        let step = begin_step(&state).unwrap();
        assert!(matches!(begin_step(&state), Err(Error::SetupBusy)));
        drop(step);
        assert!(begin_step(&state).is_ok());
    }

    // While a create or restore holds the account, nothing may swap it out
    // from under it — a disconnect then would not stop the restore, only make
    // it finish against an account the user thought they had left.
    #[test]
    fn the_account_cannot_change_while_a_step_is_using_it() {
        let state = AppState::default();
        assert!(ensure_idle(&state).is_ok());

        let step = begin_step(&state).unwrap();
        assert!(matches!(ensure_idle(&state), Err(Error::SetupBusy)));

        drop(step);
        assert!(ensure_idle(&state).is_ok());
    }

    // A consent the user backed out of comes back stale: `report` compares the
    // number it was started with against the current one.
    #[test]
    fn backing_out_disowns_the_attempt_in_flight() {
        let state = AppState::default();
        let attempt = begin_attempt(&state);
        assert_eq!(current_attempt(&state), attempt);

        abandon_attempt(&state);
        assert_ne!(current_attempt(&state), attempt);

        // ...and the next connect is its own attempt, distinct from both.
        let next = begin_attempt(&state);
        assert_ne!(next, attempt);
        assert_eq!(current_attempt(&state), next);
    }

    #[test]
    fn a_cancelled_or_replaced_attempt_cannot_be_peeked() {
        let state = AppState::default();
        let cancelled = set_pending(&state, "first");
        abandon_attempt(&state);
        assert_ne!(current_attempt(&state), cancelled);
        assert!(matches!(
            peek_pending_attempt(&state),
            Err(Error::DriveNotConnected)
        ));

        let current = set_pending(&state, "second");
        let (attempt, tokens) = peek_pending_attempt(&state).unwrap();
        assert_eq!(attempt, current);
        assert_eq!(tokens.access_token.as_deref(), Some("second"));
    }

    #[test]
    fn finishing_an_old_attempt_cannot_remove_the_new_account() {
        let state = AppState::default();
        let old = set_pending(&state, "first");
        let current = set_pending(&state, "second");

        assert!(take_pending_attempt(&state, old).is_none());

        let (attempt, tokens) = peek_pending_attempt(&state).unwrap();
        assert_eq!(attempt, current);
        assert_eq!(tokens.access_token.as_deref(), Some("second"));
    }

    #[test]
    fn cancellation_and_completion_have_exactly_one_winner() {
        let completed = AppState::default();
        let attempt = set_pending(&completed, "first");
        assert!(complete_attempt(&completed, attempt));
        assert!(!complete_attempt(&completed, attempt));

        let cancelled = AppState::default();
        let attempt = set_pending(&cancelled, "first");
        abandon_attempt(&cancelled);
        assert!(!complete_attempt(&cancelled, attempt));
    }

    // The event names are the frontend's contract.
    #[test]
    fn the_onboarding_events_keep_their_names() {
        assert_eq!(events::SETUP_DRIVE_PENDING, "setup:drive:pending");
        assert_eq!(events::SETUP_DRIVE_PROBED, "setup:drive:probed");
        assert_eq!(events::SETUP_DRIVE_ERROR, "setup:drive:error");
    }
}
