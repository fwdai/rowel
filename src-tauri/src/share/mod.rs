//! Credential sharing by link: seal one entry, park it in the sender's own
//! Drive for a day, hand out a link that carries the key.
//!
//! [`envelope`] holds the crypto and the link format, [`remote`] the Drive
//! folder. This module is the verb layer between them — create, open, revoke,
//! sweep, list — and it is generic over the two transport traits so the whole
//! lifecycle is testable without a network.
//!
//! The link is the only secret: it never reaches Drive, and nothing here stores
//! it. What Drive holds is ciphertext plus a handful of opaque properties,
//! which is also the only ledger of outstanding shares — any device holding the
//! vault that published one can therefore list, revoke and clean it up, and a
//! reinstall loses nothing.

pub mod envelope;
pub mod remote;
#[cfg(test)]
mod tests;

use serde::Serialize;
use tauri::AppHandle;

use crate::crypto::Cryptor;
use crate::error::{Error, Result};
use crate::models::Entry;
use crate::sync::providers::gdrive::share::{DrivePublicFetch, DriveShareRemote};
use crate::sync::{layout, Provider};
use envelope::{Link, ShareKey, SHARE_TTL_MS};
use remote::{
    AnyShareRemote, PublicFetch, ShareFile, ShareRemote, PROP_ENTRY_ID, PROP_EXPIRES_AT, PROP_KIND,
    PROP_SHARE, PROP_SHARE_VALUE, PROP_VAULT_ID,
};

/// A freshly published share, as the send dialog needs it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Created {
    /// The whole share in one pasteable token. Never persisted anywhere.
    pub link: String,
    /// Kept so the dialog can revoke without re-parsing the link.
    pub file_id: String,
    pub expires_at: String,
}

/// One outstanding share in the sender's own list.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveShare {
    pub file_id: String,
    /// The sender's local entry id, which the UI resolves to a title.
    pub entry_id: Option<String>,
    pub kind: Option<String>,
    pub created_at: String,
    pub expires_at: String,
}

/// Seal `entry` and publish it, returning the link that opens it.
///
/// `vault_id` is the sharing vault's own id, stamped on the file so that vault
/// can pick its shares out of an account several vaults now write into, and so
/// no other one can revoke them. It is required rather than optional: a share
/// carrying no id belongs to nobody (see [`owned_by`]), so publishing one would
/// hand out a link its own sender could not list or take back. A vault holds an
/// id only once a sync run has succeeded, which is why the sending commands
/// refuse before they get here.
pub fn create(
    remote: &impl ShareRemote,
    entry: &Entry,
    vault_id: &str,
    now_ms: i64,
) -> Result<Created> {
    envelope::check_shareable(entry)?;
    let key = ShareKey::generate();
    let expires_ms = envelope::expires_at(now_ms);
    let sealed = envelope::seal(&key, entry, expires_ms)?;

    // Read off the caller's entry, not the sealed copy: `seal` sanitizes the id
    // away, and this property is precisely what lets the sender's own list name
    // a share that carries no title. The marker property is how every share is
    // found again, whichever folder it landed in.
    let expires = expires_ms.to_string();
    let file_id = remote.upload(
        &file_name(),
        &sealed,
        &[
            (PROP_SHARE, PROP_SHARE_VALUE),
            (PROP_ENTRY_ID, entry.id.as_str()),
            (PROP_KIND, entry.kind.as_str()),
            (PROP_EXPIRES_AT, expires.as_str()),
            (PROP_VAULT_ID, vault_id),
        ],
    )?;

    // An uploaded file nobody can read is worse than no share at all: the link
    // would go out and fail, and the orphan would sit in Drive until the sweep
    // dated it. Take it back before reporting the failure.
    if let Err(e) = remote.make_public(&file_id) {
        let _ = remote.delete(&file_id);
        return Err(e);
    }

    Ok(Created {
        expires_at: rfc3339(expires_ms),
        link: Link {
            file_id: file_id.clone(),
            key,
        }
        .format(),
        file_id,
    })
}

/// Turn a link into the entry it carries. The recipient's whole side of this:
/// the result is already sanitized and checked against `now_ms`, so the
/// frontend can hand it straight to the ordinary new-entry save.
pub fn open(fetch: &impl PublicFetch, link: &str, now_ms: i64) -> Result<Entry> {
    let link = Link::parse(link)?;
    let sealed = fetch.download(&link.file_id)?;
    envelope::unseal(&link.key, &sealed, now_ms)
}

/// Delete one share now, whatever its expiry. The link stops working.
///
/// The share has to be the asking vault's own. One account keeps every vault's
/// shares in one folder, and `file_id` is whatever the caller sent — a bearer
/// value that authorizes nothing by itself — so the file is fetched and its
/// `vaultId` read off Drive rather than taken on trust. It is the same test
/// [`list`] applies, so a vault can revoke exactly what it can see and no more,
/// and an unmarked file is not a share of ours at any id: refused, never
/// deleted.
///
/// Fetched by id rather than looked up in the listing, because Drive's query
/// index lags its files: the send dialog offers Revoke seconds after the upload,
/// and a share the index has not caught up with would otherwise read as already
/// gone and be silently left behind.
///
/// A file Drive no longer has does count as revoked: the sweep, another device,
/// or an earlier click of the same button all leave the caller with what it
/// asked for, which is why the delete underneath is idempotent too.
pub fn revoke(remote: &impl ShareRemote, file_id: &str, vault_id: &str) -> Result<()> {
    let Some(file) = remote.get(file_id)? else {
        return Ok(());
    };
    if !owned_by(&file, vault_id) {
        return Err(Error::ShareNotOwned);
    }
    remote.delete(file_id)
}

/// Delete every share whose expiry is known and has passed; returns how many.
pub fn sweep(remote: &impl ShareRemote, now_ms: i64) -> Result<usize> {
    let mut deleted = 0;
    for file in remote.list()? {
        // An absent expiry is "not known", never "expired": revoking a link the
        // sender is still handing out because we failed to date it would be a
        // far worse failure than leaving one file a day too long.
        if file.expires_ms.is_some_and(|at| at <= now_ms) {
            remote.delete(&file.id)?;
            deleted += 1;
        }
    }
    Ok(deleted)
}

/// This vault's outstanding shares, swept first so the list is never showing
/// something a recipient can no longer open.
///
/// The sweep above is account-wide — an expired share is dead wherever it came
/// from, and whichever of the sender's devices notices should say so — but the
/// list is not: an account can hold several vaults' shares now, and every row
/// here carries a Revoke.
pub fn list(remote: &impl ShareRemote, vault_id: &str, now_ms: i64) -> Result<Vec<ActiveShare>> {
    sweep(remote, now_ms)?;
    Ok(remote
        .list()?
        .iter()
        .filter(|file| owned_by(file, vault_id))
        .map(active_share)
        .collect())
}

/// Whether this vault may list `file` and revoke it. The single rule both verbs
/// go through: showing a share to a vault that may not delete it is a Revoke
/// button that fails, and the reverse is one vault deleting another's link.
///
/// It has to be one of this app's shares before it can be anyone's, which a
/// listing settles on its own but [`revoke`] does not — it is handed a file id,
/// and an arbitrary Drive file carries no marker and so belongs to no vault.
///
/// Ownership is two concrete ids agreeing, never two absences: a share carrying
/// no `vaultId` predates the property and is nobody's, shown to no vault and
/// revocable by none. Nothing is stranded by that — it expires within its 24
/// hours and the account-wide [`sweep`] takes it.
fn owned_by(file: &ShareFile, vault_id: &str) -> bool {
    file.marked && file.vault_id.as_deref() == Some(vault_id)
}

fn active_share(file: &ShareFile) -> ActiveShare {
    ActiveShare {
        file_id: file.id.clone(),
        entry_id: file.entry_id.clone(),
        kind: file.kind.clone(),
        created_at: rfc3339(file.created_ms),
        // A share we could not date still has to tell the user when it goes:
        // the TTL from its creation is the expiry it was given, whether or not
        // the property survived.
        expires_at: rfc3339(
            file.expires_ms
                .unwrap_or_else(|| file.created_ms + SHARE_TTL_MS),
        ),
    }
}

/// A name that says nothing: random, and shaped by [`layout::share_file_name`]
/// like every other name Drive sees. Drive shows the owner a file list, so the
/// name carries no title — only enough randomness never to collide.
fn file_name() -> String {
    layout::share_file_name(&crate::crypto::random_hex_id())
}

fn rfc3339(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .unwrap_or(chrono::DateTime::UNIX_EPOCH)
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub(crate) fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// The open workspace's share remote, or `None` when its provider has no share
/// support. Shares are Drive-only until a link can say which provider to fetch
/// from.
pub fn remote_for(app: &AppHandle, cryptor: Cryptor) -> Option<AnyShareRemote> {
    match crate::sync::sealed_provider(app) {
        Provider::GoogleDrive => Some(AnyShareRemote::GoogleDrive(DriveShareRemote::new(
            app.clone(),
            cryptor,
        ))),
        Provider::Dropbox | Provider::ICloud => None,
    }
}

/// What a recipient downloads a link with. Drive's, while every link is a
/// Drive link: the format names no provider yet.
pub fn public_fetch() -> impl PublicFetch {
    DrivePublicFetch
}

/// The sweep as the end of a sync run calls it. A provider without shares has
/// nothing to sweep.
pub fn sweep_account(app: &AppHandle, cryptor: Cryptor) -> Result<usize> {
    match remote_for(app, cryptor) {
        Some(remote) => sweep(&remote, now_ms()),
        None => Ok(0),
    }
}
