//! Unlocking, locking, the biometric enrollment and the master-password change.
//!
//! Thin entry points: the backoff and the rekey saga live in [`crate::auth`],
//! the vault-opening helpers in [`crate::session`]. What is left here is the
//! part that has to touch the session — and every one of these commands is
//! `async` so that the work behind it can go to the blocking pool, because a
//! non-async command body runs on the IPC (main) thread.

use crate::auth::{self, LockoutState};
use crate::crypto::{KdfParams, VaultKey};
use crate::error::{Error, Result};
use crate::models::{EntryMetaDto, UnlockResult};
use crate::secure_store::{self, GateMode, KeyStore};
use crate::session::{derive_key, open_with_key, unlock_with_password};
use crate::state::AppState;
use crate::store::SqliteStore;
use crate::workspace::PRIMARY_ID;
use crate::{appkey, biometrics, crypto, events, storage};
use std::path::Path;
use tauri::{AppHandle, State};
use zeroize::Zeroizing;

use super::blocking;

// Unlock with the master password: read the KDF sidecar, derive the key, open the
// existing store, and return the entry metadata list. The Argon2id derive + the
// SQLCipher open both run on a blocking thread so the UI is never stalled; unlock
// never migrates anything.
//
// Wrapped with the failed-unlock backoff (T-AUTH-3): a standing lockout is
// enforced *before* deriving anything (so a locked-out caller never pays the
// Argon2id cost), and a wrong password updates the lockout sidecar afterwards.
//
// The password arrives as `Zeroizing<String>`: deserialized straight into a
// buffer that is scrubbed when it drops, so the plaintext does not outlive the
// derive on whatever heap block the allocator gave it.
#[tauri::command]
pub async fn unlock(
    password: Zeroizing<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<UnlockResult> {
    // Held for the whole unlock, like `change_master_password` holds it for the
    // whole change. Without it an unlock could land while a change is still
    // re-keying (an auto-lock during a long change ends the session but not the
    // saga), and the recovery below would read that change's snapshot as an
    // interrupted one and copy it over a database the saga still has open.
    let _step = super::setup::begin_step(&state)?;
    // Before the lockout check and before anything reads the sidecar: a crash
    // during a password change can leave the DB and the sidecar disagreeing, and
    // the rollback has to land before either is consulted.
    auth::recover_interrupted_rekey(&app, &state)?;

    let lockout = LockoutState::load(&app)?;
    let now = auth::now_ms();
    if let Some(refusal) = auth::locked_out(lockout, now) {
        return Err(refusal);
    }

    // Two more copies of the password, for the things that may outlive the
    // unlock, each only once it has succeeded. One tries it against the
    // account's other vaults (`commands::autojoin`) — only for a vault that
    // syncs, since a local vault has no account to look in. The other, for a
    // workspace that is not the primary, tries it against the primary: the
    // device has one master password, so this is usually it, and proving that
    // is what opens the app at its own level (`adopt_primary_with`).
    let join = storage::sync_configured(&app).then(|| password.clone());
    let active = crate::workspace::active_id(&app);
    let master = (active != PRIMARY_ID).then(|| password.clone());

    match unlock_off_thread(&app, password).await {
        Ok((key, store, entries)) => {
            if lockout != LockoutState::default() {
                if let Err(e) = LockoutState::default().save(&app) {
                    log::warn!("failed to reset lockout sidecar: {e}");
                }
            }
            let sync_configured = storage::sync_configured(&app);
            let material = Zeroizing::new(key.biometric_material().to_vec());
            state
                .session
                .lock()
                .unwrap()
                .set(key, store, sync_configured);
            // The idle clock starts with the session, not with the frontend's
            // first activity ping: a webview that never sends one (a build
            // whose bundle failed, a page left untouched) would otherwise leave
            // the vault open for good.
            crate::autolock::touch(&app);
            #[cfg(desktop)]
            crate::browser::server::notify_unlocked();
            // And iOS suggests this vault's logins from here on.
            crate::credential_identities::publish(&app);
            seed_vault_name(&app, &state);
            // The app-level unlock: the primary's key opens every workspace
            // sealed under it; another workspace's key joins the ring, and is
            // sealed under the app key once that is known — now, if the ring
            // already holds it, or once the same password has opened the
            // primary too.
            appkey::adopt(&app, &active, &material);
            if let Some(password) = master {
                if !state.keyring.lock().unwrap().has(PRIMARY_ID) {
                    adopt_primary_with(&app, password);
                }
            }
            if let Some(password) = join {
                super::autojoin::with_password(&app, password);
            }
            Ok(UnlockResult {
                entries,
                sync_configured,
            })
        }
        Err(Error::InvalidPassword) => {
            let (updated, refusal) = auth::penalize(lockout, now);
            if let Err(e) = updated.save(&app) {
                log::warn!("failed to persist lockout sidecar: {e}");
            }
            Err(refusal)
        }
        Err(e) => Err(e),
    }
}

// A vault named before the name lived inside it: copy the registry's label into
// `meta` so the first sync publishes it instead of leaving the user's other
// devices unnamed. A workspace with no label keeps none — the frontend goes on
// showing its translated default.
//
// Stamped `MIGRATED_NAME_MS`, never `now`: the label predates stamps entirely,
// so it says nothing about when the user chose it. Stamping it now would let a
// device that upgrades late in a rollout outrank a rename another device has
// already published — the migrated label must lose to every real rename, and
// win only against a vault nobody has named.
//
// Best effort, and the registry is read before the session lock is taken, since
// that is the order every other reader of the two takes them in.
pub(crate) fn seed_vault_name(app: &AppHandle, state: &AppState) {
    let Some(name) = crate::workspace::active_name(app) else {
        return;
    };
    let session = state.session.lock().unwrap();
    let Ok(store) = session.store() else {
        return;
    };
    if !matches!(crate::store::identity::vault_name(store), Ok((None, _))) {
        return;
    }
    if let Err(e) = crate::store::identity::set_vault_name(
        store,
        &name,
        crate::store::identity::MIGRATED_NAME_MS,
    ) {
        log::warn!("could not seed the vault name from the registry: {e}");
    }
}

// Run the Argon2id derive + SQLCipher open (both CPU-bound) on a blocking thread.
async fn unlock_off_thread(
    app: &AppHandle,
    password: Zeroizing<String>,
) -> Result<(VaultKey, SqliteStore, Vec<EntryMetaDto>)> {
    let app = app.clone();
    blocking(move || unlock_with_password(&app, &password)).await
}

// Open the store for an already-resolved key (the biometric path, and a
// switch to a workspace the ring holds) off the UI thread.
pub(crate) async fn open_off_thread(
    app: &AppHandle,
    key: VaultKey,
) -> Result<(VaultKey, SqliteStore, Vec<EntryMetaDto>)> {
    let app = app.clone();
    blocking(move || {
        let (store, entries) = open_with_key(&app, &key)?;
        Ok((key, store, entries))
    })
    .await
}

// A workspace other than the primary was just opened with `password`. If the
// same password opens the primary — one master password for the device is the
// rule, and this is where a workspace restored under it before the rule existed
// catches up — the primary's key is the app key, and the ring fills from it:
// every workspace sealed under it, and the one just opened sealed under it
// now. A password that does not open the primary is a workspace of its own,
// and its sidecar waits for an unlock that has the app key.
//
// Detached: an Argon2id run and a SQLCipher open, on the primary's own files
// rather than the active paths, and nothing the unlock has to wait for.
fn adopt_primary_with(app: &AppHandle, password: Zeroizing<String>) {
    let app = app.clone();
    super::detached(move || {
        let Ok(root) = storage::root_dir(&app) else {
            return;
        };
        match super::workspace::verify_password_in(&root, &password) {
            Ok(key) => appkey::open_all(&app, key.biometric_material()),
            Err(Error::InvalidPassword) => {}
            Err(e) => log::warn!("could not try the password against the primary: {e}"),
        }
    });
}

// Clear the in-memory key and close the store, and emit `vault:locked` with it
// — the frontend reacts to the event, not to this promise (see `session::lock`).
// Announced even when there was nothing left to seal: the user asked for the
// lock screen, and a webview that somehow still shows the vault gets it.
#[tauri::command]
pub fn lock(app: AppHandle) -> Result<()> {
    if !crate::session::lock(&app) {
        events::vault_locked(&app);
    }
    Ok(())
}

// The user is at the keyboard: restart the inactivity clock. The webview sends
// this throttled off its own input events (`useActivityPing`), once on mount
// and then at most every few seconds while the user is active; the auto-lock
// comes due that long after the last one. Cheap and side-effect free on a
// locked vault, so it needs no guard.
#[tauri::command]
pub fn touch_activity(app: AppHandle) -> Result<()> {
    crate::autolock::touch(&app);
    Ok(())
}

// Unlock from a locked start using the biometric-gated key in the OS secure
// store. Retrieving the key triggers the biometric prompt; the sidecar decides
// how to interpret the stored bytes (Argon2id master vs legacy secret). The
// store then opens off the UI thread. No migration on unlock.
//
// The stored key is the app key — the primary's. It opens the primary
// directly, and any other workspace through the copy of that workspace's key
// sealed under it (`crate::appkey`); a workspace with no such copy is not
// offered this unlock (`commands::app::snapshot`).
#[tauri::command]
pub async fn unlock_biometric(app: AppHandle, state: State<'_, AppState>) -> Result<UnlockResult> {
    // Same two reasons as in `unlock`: no unlock while a change is re-keying,
    // and an interrupted change rolled back before the sidecar decides how to
    // read the stored material.
    let _step = super::setup::begin_step(&state)?;
    auth::recover_interrupted_rekey(&app, &state)?;

    let Some(marker) = storage::biometric_marker(&app) else {
        return Err(Error::Other("biometric unlock is not enabled".into()));
    };
    // Read through the gate enrollment recorded, never a re-probed one: trying
    // the other mode on failure would either downgrade an OS-enforced gate to an
    // app-enforced one behind the user's back, or just miss.
    //
    // On the blocking pool, like the open two lines below: the retrieve puts a
    // system biometric sheet on screen and waits for it on a channel, so it
    // parks its thread for as long as the user takes to answer.
    let mode = GateMode::from_marker(&marker);
    let material = match blocking(move || secure_store::Platform.retrieve(mode)).await {
        Ok(k) => k,
        Err(e) => {
            if unenroll_on(&e) {
                let _ = storage::set_biometric_marker(&app, None);
            }
            return Err(e);
        }
    };
    // A sidecar means the stored bytes are an Argon2id master; without one they
    // are the legacy secret string (a pre-sidecar dev vault). The primary's
    // sidecar, since the stored bytes are the primary's key.
    let root = storage::root_dir(&app)?;
    let app_key = VaultKey::from_material(material, root.join(storage::KDF_SIDECAR_FILE).exists());
    let app_material = Zeroizing::new(app_key.biometric_material().to_vec());
    let active = crate::workspace::active_id(&app);
    let key = if active == PRIMARY_ID {
        app_key
    } else {
        appkey::unwrap_key(&app, &active, &app_material).ok_or_else(|| {
            Error::Other("biometric unlock is not enabled for this workspace".into())
        })?
    };
    let (key, store, entries) = open_off_thread(&app, key).await?;
    let sync_configured = storage::sync_configured(&app);
    state
        .session
        .lock()
        .unwrap()
        .set(key, store, sync_configured);
    // As in `unlock`: the session arms its own idle clock, a connected browser
    // extension hears of it, iOS is handed its logins, and a vault named before
    // names lived inside it takes the registry's label.
    crate::autolock::touch(&app);
    #[cfg(desktop)]
    crate::browser::server::notify_unlocked();
    crate::credential_identities::publish(&app);
    seed_vault_name(&app, &state);
    // The app key opens the app: every workspace sealed under it joins the ring.
    appkey::open_all(&app, &app_material);
    Ok(UnlockResult {
        entries,
        sync_configured,
    })
}

// Whether a failed biometric retrieve means the enrollment itself is gone and
// the marker should be cleared.
//
// Only [`Error::NotFound`] qualifies: the keychain item is provably absent (the
// OS invalidated it because the enrolled fingerprints changed, or the user
// removed it), so keeping the marker would leave a Touch ID button that can
// never work. Every other failure — a build that lost its code-signing
// entitlement, a cancelled prompt, a transient keychain error — leaves the key
// sitting in the keychain intact, so un-enrolling would turn a temporary
// problem into a permanent one and force the user to re-enroll for nothing.
fn unenroll_on(err: &Error) -> bool {
    matches!(err, Error::NotFound)
}

// Opt in: store the app key in the OS secure store, biometry-gated. From any
// workspace, since the one key opens them all — it just has to be known, which
// it is once the primary has been opened this session. Returns the gate that
// enrollment settled on — recorded here and honoured verbatim by every later
// retrieval.
#[tauri::command]
pub async fn enable_biometric(app: AppHandle, state: State<'_, AppState>) -> Result<String> {
    if !secure_store::is_supported() || !biometrics::is_available() {
        return Err(Error::Other("biometrics not available".into()));
    }
    // Copied out from under the guard, because the write below is a blocking
    // call into the OS keychain (and a gated one on macOS) — precisely what the
    // session lock may not be held across. `Zeroizing` so the copy is scrubbed.
    let material = app_key_material(&app, &state)?;
    let mode = blocking(move || secure_store::Platform.store(&material)).await?;
    storage::set_biometric_marker(&app, Some(mode.as_marker()))?;
    Ok(mode.as_marker().to_string())
}

// Opt out: delete the stored key (in every mode) and clear the marker.
#[tauri::command]
pub async fn disable_biometric(app: AppHandle) -> Result<()> {
    blocking(move || secure_store::Platform.delete()).await?;
    storage::set_biometric_marker(&app, None)?;
    Ok(())
}

// The app key's opaque bytes, as a scrubbed copy the blocking pool can own:
// from the ring once the primary has been opened this session, or from the
// session itself when the primary is what is open. A workspace opened on its
// own password, with the primary still closed, has no app key to store — the
// launch probe says so first (`can_enroll`), so this is the backstop.
fn app_key_material(app: &AppHandle, state: &State<'_, AppState>) -> Result<Zeroizing<Vec<u8>>> {
    if let Some(material) = state.keyring.lock().unwrap().app_key() {
        return Ok(material);
    }
    if crate::workspace::is_primary(app) {
        let session = state.session.lock().unwrap();
        return Ok(Zeroizing::new(session.key()?.biometric_material().to_vec()));
    }
    Err(Error::PrimaryWorkspaceOnly)
}

// Re-derive a fresh Argon2id key (new salt), re-seal every payload under the new
// payload key, re-key the encrypted DB and rewrite the sidecar. Requires an
// unlocked session. The saga itself is [`crate::auth::rekey`]; this is the part
// that has to touch the session.
//
// And then the same for every *other* workspace on this device that shares the
// password ([`rekey_beside`]): the device has one master password, so a change
// that stopped at the open workspace would leave the others restorable on
// another device only under the password the user has just replaced. The ids of
// the ones that kept their old password come back, so the frontend can say that
// the change did not reach all of them.
//
// Nothing expensive happens under the session lock. Both Argon2id derives run
// before it is taken, and the saga runs on the blocking pool with the store
// *out* of the session — so for its duration every other command reads the
// session as `Error::Locked` rather than waiting behind a mutex held across a
// whole-vault re-seal. The frontend has no window to observe that in: it awaits
// this command and does not poll while it is in flight.
#[tauri::command]
pub async fn change_master_password(
    current: Zeroizing<String>,
    new: Zeroizing<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<String>> {
    // This rewrites the KDF sidecar and the database through the workspace
    // paths, so it holds the same step a workspace switch has to take first —
    // and that every unlock takes, so no unlock can run recovery against the
    // snapshot this change is about to publish. Held for the workspaces beside
    // the open one too: they are re-keyed through their own paths, and a switch
    // landing between two of them would move the open workspace out from under
    // the session this one is still holding.
    let _step = super::setup::begin_step(&state)?;
    // Resolved before anything is taken out of the session, so a path that
    // cannot be resolved leaves the vault open and unchanged.
    let paths = auth::RekeyPaths::resolve(&app, &state)?;
    let root = storage::root_dir(&app)?;

    // The passwords themselves are needed again once the open workspace is
    // done: every other workspace derives `current` against its own descriptor
    // and `new` under a salt of its own, so neither key derived here is any use
    // to it.
    let (current_beside, new_beside) = (current.clone(), new.clone());

    // Deriving needs only the app handle and the passwords, so it happens off
    // the session entirely — and off the main thread, since Argon2id is the
    // single most expensive thing this app does.
    let (current_key, new_key, params) = derive_both(&app, current, new).await?;

    // Take the key and store out on a lease for the duration. A lock that lands
    // while they are away — the user's, or the auto-lock's, which sees the
    // vault as live throughout — ends the session, and `adopt` below refuses
    // to put the vault back on top of it.
    let lease = {
        let mut session = state.session.lock().unwrap();
        // Verify the current password reproduces the unlocked session key.
        if current_key.sqlcipher_key() != session.key()?.sqlcipher_key() {
            return Err(Error::InvalidPassword);
        }
        session.take_out()?
    };
    let (old_key, store, sync_configured, claim) = lease.split();
    // The Drive token is sealed under the *old* key and has to be re-sealed
    // after the change; the key itself is about to be moved into the saga.
    let old_cryptor = old_key.cryptor();

    let rekeyed =
        blocking(move || Ok(auth::rekey(store, old_key, new_key, &params, &paths))).await?;

    let (new_key, store) = match rekeyed {
        Ok(changed) => changed,
        Err(rollback) => {
            match rollback.restored {
                // Back as it was — unless a lock landed meanwhile, in which
                // case the vault stays locked and the reopened store is dropped.
                Some((key, store)) => {
                    state
                        .session
                        .lock()
                        .unwrap()
                        .adopt(claim, key, store, sync_configured);
                }
                // Even the rollback could not reopen the vault: there is no
                // session to hand back, so it ends, and the webview is told
                // rather than left looking at a vault that is no longer open.
                None => {
                    crate::session::lock(&app);
                }
            }
            return Err(rollback.error);
        }
    };

    // Both taken before the key is handed to the session, so what follows needs
    // no second lock to read them back.
    let active = crate::workspace::active_id(&app);
    let primary = active == PRIMARY_ID;
    let new_material = Zeroizing::new(new_key.biometric_material().to_vec());
    let new_cryptor = new_key.cryptor();

    // Adopt the new key + store as the continuation of the session the lease
    // came from. Refused when a lock landed while the saga ran: the change is
    // on disk either way and the next unlock takes the new password, but the
    // vault stays locked rather than reopening behind the user's back.
    let adopted = state
        .session
        .lock()
        .unwrap()
        .adopt(claim, new_key, store, sync_configured);
    if !adopted {
        log::info!(
            "vault locked during the password change; the new key waits for the next unlock"
        );
    }

    // The ring held the old key: it takes the new one, and — for the primary,
    // whose key is what every other workspace is sealed under — every sidecar
    // is resealed under it. The primary's is the one replacement of an app key
    // made on purpose; every other route into the ring refuses to overwrite
    // one, so a slower proof of the old password cannot undo this. A lock that
    // won while the saga ran has cleared the ring, and neither writes to it.
    if primary {
        appkey::replace_app_key(&app, &new_material);
    } else {
        appkey::adopt(&app, &active, &new_material);
    }

    // Re-encrypt the Drive token file under the new key if present (sync parity).
    // After the adopt and best-effort on purpose: the password change is already
    // committed on disk, so failing out here would strand the session held-out
    // with no `vault:locked` event and leave the biometric enrollment stale. The
    // worst case is a token the user has to reconnect.
    if let Err(e) = crate::sync::reseal_tokens(&app, &old_cryptor, &new_cryptor) {
        log::warn!("could not re-seal the Drive token under the new key: {e}");
    }

    // The enrollment holds the app key, so only a change of the primary's key
    // makes it stale; a change elsewhere leaves the stored bytes exactly as good
    // as they were. Done here rather than after the workspaces below, which can
    // take an Argon2id run apiece: the item is stale from the moment the
    // primary's database is re-keyed, and the shorter that window the better.
    if primary {
        restore_enrollment(&app, new_material).await;
    }

    Ok(rekey_beside(&app, &state, &root, &active, current_beside, new_beside).await)
}

// The biometric-stored key is the app key, and a change of the primary's key
// has just retired it: re-store the new material, or clear the enrollment if it
// cannot be written. Re-storing is a fresh enrollment, so the gate is decided
// again and the marker refreshed — a build that has since gained (or lost) its
// entitlement moves the key to the matching mode instead of leaving a
// mislabelled item. A device that never enrolled has nothing to re-store.
async fn restore_enrollment(app: &AppHandle, material: Zeroizing<Vec<u8>>) {
    if !storage::biometric_enrolled(app) {
        return;
    }
    match blocking(move || secure_store::Platform.store(&material)).await {
        Ok(mode) => {
            let _ = storage::set_biometric_marker(app, Some(mode.as_marker()));
        }
        Err(_) => {
            let _ = blocking(|| secure_store::Platform.delete()).await;
            let _ = storage::set_biometric_marker(app, None);
        }
    }
}

// Every workspace on this device but the open one, put on the new password.
//
// One master password per device is the rule the rest of the app already keeps
// — a create proves the typed password against the app key, and an unlock
// elsewhere tries it against the primary — so a change of that password has to
// reach every workspace that shares it. A workspace that does *not* share it is
// left exactly as it was: the password the user typed is not its password, and
// this command is not a way to take one over.
//
// One workspace at a time, each on the blocking pool: an Argon2id derive and a
// whole-vault re-seal apiece, and nothing here goes faster in parallel on a
// machine whose cores Argon2id is already meant to saturate.
//
// Each workspace's saga is atomic on its own directory, so one that fails stays
// on the old password while the ones already done keep the new one. The ids of
// both kinds come back: a workspace on a password of its own is left alone on
// purpose and one that failed is left alone by accident, but the user needs the
// same thing from both — to know the new password does not open everything on
// this device.
//
// The primary goes last. Its key is the app key every other workspace's sealed
// copy is wrapped under, so replacing it once the rest are in the ring under
// their new keys reseals every sidecar in a single pass
// (`appkey::replace_app_key`) rather than once per workspace.
async fn rekey_beside(
    app: &AppHandle,
    state: &AppState,
    root: &Path,
    active: &str,
    current: Zeroizing<String>,
    new: Zeroizing<String>,
) -> Vec<String> {
    let mut ids: Vec<String> = crate::workspace::Registry::load(root)
        .workspaces
        .into_iter()
        .map(|workspace| workspace.id)
        .filter(|id| id != active)
        .collect();
    ids.sort_by_key(|id| id == PRIMARY_ID);

    let mut unchanged = Vec::new();
    for id in ids {
        let dir = crate::workspace::dir_of(root, &id);
        // The ring's copy of this workspace's key, read under its own guard and
        // released before any of the work below: a whole vault's worth of it is
        // no thing to hold a mutex across. A lock that lands afterwards makes
        // the copy stale, and the writes back into the ring stand down on their
        // own (`appkey`); the change on disk is the user's either way.
        let held = state.keyring.lock().unwrap().get(&id);
        let (current, new) = (current.clone(), new.clone());
        match blocking(move || rekey_one(&dir, held, &current, &new)).await {
            Ok(material) => {
                if id == PRIMARY_ID {
                    appkey::replace_app_key(app, &material);
                    restore_enrollment(app, material).await;
                } else {
                    appkey::adopt(app, &id, &material);
                }
            }
            Err(Error::InvalidPassword) => {
                log::info!("workspace {id} has a password of its own and keeps it");
                unchanged.push(id);
            }
            Err(e) => {
                log::warn!("workspace {id} kept its old password: {e}");
                unchanged.push(id);
            }
        }
    }
    unchanged
}

// One workspace's whole password change, on its own files: roll back a change a
// crash left half-applied, find the key that opens it, re-key it under a salt of
// its own, and re-seal its Drive token. The material it now opens under comes
// back, for the ring.
//
// `held` is the ring's copy of its key, for a workspace the app already has
// open: it needs no derive at all, which is the point of the app being unlocked
// as a whole. Without one the typed password is proved against that workspace's
// own descriptor, and a password that does not open it is `InvalidPassword` —
// a workspace of its own rather than a failure.
//
// That proof stands outside the failed-attempt backoff, unlike every other one
// the app makes: the password was typed by someone who had already proved it
// against the open vault, so it is not a guess — and recording it as one would
// escalate a workspace's own lockout a little further with every password
// change the user makes.
//
// Blocking from end to end (a derive, a whole-vault re-seal, two file copies),
// so it only ever runs on the blocking pool.
fn rekey_one(
    dir: &Path,
    held: Option<Zeroizing<Vec<u8>>>,
    current: &str,
    new: &str,
) -> Result<Zeroizing<Vec<u8>>> {
    // Before the descriptor is read or the database opened, exactly as an
    // unlock does it: a crash during an earlier change can leave the two
    // disagreeing, and the rollback has to land before either is consulted.
    auth::recover_interrupted_rekey_in(dir)?;

    let old_key = match held {
        Some(material) => {
            VaultKey::from_material(material, dir.join(storage::KDF_SIDECAR_FILE).exists())
        }
        None => super::workspace::verify_password_in(dir, current)?,
    };
    // Its own params, so every workspace keeps its own salt: the one thing a
    // shared password must not turn into a shared derivation.
    let params = KdfParams::default_argon2id();
    let new_key = VaultKey::Argon2 {
        master: crypto::derive(new.as_bytes(), &params)?,
    };
    let material = Zeroizing::new(new_key.biometric_material().to_vec());
    // The Drive token file is sealed under the old key and has to come out
    // sealed under the new one. Read *before* the change commits: a file that
    // is there and will not open under the key that opens the vault would
    // otherwise be found out only once the vault was on the new password, with
    // the connection lost and the change reported complete. Refused here,
    // nothing has been touched — this workspace stays on the password it had
    // and is reported as unchanged, which is true.
    let tokens = crate::sync::read_tokens_in(dir, &old_key.cryptor())?;
    let new_cryptor = new_key.cryptor();

    auth::rekey_workspace(dir, old_key, new_key, &params)?;

    // The write is best effort, after the commit, for the same reason it is on
    // the open workspace: the change is on disk and reporting a failure here
    // would undo nothing, and reporting the workspace as unchanged would be
    // false. What is left to fail is one small write into a directory the saga
    // has just rewritten a whole database in.
    if let Some(tokens) = tokens {
        if let Err(e) = crate::sync::persist_tokens_in(dir, &new_cryptor, &tokens) {
            log::warn!(
                "could not re-seal this workspace's Drive token under the new password: {e}"
            );
        }
    }
    Ok(material)
}

// Both Argon2id derives, on the blocking pool and before the session lock is
// taken. The current password's key is checked against the session afterwards;
// the new one gets fresh params (a new salt), and nothing on disk is touched
// here, so a failure costs nothing but the error.
async fn derive_both(
    app: &AppHandle,
    current: Zeroizing<String>,
    new: Zeroizing<String>,
) -> Result<(VaultKey, VaultKey, KdfParams)> {
    let app = app.clone();
    blocking(move || {
        let current_key = derive_key(&app, &current)?;
        let params = KdfParams::default_argon2id();
        let new_key = VaultKey::Argon2 {
            master: crypto::derive(new.as_bytes(), &params)?,
        };
        Ok((current_key, new_key, params))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_keychain_item_un_enrolls() {
        assert!(unenroll_on(&Error::NotFound));
    }

    #[test]
    fn an_unreadable_but_present_key_keeps_the_enrollment() {
        // The entitlement error the protected gate raises in an unsigned build,
        // plus the everyday failures. None of these mean the key is gone.
        for err in [
            Error::Other("this build is not entitled to read the protected keychain item".into()),
            Error::Other("biometric authentication failed".into()),
            Error::Cancelled,
            Error::Locked,
        ] {
            assert!(!unenroll_on(&err), "{err} must not clear the marker");
        }
    }

    // --- the change a workspace beside the open one gets ------------------------

    use crate::sync::Tokens;

    fn workspace_under(old: &VaultKey) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join(storage::DB_FILE);
        drop(SqliteStore::open(&db, &*old.sqlcipher_key()).unwrap());
        dir
    }

    fn opens_under(dir: &Path, key: &VaultKey) -> bool {
        SqliteStore::open(&dir.join(storage::DB_FILE), &*key.sqlcipher_key()).is_ok()
    }

    #[test]
    fn a_workspaces_drive_token_comes_out_under_its_new_password() {
        let old = VaultKey::legacy_from_password("old-pw");
        let dir = workspace_under(&old);
        let tokens = Tokens {
            access_token: None,
            refresh_token: Some("refresh".into()),
            expires_at: None,
        };
        crate::sync::persist_tokens_in(dir.path(), &old.cryptor(), &tokens).unwrap();

        let held = Zeroizing::new(old.biometric_material().to_vec());
        let material = rekey_one(dir.path(), Some(held), "old-pw", "new-pw").unwrap();

        let new = VaultKey::from_material(material, true);
        assert!(opens_under(dir.path(), &new));
        let resealed = crate::sync::read_tokens_in(dir.path(), &new.cryptor())
            .unwrap()
            .expect("the token file is still there");
        assert_eq!(resealed.refresh_token.as_deref(), Some("refresh"));
    }

    // A token file that will not open under the key that opens the vault is
    // found before anything is changed, so the workspace keeps its password —
    // and is reported as unchanged — rather than coming out on the new one
    // with a Drive connection nothing can read.
    #[test]
    fn a_workspace_whose_drive_token_will_not_read_keeps_its_password() {
        let old = VaultKey::legacy_from_password("old-pw");
        let dir = workspace_under(&old);
        storage::write_gdrive_in(dir.path(), "not a sealed token file").unwrap();

        let held = Zeroizing::new(old.biometric_material().to_vec());
        assert!(rekey_one(dir.path(), Some(held), "old-pw", "new-pw").is_err());

        assert!(opens_under(dir.path(), &old), "nothing was changed");
        assert!(!dir.path().join(storage::KDF_SIDECAR_FILE).exists());
        assert_eq!(
            std::fs::read_to_string(dir.path().join(storage::GDRIVE_FILE)).unwrap(),
            "not a sealed token file"
        );
    }
}
