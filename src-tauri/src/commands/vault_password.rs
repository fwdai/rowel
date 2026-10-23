//! One vault password.
//!
//! Rowel is one vault opened by one password: the primary lockbox's key is the
//! app key, and every other lockbox's key is sealed under it (`crate::appkey`),
//! so the password that opens the primary opens them all. A lockbox on a
//! password of its own is not a feature but drift — a restore or an unlock of a
//! pack sealed under another password — and the app no longer lets it in
//! quietly. Every path that opens or installs a lockbox other than the primary
//! with a typed password asks [`require_vault_password`] once that password
//! has opened it: the vault password is sealed under the app key as before;
//! any other is refused with [`Error::VaultPasswordRequired`], the lockbox is
//! left closed, and the user is asked for the vault password to move it over
//! ([`workspace_adopt_vault_password`]).

use std::path::Path;

use subtle::ConstantTimeEq;
use tauri::{AppHandle, State};
use zeroize::Zeroizing;

use super::blocking;
use super::setup::begin_step;
use super::workspace::{derive_in, verify_password_in};
use crate::auth::{self, LockoutState};
use crate::crypto::VaultKey;
use crate::error::{Error, Result};
use crate::state::AppState;
use crate::workspace::{self, Registry, PRIMARY_ID};
use crate::{appkey, storage};

/// Prove `password` is the vault password — the one the primary's key derives
/// from — and hand back the app key it proves. Once the primary has been opened
/// this session its key is in the ring as the app key, and a derive with the
/// primary's own descriptor is compared against it in constant time. Before
/// that, the primary's own database is asked, and the key it proves opens the
/// app at its level from here on (`appkey::open_all`), so the next proof is the
/// cheap one.
///
/// Under the primary's own failed-attempt backoff, exactly as an unlock of it
/// and a delete of it run: this proves the primary's password on demand, and
/// without the backoff it would be a way to guess that password at full
/// Argon2id speed while the lock screen beside it escalates. A wrong password
/// is the backoff's refusal (`InvalidPassword`, or `TooManyAttempts` once it
/// escalates).
pub(crate) async fn verify_master_password(
    app: &AppHandle,
    root: &Path,
    state: &AppState,
    password: Zeroizing<String>,
) -> Result<Zeroizing<Vec<u8>>> {
    match check_master_password(app, root, state, password).await? {
        Proof::Proved(app_key) => Ok(app_key),
        Proof::Refused(refusal) => Err(refusal),
    }
}

/// A lockbox other than the primary — `workspace_id` — has just been opened
/// with `password`: is that the vault password? The app key when it is, for
/// the caller to seal the lockbox under. When it is not, the lockbox's own
/// password is not one the app keeps, and the answer is
/// [`Error::VaultPasswordRequired`] naming it; the caller leaves it closed.
///
/// [`verify_master_password`] underneath, backoff and all: the typed password
/// has already opened the lockbox, but a restore can bring in a pack sealed
/// under any password an attacker likes, so the comparison with the vault
/// password must not be a free oracle. A primary that is locked out refuses
/// before anything is derived, and that refusal is returned as it is — the
/// password may well be the vault password, and the user only has to wait.
pub(crate) async fn require_vault_password(
    app: &AppHandle,
    root: &Path,
    state: &AppState,
    workspace_id: &str,
    password: Zeroizing<String>,
) -> Result<Zeroizing<Vec<u8>>> {
    match check_master_password(app, root, state, password).await? {
        Proof::Proved(app_key) => Ok(app_key),
        Proof::Refused(_) => Err(Error::VaultPasswordRequired {
            workspace_id: workspace_id.to_string(),
        }),
    }
}

/// What a proof of the vault password came to, once it ran.
enum Proof {
    /// The vault password: the app key it derives.
    Proved(Zeroizing<Vec<u8>>),
    /// Not the vault password, recorded against the primary's backoff: the
    /// refusal that backoff owes the caller.
    Refused(Error),
}

async fn check_master_password(
    app: &AppHandle,
    root: &Path,
    state: &AppState,
    password: Zeroizing<String>,
) -> Result<Proof> {
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
            let material = Zeroizing::new(derived.biometric_material().to_vec());
            bool::from(material.ct_eq(&app_key)).then_some(material)
        }
        None => match blocking(move || verify_password_in(&dir, &password)).await {
            Ok(key) => {
                let material = Zeroizing::new(key.biometric_material().to_vec());
                appkey::open_all(app, &material);
                Some(material)
            }
            Err(Error::InvalidPassword) => None,
            Err(e) => return Err(e),
        },
    };
    match proved {
        Some(app_key) => {
            reset_lockout(lockout, root);
            Ok(Proof::Proved(app_key))
        }
        None => Ok(Proof::Refused(penalize(lockout, now, root))),
    }
}

/// Prove `password` opens the lockbox in `dir`, under that lockbox's own
/// failed-attempt backoff, and hand back the key that did it. What a delete
/// asks of its target and a move asks of the lockbox it moves: both prove a
/// password on demand, from a webview or a lock screen, so neither may be a
/// way to guess one at full speed while the unlock beside it escalates.
pub(crate) async fn prove_lockbox_password(
    dir: &Path,
    password: Zeroizing<String>,
) -> Result<VaultKey> {
    let lockout = LockoutState::load_in(dir)?;
    let now = auth::now_ms();
    if let Some(refusal) = auth::locked_out(lockout, now) {
        return Err(refusal);
    }
    // Argon2id and a SQLCipher open, so off the command thread.
    let target = dir.to_path_buf();
    match blocking(move || verify_password_in(&target, &password)).await {
        Ok(key) => {
            // Reset here rather than left to whatever follows: the caller can
            // still fail after the proof, and a proven password must not leave
            // attempts standing against it.
            reset_lockout(lockout, dir);
            Ok(key)
        }
        Err(Error::InvalidPassword) => Err(penalize(lockout, now, dir)),
        Err(e) => Err(e),
    }
}

fn reset_lockout(lockout: LockoutState, dir: &Path) {
    if lockout != LockoutState::default() {
        if let Err(e) = LockoutState::default().save_in(dir) {
            log::warn!("failed to reset lockout sidecar: {e}");
        }
    }
}

fn penalize(lockout: LockoutState, now: i64, dir: &Path) -> Error {
    let (updated, refusal) = auth::penalize(lockout, now);
    if let Err(e) = updated.save_in(dir) {
        log::warn!("failed to persist lockout sidecar: {e}");
    }
    refusal
}

/// Move lockbox `workspace_id` onto the vault password.
///
/// The answer to [`Error::VaultPasswordRequired`]: the lockbox opens with
/// `lockbox_password` and the vault opens with `vault_password`, and from here
/// on the lockbox opens with the vault password too. Both are proved first,
/// each under its own backoff — the vault password against the app key, the
/// lockbox password against the lockbox's own files. Then the lockbox is
/// re-keyed under the vault password with a descriptor of its own (a fresh
/// salt, its generation bumped, so its other devices are asked for the new
/// password exactly as after a password change), through the same saga a
/// change runs — which keeps the epoch it leaves in its retired list — and its
/// account's token file is re-sealed under the new key. Only once that is on
/// disk is its key sealed under the app key.
///
/// The lockbox must not be the one the session has open: it is re-keyed on its
/// own files, as a password change re-keys every lockbox beside the open one.
/// Nothing the user can reach asks that — a lockbox in this state never opens
/// — so it is refused rather than handled.
#[tauri::command]
pub async fn workspace_adopt_vault_password(
    workspace_id: String,
    lockbox_password: Zeroizing<String>,
    vault_password: Zeroizing<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<()> {
    if lockbox_password.is_empty() || vault_password.is_empty() {
        return Err(Error::WorkspacePasswordRequired);
    }
    if workspace_id == PRIMARY_ID {
        return Err(Error::Other(
            "the primary lockbox already opens with the vault password".into(),
        ));
    }
    // It re-keys a lockbox's files, as a password change does, and a switch
    // must not open them halfway.
    let _step = begin_step(&state)?;
    let root = storage::root_dir(&app)?;
    if !Registry::load(&root).contains(&workspace_id) {
        return Err(Error::NotFound);
    }
    {
        let _paths = state.workspace_lock.lock().unwrap();
        let open = *state.active_workspace.lock().unwrap() == workspace_id
            && state.session.lock().unwrap().is_live();
        if open {
            return Err(Error::Other(
                "this lockbox is open; lock it before moving it to the vault password".into(),
            ));
        }
    }

    let app_key = verify_master_password(&app, &root, &state, vault_password.clone()).await?;

    let dir = workspace::dir_of(&root, &workspace_id);
    // As an unlock does, before the descriptor is read: a crash during an
    // earlier change can leave the database and the descriptor disagreeing.
    {
        let dir = dir.clone();
        blocking(move || auth::recover_interrupted_rekey_in(&dir)).await?;
    }
    let old_key = prove_lockbox_password(&dir, lockbox_password).await?;
    let held = Zeroizing::new(old_key.biometric_material().to_vec());

    let material = {
        let (dir, id) = (dir.clone(), workspace_id.clone());
        blocking(move || super::auth::rekey_one(&dir, &id, Some(held), "", &vault_password)).await?
    };
    appkey::seal_under(&app, &workspace_id, &app_key, &material);
    Ok(())
}
