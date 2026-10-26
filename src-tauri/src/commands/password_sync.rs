//! The master password of a synced lockbox was changed on another device.
//!
//! The account's pack then carries a key descriptor of a later generation than
//! this device's, and every run stops with `PasswordChangedRemotely` (see
//! `sync::engine::decode_pack`). Two ways out, both explicit:
//!
//! - [`sync_adopt_password`]: the user types the new password; it is proved by
//!   opening the account's pack with it, and this device re-keys onto the
//!   account's descriptor, verbatim, through the same crash-safe saga a local
//!   change runs. Lockboxes beside it on the same password follow.
//! - [`sync_replace_remote`]: the account's copy is not one the user can (or
//!   wants to) open — a password they never set, or an epoch this device has
//!   retired and lost the key of — and this device's pack goes up over it.
//!
//! See `docs/password-change-sync-design.md`.

use std::fs;
use std::path::Path;

use tauri::{AppHandle, Manager, State};
use zeroize::Zeroizing;

use super::blocking;
use crate::auth::{self, LockoutState};
use crate::crypto::{self, Cryptor, KdfParams, VaultKey};
use crate::error::{Error, Result};
use crate::state::AppState;
use crate::sync::{self, engine, pack, MarkerKey};
use crate::workspace::PRIMARY_ID;
use crate::{appkey, retired, storage};

/// Adopt the master password another device changed this lockbox to.
///
/// Proves `password` against the account's pack — derived under the pack's own
/// descriptor (bounds-checked: a hostile header cannot ask for more than the
/// ceiling) and used to open its snapshot — then re-keys the open workspace
/// onto that key with the header as its new descriptor, byte for byte, so the
/// next run's pull matches it. The epoch left behind goes into the retired
/// list, which lets this device still merge what a slower device pushes under
/// it. Every other sync-configured lockbox whose key the ring holds is brought
/// along the same way; the ids of those the password did not open come back.
///
/// A wrong password is penalized exactly like a wrong unlock: the pack bytes
/// are within an attacker's reach, so this must not be a free oracle. A pack
/// already on this device's descriptor means the adopt has happened (or was
/// never needed): a run is started and nothing else changes.
#[tauri::command]
pub async fn sync_adopt_password(
    password: Zeroizing<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<String>> {
    // It re-keys through the workspace paths, like a password change.
    let _step = super::setup::begin_step(&state)?;

    let lockout = LockoutState::load(&app)?;
    let now = auth::now_ms();
    if let Some(refusal) = auth::locked_out(lockout, now) {
        return Err(refusal);
    }
    let open = OpenVault::capture(&app, &state)?;

    // The run claim for the whole adopt: no run reads the account or the
    // session while the store is out for the saga. A run in flight is waited
    // for, briefly.
    let claim = {
        let app = app.clone();
        blocking(move || Ok(super::sync::wait_for_claim(&app.state::<AppState>()))).await?
    }
    .ok_or(Error::SyncBusy)?;

    let OpenVault {
        material,
        cryptor,
        marker_keys,
        vault_id,
        dir,
        workspace_id,
    } = open;
    let fetched = {
        let app = app.clone();
        blocking(move || {
            let store = sync::store_for(sync::Auth::Sealed { app, cryptor });
            sync::fetch_pack(store, vault_id, marker_keys).map(|(file, _)| file)
        })
        .await?
    };
    let file = fetched.ok_or(Error::NoRemoteVault)?;

    // The lockboxes beside need the password once this one is done.
    let password_beside = password.clone();
    let current_key = VaultKey::from_material(material, true);
    let (adopt, current_key) = blocking(move || {
        let adopt = key_for_pack(&dir, &workspace_id, &current_key, &file.bytes, &password);
        Ok((adopt, current_key))
    })
    .await?;
    let (header, new_key) = match adopt {
        Ok(Some(adopt)) => adopt,
        // Nothing to adopt: the run sorts out whatever is left.
        Ok(None) => {
            drop(claim);
            super::sync::request_run_if_ready(&app, &state);
            return Ok(Vec::new());
        }
        Err(Error::InvalidPassword) => {
            let (updated, refusal) = auth::penalize(lockout, now);
            if let Err(e) = updated.save(&app) {
                log::warn!("failed to persist lockout sidecar: {e}");
            }
            return Err(refusal);
        }
        Err(e) => return Err(e),
    };
    if lockout != LockoutState::default() {
        if let Err(e) = LockoutState::default().save(&app) {
            log::warn!("failed to reset lockout sidecar: {e}");
        }
    }

    let active = super::auth::rekey_open_workspace(&app, &state, current_key, new_key, header)
        .await?
        .workspace_id;
    let unchanged = adopt_beside(&app, &state, &active, password_beside).await;

    // The pull now matches this device's descriptor: it merges, and pushes
    // whatever this device had that the account has not.
    drop(claim);
    super::sync::request_run_if_ready(&app, &state);
    Ok(unchanged)
}

/// Push this device's pack over the account's, after the user confirmed that
/// what only the account's copy holds is lost.
///
/// Offered only while the last run of the open lockbox failed with one of the
/// two errors that mean this device cannot open the account's pack:
/// `VaultUnderRetiredPassword` (an epoch this device retired and lost the key
/// of), and `PasswordChangedRemotely` — where a user who never changed the
/// password anywhere is looking at a copy that is not theirs, and needs the
/// same confirmed way out. The upload is conditional on the head revision it
/// reads, so a push another device lands meanwhile wins and this fails. A run
/// follows; the other device's next run then asks it for this password.
#[tauri::command]
pub async fn sync_replace_remote(app: AppHandle, state: State<'_, AppState>) -> Result<()> {
    // Claim and keys under the workspace lock, as `launch` takes them, so a
    // switch cannot move the token file from under the push. The eligibility
    // check sits inside the same section, after the claim: read before it, a
    // run landing in between could clear the failure it answers to, or a
    // switch could make it describe a lockbox other than the one whose keys
    // are captured below — and the push would then replace the wrong copy, or
    // one this device could have pulled first.
    let (claim, cryptor, marker_key) = {
        let _paths = state.workspace_lock.lock().unwrap();
        let claim = super::sync::claim_or_busy(&state)?;
        let failed_with = state.sync_run(|run| run.error.as_ref().map(|e| e.kind));
        if !may_replace(failed_with) {
            return Err(Error::Other(
                "the account's copy can only be replaced when this device cannot open it".into(),
            ));
        }
        let session = state.session.lock().unwrap();
        if !session.sync_configured {
            return Err(Error::SyncNotConfigured);
        }
        let key = session.key()?;
        (claim, key.cryptor(), key.delete_marker_key())
    };
    {
        let app = app.clone();
        blocking(move || sync::replace_remote(&app, cryptor, marker_key)).await?;
    }
    drop(claim);
    super::sync::request_run_if_ready(&app, &state);
    Ok(())
}

/// Whether a lockbox whose last run failed with the kind `failed_with` may
/// have its account copy replaced from here.
fn may_replace(failed_with: Option<&str>) -> bool {
    failed_with.is_some_and(|kind| {
        kind == Error::PasswordChangedRemotely.kind()
            || kind == Error::VaultUnderRetiredPassword.kind()
    })
}

/// What the adopt needs from the open session, read in one go.
struct OpenVault {
    material: Zeroizing<Vec<u8>>,
    cryptor: Cryptor,
    marker_keys: Vec<MarkerKey>,
    vault_id: String,
    dir: std::path::PathBuf,
    workspace_id: String,
}

impl OpenVault {
    fn capture(app: &AppHandle, state: &AppState) -> Result<Self> {
        let (dir, workspace_id) = {
            let _paths = state.workspace_lock.lock().unwrap();
            let id = crate::workspace::active_id(app);
            (crate::workspace::dir_of(&storage::root_dir(app)?, &id), id)
        };
        let (material, cryptor, marker_key, vault_id) = {
            let session = state.session.lock().unwrap();
            let key = session.key()?;
            if !session.sync_configured {
                return Err(Error::SyncNotConfigured);
            }
            let vault_id = crate::store::identity::vault_id(session.store()?)
                .map_err(crate::session::store_err)?;
            (
                Zeroizing::new(key.biometric_material().to_vec()),
                key.cryptor(),
                key.delete_marker_key(),
                vault_id,
            )
        };
        // A vault that never synced has no pack to adopt a password from.
        let vault_id = vault_id.ok_or(Error::NoRemoteVault)?;
        let key = VaultKey::from_material(material.clone(), true);
        let mut marker_keys = vec![marker_key];
        marker_keys.extend(
            retired::load(&dir, &key, &workspace_id)
                .iter()
                .map(|r| r.key().delete_marker_key()),
        );
        Ok(Self {
            material,
            cryptor,
            marker_keys,
            vault_id,
            dir,
            workspace_id,
        })
    }
}

/// The key `password` derives under the descriptor in `pack_bytes`' header,
/// proved by opening the pack's snapshot with it, together with that header —
/// or `None` when the workspace at `dir` (on `current_key`) has nothing to
/// adopt: the header is its own descriptor, or one it has retired (the run
/// merges and re-seals that one by itself), or it has no descriptor at all.
///
/// `InvalidPassword` when the derived key does not open the snapshot: a typo,
/// or a copy in the account that this user never re-keyed. Blocking: an
/// Argon2id derive and a SQLCipher open.
pub(crate) fn key_for_pack(
    dir: &Path,
    workspace_id: &str,
    current_key: &VaultKey,
    pack_bytes: &[u8],
    password: &str,
) -> Result<Option<(String, VaultKey)>> {
    let Ok(sidecar) = fs::read_to_string(dir.join(storage::KDF_SIDECAR_FILE)) else {
        return Ok(None);
    };
    let unpacked = pack::unpack(pack_bytes).map_err(engine::remote_pack_error)?;
    let header = unpacked.kdf_params_json;
    if header == sidecar
        || retired::load(dir, current_key, workspace_id)
            .iter()
            .any(|r| r.kdf == header)
    {
        return Ok(None);
    }
    // Bounds-checked on the way in: a header is attacker-writable.
    let params = KdfParams::from_json(&header)?;
    let key = VaultKey::Argon2 {
        master: crypto::derive(password.as_bytes(), &params)?,
    };
    match engine::records_from_snapshot(
        &unpacked.snapshot,
        &*key.sqlcipher_key(),
        &storage::sync_scratch_dir_in(dir),
    ) {
        Ok(_) => Ok(Some((header, key))),
        Err(Error::VaultTooNew) => Err(Error::VaultTooNew),
        Err(_) => Err(Error::InvalidPassword),
    }
}

/// Every other lockbox on this device that syncs and whose key the ring holds,
/// moved onto `password` if its own pack in the account is under it. Best
/// effort and one at a time, the primary last (its key is the app key, and
/// replacing it once reseals every other sidecar). A lockbox the ring does
/// not hold is on a password of its own as far as this device knows; it is
/// left to prompt when it is next opened. One whose pack the password does
/// not open, or that failed, is left alone and its id returned. Each one moved
/// is synced straight after, under the adopt's run claim
/// (`sync::run_workspace`), best effort.
async fn adopt_beside(
    app: &AppHandle,
    state: &AppState,
    active: &str,
    password: Zeroizing<String>,
) -> Vec<String> {
    let Ok(root) = storage::root_dir(app) else {
        return Vec::new();
    };
    let mut beside: Vec<(String, String)> = crate::workspace::Registry::load(&root)
        .workspaces
        .into_iter()
        .filter(|w| w.id != active)
        // An org lockbox's key is its server's, not a password's: nothing in
        // its account is ever under the password being adopted.
        .filter(|w| {
            !crate::org::lockboxes::is_org_lockbox_dir(&crate::workspace::dir_of(&root, &w.id))
        })
        .filter_map(|w| Some((w.id, w.vault_id?)))
        .collect();
    beside.sort_by_key(|(id, _)| id == PRIMARY_ID);

    let mut unchanged = Vec::new();
    for (id, vault_id) in beside {
        // The app key with it, for the sealed copy to be written under when a
        // lock clears the ring during the re-key (`appkey::rekeyed`).
        let (held, app_key) = {
            let ring = state.keyring.lock().unwrap();
            (ring.get(&id), ring.app_key())
        };
        let Some(held) = held else {
            continue;
        };
        let dir = crate::workspace::dir_of(&root, &id);
        let (app_handle, adopting, workspace_id, password) =
            (app.clone(), dir.clone(), id.clone(), password.clone());
        let adopted = blocking(move || {
            adopt_one(
                &app_handle,
                &adopting,
                &workspace_id,
                vault_id,
                held,
                &password,
            )
        })
        .await;
        match adopted {
            Ok(None) => {}
            Ok(Some(material)) => {
                if id == PRIMARY_ID {
                    appkey::replace_app_key(app, &material);
                    super::auth::restore_enrollment(app, material.clone()).await;
                } else if let Err(e) = appkey::rekeyed(app, &id, app_key, &material) {
                    // On the new password regardless: its own unlock opens it.
                    log::warn!("lockbox {id}'s new key could not be kept for the next unlock: {e}");
                }
                // Its edits made under the old password go up now, under the
                // new one, rather than at its next open.
                super::sync::sync_rekeyed(app, &dir, &id, &material).await;
            }
            Err(Error::InvalidPassword) => {
                log::info!("lockbox {id} is on a password of its own in the account");
                unchanged.push(id);
            }
            Err(e) => {
                log::warn!("lockbox {id} could not take the new password: {e}");
                unchanged.push(id);
            }
        }
    }
    unchanged
}

/// One lockbox beside the open one: fetch its pack with its own account, and
/// re-key it onto the pack's descriptor if `password` opens it — the saga, then
/// its token file re-sealed under the new key. The material it now opens under
/// comes back, for the ring; `None` when it does not sync or had nothing to
/// adopt. Blocking.
fn adopt_one(
    app: &AppHandle,
    dir: &Path,
    workspace_id: &str,
    vault_id: String,
    held: Zeroizing<Vec<u8>>,
    password: &str,
) -> Result<Option<Zeroizing<Vec<u8>>>> {
    // As an unlock would, before the descriptor is read.
    auth::recover_interrupted_rekey_in(dir)?;
    if !dir.join(storage::KDF_SIDECAR_FILE).exists() {
        return Ok(None);
    }
    let old_key = VaultKey::from_material(held, true);
    let old_cryptor = old_key.cryptor();
    let Some(tokens) = sync::read_tokens_in(dir, &old_cryptor)? else {
        return Ok(None);
    };
    let mut marker_keys = vec![old_key.delete_marker_key()];
    marker_keys.extend(
        retired::load(dir, &old_key, workspace_id)
            .iter()
            .map(|r| r.key().delete_marker_key()),
    );
    let (fetched, store) = sync::fetch_pack(
        sync::pending_store(app, tokens.clone()),
        vault_id,
        marker_keys,
    )?;
    // A refresh during the fetch is kept, under whichever key the file ends
    // up sealed under.
    let tokens = store.into_tokens().unwrap_or(tokens);
    let Some(file) = fetched else {
        return Ok(None);
    };
    let Some((header, new_key)) = key_for_pack(dir, workspace_id, &old_key, &file.bytes, password)?
    else {
        return Ok(None);
    };
    let material = Zeroizing::new(new_key.biometric_material().to_vec());
    let new_cryptor = new_key.cryptor();
    auth::rekey_workspace(dir, workspace_id, old_key, new_key, &header)?;
    if let Err(e) = sync::persist_tokens_in(dir, &new_cryptor, &tokens) {
        log::warn!("could not re-seal this lockbox's account under the new password: {e}");
    }
    Ok(Some(material))
}

// The "two devices" acceptance test of the design, one layer below the
// commands (they need an `AppHandle`).
#[cfg(test)]
pub(crate) mod two_devices;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_account_copy_is_replaceable_only_from_the_two_epoch_errors() {
        assert!(may_replace(Some("passwordChangedRemotely")));
        assert!(may_replace(Some("vaultUnderRetiredPassword")));
        assert!(!may_replace(Some("vaultDeletedRemotely")));
        assert!(!may_replace(Some("other")));
        assert!(!may_replace(None));
    }
}
