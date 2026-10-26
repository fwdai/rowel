//! An org lockbox as a workspace on this device.
//!
//! What tells one from the lockboxes the user holds themselves is its KDF
//! sidecar: `KdfParams::External`, the server's descriptor verbatim, naming a
//! master no password derives. Everything that would run Argon2id on a
//! workspace asks here first — an unlock typed for it, a master-password
//! change reaching the workspaces beside the open one — and leaves an org
//! lockbox to the key the server hands this device. The master itself goes
//! where every non-primary workspace's key goes: the ring, and the sidecar
//! sealed under the app key (`appkey::adopt`).
//!
//! On plain paths wherever the work is files, so the install and the re-key
//! are testable without an app around them; the commands over them are
//! `commands::org`.

use std::path::Path;

use tauri::AppHandle;
use zeroize::Zeroizing;

use super::client::Lockbox;
use super::{connections, enrollment};
use crate::crypto::{KdfParams, VaultKey};
use crate::error::{Error, Result};
use crate::session::store_err;
use crate::store::{identity, SqliteStore, StoreError};
use crate::sync::{self, Provider, Tokens};
use crate::workspace::{self, Registry, Workspace};
use crate::{auth, storage};

/// Whether the lockbox in `dir` is an org lockbox: its sidecar names a
/// master a server holds. A workspace with no sidecar, or one that does not
/// parse, is not one — it is whatever its own unlock makes of it.
pub(crate) fn is_org_lockbox_dir(dir: &Path) -> bool {
    std::fs::read_to_string(dir.join(storage::KDF_SIDECAR_FILE))
        .ok()
        .and_then(|json| KdfParams::from_json(&json).ok())
        .is_some_and(|params| params.is_external())
}

/// The key generation the sidecar in `dir` names.
#[cfg(test)]
pub(crate) fn local_gen(dir: &Path) -> Result<u32> {
    let json = std::fs::read_to_string(dir.join(storage::KDF_SIDECAR_FILE))?;
    Ok(KdfParams::from_json(&json)?.gen())
}

/// What a key the server lists is to the lockbox in `dir`, decided here for
/// every caller so none can treat one generation as another.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum KeyStep {
    /// A later generation: a rotation, and the local copy is re-keyed onto
    /// it through the re-key saga, retiring the epoch it was on.
    Rotate,
    /// The key the lockbox already has, handed over again: the ring and the
    /// sealed copy beside the database may take it, once it has been seen to
    /// open the database ([`check_opens`]).
    Repair,
}

/// Compare the descriptor `kdf` a listing carries with the lockbox's own in
/// `dir`. A later generation is a rotation. The same generation is a repair
/// only when the descriptor is the very string the lockbox holds, since the
/// sync engine compares them byte for byte and a different `kid` at the same
/// generation is a different key. An earlier generation is refused: a stale
/// or rolled-back listing, whose master would replace the one the database is
/// under without re-keying it, and leave the next unlock unable to open it.
pub(crate) fn key_step(dir: &Path, kdf: &str) -> Result<KeyStep> {
    let held = std::fs::read_to_string(dir.join(storage::KDF_SIDECAR_FILE))?;
    let local = KdfParams::from_json(&held)?.gen();
    let offered = KdfParams::from_json(kdf)?.gen();
    match offered.cmp(&local) {
        std::cmp::Ordering::Greater => Ok(KeyStep::Rotate),
        std::cmp::Ordering::Equal if kdf == held => Ok(KeyStep::Repair),
        std::cmp::Ordering::Equal => Err(Error::Other(format!(
            "the Rowel Server lists another key at this lockbox's generation ({local}); the \
             local key is kept"
        ))),
        std::cmp::Ordering::Less => Err(Error::Other(format!(
            "the Rowel Server lists an older key for this lockbox (generation {offered}, where \
             this device is at {local}); the local key is kept"
        ))),
    }
}

/// Whether `master` is the key the database in `dir` is under, before it is
/// written anywhere a later unlock would trust it: a repaired sidecar that
/// does not open the database is a lockbox that no longer opens. A database
/// that is not there is refused too, rather than opened into being. Blocking.
pub(crate) fn check_opens(dir: &Path, master: &[u8]) -> Result<()> {
    let path = dir.join(storage::DB_FILE);
    if !path.exists() {
        return Err(Error::Other(
            "this lockbox has no database here to check the key against".into(),
        ));
    }
    let key = VaultKey::from_material(Zeroizing::new(master.to_vec()), true);
    match SqliteStore::open(&path, &*key.sqlcipher_key()) {
        Ok(_) => Ok(()),
        Err(StoreError::WrongKey) => Err(Error::Other(
            "the key the Rowel Server handed over does not open this lockbox; the local key is \
             kept"
                .into(),
        )),
        Err(e) => Err(store_err(e)),
    }
}

/// Workspace `id` of the registry at `root`, which has to be an org lockbox:
/// the record says which server it came from.
pub(crate) fn org_workspace(root: &Path, id: &str) -> Result<Workspace> {
    let workspace = Registry::load(root)
        .workspaces
        .into_iter()
        .find(|w| w.id == id)
        .ok_or(Error::NotFound)?;
    if workspace.server.is_none() {
        return Err(Error::Other(
            "this lockbox did not come from a Rowel Server".into(),
        ));
    }
    Ok(workspace)
}

/// The workspace on this device holding lockbox `lockbox_id` of the server
/// at `url`, if one does.
pub(crate) fn workspace_holding(
    registry: &Registry,
    url: &str,
    lockbox_id: &str,
) -> Option<String> {
    registry
        .workspaces
        .iter()
        .find(|w| w.server.as_deref() == Some(url) && w.vault_id.as_deref() == Some(lockbox_id))
        .map(|w| w.id.clone())
}

/// Who holds the open workspace's key when it is an org lockbox, as a
/// sentence names them ([`server_phrase`]); `None` for a workspace whose key
/// a password derives. For the refusals that have to say why a password is
/// no use here.
pub(crate) fn held_by_server(app: &AppHandle) -> Result<Option<String>> {
    let Some(json) = storage::read_kdf_sidecar(app)? else {
        return Ok(None);
    };
    if !KdfParams::from_json(&json)?.is_external() {
        return Ok(None);
    }
    let root = storage::root_dir(app)?;
    let id = workspace::active_id(app);
    let server = Registry::load(&root)
        .workspaces
        .into_iter()
        .find(|w| w.id == id)
        .and_then(|w| w.server);
    Ok(Some(server_phrase(app, server.as_deref())))
}

/// The company whose server `url` is, as a sentence names it: "Acme's Rowel
/// Server" when the install's connection to it opens — the tenant's name is
/// in that sealed record, nowhere else — and the server by its host when it
/// does not (the vault is locked, or the server was forgotten).
pub(crate) fn server_phrase(app: &AppHandle, url: Option<&str>) -> String {
    let Some(url) = url else {
        return "a Rowel Server".into();
    };
    if let Ok((root, app_key)) = connections::app_root_and_key(app) {
        if let Ok(Some(connection)) = connections::find(&root, &app_key, url) {
            return format!("{}'s Rowel Server", connection.tenant_name);
        }
    }
    let host = url::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(String::from))
        .unwrap_or_else(|| url.to_string());
    format!("the Rowel Server at {host}")
}

/// The master of `lockbox` as the server wrapped it to this device, opened
/// with the enrollment key, beside the descriptor that names it. Refused
/// when the listing carries no wrapped master — the server hands one to
/// approved devices alone — and when the descriptor is not a server-held
/// master's, which no org lockbox's ever is.
pub(crate) fn unseal(app: &AppHandle, lockbox: &Lockbox) -> Result<(Zeroizing<Vec<u8>>, String)> {
    let Some(wrapped) = lockbox.wrapped_master.as_deref() else {
        return Err(Error::Other(format!(
            "this device is not approved at the Rowel Server yet, so it was handed no key for {}",
            lockbox.name
        )));
    };
    if !KdfParams::from_json(&lockbox.kdf)?.is_external() {
        return Err(Error::Other(format!(
            "the Rowel Server describes {}'s key as one a password derives, which an org \
             lockbox's never is",
            lockbox.name
        )));
    }
    let master = enrollment::load_or_create_for(app)?.unseal_master(wrapped)?;
    Ok((master, lockbox.kdf.clone()))
}

/// The lockbox's files in `dir`, as a device that holds it: the server's
/// descriptor as the KDF sidecar, the database keyed by `master`, the vault
/// id the server's lockbox id, the name the server's, and a token file that
/// names `server` as the account the pack syncs through. The key comes back
/// for the ring; the store is closed, since the lockbox is added beside the
/// open workspace rather than switched to. Blocking.
pub(crate) fn install(
    dir: &Path,
    master: Zeroizing<Vec<u8>>,
    kdf: &str,
    lockbox_id: &str,
    name: &str,
    server: &str,
    now_ms: i64,
) -> Result<VaultKey> {
    crate::store::create_private_dir(dir)?;
    // The sidecar first, as `session::create_vault` orders them: a database
    // must never exist without the descriptor needed to open it. Verbatim,
    // because every device and every pack header carry the server's exact
    // string and `engine::decode_pack` compares them byte for byte.
    storage::atomic_write_file(&dir.join(storage::KDF_SIDECAR_FILE), kdf)?;
    let key = VaultKey::from_material(master, true);
    let store =
        SqliteStore::open(&dir.join(storage::DB_FILE), &*key.sqlcipher_key()).map_err(store_err)?;
    crate::session::record_kdf_meta(&store, &KdfParams::from_json(kdf)?)?;
    // The server's id is the vault id: the pack's name in the account is made
    // from it, so the first run addresses that pack rather than minting an id
    // and publishing a second one.
    identity::adopt_vault_id(&store, lockbox_id).map_err(store_err)?;
    // Named as the server names it, stamped now: the server's name is the
    // admins', and outranks one a member's device stamped earlier.
    identity::set_vault_name(&store, name, now_ms).map_err(store_err)?;
    sync::persist_tokens_in(
        dir,
        &key.cryptor(),
        &Tokens {
            provider: Provider::RowelServer,
            account: Some(server.to_string()),
            ..Default::default()
        },
    )?;
    drop(store);
    Ok(key)
}

/// The registry record of a lockbox [`install`] just wrote: not active (the
/// user stays in the workspace they are in), agents off as every lockbox
/// starts, and the server named so the sync adapter can find the session.
pub(crate) fn record(id: &str, name: &str, lockbox_id: &str, server: &str) -> Workspace {
    Workspace {
        id: id.to_string(),
        name: Some(name.to_string()),
        vault_id: Some(lockbox_id.to_string()),
        item_count: None,
        color: None,
        agents: false,
        server: Some(server.to_string()),
    }
}

/// A closed org lockbox moved from the master `held` (the ring's copy) onto
/// the rotated `master` under the server's new descriptor `kdf`: the re-key
/// saga on its own files, the epoch it leaves retired so packs still under
/// it open, and its token file re-sealed under the new key — the second half
/// of `password_sync::adopt_one`, with the key handed over instead of proved.
/// Blocking.
pub(crate) fn rekey_closed(
    dir: &Path,
    workspace_id: &str,
    held: Zeroizing<Vec<u8>>,
    master: Zeroizing<Vec<u8>>,
    kdf: &str,
) -> Result<()> {
    // As an unlock would, before the descriptor is read.
    auth::recover_interrupted_rekey_in(dir)?;
    let old_key = VaultKey::from_material(held, true);
    // Read before the change commits, as `rekey_one` reads it: a token file
    // that will not open under the key that opens the vault is found while
    // nothing has been touched.
    let tokens = sync::read_tokens_in(dir, &old_key.cryptor())?;
    let new_key = VaultKey::from_material(master, true);
    let new_cryptor = new_key.cryptor();
    auth::rekey_workspace(dir, workspace_id, old_key, new_key, kdf)?;
    if let Some(tokens) = tokens {
        if let Err(e) = sync::persist_tokens_in(dir, &new_cryptor, &tokens) {
            log::warn!("could not re-seal this lockbox's server record under its new key: {e}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::retired;

    const NOW: i64 = 1_700_000_000_000;

    fn master(byte: u8) -> Zeroizing<Vec<u8>> {
        Zeroizing::new(vec![byte; 32])
    }

    fn descriptor(gen: u32) -> String {
        KdfParams::external(format!("kid-{gen}"), gen)
            .to_json()
            .unwrap()
    }

    // One generation is never taken for another: later rotates, the same
    // descriptor repairs, and an older one — or another key at the same
    // generation — is refused with the local key kept.
    #[test]
    fn a_listed_key_is_a_rotation_a_repair_or_refused() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("ws");
        let gen2 = descriptor(2);
        install(
            &dir,
            master(2),
            &gen2,
            "lb-1",
            "Deploy",
            "https://a.example",
            NOW,
        )
        .unwrap();

        assert_eq!(key_step(&dir, &descriptor(3)).unwrap(), KeyStep::Rotate);
        assert_eq!(key_step(&dir, &gen2).unwrap(), KeyStep::Repair);
        let other_kid = KdfParams::external("kid-other", 2).to_json().unwrap();
        match key_step(&dir, &other_kid) {
            Err(Error::Other(why)) => assert!(why.contains("another key"), "{why}"),
            other => panic!("another key at the same generation is refused, got {other:?}"),
        }
        match key_step(&dir, &descriptor(1)) {
            Err(Error::Other(why)) => assert!(why.contains("older key"), "{why}"),
            other => panic!("an older generation is refused, got {other:?}"),
        }
        assert_eq!(local_gen(&dir).unwrap(), 2, "nothing was written");
    }

    // A repaired key is checked against the database before anything trusts
    // it, and a database that is not there is not opened into being.
    #[test]
    fn a_repair_is_checked_against_the_database() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("ws");
        install(
            &dir,
            master(2),
            &descriptor(2),
            "lb-1",
            "Deploy",
            "https://a.example",
            NOW,
        )
        .unwrap();
        assert!(check_opens(&dir, &master(2)).is_ok());
        match check_opens(&dir, &master(9)) {
            Err(Error::Other(why)) => assert!(why.contains("does not open"), "{why}"),
            other => panic!("another master is refused, got {other:?}"),
        }
        let empty = root.path().join("empty");
        std::fs::create_dir(&empty).unwrap();
        assert!(check_opens(&empty, &master(2)).is_err());
        assert!(!empty.join(storage::DB_FILE).exists());
    }

    // The files an added org lockbox is made of, as the next unlock and the
    // first sync read them.
    #[test]
    fn an_installed_lockbox_carries_the_servers_descriptor_id_name_and_account() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("ws");
        let gen1 = descriptor(1);
        let key = install(
            &dir,
            master(1),
            &gen1,
            "lb-1",
            "Deploy",
            "https://rowel.acme.example",
            NOW,
        )
        .unwrap();

        assert_eq!(
            std::fs::read_to_string(dir.join(storage::KDF_SIDECAR_FILE)).unwrap(),
            gen1,
            "verbatim"
        );
        assert!(is_org_lockbox_dir(&dir));
        assert_eq!(local_gen(&dir).unwrap(), 1);
        let store = SqliteStore::open(&dir.join(storage::DB_FILE), &*key.sqlcipher_key()).unwrap();
        assert_eq!(identity::vault_id(&store).unwrap().as_deref(), Some("lb-1"));
        assert_eq!(
            identity::vault_name(&store).unwrap(),
            (Some("Deploy".into()), NOW)
        );
        let tokens = sync::read_tokens_in(&dir, &key.cryptor()).unwrap().unwrap();
        assert_eq!(tokens.provider, Provider::RowelServer);
        assert_eq!(
            tokens.account.as_deref(),
            Some("https://rowel.acme.example")
        );

        assert!(
            !is_org_lockbox_dir(root.path()),
            "no sidecar is not an org lockbox"
        );
    }

    // A rotation reaches a closed copy: it opens under the new master alone,
    // keeps the old epoch for the packs still under it, and its token file
    // comes along.
    #[test]
    fn a_closed_lockbox_is_rekeyed_onto_the_rotated_master() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("ws");
        let (gen1, gen2) = (descriptor(1), descriptor(2));
        drop(
            install(
                &dir,
                master(1),
                &gen1,
                "lb-1",
                "Deploy",
                "https://a.example",
                NOW,
            )
            .unwrap(),
        );

        rekey_closed(&dir, "ws", master(1), master(2), &gen2).unwrap();

        let new_key = VaultKey::from_material(master(2), true);
        assert!(SqliteStore::open(&dir.join(storage::DB_FILE), &*new_key.sqlcipher_key()).is_ok());
        assert!(SqliteStore::open(
            &dir.join(storage::DB_FILE),
            &*VaultKey::from_material(master(1), true).sqlcipher_key()
        )
        .is_err());
        assert_eq!(local_gen(&dir).unwrap(), 2);
        assert_eq!(
            retired::load(&dir, &new_key, "ws")
                .iter()
                .map(|r| r.kdf.clone())
                .collect::<Vec<_>>(),
            [gen1]
        );
        let tokens = sync::read_tokens_in(&dir, &new_key.cryptor())
            .unwrap()
            .unwrap();
        assert_eq!(tokens.account.as_deref(), Some("https://a.example"));

        // The wrong old key changes nothing: the token file, read before the
        // saga as `rekey_one` reads it, does not open under it.
        assert!(rekey_closed(&dir, "ws", master(9), master(3), &descriptor(3)).is_err());
        assert_eq!(local_gen(&dir).unwrap(), 2);
        assert!(SqliteStore::open(&dir.join(storage::DB_FILE), &*new_key.sqlcipher_key()).is_ok());
    }

    #[test]
    fn the_registry_says_which_workspace_holds_a_servers_lockbox() {
        let registry = Registry {
            active: workspace::PRIMARY_ID.into(),
            workspaces: vec![
                record("w1", "Deploy", "lb-1", "https://a.example"),
                Workspace {
                    server: None,
                    ..record("w2", "Mine", "lb-1", "https://a.example")
                },
            ],
        };
        assert_eq!(
            workspace_holding(&registry, "https://a.example", "lb-1").as_deref(),
            Some("w1")
        );
        assert!(workspace_holding(&registry, "https://b.example", "lb-1").is_none());
        assert!(workspace_holding(&registry, "https://a.example", "lb-2").is_none());
        let root = tempfile::tempdir().unwrap();
        registry.save(root.path()).unwrap();
        assert_eq!(org_workspace(root.path(), "w1").unwrap().id, "w1");
        assert!(
            org_workspace(root.path(), "w2").is_err(),
            "not an org lockbox"
        );
        assert!(matches!(
            org_workspace(root.path(), "w3"),
            Err(Error::NotFound)
        ));
    }
}
