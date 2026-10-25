//! Two devices, one account: a master-password change made on one of them
//! reaches the other (design section 9, "Commands").
//!
//! The commands need an `AppHandle`, which this harness has no way to build,
//! so the same sequence runs one layer down, on the pieces they are made of:
//! real workspace directories (database, sidecar, retired list), the re-key
//! saga (`auth::rekey`, `auth::rekey_workspace`), the engine (`engine::sync`
//! over `PackRemote`), the adopt's proof (`key_for_pack`), and the restore
//! (`restore::restore_at`) — all over one `FakeStore`.
//!
//! The same two devices are what the audit log's sync is proved on (design
//! section 09, "Sync"): each writes its own chain, and a run carries both.

use std::fs;
use std::path::{Path, PathBuf};

use tempfile::TempDir;
use zeroize::Zeroizing;

use super::key_for_pack;
use crate::auth::{self, LockoutState, RekeyPaths};
use crate::crypto::{self, KdfParams, VaultKey};
use crate::error::{Error, Result};
use crate::models::Entry;
use crate::retired;
use crate::session::store_err;
use crate::storage;
use crate::store::{identity, migrate, SqliteStore, VaultStore};
use crate::sync::engine::{self, LocalVault, Reclaim, Snapshot, SyncOutcome};
use crate::sync::store::fake::FakeStore;
use crate::sync::{layout, pack, restore, PackRemote, Provider};

const NOW: i64 = 1_700_000_000_000;
const OLD: &str = "old password";
const NEW: &str = "new password";

// Cheap Argon2id at a given generation, so the suite stays fast; the app's
// own re-keys use the default costs.
fn params(salt: u8, gen: u32) -> String {
    KdfParams::Argon2id {
        m_cost: 256,
        t_cost: 1,
        p_cost: 1,
        salt: base64_salt(salt),
        gen,
    }
    .to_json()
    .unwrap()
}

fn base64_salt(salt: u8) -> String {
    use base64::{engine::general_purpose::STANDARD, Engine};
    STANDARD.encode([salt; 32])
}

fn derive(password: &str, params_json: &str) -> VaultKey {
    VaultKey::Argon2 {
        master: crypto::derive(
            password.as_bytes(),
            &KdfParams::from_json(params_json).unwrap(),
        )
        .unwrap(),
    }
}

fn copy(key: &VaultKey) -> VaultKey {
    VaultKey::from_material(Zeroizing::new(key.biometric_material().to_vec()), true)
}

/// One lockbox on one device: its directory, the key its session holds, and
/// its open store.
struct Device {
    dir: TempDir,
    id: &'static str,
    vault: String,
    key: VaultKey,
    store: Option<SqliteStore>,
}

impl Device {
    /// A new lockbox under `password` and descriptor `params_json`.
    fn create(id: &'static str, vault: &str, password: &str, params_json: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        storage::atomic_write_file(&dir.path().join(storage::KDF_SIDECAR_FILE), params_json)
            .unwrap();
        let key = derive(password, params_json);
        let store =
            SqliteStore::open(&dir.path().join(storage::DB_FILE), &*key.sqlcipher_key()).unwrap();
        // The id its pack is addressed by, as a first run settles it.
        identity::adopt_vault_id(&store, vault).unwrap();
        Self {
            dir,
            id,
            vault: vault.into(),
            key,
            store: Some(store),
        }
    }

    /// A device joining the lockbox from the account, as a restore does.
    fn restore(id: &'static str, vault: &str, account: &FakeStore, password: &str) -> Result<Self> {
        let dir = tempfile::tempdir().unwrap();
        let bytes = account.bytes(&layout::vault_file_name(vault)).unwrap();
        let (key, store) = restore::restore_at(
            &dir.path().join(storage::DB_FILE),
            &dir.path().join(storage::KDF_SIDECAR_FILE),
            &bytes,
            password,
        )?;
        // As every restore stamps it, off the pack's file name.
        identity::adopt_vault_id(&store, vault).unwrap();
        Ok(Self {
            dir,
            id,
            vault: vault.into(),
            key,
            store: Some(store),
        })
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn store(&self) -> &SqliteStore {
        self.store.as_ref().unwrap()
    }

    fn sidecar(&self) -> String {
        fs::read_to_string(self.path().join(storage::KDF_SIDECAR_FILE)).unwrap()
    }

    fn gen(&self) -> u32 {
        KdfParams::from_json(&self.sidecar()).unwrap().gen()
    }

    fn retired(&self) -> Vec<String> {
        retired::load(self.path(), &self.key, self.id)
            .into_iter()
            .map(|r| r.kdf)
            .collect()
    }

    fn scratch(&self) -> PathBuf {
        storage::sync_scratch_dir_in(self.path())
    }

    fn write(&self, id: &str, title: &str) {
        let entry = Entry {
            id: id.into(),
            title: title.into(),
            ..Default::default()
        };
        let payload = self.key.payload_cipher().seal(&entry).unwrap();
        self.store()
            .upsert(&migrate::build_record(&entry, payload).unwrap())
            .unwrap();
    }

    fn title(&self, id: &str) -> Option<String> {
        let record = self.store().get(id).unwrap()?;
        Some(
            self.key
                .payload_cipher()
                .unseal(id, &record.payload)
                .unwrap()
                .title,
        )
    }

    fn digest(&self) -> [u8; 32] {
        self.store().state_digest().unwrap()
    }

    /// One run against the account, as `sync::run` makes it: the marker keys
    /// of the current epoch and of every retired one.
    fn sync(&self, account: &FakeStore) -> Result<SyncOutcome> {
        self.sync_at(account, NOW)
    }

    /// [`Device::sync`] as of `now_ms`, for what a run a year on reclaims.
    fn sync_at(&self, account: &FakeStore, now_ms: i64) -> Result<SyncOutcome> {
        let mut marker_keys = vec![self.key.delete_marker_key()];
        marker_keys.extend(
            retired::load(self.path(), &self.key, self.id)
                .iter()
                .map(|r| r.key().delete_marker_key()),
        );
        let remote = PackRemote::new(account, self.vault.clone(), marker_keys);
        engine::sync(&remote, self, now_ms, || Ok(()))
    }

    /// The saga a password change (or an adopt) runs on the open workspace —
    /// `rekey_open_workspace` minus the session around it.
    fn rekey(&mut self, new_key: VaultKey, params_json: &str) {
        let store = self.store.take().unwrap();
        let (key, store) = auth::rekey(
            store,
            copy(&self.key),
            new_key,
            params_json,
            &RekeyPaths::in_dir(self.path()),
            self.id,
        )
        .unwrap_or_else(|rollback| panic!("the re-key rolled back: {}", rollback.error));
        self.key = key;
        self.store = Some(store);
    }

    /// The account's pack, proved and keyed under `password` — the adopt's
    /// first half (`key_for_pack`), fetched as the command fetches it.
    fn key_for_account(
        &self,
        account: &FakeStore,
        password: &str,
    ) -> Result<Option<(String, VaultKey)>> {
        let bytes = account
            .bytes(&layout::vault_file_name(&self.vault))
            .unwrap();
        key_for_pack(self.path(), self.id, &self.key, &bytes, password)
    }
}

impl LocalVault for Device {
    fn decode(&self, pack_bytes: &[u8]) -> Result<Snapshot> {
        engine::decode_pack(
            pack_bytes,
            &self.sidecar(),
            &self.key,
            &retired::load(self.path(), &self.key, self.id),
            &self.scratch(),
            Provider::GoogleDrive,
        )
    }
    fn merge(&self, incoming: &Snapshot) -> Result<usize> {
        self.store()
            .merge(&incoming.records, &incoming.attachments, &incoming.audit)
            .map_err(store_err)
    }
    fn digest(&self) -> Result<[u8; 32]> {
        self.store().state_digest().map_err(store_err)
    }
    fn pack(&self, reclaim: Reclaim) -> Result<Vec<u8>> {
        engine::reclaim(self.store(), reclaim, Some(self.id))?;
        Ok(pack::pack_store(
            self.store(),
            &*self.key.sqlcipher_key(),
            &self.sidecar(),
            &self.scratch(),
        )?)
    }
    fn note_push(&self, _revision: &str, _at_ms: i64) -> Result<()> {
        Ok(())
    }
    fn vault_id(&self) -> Result<Option<String>> {
        identity::vault_id(self.store()).map_err(store_err)
    }
    fn adopt_vault_id(&self, id: &str) -> Result<()> {
        identity::adopt_vault_id(self.store(), id).map_err(store_err)
    }
    fn name(&self) -> Result<(Option<String>, i64)> {
        identity::vault_name(self.store()).map_err(store_err)
    }
    fn adopt_name(&self, name: &str, at_ms: i64) -> Result<()> {
        identity::set_vault_name(self.store(), name, at_ms).map_err(store_err)
    }
}

fn header(account: &FakeStore, vault: &str) -> String {
    pack::unpack(&account.bytes(&layout::vault_file_name(vault)).unwrap())
        .unwrap()
        .kdf_params_json
}

#[test]
fn a_password_change_on_one_device_reaches_the_other() {
    let account = FakeStore::default();
    let gen0 = params(1, 0);

    // 1. A and B hold the same lockbox and have synced.
    let mut a = Device::create("a", "v1", OLD, &gen0);
    a.write("1", "first");
    assert!(a.sync(&account).unwrap().pushed);
    let gen0_pack = account.bytes(&layout::vault_file_name("v1")).unwrap();
    let mut b = Device::restore("b", "v1", &account, OLD).unwrap();
    assert!(!b.sync(&account).unwrap().pushed);
    assert_eq!(a.digest(), b.digest());

    // 2. A changes the password: generation 1, the old epoch retired, and the
    //    run after the change re-seals the account's pack under it.
    let gen1 = params(2, 1);
    a.rekey(derive(NEW, &gen1), &gen1);
    assert_eq!(a.gen(), 1);
    assert_eq!(a.retired(), std::slice::from_ref(&gen0));
    let outcome = a.sync(&account).unwrap();
    assert!(outcome.pushed && outcome.rekeyed_remote);
    assert_eq!(header(&account, "v1"), gen1);

    // 3. B edits, and its run stops: the password changed elsewhere. The edit
    //    is intact and nothing went up.
    b.write("2", "made on b");
    let before = account.bytes(&layout::vault_file_name("v1"));
    assert!(matches!(
        b.sync(&account),
        Err(Error::PasswordChangedRemotely)
    ));
    assert_eq!(account.bytes(&layout::vault_file_name("v1")), before);
    assert_eq!(b.title("2").as_deref(), Some("made on b"));

    // 4. A wrong password proves nothing, changes nothing, and counts as a
    //    failed attempt against B's backoff.
    assert!(matches!(
        b.key_for_account(&account, "not it"),
        Err(Error::InvalidPassword)
    ));
    let (penalized, _) = auth::penalize(LockoutState::load_in(b.path()).unwrap(), NOW);
    penalized.save_in(b.path()).unwrap();
    assert_ne!(
        LockoutState::load_in(b.path()).unwrap(),
        LockoutState::default()
    );
    assert_eq!(b.sidecar(), gen0);
    assert!(b.retired().is_empty());
    assert_eq!(b.title("2").as_deref(), Some("made on b"));

    // 5. The new password: B takes the account's descriptor byte for byte,
    //    retires its own, and its next run pushes its edit; A's has it.
    let (adopted, key) = b.key_for_account(&account, NEW).unwrap().unwrap();
    assert_eq!(adopted, gen1);
    b.rekey(key, &adopted);
    assert_eq!(b.sidecar(), a.sidecar());
    assert_eq!(b.retired(), std::slice::from_ref(&gen0));
    assert!(SqliteStore::open(
        &b.path().join(storage::DB_FILE),
        &*derive(NEW, &gen1).sqlcipher_key()
    )
    .is_ok());
    assert!(b.sync(&account).unwrap().pushed);
    // B's edit, and possibly B's re-sealed copy of entry 1: each side sealed
    // it afresh under the same key (new nonce, same stamp), and the tie-break
    // on the row hash picks one copy for both.
    assert!(a.sync(&account).unwrap().merged >= 1);
    assert_eq!(a.title("2").as_deref(), Some("made on b"));
    assert_eq!(a.digest(), b.digest());
    // Adopting again is a no-op: the account is on B's descriptor now.
    assert!(b.key_for_account(&account, NEW).unwrap().is_none());

    // 6. A device still on generation 0 puts its copy back, with an entry of
    //    its own. A and B merge it with their retired key and re-seal it.
    let old_account = FakeStore::default();
    old_account.put(&layout::vault_file_name("v1"), &gen0_pack);
    let c = Device::restore("c", "v1", &old_account, OLD).unwrap();
    c.write("3", "from the old epoch");
    let c_pack = pack::pack_store(
        c.store(),
        &*c.key.sqlcipher_key(),
        &c.sidecar(),
        &c.scratch(),
    )
    .unwrap();
    account.put(&layout::vault_file_name("v1"), &c_pack);
    let outcome = a.sync(&account).unwrap();
    assert!(outcome.pushed && outcome.rekeyed_remote);
    assert_eq!(a.title("3").as_deref(), Some("from the old epoch"));
    assert_eq!(header(&account, "v1"), gen1);
    b.sync(&account).unwrap();
    assert_eq!(b.title("3").as_deref(), Some("from the old epoch"));
    assert_eq!(a.digest(), b.digest());
    // ...and C, on the retired epoch, is told the password changed.
    assert!(matches!(
        c.sync(&account),
        Err(Error::PasswordChangedRemotely)
    ));

    // 7. A fresh device restores generation 1 verbatim, under the new password
    //    only.
    assert!(matches!(
        Device::restore("d", "v1", &account, OLD),
        Err(Error::InvalidPassword)
    ));
    let d = Device::restore("d", "v1", &account, NEW).unwrap();
    assert_eq!(d.sidecar(), gen1);
    assert_eq!(d.title("2").as_deref(), Some("made on b"));
}

// A device that retired an epoch and lost its key (here: never held it) finds
// the account under that epoch: it neither opens nor overwrites it, until the
// user confirms the replace.
#[test]
fn a_pack_under_an_epoch_this_device_cannot_open_waits_for_the_replace() {
    let account = FakeStore::default();
    let gen0 = params(1, 0);
    let gen1 = params(2, 1);
    let stale = Device::create("s", "v1", OLD, &gen0);
    stale.write("1", "only in the account");
    stale.sync(&account).unwrap();
    let current = Device::create("c", "v1", NEW, &gen1);
    current.write("2", "only here");

    assert!(matches!(
        current.sync(&account),
        Err(Error::VaultUnderRetiredPassword)
    ));
    assert_eq!(header(&account, "v1"), gen0);

    let remote = PackRemote::new(&account, "v1".into(), vec![current.key.delete_marker_key()]);
    engine::replace(&remote, &current, NOW).unwrap();
    assert_eq!(header(&account, "v1"), gen1);
    assert!(!current.sync(&account).unwrap().pushed);
    // The stale device is now the one asked for the new password.
    assert!(matches!(
        stale.sync(&account),
        Err(Error::PasswordChangedRemotely)
    ));
}

// A lockbox that is not open, synced as a change or an adopt syncs it
// (`sync::run_workspace`): a `WorkspaceVault` over its directory, and the run's
// account half over the shared `FakeStore`.
fn sync_closed(dir: &Path, id: &str, key: VaultKey, account: &FakeStore) -> Result<SyncOutcome> {
    let marker_key = key.delete_marker_key();
    let local =
        engine::WorkspaceVault::open(dir, id, key, Provider::GoogleDrive, Some(id.to_string()))?;
    let (ran, _) = crate::sync::run_closed(account, &local, marker_key, NOW);
    ran.map(|(outcome, _)| outcome)
}

// The password change touches every lockbox, so it syncs every lockbox: after
// A's change both packs in the account are on the new password without A ever
// opening its second lockbox, and B answers one prompt for both — its own edit
// in the second one, made before it adopted, reaching A.
#[test]
fn a_password_change_reseals_every_lockbox_and_one_adopt_follows_it() {
    let account = FakeStore::default();

    // Two lockboxes on one password, each with its own salt, on both devices.
    let (one0, two0) = (params(7, 0), params(8, 0));
    let mut a1 = Device::create("a1", "v1", OLD, &one0);
    a1.write("1", "in lockbox one");
    a1.sync(&account).unwrap();
    let mut a2 = Device::create("a2", "v2", OLD, &two0);
    a2.write("x", "in lockbox two");
    a2.sync(&account).unwrap();
    let mut b1 = Device::restore("b1", "v1", &account, OLD).unwrap();
    let mut b2 = Device::restore("b2", "v2", &account, OLD).unwrap();

    // B edits its second lockbox, and has not synced it when A changes.
    b2.write("y", "made on b in lockbox two");

    // A's change: the open lockbox through the saga, the one beside it on its
    // own files (as `rekey_one` does), closed from here on.
    let (one1, two1) = (params(9, 1), params(10, 1));
    a1.rekey(derive(NEW, &one1), &one1);
    a2.store = None;
    auth::rekey_workspace(a2.path(), a2.id, copy(&a2.key), derive(NEW, &two1), &two1).unwrap();
    a2.key = derive(NEW, &two1);

    // The runs after it: the open lockbox's own, and the one beside it, which
    // is synced without being opened as the session's.
    assert!(a1.sync(&account).unwrap().rekeyed_remote);
    let outcome = sync_closed(a2.path(), a2.id, copy(&a2.key), &account).unwrap();
    assert!(outcome.pushed && outcome.rekeyed_remote);
    assert!(a2.store.is_none(), "lockbox two was never opened on A");
    assert_eq!(header(&account, "v1"), one1);
    assert_eq!(header(&account, "v2"), two1);

    // B: one prompt, for the open lockbox, and the one beside it follows.
    assert!(matches!(
        b1.sync(&account),
        Err(Error::PasswordChangedRemotely)
    ));
    let (h1, k1) = b1.key_for_account(&account, NEW).unwrap().unwrap();
    b1.rekey(k1, &h1);
    let (h2, k2) = b2.key_for_account(&account, NEW).unwrap().unwrap();
    b2.store = None;
    auth::rekey_workspace(b2.path(), b2.id, copy(&b2.key), copy(&k2), &h2).unwrap();
    b2.key = k2;
    assert_eq!(b1.sidecar(), one1);
    assert_eq!(b2.sidecar(), two1);

    // The adopt's runs: B's edit in lockbox two goes up with its beside run.
    b1.sync(&account).unwrap();
    assert!(
        sync_closed(b2.path(), b2.id, copy(&b2.key), &account)
            .unwrap()
            .pushed
    );

    // ...and A's next run of lockbox two, still never opened as a session, has
    // it. Opened afterwards to look, on the new password.
    assert!(
        sync_closed(a2.path(), a2.id, copy(&a2.key), &account)
            .unwrap()
            .merged
            >= 1
    );
    a2.store = Some(
        SqliteStore::open(&a2.path().join(storage::DB_FILE), &*a2.key.sqlcipher_key()).unwrap(),
    );
    assert_eq!(a2.title("y").as_deref(), Some("made on b in lockbox two"));
    b2.store = Some(
        SqliteStore::open(&b2.path().join(storage::DB_FILE), &*b2.key.sqlcipher_key()).unwrap(),
    );
    assert_eq!(a2.digest(), b2.digest());
}

// The lockboxes beside the one adopted: one on the same password follows (its
// own pack, its own salt), one on a password of its own is left alone.
#[test]
fn lockboxes_beside_follow_only_on_the_same_password() {
    let account = FakeStore::default();

    // A second lockbox on the shared password, synced to both devices; A moves
    // it to the new password as `rekey_one` does (its own next generation).
    let shared0 = params(3, 0);
    let mut a2 = Device::create("a2", "v2", OLD, &shared0);
    a2.write("x", "shared");
    a2.sync(&account).unwrap();
    let mut b2 = Device::restore("b2", "v2", &account, OLD).unwrap();
    let shared1 = params(4, 1);
    a2.rekey(derive(NEW, &shared1), &shared1);
    a2.sync(&account).unwrap();

    // A third on a password of its own, which changed on A to another one.
    let own0 = params(5, 0);
    let a3 = Device::create("a3", "v3", "own password", &own0);
    a3.sync(&account).unwrap();
    let b3 = Device::restore("b3", "v3", &account, "own password").unwrap();
    let own1 = params(6, 1);
    let mut a3 = a3;
    a3.rekey(derive("another own password", &own1), &own1);
    a3.sync(&account).unwrap();

    // B's adopt, beside: what `adopt_one` does after its fetch, on a lockbox
    // that is not open (so its store is closed).
    let (h2, k2) = b2.key_for_account(&account, NEW).unwrap().unwrap();
    b2.store = None;
    auth::rekey_workspace(b2.path(), b2.id, copy(&b2.key), k2, &h2).unwrap();
    assert_eq!(
        fs::read_to_string(b2.path().join(storage::KDF_SIDECAR_FILE)).unwrap(),
        shared1
    );
    assert_eq!(
        retired::load(b2.path(), &derive(NEW, &shared1), "b2")[0].kdf,
        shared0
    );

    assert!(matches!(
        b3.key_for_account(&account, NEW),
        Err(Error::InvalidPassword)
    ));
    assert_eq!(b3.sidecar(), own0, "left on its own password");
}

// --- the audit log ------------------------------------------------------------
//
// Each device writes its own chain under its own id. A run carries every chain
// a device holds, the merge takes them as a union, and each chain verifies on
// whichever device holds it. Desktop only: the broker that writes the rows is
// not linked on iOS (see `Cargo.toml`), and a device there holds other
// devices' chains without a pen of its own.
#[cfg(desktop)]
mod audit_log {
    use super::*;
    use crate::store::AuditRecord;
    use rowel_broker::audit::{self, ActorKind, Break, Draft, Event, Reason, Writer};

    impl Device {
        /// One row in this device's own chain, as an agent's use of a login writes
        /// it: the device's id is the writer id, as `storage::install_id` is in
        /// the app.
        fn record(&self, target: &str) -> AuditRecord {
            self.record_at(target, NOW)
        }

        /// [`Device::record`] stamped at `ts_ms`.
        fn record_at(&self, target: &str, ts_ms: i64) -> AuditRecord {
            Writer::new(self.id, self.store())
                .record(
                    Draft {
                        item_id: Some("1".into()),
                        target: Some(target.into()),
                        ..Draft::new(ActorKind::Agent, Event::Used)
                    },
                    ts_ms,
                )
                .unwrap()
        }

        /// `writer`'s chain as this device holds it.
        fn chain(&self, writer: &str) -> Vec<AuditRecord> {
            self.store().audit_rows(writer).unwrap()
        }
    }

    // A and B, holding one lockbox, converged, with no log yet.
    fn two_synced_devices(account: &FakeStore) -> (Device, Device) {
        let a = Device::create("a", "v1", OLD, &params(11, 0));
        a.write("1", "first");
        a.sync(account).unwrap();
        let b = Device::restore("b", "v1", account, OLD).unwrap();
        assert!(!b.sync(account).unwrap().pushed);
        (a, b)
    }

    #[test]
    fn audit_logs_merge_as_a_union_and_each_chain_verifies_on_both_devices() {
        let account = FakeStore::default();
        let (a, b) = two_synced_devices(&account);

        a.record("https://github.com/login");
        a.record("https://github.com/settings");
        b.record("https://gitlab.com/login");
        b.record("https://gitlab.com/profile");
        b.record("https://gitlab.com/keys");

        assert!(a.sync(&account).unwrap().pushed);
        let outcome = b.sync(&account).unwrap();
        assert!(outcome.pushed, "B's own rows go up beside A's");
        assert_eq!(
            outcome.merged, 0,
            "log rows are not a change to the entry list"
        );
        assert!(
            !a.sync(&account).unwrap().pushed,
            "A took B's rows and matched"
        );

        let union = a.store().audit_all().unwrap();
        assert_eq!(union.len(), 5);
        assert_eq!(union, b.store().audit_all().unwrap());
        assert_eq!(a.digest(), b.digest());
        for device in [&a, &b] {
            for writer in ["a", "b"] {
                assert_eq!(audit::verify(&device.chain(writer)), Ok(()));
            }
        }
        assert_eq!(a.chain("b"), b.chain("b"));
        assert_eq!(b.chain("a"), a.chain("a"));
    }

    // Nothing but the log changed, so a digest blind to it would call this "in
    // sync" and the other device would never learn what this one's agents did.
    #[test]
    fn new_audit_rows_alone_are_enough_to_push_and_reach_the_peer() {
        let account = FakeStore::default();
        let (a, b) = two_synced_devices(&account);
        assert!(!a.sync(&account).unwrap().pushed);

        let row = a.record("https://github.com/login");
        assert!(a.sync(&account).unwrap().pushed);

        let outcome = b.sync(&account).unwrap();
        assert!(!outcome.pushed, "B matched the remote, log included");
        assert_eq!(outcome.merged, 0);
        assert_eq!(b.chain("a"), vec![row]);
        assert_eq!(a.digest(), b.digest());
    }

    // Two installs writing as one writer — an app folder copied to a new Mac
    // carries its install id — each chain a different row at the same seq. The
    // union keeps both sides on both devices, so the digests agree and the runs
    // settle instead of each pushing its own side forever, and verification
    // names the fork at the same seq on both.
    #[test]
    fn a_forked_chain_converges_on_both_sides_and_is_named_on_both_devices() {
        let account = FakeStore::default();
        let (a, b) = two_synced_devices(&account);
        let draft = |target: &str| Draft {
            target: Some(target.into()),
            ..Draft::new(ActorKind::Agent, Event::Used)
        };
        Writer::new("a", a.store())
            .record(draft("https://github.com/login"), NOW)
            .unwrap();
        Writer::new("a", b.store())
            .record(draft("https://gitlab.com/login"), NOW)
            .unwrap();

        assert!(a.sync(&account).unwrap().pushed);
        assert!(
            b.sync(&account).unwrap().pushed,
            "B's side goes up beside A's"
        );
        assert!(
            !a.sync(&account).unwrap().pushed,
            "A took B's side and matched"
        );
        assert!(!b.sync(&account).unwrap().pushed);

        assert_eq!(a.digest(), b.digest());
        assert_eq!(a.chain("a").len(), 2);
        assert_eq!(a.chain("a"), b.chain("a"));
        let fork = Err(Break {
            seq: 1,
            reason: Reason::Fork,
        });
        assert_eq!(audit::verify(&a.chain("a")), fork);
        assert_eq!(audit::verify(&b.chain("a")), fork);
    }

    // A copy of A's row altered behind its unchanged id and hash — how tampering
    // looks — reaches the account through B. The clean device does not take it
    // for the row it holds: it keeps both, the digests agree only once both
    // devices do, and verification on each — and on a fresh restore from the
    // account — names the altered row.
    #[test]
    fn a_row_altered_behind_its_hash_is_kept_beside_the_original_and_named_everywhere() {
        let account = FakeStore::default();
        let (a, b) = two_synced_devices(&account);
        let row = a.record("https://github.com/login");
        assert!(a.sync(&account).unwrap().pushed);
        b.store()
            .audit_insert(&AuditRecord {
                target: Some("https://evil.example/login".into()),
                ..row.clone()
            })
            .unwrap();

        assert!(b.sync(&account).unwrap().pushed, "B holds a row A does not");
        let outcome = a.sync(&account).unwrap();
        assert!(
            !outcome.pushed,
            "A took the copy beside its own and matched"
        );
        assert_eq!(outcome.merged, 0);
        assert!(!b.sync(&account).unwrap().pushed);
        assert_eq!(a.digest(), b.digest());

        let restored = Device::restore("c", "v1", &account, OLD).unwrap();
        let altered = Err(Break {
            seq: row.seq,
            reason: Reason::Hash,
        });
        for device in [&a, &b, &restored] {
            assert_eq!(device.chain("a").len(), 2);
            assert!(device.chain("a").contains(&row));
            assert_eq!(audit::verify(&device.chain("a")), altered);
        }
    }

    // The merge is a union by id and cannot tell a forged row from a real one; that
    // is the verification's job. Here the device holding A's chain appends a row
    // under A's id whose contents were changed after it was hashed — what an
    // altered row looks like once it has travelled — and A, pulling it back,
    // names it by its `seq`.
    #[test]
    fn an_altered_row_arriving_through_sync_breaks_verification_at_its_seq() {
        let account = FakeStore::default();
        let (a, b) = two_synced_devices(&account);
        a.record("https://github.com/login");
        let last = a.record("https://github.com/settings");
        a.sync(&account).unwrap();
        b.sync(&account).unwrap();
        assert_eq!(b.chain("a"), a.chain("a"));

        let mut forged = AuditRecord {
            id: crypto::random_hex_id(),
            seq: last.seq + 1,
            prev_hash: last.hash,
            ..last.clone()
        };
        forged.hash = audit::hash_of(&forged).unwrap();
        forged.target = Some("https://evil.example/login".into());
        b.store().audit_insert(&forged).unwrap();
        assert!(b.sync(&account).unwrap().pushed);

        a.sync(&account).unwrap();
        let chain = a.chain("a");
        assert_eq!(chain.len(), 3, "the union let the row in");
        assert_eq!(
            audit::verify(&chain),
            Err(Break {
                seq: 3,
                reason: Reason::Hash
            })
        );
        assert_eq!(
            audit::verify(&chain[..2]),
            Ok(()),
            "the rows before it stand"
        );
        assert_eq!(audit::verify(&b.chain("a")), audit::verify(&chain));
    }

    // Retention converges. B's push a year on reclaims A's aged rows from B's copy
    // (with no note: B cannot write A's chain); A, pulling that, is left holding
    // rows the account no longer has, pushes, reclaims them itself and notes the
    // cut in its own chain; B pulls the note. From there neither has anything to
    // say, and A's chain — starting at the note — verifies on both.
    #[test]
    fn aged_audit_rows_reclaimed_by_a_peers_push_converge_on_both_devices() {
        const DAY_MS: i64 = 24 * 60 * 60 * 1000;
        const LATER: i64 = NOW + 400 * DAY_MS;
        let account = FakeStore::default();
        let (a, b) = two_synced_devices(&account);
        a.record("https://github.com/login");
        a.record("https://github.com/settings");
        a.sync(&account).unwrap();
        b.sync(&account).unwrap();
        assert_eq!(b.chain("a").len(), 2);

        // A year on, B has a row of its own to push; its pack reclaims A's.
        b.record_at("https://gitlab.com/login", LATER);
        assert!(b.sync_at(&account, LATER).unwrap().pushed);
        assert!(b.chain("a").is_empty());
        assert_eq!(b.chain("b").len(), 1, "B's own row is young, so no note");

        // A still holds its two rows, so it pushes — and reclaims them as it packs.
        assert!(a.sync_at(&account, LATER).unwrap().pushed);
        let note = a.chain("a");
        assert_eq!(note.len(), 1);
        assert_eq!((note[0].seq, note[0].event.as_str()), (3, "pruned_through"));
        assert_eq!(a.chain("b"), b.chain("b"));

        assert!(!b.sync_at(&account, LATER).unwrap().pushed);
        assert!(!a.sync_at(&account, LATER).unwrap().pushed);
        assert_eq!(a.digest(), b.digest());
        assert_eq!(
            a.store().audit_all().unwrap(),
            b.store().audit_all().unwrap()
        );
        for device in [&a, &b] {
            assert_eq!(device.chain("a"), note);
            for writer in ["a", "b"] {
                assert_eq!(audit::verify(&device.chain(writer)), Ok(()));
            }
        }
    }
}
