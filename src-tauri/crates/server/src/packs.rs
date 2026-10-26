//! The pack endpoints (`docs/server-api.md`, "Pack endpoints"): an org
//! lockbox synced through the server as its provider. The server is the merge
//! authority, not a file store: a `PUT` runs the receiving half of the sync
//! engine over the server's own file — open the pack as a second device
//! would, merge it, move the revision — and a `GET` packs the file as it now
//! stands, so a device's next pull is the union of every device's pushes and
//! every row the server wrote itself (spec `07`, Q19).
//!
//! Who may sync. The member must hold the lockbox, in any role; a lockbox
//! they do not hold is a 404, as one that does not exist is, so a member
//! learns nothing of the others. The session's device must be approved: a
//! pending or revoked device, or a session that never enrolled one, holds no
//! master to open a pack with, and even the ciphertext tells size and timing.
//!
//! The revision is the lockbox row's, moved by every write of the file: here
//! once per accepted `PUT`, and by the vault for every row the server writes
//! itself (`vaults::mirror_rows`). The device hands back the ETag of the HEAD
//! it pulled under, so a server-side write in between fails its `PUT` with
//! 412, and its next run pulls, merges and pushes again.
//!
//! Locks. A `PUT` holds the vault's session lock from the revision check to
//! the bump, so no grant can write a row between the two and no other push can
//! pass the same check; the metadata store's connection is taken inside it,
//! innermost, as everywhere (`vaults`). A grant's row is bumped for when it is
//! mirrored, after the grant lets the lock go, so a push can pass its check
//! with that row already in the file. It loses nothing by it — the server
//! merges and never overwrites — and the bump that follows sends the device
//! back for the row.

use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::header::{CONTENT_TYPE, ETAG, IF_MATCH, IF_NONE_MATCH};
use axum::http::{HeaderMap, HeaderName, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;

use rowel_broker::protocol::Code;
use rowel_core::store::create_private_dir;
use rowel_core::sync::pack::{self, MAX_PACK_BYTES};
use rowel_core::sync::snapshot::{self, SnapshotError};

use crate::api::Failure;
use crate::auth::Authed;
use crate::lockboxes::{self, Opened};
use crate::state::AppState;
use crate::store::{DeviceState, Lockbox};
use crate::vaults::Vault;

/// The directory under the data directory where packs are snapshotted and
/// pushed snapshots opened. Only ciphertext ever lands there, and each file
/// is removed when its request ends, but it is owner-only all the same.
const SCRATCH_DIR: &str = "scratch";

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v1/lockboxes/{id}/pack", get(get_pack).put(put_pack))
        .layer(DefaultBodyLimit::max(MAX_PACK_BYTES))
}

/// What a `PUT` is conditional on.
enum Precondition {
    /// `If-None-Match: *`: create the pack.
    Create,
    /// `If-Match: <etag>`: update the pack at that revision.
    Update(String),
}

/// The precondition of a `PUT`, or why it has none the server takes.
fn precondition(headers: &HeaderMap) -> Result<Precondition, Failure> {
    let header = |name: HeaderName| -> Result<Option<String>, Failure> {
        headers
            .get(&name)
            .map(|value| {
                value
                    .to_str()
                    .map(str::to_owned)
                    .map_err(|_| Failure::bad_request(format!("the {name} header is not text")))
            })
            .transpose()
    };
    match (header(IF_NONE_MATCH)?, header(IF_MATCH)?) {
        (Some(any), _) if any.trim() == "*" => Ok(Precondition::Create),
        // An ETag in `If-None-Match` is a conditional GET's, never a PUT's.
        (Some(_), _) => Err(Failure::bad_request(
            "If-None-Match on a pack PUT is only ever \"*\"",
        )),
        (None, Some(etag)) => Ok(Precondition::Update(etag)),
        // A blind PUT would put a device's copy over a write it never merged.
        (None, None) => Err(Failure::new(
            StatusCode::PRECONDITION_REQUIRED,
            "a pack PUT carries If-Match with the revision it pulled",
        )),
    }
}

/// The lockbox `id` as the caller may sync it: held by their member in any
/// role, from an approved device of theirs.
fn held(state: &AppState, authed: &Authed, id: &str) -> Result<Lockbox, Failure> {
    let approved = authed.device.as_ref().is_some_and(|device| {
        device.state == DeviceState::Approved && device.member_id == authed.member.id
    });
    if !approved {
        return Err(Failure::forbidden(
            "a lockbox syncs only to an approved device; approve this one from another of your \
             devices",
        ));
    }
    state
        .store
        .lockboxes_of_member(&authed.member.id)?
        .into_iter()
        .find(|lockbox| lockbox.id == id)
        .ok_or_else(|| Failure::not_found(format!("no lockbox {id}")))
}

fn frozen() -> Failure {
    Failure::new(
        StatusCode::LOCKED,
        "this lockbox is frozen by an admin; nothing is written to it until it is thawed",
    )
}

/// The vault of `lockbox`, open: its master unwrapped if it was sealed, which
/// is an unwrap like any other and logged as one (`vaults`).
fn open_vault(state: &AppState, lockbox: &Lockbox) -> Result<Arc<Vault>, Failure> {
    state
        .vaults
        .open(state, &lockbox.id)
        .map_err(|code| match code {
            // Frozen, by the row as it stands under the vault's lock.
            Code::GrantDenied => frozen(),
            _ => Failure::internal(format!("lockbox {} would not open", lockbox.id)),
        })
}

/// A vault sealed by the idle tick or a freeze between its opening and this
/// request taking its lock.
fn sealed() -> Failure {
    Failure::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "the lockbox was sealed while this request waited; sync will retry",
    )
}

fn scratch_dir(state: &AppState) -> Result<PathBuf, Failure> {
    let dir = state.data_dir.join(SCRATCH_DIR);
    create_private_dir(&dir).map_err(Failure::internal)?;
    Ok(dir)
}

fn current_etag(state: &AppState, id: &str) -> Result<String, Failure> {
    let revision = state
        .store
        .revision(id)?
        .ok_or_else(|| Failure::not_found(format!("no lockbox {id}")))?;
    Ok(lockboxes::etag(revision))
}

/// The pack of `opened` as it stands, keyed and headed as a device's.
fn pack_of(opened: &Opened, scratch: &std::path::Path) -> Result<Vec<u8>, Failure> {
    pack::pack_store(
        &opened.store,
        &*opened.key.sqlcipher_key(),
        &opened.kdf,
        scratch,
    )
    .map_err(Failure::internal)
}

/// Run `work` off the async workers: it unwraps, snapshots and merges whole
/// SQLCipher files, and takes the vault's blocking locks.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, Failure> + Send + 'static,
) -> Result<T, Failure> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(Failure::internal)?
}

/// The pack of the server's file as it stands, and for a `HEAD` the same
/// answer without its body (axum strips it), so the `Content-Length` is the
/// pack's: an empty reply would carry a length of zero, which the pack never
/// has, and its length is known only by packing it. The device's run fetches
/// right after its probe, so a probe that unwraps the master costs no unwrap
/// the fetch would not have made.
///
/// Served from a frozen lockbox too: its members already hold the master and
/// a copy of everything in it, so withholding the pack takes nothing from
/// them, while it would fail every one of their sync runs. Frozen stops what
/// reaches the file — agents, and pushes (423) — not a member's pull of what
/// is there.
async fn get_pack(
    State(state): State<Arc<AppState>>,
    authed: Authed,
    Path(id): Path<String>,
) -> Result<Response, Failure> {
    let lockbox = held(&state, &authed, &id)?;
    let member = authed.member.id.clone();
    let device = authed.device.as_ref().map(|device| device.id.clone());
    let (bytes, etag) = blocking(move || {
        let scratch = scratch_dir(&state)?;
        // Frozen or not is the vault's to say, under its lock (`Vaults::
        // open`), never the row `held` read before it: a freeze between the
        // two would otherwise send a frozen lockbox down the cached path.
        match state.vaults.open(&state, &lockbox.id) {
            Ok(vault) => {
                let session = vault.session();
                if let Ok(open) = session.open() {
                    // Read under the lock the file is written under, so the
                    // ETag is never newer than the bytes.
                    let etag = current_etag(&state, &lockbox.id)?;
                    return Ok((pack_of(&open.opened, &scratch)?, etag));
                }
                // Sealed since it was opened: by the idle tick, or a freeze.
                drop(session);
                if !is_frozen(&state, &lockbox.id)? {
                    return Err(sealed());
                }
            }
            Err(Code::GrantDenied) => {}
            Err(_) => {
                return Err(Failure::internal(format!(
                    "lockbox {} would not open",
                    lockbox.id
                )))
            }
        }
        frozen_pull(&state, &lockbox.id, &member, device.as_deref(), &scratch)
    })
    .await?;
    Ok((
        StatusCode::OK,
        [
            (CONTENT_TYPE, "application/octet-stream".to_string()),
            (ETAG, etag),
        ],
        bytes,
    )
        .into_response())
}

/// A frozen lockbox's pack, opened for this pull alone, outside the vault
/// cache, which never holds a frozen lockbox's key. The unwrap is a tenant
/// event rather than a `key_unwrapped` row: a frozen file is written by
/// nothing, and a row per pull would move its revision on every device's run.
/// Every unwrap is still on record.
fn frozen_pull(
    state: &AppState,
    id: &str,
    member: &str,
    device: Option<&str>,
    scratch: &std::path::Path,
) -> Result<(Vec<u8>, String), Failure> {
    let opened = lockboxes::open(&state.store, &*state.root, &state.data_dir, id)?;
    if let Err(e) = state
        .store
        .record_event("key_unwrapped", Some(member), device, Some(id))
    {
        log::warn!("rowel server: could not record an unwrap of {id}: {e}");
    }
    let etag = current_etag(state, id)?;
    Ok((pack_of(&opened, scratch)?, etag))
}

/// Whether lockbox `id` is frozen, by its row as it stands.
fn is_frozen(state: &AppState, id: &str) -> Result<bool, Failure> {
    Ok(state
        .store
        .lockbox(id)?
        .is_some_and(|lockbox| lockbox.frozen))
}

/// Merge a device's pack into the server's file, as a compare-and-swap on
/// the revision it pulled under.
async fn put_pack(
    State(state): State<Arc<AppState>>,
    authed: Authed,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, Failure> {
    let precondition = precondition(&headers)?;
    let lockbox = held(&state, &authed, &id)?;
    if lockbox.frozen {
        blocking(move || {
            state.vaults.seal(&state, &lockbox.id);
            Ok(())
        })
        .await?;
        return Err(frozen());
    }
    let expected = match precondition {
        // The file is made with the lockbox, so there is always a pack here.
        // The device's create then fails with its retry message, and its next
        // run finds the pack, pulls, merges and updates under `If-Match`.
        Precondition::Create => {
            return Err(Failure::new(
                StatusCode::PRECONDITION_FAILED,
                "this lockbox's pack already exists on the server; pull it and update it",
            ))
        }
        Precondition::Update(etag) => etag,
    };
    let etag = blocking(move || merge_push(&state, &lockbox, &expected, &body)).await?;
    Ok((StatusCode::OK, [(ETAG, etag)]).into_response())
}

/// The receiving half of the engine over the server's file, under the vault's
/// session lock from the revision check to the bump. The new ETag.
fn merge_push(
    state: &AppState,
    lockbox: &Lockbox,
    expected: &str,
    body: &[u8],
) -> Result<String, Failure> {
    // Parsed before anything is unwrapped: a body that is not a pack costs
    // the master nothing.
    let unpacked = pack::unpack(body).map_err(|e| Failure::bad_request(e.to_string()))?;
    let vault = open_vault(state, lockbox)?;
    let session = vault.session();
    // Sealed between the open and this lock: a freeze, which this push must
    // not outlive, or the idle tick, which the device simply retries.
    let Ok(open) = session.open() else {
        drop(session);
        return Err(if is_frozen(state, &lockbox.id)? {
            frozen()
        } else {
            sealed()
        });
    };
    let opened = &open.opened;
    if expected != current_etag(state, &lockbox.id)? {
        return Err(Failure::new(
            StatusCode::PRECONDITION_FAILED,
            "the lockbox changed on the server since this device pulled it; pull, merge and push \
             again",
        ));
    }
    // The server holds the current generation's key alone until rotation
    // keeps the retired ones (I8), so a pack under any other descriptor is
    // one it cannot open; a SQLCipher open under the wrong key would only say
    // "invalid", which is the misdiagnosis the header is there to prevent.
    if unpacked.kdf_params_json != opened.kdf {
        return Err(Failure::conflict(
            "this pack is under another key generation; refresh the lockbox key",
        ));
    }
    let scratch = scratch_dir(state)?;
    let contents = snapshot::read(&unpacked.snapshot, &*opened.key.sqlcipher_key(), &scratch)
        .map_err(|e| match e {
            SnapshotError::SchemaNewer => Failure::conflict(
                "this pack was written by a newer app than the server reads; update the server",
            ),
            SnapshotError::Invalid => {
                Failure::bad_request("this pack does not open under the lockbox's key")
            }
            other => Failure::internal(other),
        })?;
    // The pushed name is not taken: an org lockbox is named at the server,
    // and a device cannot rename it.
    opened
        .store
        .merge(&contents.records, &contents.attachments, &contents.audit)
        .map_err(Failure::internal)?;
    // The devices' rows, which reach the export only this way: the vault's
    // own mirror passes copy the server's chain alone. A failure is logged
    // rather than failing a merge that has landed; the next unwrap's full
    // pass copies every writer's rows.
    if let Err(e) = state.store.mirror_audit(&lockbox.id, &contents.audit) {
        log::warn!("rowel server: audit mirror of {}: {e}", lockbox.id);
    }
    // Moved even when the merge changed nothing: every accepted push is
    // answered with a revision of its own, so the device's record of where
    // its push landed is the one its next HEAD reads back.
    let revision = state.store.bump_revision(&lockbox.id)?;
    Ok(lockboxes::etag(revision))
}
