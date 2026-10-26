//! Settings › Lockboxes › Rowel Server: the install's servers, this device's
//! standing at each, the lockboxes a server offers, and the ones added here.
//!
//! Everything a member does with a server goes through the session the
//! sign-in left sealed under the app key (`org::connections`), so every
//! command here needs the vault open at the app level and answers `Locked`
//! otherwise. Adding a lockbox is the one that writes a workspace: the files
//! an added lockbox is made of are `org::lockboxes`'s, and the registry,
//! the ring and the first sync follow the order every other workspace
//! writer keeps (`commands::autojoin`, which adds a vault beside the open
//! one the same way).

use serde::Serialize;
use tauri::{AppHandle, Manager, State};
use zeroize::Zeroizing;

use crate::appkey;
use crate::crypto::VaultKey;
use crate::error::{Error, Result};
use crate::org::client::DeviceState;
use crate::org::connections::{self, Connection, SessionResolver};
use crate::org::{enrollment, lockboxes};
use crate::state::AppState;
use crate::sync;
use crate::workspace::{self, Registry, Workspace};
use crate::{events, storage};

use super::blocking;
use super::setup::begin_step;
use super::workspace::{discard, guard_other_vault, update_registry};

/// One server the install is signed in to, as the section lists it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrgConnection {
    url: String,
    tenant_name: String,
    member_email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    member_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    device_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    device_state: Option<DeviceState>,
}

impl From<Connection> for OrgConnection {
    fn from(connection: Connection) -> Self {
        Self {
            url: connection.url,
            tenant_name: connection.tenant_name,
            member_email: connection.member_email,
            member_name: connection.member_name,
            device_id: connection.device_id,
            device_state: connection.device_state,
        }
    }
}

/// One of the member's devices at a server, with the fingerprint of its
/// enrollment key — what the member compares with the one the new device
/// shows before approving it — and whether it is this one.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrgDevice {
    id: String,
    name: String,
    state: DeviceState,
    fingerprint: String,
    this_device: bool,
    created_at_ms: i64,
}

/// One lockbox a server offers the member, and the workspace here that
/// holds it when one does.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrgLockbox {
    id: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    workspace_id: Option<String>,
}

/// This device's enrollment fingerprint: what a member reads off this screen
/// to approve the device from another, minted on first ask.
#[tauri::command]
pub fn org_enrollment_fingerprint(app: AppHandle) -> Result<String> {
    Ok(enrollment::load_or_create_for(&app)?.fingerprint())
}

/// Every server the install is signed in to, with this device's standing
/// at each refreshed from the server where it can be reached — an approval
/// made elsewhere shows up here — and kept; a server that cannot be reached
/// is listed as last known.
#[tauri::command]
pub async fn org_connections(app: AppHandle) -> Result<Vec<OrgConnection>> {
    let (root, app_key) = connections::app_root_and_key(&app)?;
    let held = connections::load(&root, &app_key)?;
    let resolver = SessionResolver::App(app.clone());
    let mut changed = false;
    let mut listed = Vec::with_capacity(held.len());
    for mut connection in held {
        if let Some(device_id) = connection.device_id.clone() {
            let devices = resolver
                .as_member(
                    &connection.url,
                    |client| async move { client.devices().await },
                )
                .await;
            match devices {
                Ok(devices) => {
                    // A device the server no longer lists was removed there,
                    // and no listing will ever hand it a key again.
                    let state = devices
                        .iter()
                        .find(|d| d.id == device_id)
                        .map(|d| d.state)
                        .unwrap_or(DeviceState::Revoked);
                    if connection.device_state != Some(state) {
                        // In place, over the record as it stands now: the
                        // listing above may have refreshed the session, and
                        // the copy read before it holds the tokens it retired.
                        // And only onto the same member's same device: a
                        // sign-in that replaced this one meanwhile is not the
                        // device this listing was about, and keeps its own.
                        let expect = connections::Expect::sign_in(&connection);
                        let updated = resolver.update(&connection.url, &expect, |held| {
                            held.device_state = Some(state)
                        })?;
                        changed |= matches!(updated, connections::Updated::Changed(_));
                        connection = updated.into_record();
                    }
                }
                Err(e) => log::info!(
                    "this device's standing at {} could not be refreshed: {e}",
                    connection.url
                ),
            }
        }
        listed.push(OrgConnection::from(connection));
    }
    #[cfg(desktop)]
    if changed {
        crate::org::approvals::restart(&app);
    }
    #[cfg(not(desktop))]
    let _ = changed;
    Ok(listed)
}

/// Sign the install in to the server at `url` and enroll this device
/// (`org::connect`): returns the attempt's number once the browser is open,
/// and `org:connect` reports the rest under it.
#[cfg(desktop)]
#[tauri::command]
pub fn org_connect(url: String, app: AppHandle) -> Result<u64> {
    crate::org::connect::start(&app, &url)
}

/// A phone has no loopback listener to come back to; its sign-in is a
/// deep-link flow of its own, in a later release. `AppStatus::org_connect`
/// says so up front, so the section is not drawn here at all.
#[cfg(mobile)]
#[tauri::command]
pub fn org_connect(url: String) -> Result<u64> {
    connections::normalize_url(&url)?;
    Err(Error::Unsupported(
        "connecting to a Rowel Server from a phone is coming in a later release; connect from \
         the desktop app"
            .into(),
    ))
}

/// Disown the sign-in under way.
#[tauri::command]
pub fn org_cancel_connect() -> Result<()> {
    #[cfg(desktop)]
    crate::org::connect::cancel();
    Ok(())
}

/// Forget the server at `url`: the session goes, and with it this device's
/// standing there. The org lockboxes added from it stay, as copies the
/// member may still read; their runs fail as not signed in until the member
/// signs in again, which is what makes them live copies once more.
#[tauri::command]
pub fn org_disconnect(url: String, app: AppHandle) -> Result<()> {
    {
        let state = app.state::<AppState>();
        let _sidecars = state.sidecars.lock().unwrap();
        let (root, app_key) = connections::app_root_and_key(&app)?;
        connections::remove(&root, &app_key, &url)?;
    }
    events::org_changed(&app);
    #[cfg(desktop)]
    crate::org::approvals::restart(&app);
    Ok(())
}

/// The member's devices at the server at `url`.
#[tauri::command]
pub async fn org_devices(url: String, app: AppHandle) -> Result<Vec<OrgDevice>> {
    let resolver = SessionResolver::App(app.clone());
    let connection = resolver.connection(&url)?;
    let devices = resolver
        .as_member(&url, |client| async move { client.devices().await })
        .await?;
    devices
        .into_iter()
        .map(|device| {
            Ok(OrgDevice {
                fingerprint: enrollment::fingerprint_of(&device.public_key)?,
                this_device: connection.device_id.as_deref() == Some(device.id.as_str()),
                id: device.id,
                name: device.name,
                state: device.state,
                created_at_ms: device.created_at_ms,
            })
        })
        .collect()
}

/// Approve device `device_id` of the same member at the server at `url`,
/// from this device, which has to be approved itself.
#[tauri::command]
pub async fn org_approve_device(url: String, device_id: String, app: AppHandle) -> Result<()> {
    SessionResolver::App(app.clone())
        .as_member(&url, |client| {
            let device_id = device_id.clone();
            async move { client.approve_device(&device_id).await }
        })
        .await?;
    events::org_changed(&app);
    Ok(())
}

/// The lockboxes the server at `url` offers the member, each with the
/// workspace here that already holds it.
#[tauri::command]
pub async fn org_lockboxes(url: String, app: AppHandle) -> Result<Vec<OrgLockbox>> {
    let url = connections::normalize_url(&url)?;
    let listed = SessionResolver::App(app.clone())
        .as_member(&url, |client| async move { client.lockboxes().await })
        .await?;
    let registry = Registry::load(&storage::root_dir(&app)?);
    Ok(listed
        .into_iter()
        .map(|lockbox| OrgLockbox {
            workspace_id: lockboxes::workspace_holding(&registry, &url, &lockbox.id),
            id: lockbox.id,
            name: lockbox.name,
        })
        .collect())
}

/// Add lockbox `lockbox_id` of the server at `url` as a workspace beside the
/// open one, and start its first sync, so a pack the server already holds
/// is pulled. The new workspace's id comes back. Refused while this device
/// is not approved — the server hands an approved device the master and no
/// other — and for a lockbox a workspace here already holds.
#[tauri::command]
pub async fn org_add_lockbox(
    url: String,
    lockbox_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<String> {
    let url = connections::normalize_url(&url)?;
    let resolver = SessionResolver::App(app.clone());
    if resolver.connection(&url)?.device_state != Some(DeviceState::Approved) {
        return Err(Error::Other(
            "this device is not approved at the Rowel Server yet; approve it from one of your \
             other devices, or ask an admin"
                .into(),
        ));
    }
    let listed = resolver
        .as_member(&url, |client| async move { client.lockboxes().await })
        .await?;
    let lockbox = listed
        .into_iter()
        .find(|l| l.id == lockbox_id)
        .ok_or_else(|| Error::Other("the Rowel Server does not offer this lockbox".into()))?;
    let (master, kdf) = lockboxes::unseal(&app, &lockbox)?;
    let root = storage::root_dir(&app)?;

    // The same exclusion a create or a restore takes: this writes a KDF
    // sidecar and a database, and a switch may not move the paths while the
    // registry below is written. The duplicate check is made here, cheaply,
    // and again inside the registry write, where it cannot be raced.
    let _step = begin_step(&state)?;
    guard_other_vault(&state, &root, &lockbox.id)?;
    let id = crate::crypto::random_hex_id();
    let dir = workspace::dir_of(&root, &id);
    let material = master.clone();
    let key = {
        let (dir, kdf, lockbox_id, name, server) = (
            dir.clone(),
            kdf.clone(),
            lockbox.id.clone(),
            lockbox.name.clone(),
            url.clone(),
        );
        let installed = blocking(move || {
            lockboxes::install(
                &dir,
                master,
                &kdf,
                &lockbox_id,
                &name,
                &server,
                crate::auth::now_ms(),
            )
        })
        .await;
        match installed {
            Ok(key) => key,
            Err(e) => {
                // Nothing half-made: the directory goes, token file and all.
                discard(&root, &id);
                return Err(e);
            }
        }
    };

    // Registry last, as every workspace writer records it: an entry only
    // once there is a vault behind it, and a refusal takes the files with it.
    let record = lockboxes::record(&id, &lockbox.name, &lockbox.id, &url);
    let recorded = update_registry(&state, &root, |registry| {
        guard_other_vault(&state, &root, &lockbox.id)?;
        registry.workspaces.push(record);
        Ok(())
    });
    if let Err(e) = recorded {
        discard(&root, &id);
        return Err(e);
    }
    // Open at the app level from the start: into the ring, and sealed under
    // the app key for every unlock after this one.
    appkey::adopt(&app, &id, &material);
    events::workspace_added(&app, &lockbox.name);
    events::org_changed(&app);

    // The first run, off the command thread and under the run claim every
    // run holds: a pack the server already holds is pulled into the copy
    // just made. A run already in flight leaves it to the lockbox's first
    // open, which syncs it as any workspace's does.
    match super::sync::claim_or_busy(&state) {
        Ok(claim) => {
            let (app, id) = (app.clone(), id.clone());
            super::detached(move || {
                let _claim = claim;
                match sync::run_workspace(&app, &dir, &id, key) {
                    Ok(outcome) => log::info!(
                        "org lockbox {id} synced after being added (merged: {}, pushed: {})",
                        outcome.merged,
                        outcome.pushed
                    ),
                    Err(e) => log::warn!("org lockbox {id} did not sync after being added: {e}"),
                }
            });
        }
        Err(_) => {
            log::info!("a sync run is in flight; org lockbox {id} syncs when it is next opened")
        }
    }
    Ok(id)
}

/// Fetch workspace `workspace_id`'s master from its server again
/// (`lockboxes::key_step` decides what it is). A listing at a later key
/// generation than the sidecar's is a rotation: the local copy is re-keyed
/// onto the new master exactly as a password change re-keys it, retiring the
/// old epoch so older packs still open. The same descriptor is the key
/// handed over once more — checked against the database, then into the ring,
/// and the sealed copy beside the database written again, which is what
/// repairs a lockbox whose copy went missing. An older generation, or another
/// key at the same one, is refused and the local key kept.
#[tauri::command]
pub async fn org_refresh_lockbox_key(
    workspace_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<()> {
    let root = storage::root_dir(&app)?;
    let workspace = lockboxes::org_workspace(&root, &workspace_id)?;
    let (master, kdf) = fetch_master(&app, &workspace).await?;
    let dir = workspace::dir_of(&root, &workspace_id);
    if lockboxes::key_step(&dir, &kdf)? == lockboxes::KeyStep::Repair {
        let (check_dir, check) = (dir.clone(), master.clone());
        blocking(move || lockboxes::check_opens(&check_dir, &check)).await?;
        appkey::adopt(&app, &workspace_id, &master);
        return Ok(());
    }

    // A re-key writes the sidecar and the database: the exclusion a password
    // change takes, and no run beside it.
    let _step = begin_step(&state)?;
    if workspace::active_id(&app) == workspace_id {
        let claim = super::sync::sync_before_rekey(&app).await;
        rekey_open(&app, &state, master, kdf).await?;
        // The account's pack is still under the epoch just retired; this run
        // opens it with the retired key and pushes it under the new one.
        drop(claim);
        super::sync::request_run_if_ready(&app, &state);
        return Ok(());
    }
    let held = state
        .keyring
        .lock()
        .unwrap()
        .get(&workspace_id)
        .ok_or_else(|| {
            Error::Other(
                "this lockbox's previous key is not held on this device, so its copy cannot be \
                 moved onto the new one; remove the lockbox here and add it again"
                    .into(),
            )
        })?;
    let material = master.clone();
    {
        let (dir, id, kdf) = (dir.clone(), workspace_id.clone(), kdf.clone());
        blocking(move || lockboxes::rekey_closed(&dir, &id, held, master, &kdf)).await?;
    }
    appkey::adopt(&app, &workspace_id, &material);
    // Its copy in the account re-sealed under the new key now, as a password
    // change does for each lockbox it reaches, rather than at its next open.
    match super::sync::claim_or_busy(&state) {
        Ok(claim) => {
            super::sync::sync_rekeyed(&app, &dir, &workspace_id, &material).await;
            drop(claim);
        }
        Err(_) => log::info!(
            "a sync run is in flight; org lockbox {workspace_id} syncs when it is next opened"
        ),
    }
    Ok(())
}

/// The open workspace, an org lockbox whose run just found the account's
/// pack under a later key epoch than its own, moved onto the master the
/// server holds now: the sync run's half of a rotation (`commands::sync`),
/// made on the run's own thread with its claim held. Refused when the
/// server's listing is at the same generation as the sidecar — then the
/// epoch mismatch is something else, and the run's error stands.
pub(super) fn rotate_open_lockbox(app: &AppHandle) -> Result<()> {
    let state = app.state::<AppState>();
    let _step = begin_step(&state)?;
    let root = storage::root_dir(app)?;
    let id = workspace::active_id(app);
    let workspace = lockboxes::org_workspace(&root, &id)?;
    let (master, kdf) = tauri::async_runtime::block_on(fetch_master(app, &workspace))?;
    if lockboxes::key_step(&workspace::dir_of(&root, &id), &kdf)? != lockboxes::KeyStep::Rotate {
        return Err(Error::Other(
            "the Rowel Server holds no newer key for this lockbox".into(),
        ));
    }
    tauri::async_runtime::block_on(rekey_open(app, &state, master, kdf))
}

// `workspace`'s master as its server lists it for this device now, unsealed,
// with the descriptor that names it.
async fn fetch_master(
    app: &AppHandle,
    workspace: &Workspace,
) -> Result<(Zeroizing<Vec<u8>>, String)> {
    let server = workspace
        .server
        .as_deref()
        .ok_or_else(|| Error::Other("this lockbox did not come from a Rowel Server".into()))?;
    let vault_id = workspace
        .vault_id
        .as_deref()
        .ok_or_else(|| Error::Other("this lockbox has no id at its Rowel Server".into()))?;
    let listed = SessionResolver::App(app.clone())
        .as_member(server, |client| async move { client.lockboxes().await })
        .await?;
    let lockbox = listed
        .into_iter()
        .find(|l| l.id == vault_id)
        .ok_or_else(|| {
            Error::Other("the Rowel Server no longer offers this lockbox to this member".into())
        })?;
    lockboxes::unseal(app, &lockbox)
}

// The open workspace re-keyed from the session's key onto `master` under the
// server's descriptor `kdf`: the password change's saga, with the current
// key read off the session rather than proved by a password, since no
// password ever made it. The caller holds the setup step.
async fn rekey_open(
    app: &AppHandle,
    state: &AppState,
    master: Zeroizing<Vec<u8>>,
    kdf: String,
) -> Result<()> {
    let current_key = {
        let session = state.session.lock().unwrap();
        VaultKey::from_material(
            Zeroizing::new(session.key()?.biometric_material().to_vec()),
            true,
        )
    };
    super::auth::rekey_open_workspace(
        app,
        state,
        current_key,
        VaultKey::from_material(master, true),
        kdf,
    )
    .await
    .map(|_| ())
}
