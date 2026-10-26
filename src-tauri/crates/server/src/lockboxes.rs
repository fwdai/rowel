//! The org lockbox files: SQLCipher databases under the data directory,
//! one per lockbox, keyed by a master the server minted and holds wrapped
//! under its root key (`rootkey`). Each is the same file the app writes —
//! `vault.db` with its `vault.kdf.json` sidecar — so the engine that merges
//! a device's pack and the broker that serves an agent open it as they
//! open any lockbox. What a device gets is the master sealed to its
//! enrollment key, and only once the device is approved (spec `02`).
//!
//! Rotation is the next iteration's; freezing is a flag on the row
//! (`Store::set_frozen`). The in-memory cache of unwrapped masters with its
//! idle timeout is the host's; this module opens a file on request.

use std::path::{Path, PathBuf};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::Serialize;
use zeroize::Zeroizing;

use rowel_core::crypto::{random_hex_id, KdfParams, VaultKey, KEY_LEN};
use rowel_core::layout::{DB_FILE, KDF_SIDECAR_FILE};
use rowel_core::store::{create_private_dir, identity, SqliteStore, VaultStore};

use crate::rootkey::{RootKey, RootKeyError};
use crate::store::{Device, DeviceState, Lockbox, Member, Store, StoreError};

/// The directory under the data directory the lockbox directories are in.
pub const LOCKBOXES_DIR: &str = "lockboxes";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    RootKey(#[from] RootKeyError),
    #[error("the lockbox file: {0}")]
    Vault(#[from] rowel_core::store::StoreError),
    #[error(transparent)]
    Core(#[from] rowel_core::error::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("no lockbox {0}")]
    NotFound(String),
    #[error("a device public key is 32 bytes of base64")]
    DevicePublicKey,
    #[error("the master is sealed only to an approved device")]
    DeviceNotApproved,
    #[error("sealing the master to the device failed")]
    Seal,
}

pub type Result<T> = std::result::Result<T, Error>;

/// A lockbox file, open: its store, the key it opened with and the
/// descriptor string the sidecar and every pack header carry.
pub struct Opened {
    pub store: SqliteStore,
    pub key: VaultKey,
    pub kdf: String,
}

/// One entry of the app's `GET /v1/members/me/lockboxes`
/// (`docs/server-api.md`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LockboxListing {
    pub id: String,
    pub name: String,
    pub kdf: String,
    /// The master sealed to the calling device's enrollment key; only for
    /// an approved device.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wrapped_master: Option<String>,
    /// The pack's current revision; a lockbox with no file yet has none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,
}

/// The directory of lockbox `id` under `data_dir`.
pub fn dir_of(data_dir: &Path, id: &str) -> PathBuf {
    data_dir.join(LOCKBOXES_DIR).join(id)
}

/// `revision` as the ETag the pack endpoints and the listing carry: quoted,
/// as HTTP spells a strong validator and as the app's fake server answers.
/// The device compares it verbatim and hands it back.
pub fn etag(revision: i64) -> String {
    format!("\"{revision}\"")
}

/// Mint a lockbox: a random master, an `External` descriptor at generation
/// 1 under a fresh `kid`, the file keyed with the master and carrying the
/// lockbox id as its vault id (the two are one id, so every device's copy
/// and the server's agree on what they sync), the sidecar, and the row with
/// the master wrapped under the root key. The row is written last, so a
/// failure on the way leaves no lockbox the listing would name; the
/// directory it may leave is removed.
pub fn create(store: &Store, root: &dyn RootKey, data_dir: &Path, name: &str) -> Result<Lockbox> {
    let id = random_hex_id();
    let dir = dir_of(data_dir, &id);
    let kdf = KdfParams::external(random_hex_id(), 1).to_json()?;
    let mut master = Zeroizing::new(vec![0u8; KEY_LEN]);
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut master);
    let wrapped = root.wrap(&store.tenant()?.id, &id, &master)?;

    let made = make_file(&dir, &id, name, &kdf, master);
    if let Err(e) = made {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(e);
    }
    Ok(store.create_lockbox(&id, name, &kdf, &wrapped, 1)?)
}

fn make_file(
    dir: &Path,
    id: &str,
    name: &str,
    kdf: &str,
    master: Zeroizing<Vec<u8>>,
) -> Result<()> {
    create_private_dir(dir)?;
    rowel_core::atomic::atomic_write_file(&dir.join(KDF_SIDECAR_FILE), kdf)?;
    let key = VaultKey::from_material(master, true);
    let vault = SqliteStore::open(&dir.join(DB_FILE), &*key.sqlcipher_key())?;
    // The descriptor mirrored into `meta` as the app's `create_vault` does,
    // so a tool reading the file alone knows how it is keyed.
    vault.meta_set_many(&[("kdf", "external"), ("kdf_params", kdf)])?;
    identity::adopt_vault_id(&vault, id)?;
    identity::set_vault_name(&vault, name, crate::store::now_ms())?;
    Ok(())
}

/// Open lockbox `id`: unwrap its master under the root key and open the
/// file with it.
pub fn open(store: &Store, root: &dyn RootKey, data_dir: &Path, id: &str) -> Result<Opened> {
    let lockbox = store
        .lockbox(id)?
        .ok_or_else(|| Error::NotFound(id.into()))?;
    // Every path from here is built from the stored row's id, which the
    // server minted, never from the caller's string: an id that reached this
    // function from a request names a row, and only the row names a file.
    let master = master_of(store, root, &lockbox.id)?;
    let key = VaultKey::from_material(master, true);
    let vault = SqliteStore::open(
        &dir_of(data_dir, &lockbox.id).join(DB_FILE),
        &*key.sqlcipher_key(),
    )?;
    Ok(Opened {
        store: vault,
        key,
        kdf: lockbox.kdf,
    })
}

/// The master of `id`, unwrapped: in memory for as long as the caller
/// holds it and wiped after.
fn master_of(store: &Store, root: &dyn RootKey, id: &str) -> Result<Zeroizing<Vec<u8>>> {
    let wrapped = store
        .wrapped_master(id)?
        .ok_or_else(|| Error::NotFound(id.into()))?;
    Ok(root.unwrap(&store.tenant()?.id, id, &wrapped)?)
}

/// Seal the master of `lockbox_id` to `device`'s enrollment key — a
/// `crypto_box` sealed box, base64, exactly what the app's enrollment key
/// opens — and keep it with the lockbox's generation, so a later rotation
/// is seen as a stale seal. Only for an approved device: the sealed master
/// is what a pending or revoked one must never be handed.
pub fn wrap_for_device(
    store: &Store,
    root: &dyn RootKey,
    lockbox_id: &str,
    device: &Device,
) -> Result<String> {
    if device.state != DeviceState::Approved {
        return Err(Error::DeviceNotApproved);
    }
    let lockbox = store
        .lockbox(lockbox_id)?
        .ok_or_else(|| Error::NotFound(lockbox_id.into()))?;
    let master = master_of(store, root, lockbox_id)?;
    let sealed = seal_to(&device.public_key, &master)?;
    store.put_device_master(lockbox_id, &device.id, lockbox.gen, &sealed)?;
    Ok(sealed)
}

/// `master` sealed to the enrollment public key `public_key_b64`.
fn seal_to(public_key_b64: &str, master: &[u8]) -> Result<String> {
    let bytes: [u8; 32] = STANDARD
        .decode(public_key_b64.trim())
        .map_err(|_| Error::DevicePublicKey)?
        .try_into()
        .map_err(|_| Error::DevicePublicKey)?;
    let sealed = crypto_box::PublicKey::from(bytes)
        .seal(&mut rand::thread_rng(), master)
        .map_err(|_| Error::Seal)?;
    Ok(STANDARD.encode(sealed))
}

/// For every lockbox `device`'s member holds, `(lockbox id, sealed
/// master)`: the seal kept for the device when it is at the lockbox's
/// generation, a fresh one otherwise. Nothing at all for a device that is
/// not approved.
pub fn wrapped_masters_for(
    store: &Store,
    root: &dyn RootKey,
    device: &Device,
) -> Result<Vec<(String, String)>> {
    if device.state != DeviceState::Approved {
        return Ok(Vec::new());
    }
    let mut sealed = Vec::new();
    for lockbox in store.lockboxes_of_member(&device.member_id)? {
        let held = store
            .device_master(&lockbox.id, &device.id)?
            .filter(|(gen, _)| *gen == lockbox.gen)
            .map(|(_, sealed)| sealed);
        let master = match held {
            Some(sealed) => sealed,
            None => wrap_for_device(store, root, &lockbox.id, device)?,
        };
        sealed.push((lockbox.id, master));
    }
    Ok(sealed)
}

/// The listing for `member` calling from `device`: every lockbox they
/// hold, with the master sealed to the device when the session has one
/// and it is approved and the member's own. Every lockbox minted here has
/// a file from its creation — the server's own first write — so each
/// carries its revision as the ETag from the start.
pub fn listing_for(
    store: &Store,
    root: &dyn RootKey,
    member: &Member,
    device: Option<&Device>,
) -> Result<Vec<LockboxListing>> {
    let sealed = match device.filter(|device| device.member_id == member.id) {
        Some(device) => wrapped_masters_for(store, root, device)?,
        None => Vec::new(),
    };
    Ok(store
        .lockboxes_of_member(&member.id)?
        .into_iter()
        .map(|lockbox| LockboxListing {
            wrapped_master: sealed
                .iter()
                .find(|(id, _)| *id == lockbox.id)
                .map(|(_, master)| master.clone()),
            etag: Some(etag(lockbox.revision)),
            id: lockbox.id,
            name: lockbox.name,
            kdf: lockbox.kdf,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{LockboxRole, Role};
    use crate::testing::TempServer;

    #[test]
    fn a_created_lockbox_opens_with_its_master_and_carries_the_descriptor() {
        let server = TempServer::new();
        let lockbox = server.lockbox("Deploy");
        assert_eq!(lockbox.gen, 1);
        assert_eq!(lockbox.revision, 0);
        assert!(!lockbox.frozen);
        assert_eq!(server.store.lockbox(&lockbox.id).unwrap().unwrap(), lockbox);

        let dir = dir_of(&server.data_dir(), &lockbox.id);
        let sidecar = std::fs::read_to_string(dir.join(KDF_SIDECAR_FILE)).unwrap();
        assert_eq!(sidecar, lockbox.kdf);
        let kdf = KdfParams::from_json(&sidecar).unwrap();
        assert!(kdf.is_external());
        assert_eq!(kdf.gen(), 1);

        let opened = open(&server.store, &server.root, &server.data_dir(), &lockbox.id).unwrap();
        assert_eq!(opened.kdf, lockbox.kdf);
        assert_eq!(
            identity::vault_id(&opened.store).unwrap().as_deref(),
            Some(lockbox.id.as_str())
        );
        assert_eq!(
            identity::vault_name(&opened.store).unwrap().0.as_deref(),
            Some("Deploy")
        );
        // The wrapped master in the row is not the master.
        let wrapped = server.store.wrapped_master(&lockbox.id).unwrap().unwrap();
        let master = master_of(&server.store, &server.root, &lockbox.id).unwrap();
        assert_ne!(wrapped, *master);
        assert_eq!(master.len(), 32);
    }

    #[test]
    fn the_master_sealed_to_an_approved_device_opens_under_its_key() {
        let server = TempServer::new();
        let member = server.member("pat@acme.example", Role::Member);
        let (device, secret) = server.approved_device(&member);
        let lockbox = server.lockbox("Deploy");

        let sealed = wrap_for_device(&server.store, &server.root, &lockbox.id, &device).unwrap();
        let opened = secret.unseal(&STANDARD.decode(&sealed).unwrap()).unwrap();
        let master = master_of(&server.store, &server.root, &lockbox.id).unwrap();
        assert_eq!(opened, *master);
        assert_eq!(
            server.store.device_master(&lockbox.id, &device.id).unwrap(),
            Some((1, sealed))
        );
    }

    #[test]
    fn a_pending_device_is_handed_nothing() {
        let server = TempServer::new();
        let member = server.member("pat@acme.example", Role::Member);
        let (device, _secret) = server.pending_device(&member);
        let lockbox = server.lockbox("Deploy");
        server
            .store
            .add_lockbox_member(&lockbox.id, &member.id, LockboxRole::Reader)
            .unwrap();

        assert!(matches!(
            wrap_for_device(&server.store, &server.root, &lockbox.id, &device),
            Err(Error::DeviceNotApproved)
        ));
        assert_eq!(
            wrapped_masters_for(&server.store, &server.root, &device).unwrap(),
            vec![]
        );
        assert_eq!(
            server.store.device_master(&lockbox.id, &device.id).unwrap(),
            None
        );
        let listed = listing_for(&server.store, &server.root, &member, Some(&device)).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].wrapped_master, None);
    }

    #[test]
    fn the_listing_is_the_members_lockboxes_sealed_to_the_approved_device() {
        let server = TempServer::new();
        let pat = server.member("pat@acme.example", Role::Member);
        let sam = server.member("sam@acme.example", Role::Member);
        let (device, secret) = server.approved_device(&pat);
        let deploy = server.lockbox("Deploy");
        let billing = server.lockbox("Billing");
        let store = &server.store;
        store
            .add_lockbox_member(&deploy.id, &pat.id, LockboxRole::Reader)
            .unwrap();
        store
            .add_lockbox_member(&billing.id, &sam.id, LockboxRole::Admin)
            .unwrap();
        store.bump_revision(&deploy.id).unwrap();

        let listed = listing_for(store, &server.root, &pat, Some(&device)).unwrap();
        assert_eq!(listed.len(), 1);
        let entry = &listed[0];
        assert_eq!(
            (entry.id.as_str(), entry.name.as_str()),
            (deploy.id.as_str(), "Deploy")
        );
        assert_eq!(entry.kdf, deploy.kdf);
        assert_eq!(entry.etag.as_deref(), Some("\"1\""));
        let sealed = entry.wrapped_master.as_deref().unwrap();
        let opened = secret.unseal(&STANDARD.decode(sealed).unwrap()).unwrap();
        assert_eq!(opened, *master_of(store, &server.root, &deploy.id).unwrap());

        // The same seal comes back while the generation stands, and the
        // body is the API's shape.
        let again = listing_for(store, &server.root, &pat, Some(&device)).unwrap();
        assert_eq!(again, listed);
        let json = serde_json::to_value(&listed).unwrap();
        assert_eq!(json[0]["wrappedMaster"].as_str(), Some(sealed));
        assert_eq!(json[0]["etag"].as_str(), Some("\"1\""));

        // A session with no device, or another member's device, gets the
        // listing without the master.
        let bare = listing_for(store, &server.root, &pat, None).unwrap();
        assert_eq!(bare[0].wrapped_master, None);
        assert!(!serde_json::to_value(&bare).unwrap()[0]
            .as_object()
            .unwrap()
            .contains_key("wrappedMaster"));
        let (sams_device, _) = server.approved_device(&sam);
        let crossed = listing_for(store, &server.root, &pat, Some(&sams_device)).unwrap();
        assert_eq!(crossed[0].wrapped_master, None);
    }

    #[test]
    fn a_seal_behind_the_generation_is_made_again() {
        let server = TempServer::new();
        let pat = server.member("pat@acme.example", Role::Member);
        let (device, secret) = server.approved_device(&pat);
        let lockbox = server.lockbox("Deploy");
        server
            .store
            .add_lockbox_member(&lockbox.id, &pat.id, LockboxRole::Reader)
            .unwrap();
        server
            .store
            .put_device_master(&lockbox.id, &device.id, 0, "stale")
            .unwrap();

        let sealed = wrapped_masters_for(&server.store, &server.root, &device).unwrap();
        assert_eq!(sealed.len(), 1);
        assert_ne!(sealed[0].1, "stale");
        secret
            .unseal(&STANDARD.decode(&sealed[0].1).unwrap())
            .unwrap();
        assert_eq!(
            server
                .store
                .device_master(&lockbox.id, &device.id)
                .unwrap()
                .map(|(gen, _)| gen),
            Some(1)
        );
    }
}
