//! Commands for one-time sharing. The logic is in [`crate::share`]; this is the
//! boundary that turns a session into the credentials a Drive call needs.
//!
//! Two rules shape every command here. The session lock is taken, read and
//! released before anything touches the network — a mutex held across an await
//! would stall every other command for a round trip. And the Drive work goes to
//! the blocking pool through [`super::blocking`], which is where the reasoning
//! for that lives.

use tauri::{AppHandle, State};

use super::blocking;

use crate::crypto::Cryptor;
use crate::error::{Error, Result};
use crate::models::Entry;
use crate::session::{store_err, Session};
use crate::share::{self, remote::AnyShareRemote, ActiveShare, Created};
use crate::state::AppState;
use crate::store::{identity, VaultStore};

/// Seal one of this vault's entries and publish it; returns the link.
#[tauri::command]
pub async fn share_create(
    entry_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Created> {
    let (entry, vault_id, cryptor) = {
        let session = state.session.lock().unwrap();
        let vault_id = sendable(&session)?;
        let record = session
            .store()?
            .get(&entry_id)
            .map_err(store_err)?
            .ok_or(Error::NotFound)?;
        (
            session
                .payload_cipher()?
                .unseal(&record.id, &record.payload)?,
            vault_id,
            session.cryptor()?,
        )
    };

    blocking(move || {
        share::create(
            &remote_for(&app, cryptor)?,
            &entry,
            &vault_id,
            share::now_ms(),
        )
    })
    .await
}

/// Open a link someone sent. Needs no account and no unlocked vault — the
/// recipient may not even have a vault yet.
#[tauri::command]
pub async fn share_open(link: String) -> Result<Entry> {
    blocking(move || share::open(&share::public_fetch(), &link, share::now_ms())).await
}

/// Take one of this vault's shares back. The id arrives from the webview, so it
/// says which file to look at and nothing about who may delete it; the vault id
/// goes along for [`share::revoke`] to check it against.
#[tauri::command]
pub async fn share_revoke(
    file_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<()> {
    let (cryptor, vault_id) = sender_credentials(&state)?;
    blocking(move || share::revoke(&remote_for(&app, cryptor)?, &file_id, &vault_id)).await
}

/// This vault's outstanding shares, expired ones swept first. The account may
/// hold other vaults' shares too; the vault id is what keeps them apart.
#[tauri::command]
pub async fn share_list(app: AppHandle, state: State<'_, AppState>) -> Result<Vec<ActiveShare>> {
    let (cryptor, vault_id) = sender_credentials(&state)?;
    blocking(move || share::list(&remote_for(&app, cryptor)?, &vault_id, share::now_ms())).await
}

/// What the sending side requires, answered with the thing every share is keyed
/// to: the id of the vault asking.
///
/// Checked in this order because a locked session also reports
/// `sync_configured == false`, and telling someone to connect Drive when they
/// merely need to unlock is a dead end.
///
/// The id comes last because it is the subtlest of the three: a vault keeps it
/// only after a sync run has succeeded, so one that has just connected Drive, or
/// whose first run failed, is connected and still nameless. Sharing cannot
/// proceed there — the id is what stamps a share as this vault's and what every
/// later list and revoke is checked against, so a link published without one
/// would belong to nobody, not even to the vault that handed it out.
fn sendable(session: &Session) -> Result<String> {
    if !session.is_unlocked() {
        return Err(Error::Locked);
    }
    if !session.sync_configured {
        return Err(Error::SyncNotConfigured);
    }
    identity::vault_id(session.store()?)
        .map_err(store_err)?
        .ok_or(Error::ShareNeedsSync)
}

/// The open workspace's share remote, or a refusal when its provider has no
/// share support — nothing in the UI offers sharing there.
fn remote_for(app: &AppHandle, cryptor: Cryptor) -> Result<AnyShareRemote> {
    share::remote_for(app, cryptor)
        .ok_or_else(|| Error::Other("sharing is only available with Google Drive".into()))
}

/// What a sender-side Drive call needs: the cryptor that unwraps the account's
/// tokens, and the id of the vault asking — which is what says whose shares
/// these are. Both read under one lock, released before the network call.
fn sender_credentials(state: &State<'_, AppState>) -> Result<(Cryptor, String)> {
    let session = state.session.lock().unwrap();
    let vault_id = sendable(&session)?;
    Ok((session.cryptor()?, vault_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::VaultKey;
    use crate::store::SqliteStore;

    fn open(dir: &tempfile::TempDir, sync_configured: bool) -> Session {
        let key = VaultKey::legacy_from_password("pw");
        let store = SqliteStore::open(&dir.path().join("vault.db"), &*key.sqlcipher_key()).unwrap();
        let mut session = Session::default();
        session.set(key, store, sync_configured);
        session
    }

    // Each refusal names the one thing standing in the way, in the order the
    // user can act on them.
    #[test]
    fn sending_needs_an_unlocked_synced_vault_and_says_which_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(sendable(&Session::default()), Err(Error::Locked)));
        assert!(matches!(
            sendable(&open(&dir, false)),
            Err(Error::SyncNotConfigured)
        ));

        // Unlocked and connected, and still nameless: the id is written by the
        // first sync run that succeeds. This is the state the ownership rules
        // once read as owning every share that had no id either.
        let session = open(&dir, true);
        assert!(matches!(sendable(&session), Err(Error::ShareNeedsSync)));

        let id = identity::assign_vault_id(session.store().unwrap()).unwrap();
        assert_eq!(sendable(&session).unwrap(), id);
    }
}
