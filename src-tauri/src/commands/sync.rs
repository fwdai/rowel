//! Sync commands.
//!
//! Nothing here does the work: every command validates, starts a run, and
//! returns. The run itself lives on a thread of its own and reports through
//! `sync:status`. That split is the whole point of this rewrite — the first
//! Drive implementation drove Google's API from the command thread and froze
//! the window for the length of a round trip.
//!
//! The frontend never reconstructs sync state from a sequence of events: every
//! transition updates [`SyncRun`] and re-emits the whole [`SyncStatus`], which
//! the frontend stores as-is. One event, one shape, no order to agree on.
//!
//! Every run starts in [`start_run`], whichever command asked for it — a scheduled
//! sync, `sync_now`, or the run behind a connect. One launcher means one claim,
//! and one claim is what keeps two runs from addressing the account at the same
//! time.
//!
//! Connecting is not a run either. Every connect is keyless: the account is
//! asked what it holds first (`commands::setup::connect_pending`, the same
//! consent and probe onboarding runs), and only [`sync_adopt_pending`] seals
//! its tokens under the open vault — when the account is empty or already
//! holds this vault. An account holding other vaults is offered to restore
//! instead: every device connected to an account syncs the vaults it holds
//! rather than adding a pack beside them. The adoption is bound to the
//! workspace that started it: a switch during the probe is caught before the
//! tokens are sealed, never after.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
#[cfg(mobile)]
use std::time::Duration;

use tauri::{AppHandle, Manager, State};

use crate::crypto::Cryptor;
use crate::error::{Error, Result};
use crate::events;
use crate::models::EntryMetaDto;
use crate::session::list_metas;
#[cfg(mobile)]
use crate::state::AuthPurpose;
use crate::state::{AppState, SyncFailure, SyncRun, SyncStatus};
use crate::sync;
use crate::sync::setup::PackInfo;

/// Connect a sync provider (OAuth) to the vault that is open.
///
/// Keyless, always: the account is asked what it holds before anything is
/// sealed under this vault's key. Consent and probe run on the blocking pool
/// and report on the `setup:drive:*` events — the ones onboarding and Settings
/// › Workspaces listen to — and the frontend then calls [`sync_adopt_pending`],
/// which joins the account only if it is empty or already holds this vault,
/// and otherwise leaves the account's vaults to be restored. Offered only on a
/// vault that is not connected, so there is no other road to take.
///
/// Any workspace may connect: the token file resolves under the active
/// workspace's directory, and the vault's own id names the pack it syncs
/// (`Rowel/Vaults/<vault-id>.rowel`), so two workspaces share neither
/// credentials nor a file even when the user points them at the same Drive.
/// What they do share is the OAuth grant behind those credentials, which Google
/// keeps per account and client rather than per token — see [`sync_disconnect`].
///
/// A connect that cannot start (another setup step is still running) is
/// reported as status as well as rejected, so the frontend never has to turn a
/// rejected promise into state itself.
#[cfg(desktop)]
#[tauri::command]
pub fn sync_connect(app: AppHandle) -> Result<()> {
    super::setup::connect_pending(&app).inspect_err(|e| failed(&app, e))
}

/// Start the consent flow and return — see [`on_redirect`] for the other half.
///
/// Nothing here waits: Safari takes the screen and iOS suspends the app behind
/// it, so there is no result to wait for. The frontend is told by the
/// `setup:drive:*` events rather than by this promise, as on desktop.
///
/// Synchronous on purpose, unlike its desktop twin. It does no blocking work,
/// and running on the IPC (main) thread is what puts the opener plugin's
/// `UIApplication.open` where UIKit requires it.
#[cfg(mobile)]
#[tauri::command]
pub fn sync_connect(app: AppHandle, state: State<'_, AppState>) -> Result<()> {
    super::setup::connect_pending(&app, &state).inspect_err(|e| failed(&app, e))
}

/// Make the account a connect left pending this vault's — if the account will
/// have it.
///
/// The account is the source of truth for which vaults exist, so the decision
/// is read off the account itself, with the pending tokens: one holding no
/// pack takes this vault as its first, and one already holding this vault's
/// id takes it as another device of that vault. One holding only *other*
/// vaults refuses ([`Error::VaultNotInAccount`]) and leaves the tokens
/// pending, so the frontend can offer those vaults to restore instead of
/// adding a pack beside them (see [`may_adopt`]).
///
/// Adopting seals the tokens under the open vault's key, and the run that
/// follows creates the pack — or joins the one already there, since
/// `plan_vault_id` keeps an id the vault already has. Nothing pending is an
/// error rather than a silent success: the frontend would otherwise show
/// "Connected" over a vault with no token file.
///
/// The adoption is bound to the workspace that started it. The probe is a
/// network round trip with no lock held, and a pending account is
/// workspace-agnostic by design — onboarding and the Workspaces restore both
/// adopt one with no vault open, so `commands::workspace::guard_sync_idle`
/// does not treat it as busy and a switch may land while Google is answering.
/// The workspace id and vault id are therefore read together under
/// `workspace_lock` before the probe and checked again under it before anything
/// is written ([`still_the_same_workspace`]): sealing the tokens under whichever
/// vault happened to be open afterwards would give that vault the account, and
/// its first sync would create a second pack beside the one the user meant.
#[tauri::command]
pub async fn sync_adopt_pending(app: AppHandle, state: State<'_, AppState>) -> Result<()> {
    // Which workspace is asking, and which vault it holds, as one read under
    // the workspace lock: a switch moves both under that lock, so the pair
    // cannot straddle one. Released before the network call below.
    let (workspace, vault_id) = {
        let _paths = state.workspace_lock.lock().unwrap();
        let workspace = state.active_workspace.lock().unwrap().clone();
        (workspace, open_vault_id(&state)?)
    };
    // Asked of the account on a copy of the pending tokens, and with no lock
    // held: this is a network round trip (see `AppState::workspace_lock`). The
    // probe refreshes the copy in place if the access token has expired since
    // consent, which is why the copy, not the pending original, is what gets
    // sealed below.
    let (attempt, mut tokens) = super::setup::peek_pending_attempt(&state)?;
    let packs = super::setup::probe(&app, &mut tokens).await?;
    if !may_adopt(vault_id.as_deref(), &packs) {
        return Err(Error::VaultNotInAccount);
    }
    // Key, tokens and flag under the workspace lock, as one step: a switch
    // cannot land between taking this workspace's key and sealing the account
    // under it (see `commands::workspace::guard_sync_idle`).
    let run = {
        let _paths = state.workspace_lock.lock().unwrap();
        // Still the workspace that asked? The tokens belonged to a connect the
        // user walked away from, so they go: a retry from the other workspace
        // would only seal them under the wrong key again. A session that merely
        // locked during the round trip is not a switch, and keeps them for a
        // retry as before (`Error::Locked` from the read).
        if !still_the_same_workspace(&state, &workspace, vault_id.as_deref())? {
            super::setup::take_pending_attempt(&state, attempt);
            return Err(Error::Other(
                "the workspace changed while Google was answering; connect again from the workspace you want to sync".into(),
            ));
        }
        let cryptor = state.session.lock().unwrap().cryptor()?;
        // Still the exact consent result we probed? A cancel or another connect
        // advances the attempt before it touches this lock. Holding the pending
        // guard through the local write keeps a newer report from being
        // mistaken for this account, while the post-write generation check
        // catches a cancel that arrived during the write itself.
        let mut pending = state.pending_drive.lock().unwrap();
        let Some(current) = pending.as_mut() else {
            return Err(Error::DriveNotConnected);
        };
        if current.attempt != attempt || super::setup::current_attempt(&state) != attempt {
            return Err(Error::DriveNotConnected);
        }
        // Keep a refresh-token rotation for a retry if the file write fails.
        current.tokens = tokens.clone();
        sync::persist_tokens(&app, &cryptor, &tokens)?;

        // Claim the first run before clearing the pending status, so a workspace
        // switch cannot land between those two busy flags. The claim is only
        // started after the connection transition below has won.
        let claim = claim_run(&state.syncing);

        // This compare-and-swap is the connection's commit point. A cancel or a
        // replacement advances the same generation before waiting for the
        // pending-token guard, so it either wins here and makes us roll the file
        // back, or arrives after this attempt is already connected. There is no
        // unowned interval between clearing `pending` and calling `connected`.
        if !super::setup::complete_attempt(&state, attempt) {
            sync::disconnect(&app)?;
            if pending
                .as_ref()
                .is_some_and(|pending| pending.attempt == attempt)
            {
                *pending = None;
            }
            return Err(Error::DriveNotConnected);
        }

        // Announce the committed connection while both the workspace and its run
        // claim are still pinned. Only then retire the pending credentials.
        connected(&app);
        *pending = None;
        claim.map(|claim| (claim, cryptor))
    };

    if let Some((claim, cryptor)) = run {
        spawn_run(&app, claim, cryptor);
    }
    Ok(())
}

/// Whether the open vault, with `vault_id` if it has one, may take an account
/// holding `packs`: yes for an empty account, and for one that already holds
/// this very vault; no when the account holds only other vaults. A vault with
/// no id yet can only ever be an empty account's first.
fn may_adopt(vault_id: Option<&str>, packs: &[PackInfo]) -> bool {
    packs.is_empty() || vault_id.is_some_and(|id| packs.iter().any(|pack| pack.vault_id == id))
}

/// Is `workspace`, holding the vault with `vault_id`, still the one that is
/// open? The caller holds `workspace_lock`, so what this reads is what the
/// write that follows will address.
///
/// Both halves are checked: the same workspace re-unlocked is fine, a different
/// workspace is not, and neither is a different vault behind the same paths — a
/// restore over the workspace would be one. A locked session is neither answer
/// and is reported as such (`Error::Locked`), since the caller treats it
/// differently from a switch.
fn still_the_same_workspace(
    state: &AppState,
    workspace: &str,
    vault_id: Option<&str>,
) -> Result<bool> {
    if *state.active_workspace.lock().unwrap() != workspace {
        return Ok(false);
    }
    Ok(open_vault_id(state)?.as_deref() == vault_id)
}

/// The open vault's id, under the session lock alone: it is a read off `meta`.
fn open_vault_id(state: &AppState) -> Result<Option<String>> {
    let session = state.session.lock().unwrap();
    let store = session.store()?;
    crate::store::identity::vault_id(store).map_err(crate::session::store_err)
}

/// Disconnect the sync provider for the workspace that is open: its token file
/// is deleted, and nothing on this device can reconnect without fresh consent.
///
/// Local only, on purpose. Google's revocation endpoint retires the whole grant
/// for an account and OAuth client — every token this app ever got for that
/// account, in every workspace and on every device — so it is not a thing one
/// workspace can do on its own behalf, and this install cannot tell which
/// other workspaces (or other devices) share the account: their token files
/// are sealed under keys it does not hold. Signing this app out of Google is a
/// separate, explicitly global action, and one the UI does not offer yet.
#[tauri::command]
pub fn sync_disconnect(app: AppHandle, state: State<'_, AppState>) -> Result<()> {
    // Only an unlocked vault can be disconnected: the token file is its own,
    // and the paths below resolve to the workspace that is open.
    state.session.lock().unwrap().cryptor()?;
    // `?`, and before anything below it: if the delete failed the token file —
    // and the usable refresh token in it — is still on disk, so the vault is
    // still connected. Flipping the session flag or clearing the run state here
    // would show the user a disconnected account over a live credential. A
    // token refresh awaiting Google meanwhile finds the connection generation
    // changed and skips its write-back (see `AppState::sync_generation`).
    sync::disconnect(&app)?;
    state.session.lock().unwrap().sync_configured = false;
    // The timestamp goes with the connection: the next one is a new pairing,
    // and "synced 3m ago" from a previous one would be a lie about it.
    update(&app, |run| {
        run.pending = false;
        run.error = None;
        run.last_synced_at = None;
    });
    Ok(())
}

/// Start a sync. Returns as soon as the run is scheduled; `sync:status`
/// reports the rest.
#[tauri::command]
pub fn sync_now(app: AppHandle, state: State<'_, AppState>) -> Result<()> {
    request_run_if_ready(&app, &state);
    Ok(())
}

/// Ask for a run after a local change, on a vault that can serve one.
///
/// A locked or unconnected vault is a silent no-op rather than an error. This
/// is a routine call, not a user action: the debounced auto-sync fires on a
/// timer and can easily land just after an auto-lock, or on a vault that was
/// never connected — neither is a sync failure to put in front of the user.
pub(crate) fn request_run_if_ready(app: &AppHandle, state: &AppState) {
    let ready = {
        let session = state.session.lock().unwrap();
        session.is_unlocked() && session.sync_configured
    };
    if ready {
        start_run(app);
    }
}

// --- the mobile consent flow ---
//
// A consent flow is a value with a lifecycle, not a flag. It is created by
// `start_consent`, identified by its `state` nonce, and ends in exactly one of
// four ways — the redirect it was waiting for (`on_redirect`), Google's refusal
// (also `on_redirect`), the user coming back without one (`on_resume`), or old
// age (`CONSENT_TTL`). Each end is announced to the frontend, so a `pending`
// status is always followed by one that is not.

/// How long a consent request stays redeemable. Google's own codes die well
/// inside this; it exists so a flow the OS never told us about (the app was
/// killed in the background, say) cannot be finished by a redirect from a
/// previous life.
#[cfg(mobile)]
const CONSENT_TTL: Duration = Duration::from_secs(10 * 60);

/// How long after the app comes back to the foreground a redirect may still
/// arrive for a pending flow before it is written off as abandoned. iOS delivers
/// the URL around the same activation, not seconds later, so this is generous.
#[cfg(mobile)]
const RESUME_GRACE: Duration = Duration::from_secs(3);

/// Open the consent page and remember what the redirect will need.
///
/// The record is stored *after* the page is opened, which cannot race: iOS
/// delivers the redirect on a later turn of the same (main) run loop, so this
/// call has long returned by then. A flow already pending is simply replaced —
/// its nonce dies with it, so its redirect, if one ever comes, is foreign.
///
/// Crate-visible: every connect on mobile — onboarding's, Settings ›
/// Workspaces' and [`sync_connect`]'s — reaches it through
/// `commands::setup::connect_pending`, so none of them forks the bookkeeping.
///
/// No key is taken here, and so no workspace lock: the tokens the redirect
/// brings back are probed and left pending, and only [`sync_adopt_pending`]
/// seals them under a vault — under the lock, when that happens.
#[cfg(mobile)]
pub(crate) fn start_consent(app: &AppHandle, state: &AppState, purpose: AuthPurpose) -> Result<()> {
    // One consent at a time, and checked before Safari is opened for a second.
    // Replacing the pending flow would orphan it: its redirect could never be
    // matched again, and its screen would say "waiting" for good. A flow the
    // user walked away from does not block for long — `on_resume` writes it
    // off once the redirect has had its chance to arrive.
    if state.pending_auth.lock().unwrap().is_some() {
        return Err(Error::Other(
            "another Google sign-in is still waiting for its answer; finish or cancel it first"
                .into(),
        ));
    }
    let started = sync::begin(app)?;
    *state.pending_auth.lock().unwrap() = Some(crate::state::PendingAuth {
        verifier: started.verifier,
        state: started.state,
        purpose,
        started: std::time::Instant::now(),
    });
    // Every connect announces itself on the `setup:drive:*` family: it is the
    // probe's answer the frontend waits for, whichever screen asked.
    events::setup_drive_pending(app);
    Ok(())
}

/// iOS reopened the app with a URL. If it is Google's answer to the consent
/// flow we have pending, finish it; anything else is not ours and leaves the
/// pending flow exactly as it was.
///
/// Registered in `lib.rs`'s `setup`. Returns immediately — the token exchange
/// is a network round trip and must not run on the URL-open callback.
#[cfg(mobile)]
pub fn on_redirect(app: &AppHandle, url: &url::Url) {
    if !sync::redirect_matches(app, url) {
        return;
    }
    let state = app.state::<AppState>();
    let mut slot = state.pending_auth.lock().unwrap();
    let Some(pending) = slot.as_ref() else {
        return;
    };
    // Judge the URL against the pending request *before* consuming it, so a URL
    // that is not the answer costs the real answer nothing.
    let code = match sync::parse_redirect(url, &pending.state) {
        sync::Redirect::Foreign => {
            log::warn!("ignoring a redirect that does not answer the pending sign-in");
            return;
        }
        sync::Redirect::Denied(why) => {
            slot.take();
            drop(slot);
            fail(app, why);
            return;
        }
        sync::Redirect::Code(code) => code,
    };
    let pending = slot.take().expect("checked above");
    drop(slot);
    if pending.started.elapsed() > CONSENT_TTL {
        fail(app, "Google sign-in took too long; try again".into());
        return;
    }

    // Redeem the code, probe the account and leave the tokens pending — the
    // one thing every purpose does with a redirect (see `AuthPurpose`).
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        crate::commands::setup::on_consent(&app, &code, &pending.verifier).await;
    });
}

/// The window is back in front. If a consent flow is still pending once the
/// redirect has had its chance to arrive, the user came back without finishing
/// it — close the flow out, so the frontend is not left waiting on an answer
/// that is never coming.
///
/// Called from the window event hook (`window.rs`) on iOS scene activation.
#[cfg(mobile)]
pub fn on_resume(app: &AppHandle) {
    // Only ever abandon the flow that was pending *at resume*: if the user
    // starts a fresh one inside the grace period, its nonce differs and it is
    // left alone.
    let Some(nonce) = app
        .state::<AppState>()
        .pending_auth
        .lock()
        .unwrap()
        .as_ref()
        .map(|p| p.state.clone())
    else {
        return;
    };
    // The process's one re-armable timer for this, not a thread that sleeps:
    // every activation would otherwise park an OS thread for the grace period,
    // and a user flicking between apps produces a run of them. Re-arming is
    // also the behaviour we want — the latest resume is the one whose grace
    // period counts.
    let handle = app.clone();
    app.state::<AppState>()
        .consent_grace
        .arm(RESUME_GRACE, move || {
            let abandoned = {
                let state = handle.state::<AppState>();
                let mut slot = state.pending_auth.lock().unwrap();
                match slot.as_ref() {
                    Some(p) if p.state == nonce => slot.take(),
                    _ => None,
                }
            };
            if abandoned.is_some() {
                fail(&handle, "Google sign-in was cancelled".into());
            }
        });
}

/// The consent flow ended without a connection. One place, so every ending
/// reports the same way — on the `setup:drive:*` family every keyless connect
/// is started under.
#[cfg(mobile)]
fn fail(app: &AppHandle, why: String) {
    log::warn!("drive setup failed: {why}");
    events::setup_drive_error(app, &why);
}

// --- status ------------------------------------------------------------------

/// Change the run state and tell the frontend the whole of it. Through
/// `SyncRun::transition`, so the snapshot's sequence advances with the change
/// and a probe that read the state just before it is recognisably older.
fn update(app: &AppHandle, change: impl FnOnce(&mut SyncRun)) {
    app.state::<AppState>()
        .sync_run(|run| run.transition(change));
    events::sync_status(app, status(app));
}

/// A connect could not start, or the account could not be adopted: said as
/// status, so the row shows it next to the button that asked.
fn failed(app: &AppHandle, error: &Error) {
    log::warn!("sync connect failed: {error}");
    let failure = SyncFailure::from(error);
    update(app, |run| {
        run.pending = false;
        run.error = Some(failure);
    });
}

/// Mark the session connected and say so. The flag is session-only — what
/// actually makes a vault "configured" is the token file `write_tokens` just
/// wrote, which is what a later unlock reads.
fn connected(app: &AppHandle) {
    let state = app.state::<AppState>();
    let mut session = state.session.lock().unwrap();
    // A vault that locked behind the browser has no session to flag; the tokens
    // are on disk, so the next unlock picks the connection up anyway.
    if session.is_unlocked() {
        session.sync_configured = true;
    }
    drop(session);
    update(app, |run| {
        run.pending = false;
        run.error = None;
    });
}

/// A run is in flight. A retry after a failure reads as "syncing" rather than
/// staying red until it lands.
fn started(app: &AppHandle) {
    update(app, |run| {
        run.in_progress = true;
        run.error = None;
    });
}

/// A run ended — see [`SyncRun::finish`] for what that leaves standing.
fn finished(app: &AppHandle, error: Option<&Error>) {
    if let Some(why) = error {
        log::warn!("sync failed: {why}");
    }
    let failure = error.map(SyncFailure::from);
    update(app, |run| run.finish(failure));
}

/// The whole of sync as the frontend should see it right now.
pub(crate) fn status(app: &AppHandle) -> SyncStatus {
    let state = app.state::<AppState>();
    // The session flag only exists after an unlock; while locked, answer from
    // the persisted (non-secret) settings so e.g. the lock screen can say
    // where the vault lives.
    let session = state.session.lock().unwrap();
    let configured = if session.is_unlocked() {
        session.sync_configured
    } else {
        crate::storage::sync_configured(app)
    };
    drop(session);
    state.sync_run(|run| run.status(configured))
}

/// The active workspace changed: say what sync looks like for the new one.
///
/// A transition rather than a bare emit, and that is the point: the snapshot the
/// frontend is holding describes the workspace that just locked, which may have
/// been through more transitions than the one being switched to has. Going
/// through `update` gives this one the later sequence, so it is taken rather
/// than discarded as stale (see `SYNC_SEQ`). The re-probe that `vault:locked`
/// triggers then answers with the same snapshot.
pub(crate) fn switched(app: &AppHandle) {
    update(app, |_| {});
}

// --- runs ----------------------------------------------------------------------

/// Start one run against this vault's own pack, from the live session, unless
/// one is already in flight — in which case nothing is started.
///
/// **Every** run in the process starts here, whatever asked for it — the
/// debounced auto-sync, `sync_now`, the run behind a fresh connect — because
/// the claim below is the only thing keeping two of them apart, and they must
/// be kept apart: two runs racing resolve the vault id independently, so they
/// can settle on different ids and publish a pack each, leaving one vault
/// spread across two files in the account.
///
/// A run turned away here is not lost: a sync is full-state, but the run
/// underway may already have packed before the write that asked, so the
/// request is kept as a rerun flag and honoured once that run ends
/// ([`spawn_run`]). Two runs at most per burst of writes, and nothing waits
/// for the next unlock.
///
/// The run goes to the blocking pool rather than `async_runtime::spawn`. The
/// Drive calls are driven with `block_on`, which is only legal off the async
/// runtime's own worker threads — a blocking-pool thread is not one of them —
/// and it also guarantees that no amount of network latency can reach the
/// command or main thread.
fn start_run(app: &AppHandle) {
    // The request goes up before the claim is tried, so a run releasing at
    // this very instant cannot miss it: it checks after its release, and this
    // caller raised before its attempt (see `AppState::sync_rerun`).
    request_run(&app.state::<AppState>());
    launch(app);
}

/// Claim the run and start it, or do nothing if one is in flight.
///
/// The request flag is lowered on a won claim — the run starting here packs
/// after every write that has asked so far — and left standing on a lost one,
/// for the holder to honour on its way out. This is also the rerun's entry
/// point, and the reason it is separate from [`start_run`]: a rerun that finds
/// the slot taken by a run that began after the release has nothing to add,
/// since that run also packs after the write, so it must not raise the request
/// again and buy a third run.
fn launch(app: &AppHandle) {
    let state = app.state::<AppState>();
    // Claim and key under the workspace lock, so a switch cannot land between
    // them: the run either starts against the paths it was keyed for, or finds
    // them already moved and does not start at all.
    let paths = state.workspace_lock.lock().unwrap();
    let Some(claim) = claim_and_settle(&state) else {
        return;
    };

    // Cloned before the thread starts, so the run owns its credentials even if
    // the session auto-locks a moment later. Everything else it needs is taken
    // from the session per step, and a locked session simply ends the run.
    // Dropping `claim` on the way out is what releases it: a vault that locked
    // under a scheduled run must not leave sync wedged for the process. The
    // request stays lowered: with no key there is nothing to publish with, and
    // the next unlock of this vault runs a sync of its own.
    let Some(cryptor) = session_cryptor(&state) else {
        return;
    };
    drop(paths);

    spawn_run(app, claim, cryptor);
}

fn spawn_run(app: &AppHandle, claim: RunClaim, cryptor: Cryptor) {
    let app = app.clone();
    super::detached(move || {
        started(&app);
        report(&app, sync::run(&app, cryptor));
        // Release first, then look: a request that found the claim held raised
        // the flag before it looked, so whichever order the two land in, one of
        // them sees the other (see `AppState::sync_rerun`). `launch` rather
        // than `start_run`: the rerun answers a request, it does not make one.
        drop(claim);
        if take_rerun(&app.state::<AppState>()) {
            launch(&app);
        }
    });
}

/// Ask for a run. Recorded before any claim is tried, so the run that holds
/// the claim — if one does — finds the request when it ends.
fn request_run(state: &AppState) {
    state.sync_rerun.store(true, Ordering::SeqCst);
}

/// Claim the run, lowering the request it is about to serve; `None` leaves any
/// request standing for the holder.
fn claim_and_settle(state: &AppState) -> Option<RunClaim> {
    let claim = claim_run(&state.syncing)?;
    state.sync_rerun.store(false, Ordering::SeqCst);
    Some(claim)
}

/// Whether a run was asked for while one was in flight; answering lowers it.
fn take_rerun(state: &AppState) -> bool {
    state.sync_rerun.swap(false, Ordering::SeqCst)
}

fn session_cryptor(state: &State<'_, AppState>) -> Option<Cryptor> {
    state.session.lock().unwrap().cryptor().ok()
}

// Announce the result, and — when the entry list may have changed — hand the
// frontend the refreshed list. Emitting the metas rather than a bare "reload"
// signal keeps the store's update in one round trip and one render.
fn report(app: &AppHandle, result: Result<sync::engine::SyncOutcome>) {
    match result {
        Ok(outcome) => {
            if outcome.merged > 0 {
                events::vault_merged(app, entry_metas(app));
                crate::credential_identities::publish(app);
            }
            finished(app, None);
        }
        Err(e) => finished(app, Some(&e)),
    }
}

// The current entry list, or an empty one if the vault locked in the meantime.
fn entry_metas(app: &AppHandle) -> Vec<EntryMetaDto> {
    let state = app.state::<AppState>();
    let session = state.session.lock().unwrap();
    session
        .store()
        .and_then(list_metas)
        .unwrap_or_else(|_| Vec::new())
}

/// The process's one sync run, held for as long as this guard lives.
///
/// Owns its share of the flag rather than borrowing the state, so the claim can
/// be taken on the thread that decides to run — under `workspace_lock`, before
/// anything else can be told a run is under way — and then moved onto the thread
/// that does the running.
pub(super) struct RunClaim(Arc<AtomicBool>);

/// Claim the run, or `None` if one is already in flight.
fn claim_run(syncing: &Arc<AtomicBool>) -> Option<RunClaim> {
    syncing
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .ok()
        .map(|_| RunClaim(Arc::clone(syncing)))
}

/// The same claim, for a flow that is not a run but must not have one beside it
/// — the "delete everywhere" whose Drive work a run could otherwise look into
/// mid-flight (`commands::workspace::workspace_delete`). Refusing with
/// `SyncBusy` is what `commands::workspace::guard_sync_idle` answers when it
/// finds the flag raised, so the two agree on what a busy sync means.
///
/// Taken rather than merely checked, and held by the caller for its whole
/// length. Any rerun request raised meanwhile is left standing (unlike
/// [`claim_and_settle`]): this holder is not a run and serves nobody's request.
pub(super) fn claim_or_busy(state: &AppState) -> Result<RunClaim> {
    claim_run(&state.syncing).ok_or(Error::SyncBusy)
}

// Releases the claim however the run ends, panics included — a wedged flag
// would disable sync for the rest of the process.
impl Drop for RunClaim {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::PRIMARY_ID;

    // An import and an ordinary sync are not two flows with two flags: they
    // take the same claim, so whichever gets there first is the only one that
    // runs. Two runs at once resolve the vault id independently and can publish
    // a pack each under different ids.
    #[test]
    fn an_import_in_flight_turns_an_ordinary_run_away() {
        let state = AppState::default();

        let import = claim_run(&state.syncing).expect("nothing was running");
        assert!(
            claim_run(&state.syncing).is_none(),
            "a sync cannot start beside the import"
        );

        // However the import ended — returned, errored, or panicked — the next
        // run is free to start.
        drop(import);
        let next = claim_run(&state.syncing).expect("the import released it");
        assert!(claim_run(&state.syncing).is_none(), "and holds it in turn");
        drop(next);
        assert!(claim_run(&state.syncing).is_some());
    }

    // A write that lands while a run is in flight may have missed that run's
    // pack. Its request is not dropped: it is left for the run to honour on its
    // way out, and honoured exactly once.
    #[test]
    fn a_request_turned_away_is_kept_for_the_run_in_flight() {
        let state = AppState::default();

        // What `start_run` does: ask, then try.
        request_run(&state);
        let running = claim_and_settle(&state).expect("nothing was running");
        assert!(!take_rerun(&state), "a run that won the claim owes nothing");
        // Taking the answer must not have lowered a flag nobody raised again.
        assert!(!take_rerun(&state));

        request_run(&state);
        assert!(claim_and_settle(&state).is_none(), "the claim is held");
        // Two writes during one run are one rerun, not two.
        request_run(&state);
        assert!(claim_and_settle(&state).is_none());

        drop(running);
        assert!(
            take_rerun(&state),
            "the run in flight was asked to go again"
        );
        assert!(!take_rerun(&state), "and only once");
    }

    // The rerun itself is an ordinary claim: it clears the request it answers,
    // so a burst of writes ends in a bounded number of runs.
    #[test]
    fn the_rerun_clears_the_request_it_answers() {
        let state = AppState::default();

        request_run(&state);
        let running = claim_and_settle(&state).unwrap();
        request_run(&state);
        assert!(claim_and_settle(&state).is_none());
        drop(running);

        assert!(take_rerun(&state));
        let rerun = claim_and_settle(&state).expect("the claim was released");
        drop(rerun);
        assert!(!take_rerun(&state), "nothing asked during the rerun");
    }

    // Between a run's release and its rerun's claim, another caller may take
    // the slot. That run began after the release, so after the write the rerun
    // was answering: the rerun has nothing to add and must not ask again, or
    // the newcomer would owe a third run for nothing.
    #[test]
    fn a_rerun_that_finds_the_slot_taken_does_not_ask_again() {
        let state = AppState::default();

        request_run(&state);
        let running = claim_and_settle(&state).unwrap();
        request_run(&state);
        assert!(claim_and_settle(&state).is_none());
        drop(running);
        assert!(take_rerun(&state));

        // A concurrent `start_run` wins the released slot first…
        request_run(&state);
        let newcomer = claim_and_settle(&state).expect("the slot was free");
        // …and the rerun, which is a `launch`, only tries the claim.
        assert!(claim_and_settle(&state).is_none());

        drop(newcomer);
        assert!(!take_rerun(&state), "the newcomer owes no third run");
    }

    fn pack(vault_id: &str) -> PackInfo {
        PackInfo {
            id: format!("drive-file-{vault_id}"),
            name: format!("{vault_id}.rowel"),
            vault_id: vault_id.into(),
            size: 0,
            modified_time: String::new(),
        }
    }

    // The account is the source of truth for which vaults exist: this vault
    // joins it only as its first vault or as a device of a vault it already
    // holds — never as a pack beside the others.
    #[test]
    fn an_empty_account_takes_the_vault_as_its_first() {
        assert!(may_adopt(Some("this-vault"), &[]));
        // A vault with no id yet has nothing to match, and still may.
        assert!(may_adopt(None, &[]));
    }

    #[test]
    fn an_account_already_holding_this_vault_takes_it_again() {
        let packs = [pack("another-vault"), pack("this-vault"), pack("a-third")];
        assert!(may_adopt(Some("this-vault"), &packs));
    }

    #[test]
    fn an_account_holding_only_other_vaults_refuses() {
        let packs = [pack("another-vault"), pack("a-third")];
        assert!(!may_adopt(Some("this-vault"), &packs));
        // Nor may an id-less vault be minted beside them.
        assert!(!may_adopt(None, &packs));
    }

    // An unlocked primary holding the vault `vault_id`, or one with no id yet.
    fn open_primary(vault_id: Option<&str>) -> AppState {
        use crate::crypto::VaultKey;
        use crate::store::SqliteStore;

        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "rowel-sync-adopt-{}-{}.db",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let store = SqliteStore::open(&path, &[9u8; 32]).unwrap();
        if let Some(id) = vault_id {
            crate::store::identity::adopt_vault_id(&store, id).unwrap();
        }
        // A key of the right shape; nothing here derives or opens anything with it.
        let key = VaultKey::Argon2 {
            master: zeroize::Zeroizing::new(vec![0u8; 32]),
        };
        let state = AppState::default();
        state.session.lock().unwrap().set(key, store, false);
        state
    }

    // The round trip to Google is the one gap `guard_sync_idle` does not close:
    // the adoption must find, after it, exactly the workspace and vault it read
    // before it.
    #[test]
    fn the_same_workspace_holding_the_same_vault_is_still_it() {
        let state = open_primary(Some("a1b2"));
        assert!(still_the_same_workspace(&state, PRIMARY_ID, Some("a1b2")).unwrap());
    }

    #[test]
    fn another_workspace_is_not() {
        let state = open_primary(Some("a1b2"));
        *state.active_workspace.lock().unwrap() = "b2c3".into();
        assert!(!still_the_same_workspace(&state, PRIMARY_ID, Some("a1b2")).unwrap());
    }

    // The paths did not move, but what is behind them did: a restore over the
    // same workspace, say.
    #[test]
    fn the_same_workspace_holding_another_vault_is_not() {
        let state = open_primary(Some("cafe"));
        assert!(!still_the_same_workspace(&state, PRIMARY_ID, Some("a1b2")).unwrap());
    }

    // A vault that has not synced yet has no id on either side of the probe.
    #[test]
    fn a_vault_with_no_id_yet_is_still_it() {
        let state = open_primary(None);
        assert!(still_the_same_workspace(&state, PRIMARY_ID, None).unwrap());
    }

    // Locked mid-round-trip is not a switch: the caller leaves the tokens for a
    // retry rather than dropping them.
    #[test]
    fn a_locked_session_is_neither_answer() {
        assert!(matches!(
            still_the_same_workspace(&AppState::default(), PRIMARY_ID, None),
            Err(Error::Locked)
        ));
    }
}
