//! The sync engine: pull, merge, decide, push.
//!
//! Two traits keep the algorithm away from both Google and Tauri — [`Remote`]
//! is the one file on the drive, [`LocalVault`] is this device's store — so the
//! whole of [`sync`] is exercised in-process against a fake drive and real
//! SQLCipher vaults in temp dirs.
//!
//! # Why this converges
//!
//! Every push is the *whole* state, and [`SqliteStore::merge`] is a join
//! (idempotent, commutative, associative). So a device that loses a race has not
//! lost data: its digest still differs from the remote, and its next sync pulls
//! the winner, merges, and pushes the union. Divergence is temporary by
//! construction; the only permanent state two synced devices can reach is the
//! same one.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use tauri::{AppHandle, Manager};
use zeroize::Zeroizing;

use super::pack::{self, PackError};
use crate::app::APP_NAME;
use crate::error::{Error, Result};
use crate::session::{store_err, Session};
use crate::state::AppState;
use crate::storage;
use crate::store::{
    identity, vault_digest, Attachment, Record, SqliteStore, StoreError, VaultStore,
};

/// How long a tombstone is kept before it is reclaimed. A device offline for
/// longer than this can resurrect what it deleted; see
/// [`SqliteStore::purge_tombstones_before`] for the trade.
const TOMBSTONE_TTL_MS: i64 = 90 * 24 * 60 * 60 * 1000;

/// How many times a run will re-pull after losing a race before giving up. A
/// bound rather than a retry loop: if the remote revision keeps moving, another
/// device is syncing in a tight loop and the right answer is to stop and let the
/// next scheduled run settle it, not to spin.
const MAX_ATTEMPTS: usize = 3;

// Both under `SYNC_META_PREFIX`, so a restore onto a fresh install scrubs them
// (the source device's bookkeeping means nothing on the target).
const META_REMOTE_REV: &str = "sync_remote_rev";
const META_LAST_MS: &str = "sync_last_ms";

// Surfaced verbatim by the sync indicator, so it has to read as a sentence.
fn foreign_vault_error() -> String {
    format!("Google Drive holds a vault from a different {APP_NAME} install")
}

fn too_large_to_sync_error() -> String {
    "This vault is too large to sync. Remove some attachments and try again.".into()
}

/// The remote snapshot together with the revision it was read at.
pub struct RemoteFile {
    pub bytes: Vec<u8>,
    /// Empty when the backend exposes no revision id; the pre-flight check then
    /// degrades to "always looks unchanged", which costs race detection but
    /// never correctness (the loser re-converges on its next run).
    pub revision: String,
}

/// The single remote artifact this engine syncs against.
pub trait Remote {
    /// The current contents, or `None` when nothing has ever been pushed.
    fn fetch(&self) -> Result<Option<RemoteFile>>;
    /// The current revision id without downloading the body.
    fn head_revision(&self) -> Result<Option<String>>;
    /// Replace the contents, returning the resulting revision id.
    fn upload(&self, bytes: &[u8]) -> Result<String>;
}

/// This device's vault, as the engine needs it.
///
/// Every method is a complete unit of work rather than a borrow of the store,
/// so the production implementation can take and release the session lock
/// around each one. A network round trip must never be made while holding it —
/// that is precisely what made the first sync implementation freeze the UI.
///
/// A run is therefore *not* one transaction: the user can save an entry between
/// the merge and the pack. That is safe, and deliberately so. A write that
/// lands mid-run only ever makes the local state newer, so the snapshot pushed
/// is a superset of the one the digest was taken from — never a rollback — and
/// the write's own debounced sync settles whatever is left over.
pub trait LocalVault {
    /// Open a pulled `.rowel` pack and read it out. The local database is not
    /// touched: the snapshot goes into its own scratch DB.
    fn decode(&self, pack_bytes: &[u8]) -> Result<Snapshot>;
    /// Last-writer-wins merge of the pulled entries and attachments; returns
    /// the rows written.
    fn merge(&self, incoming: &Snapshot) -> Result<usize>;
    /// Fingerprint of the entry and attachment tables, tombstones included.
    fn digest(&self) -> Result<[u8; 32]>;
    /// Reclaim tombstones older than `cutoff_ms`, then pack the vault for upload.
    fn pack(&self, cutoff_ms: i64) -> Result<Vec<u8>>;
    /// Record a completed push: the revision it landed at, and when.
    fn note_push(&self, revision: &str, at_ms: i64) -> Result<()>;

    // The two below are the vault's *identity* rather than its contents. The
    // provider reads the id before a run, to learn which remote pack this vault
    // is, and writes it from the `settle` hook [`sync`] calls mid-run — after
    // the pull has been merged and before anything is pushed. Neither is called
    // by the engine itself; they live on the trait because the id sits in the
    // vault's own `meta` table, and reaching it means taking the session lock
    // exactly as everything else here does.
    //
    /// This vault's id, or `None` for a vault created before ids existed — or
    /// for one whose first run has not yet settled one.
    fn vault_id(&self) -> Result<Option<String>>;
    /// Take on an id this run has proved to be this vault's, whether it was
    /// read off the remote or minted for it.
    fn adopt_vault_id(&self, id: &str) -> Result<()>;

    // The vault's name, which unlike the id is a value two devices can each
    // change. It lives beside the entries rather than among them, so the engine
    // reads and writes it itself — see [`sync`] for how it is settled.
    //
    /// This vault's name and the stamp it was set at; `(None, 0)` when nobody
    /// has named it.
    fn name(&self) -> Result<(Option<String>, i64)>;
    /// Take on a name the pull proved to beat this vault's (see
    /// `identity::name_wins`), with the stamp it was set at elsewhere — not the
    /// time it arrived here.
    fn adopt_name(&self, name: &str, at_ms: i64) -> Result<()>;
}

/// A pulled pack, opened: the entries and attachments in it and the name it
/// was pushed under.
pub struct Snapshot {
    pub records: Vec<Record>,
    pub attachments: Vec<Attachment>,
    /// The name in the pack's `meta`, with the ms-epoch stamp it was set at.
    /// `(None, 0)` for a pack pushed before names travelled.
    pub name: Option<String>,
    pub name_ms: i64,
}

/// What one run did, for the caller's events and logs.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SyncOutcome {
    /// Rows the merge wrote locally. Non-zero means the entry list changed and
    /// the frontend has to be told.
    pub merged: usize,
    /// Whether a new snapshot was uploaded.
    pub pushed: bool,
    /// Whether the pull carried a newer name for this vault. The caller mirrors
    /// it into the workspace registry, which is what the UI draws from.
    pub renamed: bool,
}

/// Run one sync: pull, merge, and push if — and only if — the two sides differ,
/// in their entries or in the vault name that travels beside them.
///
/// `settle` is the caller's chance to write down whatever this run has just
/// proved — in practice the vault id the provider resolved (see `sync::run`).
/// It is called **once**, at the one point in the run that is on the far side of
/// a validated pull and on the near side of every push: after the remote pack
/// has been fetched, decoded and merged (or found not to exist), and before
/// anything is packed or uploaded. A run that cannot read the pack it was
/// pointed at therefore never reaches it, and no byte can leave this device
/// under an identity that is not yet on disk. A `settle` that fails ends the
/// run there, before any upload.
///
/// Once per run, not once per attempt: a re-pull after losing a race proves
/// nothing new about the identity, and the write has already happened.
pub fn sync<R: Remote, L: LocalVault>(
    remote: &R,
    local: &L,
    now_ms: i64,
    settle: impl FnOnce() -> Result<()>,
) -> Result<SyncOutcome> {
    let mut outcome = SyncOutcome::default();
    let mut settle = Some(settle);

    for _ in 0..MAX_ATTEMPTS {
        // 1-3. Pull, open the snapshot, merge it in. A remote that cannot be
        // read errors out here, before the local vault has been touched.
        let pulled = remote.fetch()?;
        // Nothing on the remote yet means this run creates it, and there is no
        // snapshot to decode or merge.
        let incoming = match &pulled {
            Some(file) => Some(local.decode(&file.bytes)?),
            None => None,
        };
        if let Some(snapshot) = &incoming {
            outcome.merged += local.merge(snapshot)?;
            // The name is last-writer-wins too, but on its own stamp: a rename
            // is a change to the vault that leaves no row in the entry table.
            // `identity::name_wins` is the whole decision — a total order, so a
            // tie in the stamp still settles on one of the two names rather
            // than on whatever each device happens to hold.
            if let Some(name) = &snapshot.name {
                let held = local.name()?;
                if identity::name_wins((Some(name), snapshot.name_ms), (held.0.as_deref(), held.1))
                {
                    local.adopt_name(name, snapshot.name_ms)?;
                    outcome.renamed = true;
                }
            }
        }

        // 4. Settle the identity, now that the pull is in and nothing has gone
        // out. See this function's own doc for why it is exactly here.
        if let Some(settle) = settle.take() {
            settle()?;
        }

        if let Some(snapshot) = &incoming {
            // 5. The push decision is that the two sides differ — in their
            // entries, or in the name that travels beside them.
            //
            // Not the dirty flag, and not "the merge changed something":
            // both miss the case where local is a strict superset of remote
            // — another device pushed a state derived from an older pull
            // and clobbered ours. Merging that back in adds nothing to us
            // (every incoming row loses last-writer-wins), so a
            // change-driven heuristic concludes "in sync" and leaves our
            // entries missing from the remote forever. The digests still
            // differ, so this pushes.
            //
            // The name is compared beside the digest rather than folded into
            // it: it is not in the entry table, so a rename with nothing else
            // to say would otherwise never leave this device. The comparison is
            // the same `name_wins` the adopt above used, so the two agree by
            // construction: this pushes exactly when the name here beats the
            // remote's, which is exactly when the adopt declined to take it.
            // Anything else — equal pairs, or a remote that just won — has
            // nothing to say and stops here.
            let held = local.name()?;
            let theirs = vault_digest(
                &snapshot.records,
                snapshot.attachments.iter().map(|a| (&a.meta, a.blob.len())),
            );
            if local.digest()? == theirs
                && !identity::name_wins(
                    (held.0.as_deref(), held.1),
                    (snapshot.name.as_deref(), snapshot.name_ms),
                )
            {
                return Ok(outcome);
            }
        }
        let base_revision = pulled.as_ref().map(|file| file.revision.clone());

        // 6. Pre-flight. If the head moved since the pull, another device
        // pushed while we were merging and our snapshot would drop its work.
        // Start over from its state instead.
        if remote.head_revision()? != base_revision {
            continue;
        }

        // Purge *before* packing, not after. The artifact we upload then
        // already excludes the reclaimed tombstones, so the digest we push is
        // the digest we hold; purging afterwards would re-dirty local state the
        // instant the push completed and earn a second, pointless push. It is
        // also safe by age alone: a tombstone this old was pushed many syncs
        // ago, so the "never purge before it has been pushed once" rule holds
        // without any extra bookkeeping.
        let bytes = local.pack(now_ms - TOMBSTONE_TTL_MS)?;
        // Every pull refuses a pack past this, so one uploaded anyway could
        // never be read again by any device, this one included. The vault
        // budget (`commands::attachments`) keeps a vault well short of it;
        // this is the net for one that got there some other way.
        if bytes.len() > pack::MAX_PACK_BYTES {
            return Err(Error::Other(too_large_to_sync_error()));
        }
        let revision = remote.upload(&bytes)?;
        local.note_push(&revision, now_ms)?;
        outcome.pushed = true;
        return Ok(outcome);
    }

    Err(Error::Other(
        "another device kept changing the remote vault; sync will retry later".into(),
    ))
}

// --- the local side, backed by the unlocked session -------------------------

/// [`LocalVault`] over the session's open store.
///
/// The key is copied once at the start of the run: it is what opens the pulled
/// snapshot, and that step deliberately touches no session state at all. The
/// *store* is not copied — it is borrowed from the session for each operation
/// — so a run that outlives its session fails with [`Error::Locked`] on its
/// next step and stops. Stopping there leaves nothing partial behind: the merge
/// and the pack are each a single transaction, and re-running a push is a
/// no-op once the digests agree.
pub struct SessionVault {
    app: AppHandle,
    key: Zeroizing<Vec<u8>>,
    kdf_params_json: String,
    scratch: PathBuf,
}

impl SessionVault {
    /// Capture what a run needs from the live session. Fails if the vault is
    /// locked, or has no KDF descriptor to put in the pack header — without one
    /// a fresh install could never derive the key to open what we upload.
    pub fn capture(app: &AppHandle) -> Result<Self> {
        let kdf_params_json = storage::read_kdf_sidecar(app)?.ok_or_else(|| {
            Error::Other("this vault predates the key descriptor and cannot be synced".into())
        })?;
        let state = app.state::<AppState>();
        let session = state.session.lock().unwrap();
        Ok(Self {
            app: app.clone(),
            key: Zeroizing::new(session.key()?.sqlcipher_key().to_vec()),
            kdf_params_json,
            scratch: storage::sync_scratch_dir(app)?,
        })
    }

    // Hold the session lock for one operation. The captured key is checked
    // against the live one first: a change-master-password that landed mid-run
    // re-keyed the database, and packing it under the old key would produce a
    // snapshot nobody can open.
    fn with_session<T>(&self, f: impl FnOnce(&Session) -> Result<T>) -> Result<T> {
        let state = self.app.state::<AppState>();
        let session = state.session.lock().unwrap();
        if *self.key != *session.key()?.sqlcipher_key() {
            return Err(Error::Other("the vault key changed during sync".into()));
        }
        f(&session)
    }

    // Borrow the session's store for one operation. Only ever used for work
    // measured in rows — a merge, a digest, a `meta` write.
    fn with_store<T>(&self, f: impl FnOnce(&SqliteStore) -> Result<T>) -> Result<T> {
        self.with_session(|session| f(session.store()?))
    }

    // The key check on its own, for the step that runs outside the lock and has
    // to be told afterwards whether the vault it read is still the live one.
    fn check_key(&self) -> Result<()> {
        self.with_session(|_| Ok(()))
    }

    // Reclaim expired tombstones (one statement, under the lock) and hand back a
    // connection of the pack's own to read the snapshot through.
    fn reclaim_and_open(&self, cutoff_ms: i64) -> Result<SqliteStore> {
        self.with_store(|store| {
            store
                .purge_tombstones_before(cutoff_ms)
                .map_err(store_err)?;
            crate::session::open_snapshot_source(&self.app, &self.key)
        })
    }
}

impl LocalVault for SessionVault {
    fn decode(&self, pack_bytes: &[u8]) -> Result<Snapshot> {
        decode_pack(pack_bytes, &self.kdf_params_json, &self.key, &self.scratch)
    }

    fn merge(&self, incoming: &Snapshot) -> Result<usize> {
        self.with_store(|store| merge_snapshot(store, incoming))
    }

    fn digest(&self) -> Result<[u8; 32]> {
        self.with_store(|store| store.state_digest().map_err(store_err))
    }

    // The one step of a run that is not measured in rows: the pack copies the
    // whole database into scratch, and the session guard is deliberately not
    // alive for any of it — `reveal_entry`, `save_entry` and `app_status` all
    // used to queue behind this.
    //
    // The guard the borrow used to provide is kept by taking the lock twice
    // instead: once to reclaim and open (so the connection is keyed to the vault
    // as it stood), and once afterwards to confirm the key never moved. A
    // password change landing in between fails the second check, so a snapshot
    // taken under a key that is no longer the vault's is never uploaded.
    fn pack(&self, cutoff_ms: i64) -> Result<Vec<u8>> {
        let source = self.reclaim_and_open(cutoff_ms)?;
        let bytes = pack::pack_store(&source, &self.key, &self.kdf_params_json, &self.scratch)?;
        drop(source);
        self.check_key()?;
        Ok(bytes)
    }

    fn note_push(&self, revision: &str, at_ms: i64) -> Result<()> {
        self.with_store(|store| note_push(store, revision, at_ms))
    }

    fn vault_id(&self) -> Result<Option<String>> {
        self.with_store(|store| identity::vault_id(store).map_err(store_err))
    }

    fn adopt_vault_id(&self, id: &str) -> Result<()> {
        self.with_store(|store| identity::adopt_vault_id(store, id).map_err(store_err))
    }

    fn name(&self) -> Result<(Option<String>, i64)> {
        self.with_store(|store| identity::vault_name(store).map_err(store_err))
    }

    fn adopt_name(&self, name: &str, at_ms: i64) -> Result<()> {
        self.with_store(|store| identity::set_vault_name(store, name, at_ms).map_err(store_err))
    }
}

// --- shared implementations -------------------------------------------------

/// Read a pulled pack, refusing one this vault could not have written.
///
/// The descriptor check is not redundant with the open below. A pack from a
/// *different* vault — a second install that ran setup instead of restoring —
/// is sealed under a key this device cannot derive, and SQLCipher reports that
/// exactly the way it reports corruption. Distinguishing them here is what
/// stops the app telling someone their remote vault is damaged when it is
/// merely someone else's.
fn decode_pack(
    bytes: &[u8],
    expected_kdf_params_json: &str,
    key: &[u8],
    scratch_dir: &Path,
) -> Result<Snapshot> {
    let unpacked = pack::unpack(bytes).map_err(remote_pack_error)?;
    if unpacked.kdf_params_json != expected_kdf_params_json {
        return Err(Error::Other(foreign_vault_error()));
    }
    records_from_snapshot(&unpacked.snapshot, key, scratch_dir)
}

/// Open a pulled snapshot in a throwaway database and export what syncs: its
/// records, and the name the pushing device held.
///
/// The snapshot must land on disk before SQLCipher can read it, so it goes to a
/// uniquely named scratch file that a guard removes on every exit path. The
/// local vault is never opened, written, or replaced here: a hostile or broken
/// remote can only ever fail this function.
fn records_from_snapshot(snapshot: &[u8], key: &[u8], scratch_dir: &Path) -> Result<Snapshot> {
    fs::create_dir_all(scratch_dir)?;
    let scratch = Scratch(scratch_dir.join(scratch_name("remote")));
    fs::write(&scratch.0, snapshot)?;

    let store = SqliteStore::open(&scratch.0, key).map_err(|e| match e {
        // A schema this build has never seen is "update the app", never
        // "your remote vault is damaged" and never "wrong password".
        StoreError::SchemaNewer => Error::VaultTooNew,
        _ => Error::Other("Remote vault file is invalid".into()),
    })?;
    let (name, name_ms) = identity::vault_name(&store).map_err(store_err)?;
    Ok(Snapshot {
        records: store.export_for_sync().map_err(store_err)?,
        attachments: store.export_attachments_for_sync().map_err(store_err)?,
        name,
        name_ms,
    })
}

// One transaction for both tables, so a file never lands without the entry
// state it is checked against (see `SqliteStore::merge`).
fn merge_snapshot(store: &SqliteStore, incoming: &Snapshot) -> Result<usize> {
    store
        .merge(&incoming.records, &incoming.attachments)
        .map_err(store_err)
}

// A pack this build cannot parse is corruption, except for a format stamped by
// a newer build — same distinction the store draws for its schema.
fn remote_pack_error(e: PackError) -> Error {
    match e {
        PackError::UnknownVersion(_) => Error::VaultTooNew,
        PackError::Io(e) => Error::Io(e),
        _ => Error::Other("Remote vault file is invalid".into()),
    }
}

fn note_push(store: &SqliteStore, revision: &str, at_ms: i64) -> Result<()> {
    store
        .meta_set(META_REMOTE_REV, revision)
        .map_err(store_err)?;
    store
        .meta_set(META_LAST_MS, &at_ms.to_string())
        .map_err(store_err)
}

// Unique per call: a scheduled run and a manual one must not share a path.
fn scratch_name(role: &str) -> String {
    static N: AtomicU64 = AtomicU64::new(0);
    format!(
        "rowel-sync-{role}-{}-{}.db",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    )
}

// Removes the scratch database and its WAL/SHM siblings however we return.
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        storage::remove_db_files(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::KdfParams;
    use crate::store::{state_digest, AttachmentMeta};
    use std::sync::atomic::AtomicBool;
    use std::sync::Mutex;

    const KEY: &[u8] = &[0x5a; 32];
    const NOW: i64 = 1_700_000_000_000;
    const DAY_MS: i64 = 24 * 60 * 60 * 1000;

    // Most tests here are about the merge, not the identity, and have nothing
    // to settle. Shadows `super::sync` on purpose so they read as the algorithm
    // does; the identity tests below call `super::sync` with a hook of their own.
    fn sync<R: Remote, L: LocalVault>(remote: &R, local: &L, now_ms: i64) -> Result<SyncOutcome> {
        super::sync(remote, local, now_ms, || Ok(()))
    }

    fn kdf_json() -> String {
        KdfParams::argon2id(b"salt-0123456789012345", 256, 1, 1)
            .to_json()
            .unwrap()
    }

    fn tmp_dir() -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "rowel-engine-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn record(id: &str, updated_at: i64, payload: &[u8]) -> Record {
        Record {
            id: id.into(),
            kind: "login".into(),
            title: format!("Entry {id}"),
            tags: "[]".into(),
            url_host: "example.com".into(),
            created_at: 100,
            updated_at,
            deleted_at: None,
            payload: payload.to_vec(),
            card_brand: None,
            favorite: false,
            has_passkey: false,
            file_name: None,
            var_count: None,
            username: None,
        }
    }

    fn tombstone(id: &str, deleted_at: i64) -> Record {
        Record {
            deleted_at: Some(deleted_at),
            updated_at: deleted_at,
            ..record(id, deleted_at, b"")
        }
    }

    // A device: its own vault directory, store, and scratch space.
    struct Device {
        store: SqliteStore,
        scratch: PathBuf,
        // What `vault_id()` answered the last time `pack()` ran. The whole point
        // of settling the id before the push is that this is never `None` for a
        // run that had one to settle.
        id_when_packed: Mutex<Option<String>>,
        // Packs come out one byte past what a pull will download, as a vault
        // stuffed past the budget by an older build or by hand would.
        oversized: AtomicBool,
    }

    impl Device {
        fn new() -> Self {
            let dir = tmp_dir();
            Self {
                store: SqliteStore::open(&dir.join("vault.db"), KEY).unwrap(),
                scratch: dir.join("scratch"),
                id_when_packed: Mutex::default(),
                oversized: AtomicBool::new(false),
            }
        }

        fn seeded(recs: &[Record]) -> Self {
            let device = Self::new();
            device.store.import(recs).unwrap();
            device
        }

        fn ids(&self) -> Vec<String> {
            self.store
                .list()
                .unwrap()
                .into_iter()
                .map(|m| m.id)
                .collect()
        }

        fn pack_bytes(&self) -> Vec<u8> {
            pack::pack_store(&self.store, KEY, &kdf_json(), &self.scratch).unwrap()
        }

        // What `workspace_rename` does to the vault it renames.
        fn rename(&self, name: &str, at_ms: i64) {
            identity::set_vault_name(&self.store, name, at_ms).unwrap();
        }

        // What `commands::auth::seed_vault_name` does on the first unlock of a
        // vault named before names lived inside one.
        fn seed_name(&self, name: &str) {
            identity::set_vault_name(&self.store, name, identity::MIGRATED_NAME_MS).unwrap();
        }
    }

    impl LocalVault for Device {
        fn decode(&self, pack_bytes: &[u8]) -> Result<Snapshot> {
            decode_pack(pack_bytes, &kdf_json(), KEY, &self.scratch)
        }
        fn merge(&self, incoming: &Snapshot) -> Result<usize> {
            merge_snapshot(&self.store, incoming)
        }
        fn digest(&self) -> Result<[u8; 32]> {
            self.store.state_digest().map_err(store_err)
        }
        fn pack(&self, cutoff_ms: i64) -> Result<Vec<u8>> {
            *self.id_when_packed.lock().unwrap() = self.vault_id()?;
            if self.oversized.load(Ordering::SeqCst) {
                return Ok(vec![0; pack::MAX_PACK_BYTES + 1]);
            }
            self.store
                .purge_tombstones_before(cutoff_ms)
                .map_err(store_err)?;
            Ok(pack::pack_store(
                &self.store,
                KEY,
                &kdf_json(),
                &self.scratch,
            )?)
        }
        fn note_push(&self, revision: &str, at_ms: i64) -> Result<()> {
            note_push(&self.store, revision, at_ms)
        }
        fn vault_id(&self) -> Result<Option<String>> {
            identity::vault_id(&self.store).map_err(store_err)
        }
        fn adopt_vault_id(&self, id: &str) -> Result<()> {
            identity::adopt_vault_id(&self.store, id).map_err(store_err)
        }
        fn name(&self) -> Result<(Option<String>, i64)> {
            identity::vault_name(&self.store).map_err(store_err)
        }
        fn adopt_name(&self, name: &str, at_ms: i64) -> Result<()> {
            identity::set_vault_name(&self.store, name, at_ms).map_err(store_err)
        }
    }

    #[derive(Default)]
    struct FakeDrive {
        content: Option<Vec<u8>>,
        revision: u64,
        uploads: usize,
        // Bumped by the test between a pull and the push that follows it, to
        // stand in for another device landing a snapshot mid-run.
        interlopers: Vec<Vec<u8>>,
        // Every upload fails: the link dropped, Drive said no. The pull side
        // keeps working, as it would.
        refuse_uploads: bool,
    }

    #[derive(Default)]
    struct FakeRemote {
        state: Mutex<FakeDrive>,
    }

    impl FakeRemote {
        fn with(bytes: Vec<u8>) -> Self {
            let remote = Self::default();
            remote.put(bytes);
            remote
        }

        // A direct write, as another device's push would look.
        fn put(&self, bytes: Vec<u8>) {
            let mut state = self.state.lock().unwrap();
            state.revision += 1;
            state.content = Some(bytes);
        }

        // Land `bytes` on the drive the next time the engine reads the head —
        // i.e. exactly in the pull/push window.
        fn interlope(&self, bytes: Vec<u8>) {
            self.state.lock().unwrap().interlopers.push(bytes);
        }

        fn uploads(&self) -> usize {
            self.state.lock().unwrap().uploads
        }

        // Turn the push side off, and back on for the retry that follows.
        fn refuse_uploads(&self, refuse: bool) {
            self.state.lock().unwrap().refuse_uploads = refuse;
        }

        fn content(&self) -> Option<Vec<u8>> {
            self.state.lock().unwrap().content.clone()
        }
    }

    impl Remote for FakeRemote {
        fn fetch(&self) -> Result<Option<RemoteFile>> {
            let state = self.state.lock().unwrap();
            Ok(state.content.clone().map(|bytes| RemoteFile {
                bytes,
                revision: state.revision.to_string(),
            }))
        }

        fn head_revision(&self) -> Result<Option<String>> {
            let mut state = self.state.lock().unwrap();
            if let Some(bytes) = state.interlopers.pop() {
                state.revision += 1;
                state.content = Some(bytes);
            }
            Ok(state.content.is_some().then(|| state.revision.to_string()))
        }

        fn upload(&self, bytes: &[u8]) -> Result<String> {
            let mut state = self.state.lock().unwrap();
            if state.refuse_uploads {
                return Err(Error::Other("the upload failed".into()));
            }
            state.revision += 1;
            state.uploads += 1;
            state.content = Some(bytes.to_vec());
            Ok(state.revision.to_string())
        }
    }

    // Read a drive's contents back as records, using a scratch dir of its own.
    fn remote_records(remote: &FakeRemote) -> Vec<Record> {
        let unpacked = pack::unpack(&remote.content().unwrap()).unwrap();
        records_from_snapshot(&unpacked.snapshot, KEY, &tmp_dir())
            .unwrap()
            .records
    }

    // The name a drive's pack carries in its own `meta`.
    fn remote_name(remote: &FakeRemote) -> (Option<String>, i64) {
        let unpacked = pack::unpack(&remote.content().unwrap()).unwrap();
        let snapshot = records_from_snapshot(&unpacked.snapshot, KEY, &tmp_dir()).unwrap();
        (snapshot.name, snapshot.name_ms)
    }

    #[test]
    fn an_empty_remote_gets_the_first_push() {
        let a = Device::seeded(&[record("1", 200, b"one")]);
        let remote = FakeRemote::default();

        let outcome = sync(&remote, &a, NOW).unwrap();

        assert_eq!(
            outcome,
            SyncOutcome {
                merged: 0,
                pushed: true,
                renamed: false
            }
        );
        assert_eq!(remote_records(&remote), a.store.export_for_sync().unwrap());
    }

    #[test]
    fn two_devices_converge_in_both_directions() {
        let a = Device::seeded(&[record("1", 200, b"one")]);
        let b = Device::new();
        let remote = FakeRemote::default();

        // A publishes; B, empty, adopts it.
        sync(&remote, &a, NOW).unwrap();
        let outcome = sync(&remote, &b, NOW).unwrap();
        assert_eq!(outcome.merged, 1);
        assert!(!outcome.pushed, "B matched the remote exactly");
        assert_eq!(
            b.store.state_digest().unwrap(),
            a.store.state_digest().unwrap()
        );

        // B edits and pushes; A picks it up on its next run.
        b.store.import(&[record("2", 300, b"two")]).unwrap();
        assert!(sync(&remote, &b, NOW).unwrap().pushed);

        let outcome = sync(&remote, &a, NOW).unwrap();
        assert_eq!(outcome.merged, 1);
        assert!(!outcome.pushed);
        assert_eq!(a.ids(), vec!["1", "2"]);
        assert_eq!(
            a.store.state_digest().unwrap(),
            b.store.state_digest().unwrap()
        );
    }

    #[test]
    fn a_second_run_with_nothing_to_do_uploads_nothing() {
        let a = Device::seeded(&[record("1", 200, b"one")]);
        let remote = FakeRemote::default();

        sync(&remote, &a, NOW).unwrap();
        let outcome = sync(&remote, &a, NOW).unwrap();

        assert_eq!(
            outcome,
            SyncOutcome {
                merged: 0,
                pushed: false,
                renamed: false
            }
        );
        assert_eq!(remote.uploads(), 1);
    }

    // A backfilled username has the same `updated_at` as the peer's pre-column
    // copy, so the two tie on time. The remote's NULL copy must not win that
    // tie: the first run publishes the stamped row, and the next has nothing to
    // say.
    #[test]
    fn a_backfilled_username_survives_a_pull_of_an_older_snapshot() {
        // A payload whose NULL copy would beat the stamped one on hash alone —
        // the case a hash-only tie-break loses.
        let (unstamped, backfilled) = (0u8..)
            .map(|n| {
                let unstamped = record("1", 200, &[b"login-", &[n][..]].concat());
                let backfilled = Record {
                    username: Some("alice".into()),
                    ..unstamped.clone()
                };
                (unstamped, backfilled)
            })
            .find(|(u, b)| crate::store::record_hash(u) > crate::store::record_hash(b))
            .unwrap();

        let a = Device::seeded(&[backfilled]);
        let remote = FakeRemote::with(Device::seeded(&[unstamped]).pack_bytes());

        let first = sync(&remote, &a, NOW).unwrap();
        assert!(first.pushed);
        assert_eq!(
            remote_records(&remote)[0].username.as_deref(),
            Some("alice")
        );

        let second = sync(&remote, &a, NOW).unwrap();
        assert!(!second.pushed);
        assert_eq!(remote.uploads(), 1);
        assert_eq!(
            a.store.export_for_sync().unwrap()[0].username.as_deref(),
            Some("alice")
        );
    }

    // F1. Another device pushed a state derived from a stale pull, clobbering
    // ours. Merging it back adds nothing here — every incoming row loses — so
    // any "did the merge change anything" heuristic would call this in sync and
    // leave our entry missing from the remote for good.
    #[test]
    fn a_remote_that_is_a_strict_subset_of_local_is_still_pushed() {
        let a = Device::seeded(&[record("1", 200, b"one"), record("2", 300, b"two")]);
        let remote = FakeRemote::default();
        sync(&remote, &a, NOW).unwrap();

        // A stale device overwrites the drive with a state that predates
        // record 2 (and knows an older record 1).
        let stale = Device::seeded(&[record("1", 100, b"older")]);
        remote.put(stale.pack_bytes());

        let outcome = sync(&remote, &a, NOW).unwrap();

        assert_eq!(
            outcome.merged, 0,
            "nothing on the remote is newer than local"
        );
        assert!(outcome.pushed, "but the remote is missing record 2");
        assert_eq!(remote.uploads(), 2);

        let published: Vec<String> = remote_records(&remote).into_iter().map(|r| r.id).collect();
        assert_eq!(published, vec!["1", "2"]);
        assert_eq!(
            state_digest(&remote_records(&remote)),
            a.store.state_digest().unwrap()
        );
    }

    #[test]
    fn a_push_that_loses_a_race_re_pulls_and_uploads_the_union() {
        let a = Device::seeded(&[record("1", 200, b"one")]);
        let remote = FakeRemote::default();
        sync(&remote, &a, NOW).unwrap();

        // A has something new to push...
        a.store.import(&[record("2", 400, b"two")]).unwrap();
        // ...but C lands its own snapshot in the pull/push window.
        let c = Device::seeded(&[record("1", 200, b"one"), record("3", 500, b"three")]);
        remote.interlope(c.pack_bytes());

        let outcome = sync(&remote, &a, NOW).unwrap();

        assert!(outcome.pushed);
        assert_eq!(
            outcome.merged, 1,
            "C's record was merged on the second pass"
        );
        let published: Vec<String> = remote_records(&remote).into_iter().map(|r| r.id).collect();
        assert_eq!(published, vec!["1", "2", "3"], "the union, not either side");
    }

    #[test]
    fn a_remote_that_never_stops_moving_errors_instead_of_spinning() {
        let a = Device::seeded(&[record("1", 200, b"one")]);
        let remote = FakeRemote::default();
        sync(&remote, &a, NOW).unwrap();
        a.store.import(&[record("2", 400, b"two")]).unwrap();

        // One interloper per attempt, plus one to spare.
        let c = Device::seeded(&[record("9", 900, b"nine")]);
        for _ in 0..MAX_ATTEMPTS + 1 {
            remote.interlope(c.pack_bytes());
        }

        assert!(sync(&remote, &a, NOW).is_err());
        assert_eq!(remote.uploads(), 1, "only the very first push ever landed");
    }

    #[test]
    fn a_truncated_remote_errors_and_leaves_the_local_vault_alone() {
        let a = Device::seeded(&[record("1", 200, b"one")]);
        let before = a.store.state_digest().unwrap();

        let good = Device::seeded(&[record("2", 300, b"two")]).pack_bytes();
        let remote = FakeRemote::with(good[..good.len() / 2].to_vec());

        assert!(sync(&remote, &a, NOW).is_err());
        assert_eq!(a.store.state_digest().unwrap(), before);
        assert_eq!(a.ids(), vec!["1"]);
        assert_eq!(remote.uploads(), 0);
    }

    #[test]
    fn a_remote_we_cannot_decrypt_errors_and_leaves_the_local_vault_alone() {
        let a = Device::seeded(&[record("1", 200, b"one")]);
        let before = a.store.state_digest().unwrap();

        // Structurally a pack, but the snapshot is not our database.
        let remote = FakeRemote::with(pack::pack(&kdf_json(), &[0xab; 4096]));

        match sync(&remote, &a, NOW) {
            Err(Error::Other(msg)) => assert_eq!(msg, "Remote vault file is invalid"),
            other => panic!("expected an invalid-remote error, got {other:?}"),
        }
        assert_eq!(a.store.state_digest().unwrap(), before);
        assert_eq!(remote.uploads(), 0);
    }

    #[test]
    fn a_pack_from_another_install_is_named_as_such_not_as_corruption() {
        let a = Device::seeded(&[record("1", 200, b"one")]);
        let foreign = KdfParams::argon2id(b"a-completely-different-salt-here", 256, 1, 1)
            .to_json()
            .unwrap();
        let remote = FakeRemote::with(pack::pack(&foreign, &[0u8; 4096]));

        match sync(&remote, &a, NOW) {
            Err(Error::Other(msg)) => assert_eq!(msg, foreign_vault_error()),
            other => panic!("expected the foreign-vault error, got {other:?}"),
        }
        assert_eq!(remote.uploads(), 0);
    }

    #[test]
    fn a_remote_from_a_future_build_says_update_the_app() {
        let a = Device::seeded(&[record("1", 200, b"one")]);
        let before = a.store.state_digest().unwrap();

        let future = Device::seeded(&[record("2", 300, b"two")]);
        future.store.set_user_version(99).unwrap();
        let remote = FakeRemote::with(future.pack_bytes());

        match sync(&remote, &a, NOW) {
            Err(Error::VaultTooNew) => {}
            other => panic!("expected VaultTooNew, got {other:?}"),
        }
        assert_eq!(a.store.state_digest().unwrap(), before);
        assert_eq!(remote.uploads(), 0);
    }

    #[test]
    fn a_pack_stamped_with_an_unknown_format_says_update_the_app() {
        let a = Device::seeded(&[record("1", 200, b"one")]);
        let mut bytes = Device::seeded(&[record("2", 300, b"two")]).pack_bytes();
        bytes[4] = 2; // the format byte

        match sync(&FakeRemote::with(bytes), &a, NOW) {
            Err(Error::VaultTooNew) => {}
            other => panic!("expected VaultTooNew, got {other:?}"),
        }
    }

    #[test]
    fn an_expired_tombstone_is_reclaimed_and_a_fresh_one_propagates() {
        let a = Device::seeded(&[
            record("live", 200, b"live"),
            tombstone("ancient", NOW - 100 * DAY_MS),
            tombstone("recent", NOW - DAY_MS),
        ]);
        let remote = FakeRemote::default();

        sync(&remote, &a, NOW).unwrap();

        let published: Vec<String> = remote_records(&remote).into_iter().map(|r| r.id).collect();
        assert_eq!(
            published,
            vec!["live", "recent"],
            "the 90-day-old tombstone is gone"
        );

        // The fresh tombstone reaches a peer that still holds the entry as live.
        let b = Device::seeded(&[record("recent", 100, b"stale copy")]);
        sync(&remote, &b, NOW).unwrap();
        assert_eq!(b.ids(), vec!["live"], "the delete propagated");
    }

    // --- attachments -------------------------------------------------------
    //
    // They ride the same pack and merge per row beside the entries, so a file
    // added, or removed, on one device reaches the other — and a removal is
    // never undone by a peer that still holds the file.

    fn attachment(id: &str, entry_id: &str, blob: &[u8]) -> Attachment {
        Attachment {
            meta: AttachmentMeta {
                id: id.into(),
                entry_id: entry_id.into(),
                name: format!("{id}.pdf"),
                mime: None,
                size: blob.len() as i64,
                created_at: 0,
                updated_at: 0,
                deleted_at: None,
            },
            blob: blob.to_vec(),
        }
    }

    fn files(device: &Device, entry_id: &str) -> Vec<String> {
        device
            .store
            .list_attachments(entry_id)
            .unwrap()
            .into_iter()
            .map(|m| m.id)
            .collect()
    }

    #[test]
    fn an_attachment_added_on_one_device_reaches_the_other() {
        let a = Device::seeded(&[record("1", 200, b"one")]);
        let b = Device::seeded(&[record("1", 200, b"one")]);
        let remote = FakeRemote::default();
        sync(&remote, &a, NOW).unwrap();
        sync(&remote, &b, NOW).unwrap();

        // Only the attachment table changed, so the push has to see it.
        a.store
            .insert_attachment(&attachment("f1", "1", b"sealed"))
            .unwrap();
        assert!(sync(&remote, &a, NOW).unwrap().pushed);

        let outcome = sync(&remote, &b, NOW).unwrap();
        assert_eq!(outcome.merged, 1);
        assert!(!outcome.pushed);
        assert_eq!(files(&b, "1"), vec!["f1"]);
        assert_eq!(
            b.store.get_attachment("f1").unwrap().unwrap().blob,
            b"sealed"
        );
        assert_eq!(
            a.store.state_digest().unwrap(),
            b.store.state_digest().unwrap()
        );
    }

    #[test]
    fn a_removed_attachment_stays_removed_on_every_device() {
        let a = Device::seeded(&[record("1", 200, b"one")]);
        a.store
            .insert_attachment(&attachment("f1", "1", b"sealed"))
            .unwrap();
        let b = Device::new();
        let remote = FakeRemote::default();
        sync(&remote, &a, NOW).unwrap();
        sync(&remote, &b, NOW).unwrap();
        assert_eq!(files(&b, "1"), vec!["f1"]);

        // B removes it; A, which still holds it live, pulls the removal.
        b.store.remove_attachment("f1").unwrap();
        assert!(sync(&remote, &b, NOW).unwrap().pushed);
        sync(&remote, &a, NOW).unwrap();
        assert!(files(&a, "1").is_empty());

        // A stale snapshot from before the removal cannot bring it back.
        let stale = Device::seeded(&[record("1", 200, b"one")]);
        stale
            .store
            .import_attachments(&[Attachment {
                meta: AttachmentMeta {
                    updated_at: i64::MAX,
                    ..attachment("f1", "1", b"").meta
                },
                blob: b"sealed".to_vec(),
            }])
            .unwrap();
        remote.put(stale.pack_bytes());
        sync(&remote, &a, NOW).unwrap();
        assert!(files(&a, "1").is_empty());
        assert!(a.store.get_attachment("f1").unwrap().is_none());
        assert!(remote_files(&remote).iter().all(|f| f.blob.is_empty()));
    }

    // A peer attached a file to an entry this device purged before it saw the
    // file. The merge keeps the entry purged and takes the file with it, and
    // the two settle on the same shell rather than pushing at each other.
    #[test]
    fn a_file_added_to_an_entry_purged_elsewhere_ends_a_shell_everywhere() {
        let a = Device::seeded(&[record("1", 200, b"one")]);
        let b = Device::new();
        let remote = FakeRemote::default();
        sync(&remote, &a, NOW).unwrap();
        sync(&remote, &b, NOW).unwrap();
        b.store
            .insert_attachment(&attachment("f1", "1", b"sealed"))
            .unwrap();
        assert!(sync(&remote, &b, NOW).unwrap().pushed);

        a.store.delete("1").unwrap();
        a.store.purge("1").unwrap();
        sync(&remote, &a, NOW).unwrap();
        assert!(a.store.get_attachment("f1").unwrap().is_none());
        assert!(remote_files(&remote).iter().all(|f| f.blob.is_empty()));

        sync(&remote, &b, NOW).unwrap();
        assert!(files(&b, "1").is_empty());
        assert!(b.store.get_attachment("f1").unwrap().is_none());
        assert_eq!(
            a.store.state_digest().unwrap(),
            b.store.state_digest().unwrap()
        );
        assert!(!sync(&remote, &a, NOW).unwrap().pushed);
        assert!(!sync(&remote, &b, NOW).unwrap().pushed);
    }

    // A peer attached a file to an entry this device archived meanwhile: the
    // file is archived with it, and a restore here brings it back everywhere.
    #[test]
    fn a_file_added_to_an_entry_archived_elsewhere_is_archived_and_restored_with_it() {
        let a = Device::seeded(&[record("1", 200, b"one")]);
        let b = Device::new();
        let remote = FakeRemote::default();
        sync(&remote, &a, NOW).unwrap();
        sync(&remote, &b, NOW).unwrap();
        b.store
            .insert_attachment(&attachment("f1", "1", b"sealed"))
            .unwrap();
        assert!(sync(&remote, &b, NOW).unwrap().pushed);

        a.store.delete("1").unwrap();
        sync(&remote, &a, NOW).unwrap();
        sync(&remote, &b, NOW).unwrap();
        assert!(files(&b, "1").is_empty());
        assert_eq!(
            a.store.state_digest().unwrap(),
            b.store.state_digest().unwrap()
        );

        a.store.restore("1").unwrap();
        assert_eq!(files(&a, "1"), vec!["f1"]);
        sync(&remote, &a, NOW).unwrap();
        sync(&remote, &b, NOW).unwrap();
        assert_eq!(files(&b, "1"), vec!["f1"]);
        assert_eq!(
            b.store.get_attachment("f1").unwrap().unwrap().blob,
            b"sealed"
        );
    }

    // A pack past the download cap would be one no device could pull again,
    // this one included, so it never goes up.
    #[test]
    fn a_pack_too_large_to_download_is_never_uploaded() {
        let a = Device::seeded(&[record("1", 200, b"one")]);
        a.oversized.store(true, Ordering::SeqCst);
        let remote = FakeRemote::default();

        match sync(&remote, &a, NOW) {
            Err(Error::Other(msg)) => assert_eq!(msg, too_large_to_sync_error()),
            other => panic!("expected the too-large error, got {other:?}"),
        }
        assert_eq!(remote.uploads(), 0);
        assert_eq!(remote.content(), None);
    }

    fn remote_files(remote: &FakeRemote) -> Vec<Attachment> {
        let unpacked = pack::unpack(&remote.content().unwrap()).unwrap();
        records_from_snapshot(&unpacked.snapshot, KEY, &tmp_dir())
            .unwrap()
            .attachments
    }

    #[test]
    fn a_completed_push_records_the_revision_it_landed_at() {
        let a = Device::seeded(&[record("1", 200, b"one")]);
        let remote = FakeRemote::default();

        sync(&remote, &a, NOW).unwrap();

        assert_eq!(
            a.store.meta_get(META_REMOTE_REV).unwrap(),
            Some("1".to_string())
        );
        assert_eq!(
            a.store.meta_get(META_LAST_MS).unwrap(),
            Some(NOW.to_string())
        );
    }

    #[test]
    fn a_local_edit_that_only_loses_the_merge_still_leaves_the_vaults_equal() {
        // Both devices edit the same entry with the same timestamp; the content
        // hash decides, and it decides the same way on both sides.
        let a = Device::seeded(&[record("1", 500, b"from-a")]);
        let b = Device::seeded(&[record("1", 500, b"from-b")]);
        let remote = FakeRemote::default();

        sync(&remote, &a, NOW).unwrap();
        sync(&remote, &b, NOW).unwrap();
        sync(&remote, &a, NOW).unwrap();
        sync(&remote, &b, NOW).unwrap();

        assert_eq!(
            a.store.state_digest().unwrap(),
            b.store.state_digest().unwrap()
        );
        // Converged: neither side has anything left to say.
        assert!(!sync(&remote, &a, NOW).unwrap().pushed);
        assert!(!sync(&remote, &b, NOW).unwrap().pushed);
    }

    #[test]
    fn the_scratch_directory_is_empty_once_a_run_finishes() {
        let a = Device::seeded(&[record("1", 200, b"one")]);
        let remote = FakeRemote::default();

        sync(&remote, &a, NOW).unwrap();
        sync(&remote, &a, NOW).unwrap();

        assert!(fs::read_dir(&a.scratch).unwrap().next().is_none());
    }

    // --- the vault's name ---------------------------------------------------
    //
    // A rename is a change to the vault that leaves no row in the entry table,
    // so it has a decision of its own on both sides of a run.

    #[test]
    fn a_rename_on_one_device_reaches_the_other() {
        let a = Device::seeded(&[record("1", 200, b"one")]);
        let b = Device::new();
        let remote = FakeRemote::default();
        a.rename("Work", NOW);

        sync(&remote, &a, NOW).unwrap();
        assert_eq!(remote_name(&remote), (Some("Work".into()), NOW));

        let outcome = sync(&remote, &b, NOW).unwrap();
        assert!(outcome.renamed);
        assert_eq!(b.name().unwrap(), (Some("Work".into()), NOW));
        assert!(!outcome.pushed, "B matched the remote, name included");
    }

    #[test]
    fn an_older_remote_name_does_not_overwrite_a_newer_local_one() {
        let a = Device::seeded(&[record("1", 200, b"one")]);
        let b = Device::seeded(&[record("1", 200, b"one")]);
        let remote = FakeRemote::default();

        b.rename("Old", NOW - DAY_MS);
        sync(&remote, &b, NOW).unwrap();

        a.rename("New", NOW);
        let outcome = sync(&remote, &a, NOW).unwrap();

        assert!(
            !outcome.renamed,
            "the remote's name is the older of the two"
        );
        assert_eq!(a.name().unwrap(), (Some("New".into()), NOW));
        assert!(outcome.pushed, "and the newer name goes out");
        assert_eq!(remote_name(&remote), (Some("New".into()), NOW));
    }

    // Nothing but the name changed, so a digest-only decision would call this
    // "in sync" and leave the other devices with the old label for good.
    #[test]
    fn a_name_change_on_its_own_is_enough_to_push() {
        let a = Device::seeded(&[record("1", 200, b"one")]);
        let remote = FakeRemote::default();
        sync(&remote, &a, NOW).unwrap();

        a.rename("Work", NOW);
        assert!(sync(&remote, &a, NOW).unwrap().pushed);
        assert_eq!(remote.uploads(), 2);
        assert_eq!(remote_name(&remote), (Some("Work".into()), NOW));
    }

    // The other half: once both sides hold the same name and stamp, neither has
    // anything to say — a run that pushed on the strength of the name alone
    // must not push again on every run after it.
    #[test]
    fn two_devices_agreeing_on_a_name_stop_pushing() {
        let a = Device::seeded(&[record("1", 200, b"one")]);
        let b = Device::new();
        let remote = FakeRemote::default();
        a.rename("Work", NOW);

        sync(&remote, &a, NOW).unwrap();
        sync(&remote, &b, NOW).unwrap();
        let uploads = remote.uploads();

        assert!(!sync(&remote, &a, NOW).unwrap().pushed);
        assert!(!sync(&remote, &b, NOW).unwrap().pushed);
        assert_eq!(remote.uploads(), uploads);
    }

    // Two renames in the same millisecond: the stamp cannot separate them, so
    // strict recency would reject both and leave each device pushing its own
    // name at the other for good. The name itself breaks the tie, the same way
    // the merge breaks one on the record hash, so the two land on one value.
    #[test]
    fn two_renames_in_the_same_millisecond_converge() {
        let a = Device::seeded(&[record("1", 200, b"one")]);
        let b = Device::seeded(&[record("1", 200, b"one")]);
        let remote = FakeRemote::default();
        a.rename("Home", NOW);
        b.rename("Work", NOW);

        // Each device runs until it has seen the other's name.
        sync(&remote, &a, NOW).unwrap();
        sync(&remote, &b, NOW).unwrap();
        sync(&remote, &a, NOW).unwrap();

        assert_eq!(a.name().unwrap(), b.name().unwrap());
        assert_eq!(remote_name(&remote), a.name().unwrap());

        // And it is settled: neither has anything left to say.
        let uploads = remote.uploads();
        assert!(!sync(&remote, &a, NOW).unwrap().pushed);
        assert!(!sync(&remote, &b, NOW).unwrap().pushed);
        assert_eq!(remote.uploads(), uploads);
    }

    // A label migrated out of this device's registry says only "before this
    // build", so it must lose to a name a device actually chose — otherwise an
    // install that upgrades late in a rollout overwrites everyone else's.
    #[test]
    fn a_migrated_label_loses_to_a_stamped_remote_name() {
        let a = Device::seeded(&[record("1", 200, b"one")]);
        let b = Device::seeded(&[record("1", 200, b"one")]);
        let remote = FakeRemote::default();
        a.rename("Work", NOW);
        sync(&remote, &a, NOW).unwrap();

        b.seed_name("Old laptop");
        let outcome = sync(&remote, &b, NOW).unwrap();

        assert!(outcome.renamed);
        assert_eq!(b.name().unwrap(), (Some("Work".into()), NOW));
        assert!(!outcome.pushed, "a stale label has nothing to publish");
    }

    // The other half: when nobody has named the vault, the migrated label is
    // the only name there is and it still has to reach the other devices.
    #[test]
    fn a_migrated_label_reaches_a_remote_with_no_name() {
        let a = Device::seeded(&[record("1", 200, b"one")]);
        let remote = FakeRemote::default();
        sync(&remote, &a, NOW).unwrap();

        a.seed_name("Old laptop");

        assert!(sync(&remote, &a, NOW).unwrap().pushed);
        assert_eq!(
            remote_name(&remote),
            (Some("Old laptop".into()), identity::MIGRATED_NAME_MS)
        );
    }

    // --- settling the vault id ---------------------------------------------
    //
    // `sync::run` passes a `settle` that writes the id it resolved. These pin
    // the ordering down from both sides: nothing is written before the pull has
    // been read, and nothing is uploaded before the write.

    const ID: &str = "0123456789abcdef0123456789abcdef";

    // The vault locked, or the `meta` write failed, at the moment the run went
    // to settle its identity. That has to end the run *before* the push: an
    // upload here would put a pack on Drive under an id this vault does not
    // answer to, and its next run would mint another one and publish a second.
    #[test]
    fn a_settle_that_fails_stops_the_run_before_anything_is_uploaded() {
        let a = Device::seeded(&[record("1", 200, b"one")]);
        let remote = FakeRemote::default();

        let result = super::sync(&remote, &a, NOW, || {
            Err(Error::Other("the vault locked during sync".into()))
        });

        assert!(result.is_err());
        assert_eq!(remote.uploads(), 0);
        assert_eq!(remote.content(), None, "the account is as the run found it");
        assert_eq!(a.vault_id().unwrap(), None, "and the vault took no id");
    }

    // The other side of the same guarantee: the id is already on disk when the
    // push is attempted, so a push that fails leaves the two in step. The next
    // run addresses the same pack rather than minting a second id for it.
    #[test]
    fn an_upload_that_fails_after_the_settle_retries_under_the_same_id() {
        let a = Device::seeded(&[record("1", 200, b"one")]);
        let remote = FakeRemote::default();
        remote.refuse_uploads(true);

        assert!(super::sync(&remote, &a, NOW, || a.adopt_vault_id(ID)).is_err());
        assert_eq!(
            a.vault_id().unwrap().as_deref(),
            Some(ID),
            "written before the push was even attempted"
        );
        assert_eq!(remote.uploads(), 0);
        assert_eq!(remote.content(), None);

        // The retry finds the vault already named, so it has nothing to settle
        // — `resolve_vault_id` would answer `Use` — and converges in one push.
        remote.refuse_uploads(false);
        assert!(sync(&remote, &a, NOW).unwrap().pushed);
        assert_eq!(remote.uploads(), 1, "one pack, under the one id");
        assert_eq!(a.vault_id().unwrap().as_deref(), Some(ID));
        assert_eq!(remote_records(&remote), a.store.export_for_sync().unwrap());
    }

    // An import points this vault at a pack the account already holds. If that
    // pack cannot be read there is nothing to adopt: taking its id anyway would
    // leave the vault answering to a pack it has never seen a record of.
    #[test]
    fn an_import_of_a_pack_it_cannot_read_never_takes_that_packs_id() {
        let a = Device::seeded(&[record("1", 200, b"one")]);
        // Structurally a pack, but sealed by a different install.
        let foreign = KdfParams::argon2id(b"a-completely-different-salt-here", 256, 1, 1)
            .to_json()
            .unwrap();
        let remote = FakeRemote::with(pack::pack(&foreign, &[0u8; 4096]));

        let settled = AtomicBool::new(false);
        let result = super::sync(&remote, &a, NOW, || {
            settled.store(true, Ordering::SeqCst);
            a.adopt_vault_id(ID)
        });

        assert!(result.is_err());
        assert!(
            !settled.load(Ordering::SeqCst),
            "the pull never got far enough to prove the id"
        );
        assert_eq!(a.vault_id().unwrap(), None);
        assert_eq!(remote.uploads(), 0);
    }

    // The happy path, and the reason the file-name stamp on restore is now only
    // belt and braces: the snapshot is taken after the id is written, so the
    // very first pack a newly named vault pushes carries that id in its `meta`.
    #[test]
    fn the_first_pack_of_a_newly_named_vault_already_carries_its_id() {
        let a = Device::seeded(&[record("1", 200, b"one")]);
        let remote = FakeRemote::default();

        let outcome = super::sync(&remote, &a, NOW, || a.adopt_vault_id(ID)).unwrap();

        assert!(outcome.pushed);
        assert_eq!(remote.uploads(), 1);
        assert_eq!(
            a.id_when_packed.lock().unwrap().as_deref(),
            Some(ID),
            "the id was on disk by the time the snapshot was taken"
        );
    }

    // A re-pull after losing a race proves nothing new about the identity, and
    // the write has already happened — so the hook runs once per run, not once
    // per attempt.
    #[test]
    fn a_run_that_loses_a_race_settles_its_id_once_not_once_per_attempt() {
        let a = Device::seeded(&[record("1", 200, b"one")]);
        let remote = FakeRemote::default();
        sync(&remote, &a, NOW).unwrap();
        a.store.import(&[record("2", 400, b"two")]).unwrap();

        // C lands its own snapshot in the pull/push window, forcing a second pass.
        let c = Device::seeded(&[record("1", 200, b"one"), record("3", 500, b"three")]);
        remote.interlope(c.pack_bytes());

        let settles = AtomicU64::new(0);
        let outcome = super::sync(&remote, &a, NOW, || {
            settles.fetch_add(1, Ordering::SeqCst);
            a.adopt_vault_id(ID)
        })
        .unwrap();

        assert!(outcome.pushed);
        assert_eq!(settles.load(Ordering::SeqCst), 1);
    }
}
