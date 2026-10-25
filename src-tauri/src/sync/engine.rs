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

use super::marker::MarkerKey;
use super::pack::{self, PackError};
use super::Provider;
use crate::app::APP_NAME;
use crate::crypto::{KdfParams, VaultKey};
use crate::error::{Error, Result};
use crate::retired::RetiredKey;
use crate::session::{store_err, Session};
use crate::state::AppState;
use crate::storage;
use crate::store::{
    identity, vault_digest, Attachment, AuditRecord, Record, SqliteStore, StoreError, VaultStore,
};
// The broker — and the audit writer with it — is linked on desktop only (see
// `Cargo.toml`): the rows travel, merge and age out everywhere, since that is
// the store's doing, but only a desktop build has a chain of its own to note
// the cut in.
#[cfg(desktop)]
use rowel_broker::audit;

/// How long a tombstone is kept before it is reclaimed. A device offline for
/// longer than this can resurrect what it deleted; see
/// [`SqliteStore::purge_tombstones_before`] for the trade.
const TOMBSTONE_TTL_MS: i64 = 90 * 24 * 60 * 60 * 1000;

/// How long an audit row is kept before this device reclaims it from its own
/// chain. A year: long enough for "what did my agents do?" to be answerable
/// for any question asked in the ordinary course of things, short enough that
/// a busy agent's log does not grow the vault without bound. The chain says
/// where it was cut (`pruned_through`), so what is left still verifies.
const AUDIT_RETENTION_MS: i64 = 365 * 24 * 60 * 60 * 1000;

/// What a pack reclaims before the snapshot is taken: the cutoffs for the two
/// kinds of row that age out, and the instant the reclaim runs at — which is
/// what the row noting an audit prune is stamped with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reclaim {
    /// Tombstones deleted before this are reclaimed.
    pub tombstones_before: i64,
    /// Audit rows stamped before this are reclaimed, whichever writer's.
    pub audit_before: i64,
    /// When the reclaim runs: the stamp of the `pruned_through` row it writes.
    /// Read by the desktop prune alone; a mobile build writes no note.
    #[cfg_attr(not(desktop), allow(dead_code))]
    pub at_ms: i64,
}

impl Reclaim {
    /// The cutoffs for a run at `now_ms`.
    pub fn at(now_ms: i64) -> Self {
        Self {
            tombstones_before: now_ms - TOMBSTONE_TTL_MS,
            audit_before: now_ms - AUDIT_RETENTION_MS,
            at_ms: now_ms,
        }
    }
}

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
fn foreign_vault_error(provider: Provider) -> String {
    format!(
        "{} holds a vault from a different {APP_NAME} install",
        provider.display_name()
    )
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
    ///
    /// `base_revision` is the revision this run pulled (`None`: it pulled
    /// nothing). The write is conditional on it as far as the backend allows:
    /// a remote that has moved past it, or that holds a pack when the run
    /// pulled none, is an error rather than an overwrite of a write this run
    /// never merged.
    fn upload(&self, bytes: &[u8], base_revision: Option<&str>) -> Result<String>;
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
    /// Last-writer-wins merge of the pulled entries and attachments, and the
    /// pulled audit rows taken as a union; returns the entry and attachment
    /// rows written (the log rows change nothing the frontend shows).
    fn merge(&self, incoming: &Snapshot) -> Result<usize>;
    /// Fingerprint of the entry, attachment and audit tables, tombstones
    /// included.
    fn digest(&self) -> Result<[u8; 32]>;
    /// Reclaim what `reclaim` says has aged out — tombstones, and this device's
    /// own audit rows — then pack the vault for upload.
    fn pack(&self, reclaim: Reclaim) -> Result<Vec<u8>>;
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

/// A pulled pack, opened: the entries, attachments and audit rows in it and
/// the name it was pushed under.
pub struct Snapshot {
    pub records: Vec<Record>,
    pub attachments: Vec<Attachment>,
    /// Every writer's chain the pushing device held, in `(writer_id, seq)`
    /// order. Merged as a union, never last-writer-wins: a log row is a fact
    /// about what happened, and no device's copy outranks another's.
    pub audit: Vec<AuditRecord>,
    /// The name in the pack's `meta`, with the ms-epoch stamp it was set at.
    /// `(None, 0)` for a pack pushed before names travelled.
    pub name: Option<String>,
    pub name_ms: i64,
    /// The pack was sealed under a key epoch this device has retired (a slower
    /// device pushed under the password this one replaced, or an old revision
    /// was put back), and was opened with the retired key. Its rows have been
    /// carried onto the current key; the run has to push even when nothing
    /// else differs, or the account stays under the old password.
    pub opened_under_retired: bool,
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
    /// Whether the push re-sealed a pack the account held under a retired key
    /// epoch under the current one.
    pub rekeyed_remote: bool,
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
        // A pack under a retired epoch is pushed whatever the digests say: the
        // push is what moves the account onto the current password.
        let rekeys_remote = incoming.as_ref().is_some_and(|s| s.opened_under_retired);
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

        if let Some(snapshot) = incoming.as_ref().filter(|_| !rekeys_remote) {
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
                snapshot.audit.iter().map(|r| (r.id.as_str(), &r.hash)),
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
        // without any extra bookkeeping. The audit prune rides the same step
        // for the same reasons, and its `pruned_through` row goes up in this
        // very pack.
        let bytes = local.pack(Reclaim::at(now_ms))?;
        // Every pull refuses a pack past this, so one uploaded anyway could
        // never be read again by any device, this one included. The vault
        // budget (`commands::attachments`) keeps a vault well short of it;
        // this is the net for one that got there some other way.
        if bytes.len() > pack::MAX_PACK_BYTES {
            return Err(Error::Other(too_large_to_sync_error()));
        }
        // The pre-flight above narrows the race; the revision passed here
        // closes it wherever the backend can compare-and-swap, so a push that
        // lands between the head check and this write is not overwritten.
        let revision = remote.upload(&bytes, base_revision.as_deref())?;
        local.note_push(&revision, now_ms)?;
        outcome.pushed = true;
        outcome.rekeyed_remote = rekeys_remote;
        return Ok(outcome);
    }

    Err(Error::Other(
        "another device kept changing the remote vault; sync will retry later".into(),
    ))
}

/// Overwrite the remote with this vault's pack without pulling it: the
/// explicit replace a user confirms when the account holds a pack this device
/// cannot open. The write is conditional on the head revision read here, so a
/// push that lands in between is not overwritten (the call fails instead).
/// Everything only the account's copy held is lost; that is what the user
/// confirmed.
pub fn replace<R: Remote, L: LocalVault>(remote: &R, local: &L, now_ms: i64) -> Result<()> {
    let base_revision = remote.head_revision()?;
    let bytes = local.pack(Reclaim::at(now_ms))?;
    if bytes.len() > pack::MAX_PACK_BYTES {
        return Err(Error::Other(too_large_to_sync_error()));
    }
    let revision = remote.upload(&bytes, base_revision.as_deref())?;
    local.note_push(&revision, now_ms)
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
    keys: PackKeys,
    /// The audit writer whose chain this device may prune (see [`reclaim`]).
    writer_id: Option<String>,
}

impl SessionVault {
    /// Capture what a run needs from the live session. Fails if the vault is
    /// locked, or has no KDF descriptor to put in the pack header — without one
    /// a fresh install could never derive the key to open what we upload.
    ///
    /// `writer_id` is this install's audit writer id (`storage::install_id`),
    /// or `None` on a platform that writes no audit rows and so has none to
    /// reclaim.
    pub fn capture(app: &AppHandle, provider: Provider, writer_id: Option<String>) -> Result<Self> {
        let kdf_params_json = storage::read_kdf_sidecar(app)?.ok_or_else(no_descriptor)?;
        let state = app.state::<AppState>();
        let (key, material) = {
            let session = state.session.lock().unwrap();
            let key = session.key()?;
            (
                Zeroizing::new(key.sqlcipher_key().to_vec()),
                Zeroizing::new(key.biometric_material().to_vec()),
            )
        };
        // A vault with a descriptor is an Argon2id one, which is what the
        // material is read back as.
        let vault_key = VaultKey::from_material(material, true);
        let retired = crate::retired::load(
            &storage::workspace_dir(app)?,
            &vault_key,
            &crate::workspace::active_id(app),
        );
        Ok(Self {
            app: app.clone(),
            key,
            keys: PackKeys {
                vault_key,
                retired,
                kdf_params_json,
                scratch: storage::sync_scratch_dir(app)?,
                provider,
            },
            writer_id,
        })
    }

    /// The deletion-marker keys of the retired epochs: a "delete everywhere"
    /// written just before a password change reached this device is still the
    /// account's word on the vault.
    pub fn retired_marker_keys(&self) -> Vec<MarkerKey> {
        self.keys.retired_marker_keys()
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

    // Reclaim what has aged out (a few statements, under the lock) and hand
    // back a connection of the pack's own to read the snapshot through. The
    // audit prune is in here with the tombstone purge, not beside the pack: the
    // key check that follows the pack then covers it too, and a password
    // change landing mid-run cannot leave a `pruned_through` row written under
    // one key in a snapshot taken under another.
    fn reclaim_and_open(&self, reclaim: Reclaim) -> Result<SqliteStore> {
        self.with_store(|store| {
            self::reclaim(store, reclaim, self.writer_id.as_deref())?;
            crate::session::open_snapshot_source(&self.app, &self.key)
        })
    }
}

impl LocalVault for SessionVault {
    fn decode(&self, pack_bytes: &[u8]) -> Result<Snapshot> {
        self.keys.decode(pack_bytes)
    }

    fn merge(&self, incoming: &Snapshot) -> Result<usize> {
        self.with_store(|store| merge_snapshot(store, incoming))
    }

    fn digest(&self) -> Result<[u8; 32]> {
        self.with_store(digest_of)
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
    fn pack(&self, reclaim: Reclaim) -> Result<Vec<u8>> {
        let source = self.reclaim_and_open(reclaim)?;
        let bytes = self.keys.pack(&source)?;
        drop(source);
        self.check_key()?;
        Ok(bytes)
    }

    fn note_push(&self, revision: &str, at_ms: i64) -> Result<()> {
        self.with_store(|store| note_push(store, revision, at_ms))
    }

    fn vault_id(&self) -> Result<Option<String>> {
        self.with_store(vault_id_of)
    }

    fn adopt_vault_id(&self, id: &str) -> Result<()> {
        self.with_store(|store| adopt_vault_id_in(store, id))
    }

    fn name(&self) -> Result<(Option<String>, i64)> {
        self.with_store(name_of)
    }

    fn adopt_name(&self, name: &str, at_ms: i64) -> Result<()> {
        self.with_store(|store| adopt_name_in(store, name, at_ms))
    }
}

// --- the local side, over a lockbox that is not open -------------------------

/// [`LocalVault`] over a lockbox the session does not have open: its own
/// directory, opened here with `key`.
///
/// What a password change uses to re-seal the account copy of every lockbox it
/// re-keyed beside the open one (`commands::auth::rekey_beside`), and a move
/// onto the vault password the copy of the lockbox it moved. There is no
/// session to borrow the store from or to check the key against: the store is
/// this vault's own connection, and the caller holds the setup step, so nothing
/// re-keys or opens the lockbox under it for the length of the run.
pub struct WorkspaceVault {
    store: SqliteStore,
    keys: PackKeys,
    /// The audit writer whose chain this device may prune (see [`reclaim`]).
    writer_id: Option<String>,
}

impl WorkspaceVault {
    /// Open the lockbox in `dir` — `workspace_id` in the registry — with `key`.
    /// Fails as [`SessionVault::capture`] does on a lockbox with no descriptor,
    /// and as an unlock does on a key that does not open it. `writer_id` is as
    /// for [`SessionVault::capture`]: the install's, not the lockbox's — one
    /// device writes every lockbox's log under the same id.
    pub fn open(
        dir: &Path,
        workspace_id: &str,
        key: VaultKey,
        provider: Provider,
        writer_id: Option<String>,
    ) -> Result<Self> {
        let kdf_params_json = match fs::read_to_string(dir.join(storage::KDF_SIDECAR_FILE)) {
            Ok(json) => json,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(no_descriptor()),
            Err(e) => return Err(e.into()),
        };
        let store =
            SqliteStore::open(&dir.join(storage::DB_FILE), &*key.sqlcipher_key()).map_err(|e| {
                match e {
                    StoreError::WrongKey => Error::InvalidPassword,
                    StoreError::SchemaNewer => Error::VaultTooNew,
                    e => Error::Other(format!("could not open the vault: {e}")),
                }
            })?;
        let retired = crate::retired::load(dir, &key, workspace_id);
        Ok(Self {
            store,
            keys: PackKeys {
                vault_key: key,
                retired,
                kdf_params_json,
                scratch: storage::sync_scratch_dir_in(dir),
                provider,
            },
            writer_id,
        })
    }

    /// See [`SessionVault::retired_marker_keys`].
    pub fn retired_marker_keys(&self) -> Vec<MarkerKey> {
        self.keys.retired_marker_keys()
    }
}

impl LocalVault for WorkspaceVault {
    fn decode(&self, pack_bytes: &[u8]) -> Result<Snapshot> {
        self.keys.decode(pack_bytes)
    }

    fn merge(&self, incoming: &Snapshot) -> Result<usize> {
        merge_snapshot(&self.store, incoming)
    }

    fn digest(&self) -> Result<[u8; 32]> {
        digest_of(&self.store)
    }

    // The store is this run's alone, so the pack reads through it directly
    // rather than through a second connection.
    fn pack(&self, reclaim: Reclaim) -> Result<Vec<u8>> {
        self::reclaim(&self.store, reclaim, self.writer_id.as_deref())?;
        self.keys.pack(&self.store)
    }

    fn note_push(&self, revision: &str, at_ms: i64) -> Result<()> {
        note_push(&self.store, revision, at_ms)
    }

    fn vault_id(&self) -> Result<Option<String>> {
        vault_id_of(&self.store)
    }

    fn adopt_vault_id(&self, id: &str) -> Result<()> {
        adopt_vault_id_in(&self.store, id)
    }

    fn name(&self) -> Result<(Option<String>, i64)> {
        name_of(&self.store)
    }

    fn adopt_name(&self, name: &str, at_ms: i64) -> Result<()> {
        adopt_name_in(&self.store, name, at_ms)
    }
}

// --- shared implementations -------------------------------------------------

fn no_descriptor() -> Error {
    Error::Other("this vault predates the key descriptor and cannot be synced".into())
}

/// What both local vaults open and seal packs with: the key epoch the vault is
/// on and the ones it has retired, the descriptor that goes in the header, and
/// where a pulled pack is opened.
struct PackKeys {
    /// The vault's whole key, for carrying a pack opened under a retired epoch
    /// onto it.
    vault_key: VaultKey,
    /// The epochs this vault was re-keyed away from, read once per run.
    retired: Vec<RetiredKey>,
    kdf_params_json: String,
    scratch: PathBuf,
    /// Whose account the pulled pack came from, for the error that says so.
    provider: Provider,
}

impl PackKeys {
    fn decode(&self, pack_bytes: &[u8]) -> Result<Snapshot> {
        decode_pack(
            pack_bytes,
            &self.kdf_params_json,
            &self.vault_key,
            &self.retired,
            &self.scratch,
            self.provider,
        )
    }

    // `source` is a connection keyed to this epoch, reclaimed already.
    fn pack(&self, source: &SqliteStore) -> Result<Vec<u8>> {
        Ok(pack::pack_store(
            source,
            &*self.vault_key.sqlcipher_key(),
            &self.kdf_params_json,
            &self.scratch,
        )?)
    }

    fn retired_marker_keys(&self) -> Vec<MarkerKey> {
        self.retired
            .iter()
            .map(|r| r.key().delete_marker_key())
            .collect()
    }
}

fn digest_of(store: &SqliteStore) -> Result<[u8; 32]> {
    store.state_digest().map_err(store_err)
}

/// The step before every pack: reclaim the tombstones past their TTL, and the
/// audit rows past retention of *every* chain the vault holds — noted, when
/// this device has a writer id and its own rows were among them, by a
/// `pruned_through` row in its own chain (`rowel_broker::audit::prune`).
///
/// Every chain, on every platform, because the log merges as a union: a device
/// that reclaimed only its own rows would be handed them back by any peer
/// still holding them, and a device that reclaimed nothing (iOS, which writes
/// no rows) would hold the old rows forever and never match a desktop's
/// digest, so the two would push at each other for good. With every device
/// cutting every chain's aged prefix, the set converges once each peer has
/// pushed; `verify` takes a chain that starts past `seq` 1 as pruned, so a
/// chain cut here without its writer's note stays verifiable.
pub(crate) fn reclaim(
    store: &SqliteStore,
    reclaim: Reclaim,
    writer_id: Option<&str>,
) -> Result<()> {
    store
        .purge_tombstones_before(reclaim.tombstones_before)
        .map_err(store_err)?;
    prune_audit(store, reclaim, writer_id)
}

#[cfg(desktop)]
fn prune_audit(store: &SqliteStore, reclaim: Reclaim, writer_id: Option<&str>) -> Result<()> {
    match writer_id {
        Some(writer_id) => {
            audit::prune(store, writer_id, reclaim.audit_before, reclaim.at_ms)?;
        }
        None => {
            store.audit_prune(reclaim.audit_before).map_err(store_err)?;
        }
    }
    Ok(())
}

// No broker, and no writer handed in (see `sync::audit_writer_id`): the aged
// rows go, and no chain here is this device's to note the cut in.
#[cfg(not(desktop))]
fn prune_audit(store: &SqliteStore, reclaim: Reclaim, _writer_id: Option<&str>) -> Result<()> {
    store.audit_prune(reclaim.audit_before).map_err(store_err)?;
    Ok(())
}

fn vault_id_of(store: &SqliteStore) -> Result<Option<String>> {
    identity::vault_id(store).map_err(store_err)
}

fn adopt_vault_id_in(store: &SqliteStore, id: &str) -> Result<()> {
    identity::adopt_vault_id(store, id).map_err(store_err)
}

fn name_of(store: &SqliteStore) -> Result<(Option<String>, i64)> {
    identity::vault_name(store).map_err(store_err)
}

fn adopt_name_in(store: &SqliteStore, name: &str, at_ms: i64) -> Result<()> {
    identity::set_vault_name(store, name, at_ms).map_err(store_err)
}

/// Read a pulled pack, by the key epoch its header names.
///
/// - The current descriptor: opened with the current key.
/// - A descriptor in the retired list: opened with that epoch's key, and its
///   rows carried onto the current key before anything merges them, flagged so
///   the run pushes the pack back under the current epoch.
/// - Anything else is refused, and the generations say why ([`epoch_error`]).
///
/// The descriptor check is not redundant with the open below. A pack from a
/// *different* vault — a second install that ran setup instead of restoring —
/// is sealed under a key this device cannot derive, and SQLCipher reports that
/// exactly the way it reports corruption. Distinguishing them here is what
/// stops the app telling someone their remote vault is damaged when it is
/// merely someone else's, or that its password changed.
pub(crate) fn decode_pack(
    bytes: &[u8],
    local_kdf_params_json: &str,
    key: &VaultKey,
    retired: &[RetiredKey],
    scratch_dir: &Path,
    provider: Provider,
) -> Result<Snapshot> {
    let unpacked = pack::unpack(bytes).map_err(remote_pack_error)?;
    let header = unpacked.kdf_params_json.as_str();
    if header == local_kdf_params_json {
        return records_from_snapshot(&unpacked.snapshot, &*key.sqlcipher_key(), scratch_dir);
    }
    let Some(epoch) = retired.iter().find(|r| r.kdf == header) else {
        return Err(epoch_error(header, local_kdf_params_json, provider));
    };
    let old_key = epoch.key();
    let mut snapshot =
        records_from_snapshot(&unpacked.snapshot, &*old_key.sqlcipher_key(), scratch_dir)?;
    snapshot.records =
        crate::auth::reseal_records(std::mem::take(&mut snapshot.records), &old_key, key)?;
    crate::auth::reseal_attachment_blobs(&mut snapshot.attachments, &old_key, key)?;
    snapshot.opened_under_retired = true;
    Ok(snapshot)
}

/// Why a pack under a descriptor this device neither uses nor has retired
/// cannot be read, told by the key generations on either side.
///
/// - Neither has one: the vault of another install, as it has always been.
/// - The account's is the same or later: another device re-keyed (equal and
///   non-zero is two changes made concurrently), and this one has to adopt.
/// - The account's is earlier: an epoch this device retired and no longer has
///   the key of. Pushing would overwrite what is in it, so nothing is pushed.
///
/// A header that does not parse is the other install's too.
pub(crate) fn epoch_error(header: &str, local_kdf_params_json: &str, provider: Provider) -> Error {
    let Ok(remote) = KdfParams::from_json(header) else {
        return Error::Other(foreign_vault_error(provider));
    };
    let local_gen = KdfParams::from_json(local_kdf_params_json)
        .map(|p| p.gen())
        .unwrap_or(0);
    match (remote.gen(), local_gen) {
        (0, 0) => Error::Other(foreign_vault_error(provider)),
        (remote, local) if remote >= local => Error::PasswordChangedRemotely,
        _ => Error::VaultUnderRetiredPassword,
    }
}

/// Open a pulled snapshot in a throwaway database and export what syncs: its
/// records, attachments and audit rows, and the name the pushing device held.
///
/// The snapshot must land on disk before SQLCipher can read it, so it goes to a
/// uniquely named scratch file that a guard removes on every exit path. The
/// local vault is never opened, written, or replaced here: a hostile or broken
/// remote can only ever fail this function.
pub(crate) fn records_from_snapshot(
    snapshot: &[u8],
    key: &[u8],
    scratch_dir: &Path,
) -> Result<Snapshot> {
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
        audit: store.audit_all().map_err(store_err)?,
        name,
        name_ms,
        opened_under_retired: false,
    })
}

// One transaction for all three tables, so a file never lands without the
// entry state it is checked against (see `SqliteStore::merge`).
fn merge_snapshot(store: &SqliteStore, incoming: &Snapshot) -> Result<usize> {
    store
        .merge(&incoming.records, &incoming.attachments, &incoming.audit)
        .map_err(store_err)
}

// A pack this build cannot parse is corruption, except for a format stamped by
// a newer build — same distinction the store draws for its schema.
pub(crate) fn remote_pack_error(e: PackError) -> Error {
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

    // The key every device in these tests holds unless it says otherwise, and
    // the SQLCipher key that opens its database and its packs.
    fn vault_key() -> VaultKey {
        VaultKey::Argon2 {
            master: Zeroizing::new(vec![0x5a; 32]),
        }
    }

    fn key_bytes() -> [u8; 32] {
        *vault_key().sqlcipher_key()
    }

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
            doc_type: None,
            doc_country: None,
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
        // The key epoch this device is on, and the ones it has retired.
        key: VaultKey,
        kdf: String,
        retired: Vec<RetiredKey>,
        // The audit writer this device prunes for, as the production vaults
        // hold it: `None` is a device that writes no log.
        writer: Option<String>,
    }

    impl Device {
        fn new() -> Self {
            Self::keyed(vault_key(), kdf_json())
        }

        fn keyed(key: VaultKey, kdf: String) -> Self {
            let dir = tmp_dir();
            Self {
                store: SqliteStore::open(&dir.join("vault.db"), &*key.sqlcipher_key()).unwrap(),
                scratch: dir.join("scratch"),
                id_when_packed: Mutex::default(),
                oversized: AtomicBool::new(false),
                key,
                kdf,
                retired: Vec::new(),
                writer: None,
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
            pack::pack_store(
                &self.store,
                &*self.key.sqlcipher_key(),
                &self.kdf,
                &self.scratch,
            )
            .unwrap()
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
            decode_pack(
                pack_bytes,
                &self.kdf,
                &self.key,
                &self.retired,
                &self.scratch,
                Provider::GoogleDrive,
            )
        }
        fn merge(&self, incoming: &Snapshot) -> Result<usize> {
            merge_snapshot(&self.store, incoming)
        }
        fn digest(&self) -> Result<[u8; 32]> {
            self.store.state_digest().map_err(store_err)
        }
        fn pack(&self, reclaim: Reclaim) -> Result<Vec<u8>> {
            *self.id_when_packed.lock().unwrap() = self.vault_id()?;
            if self.oversized.load(Ordering::SeqCst) {
                return Ok(vec![0; pack::MAX_PACK_BYTES + 1]);
            }
            super::reclaim(&self.store, reclaim, self.writer.as_deref())?;
            Ok(pack::pack_store(
                &self.store,
                &*self.key.sqlcipher_key(),
                &self.kdf,
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
        // Landed at the start of the next upload instead: after the head
        // check has passed, in the window only the upload's own
        // compare-and-swap can see.
        upload_interlopers: Vec<Vec<u8>>,
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

        // Land `bytes` on the drive the next time the engine uploads — after
        // its head check passed, before its write.
        fn interlope_at_upload(&self, bytes: Vec<u8>) {
            self.state.lock().unwrap().upload_interlopers.push(bytes);
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

        // A compare-and-swap on the revision the run pulled, as `PackRemote`
        // over Dropbox is: a remote that moved since is an error, never an
        // overwrite.
        fn upload(&self, bytes: &[u8], base_revision: Option<&str>) -> Result<String> {
            let mut state = self.state.lock().unwrap();
            if state.refuse_uploads {
                return Err(Error::Other("the upload failed".into()));
            }
            if let Some(bytes) = state.upload_interlopers.pop() {
                state.revision += 1;
                state.content = Some(bytes);
            }
            let head = state.content.is_some().then(|| state.revision.to_string());
            if head.as_deref() != base_revision {
                return Err(Error::Other("the remote changed since it was read".into()));
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
        records_from_snapshot(&unpacked.snapshot, &key_bytes(), &tmp_dir())
            .unwrap()
            .records
    }

    // The name a drive's pack carries in its own `meta`.
    fn remote_name(remote: &FakeRemote) -> (Option<String>, i64) {
        let unpacked = pack::unpack(&remote.content().unwrap()).unwrap();
        let snapshot = records_from_snapshot(&unpacked.snapshot, &key_bytes(), &tmp_dir()).unwrap();
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
                renamed: false,
                rekeyed_remote: false,
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
                renamed: false,
                rekeyed_remote: false,
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

    // A push that lands after the head check passed is past what the
    // pre-flight can see. The upload is conditional on the revision the run
    // pulled, so it fails rather than overwrite C's work, and the next run
    // pulls C's snapshot and pushes the union.
    #[test]
    fn a_push_landing_after_the_head_check_fails_the_upload_rather_than_being_overwritten() {
        let a = Device::seeded(&[record("1", 200, b"one")]);
        let remote = FakeRemote::default();
        sync(&remote, &a, NOW).unwrap();
        a.store.import(&[record("2", 400, b"two")]).unwrap();

        let c = Device::seeded(&[record("1", 200, b"one"), record("3", 500, b"three")]);
        let theirs = c.pack_bytes();
        remote.interlope_at_upload(theirs.clone());

        assert!(sync(&remote, &a, NOW).is_err());
        assert_eq!(remote.uploads(), 1, "the second push never landed");
        assert_eq!(remote.content().unwrap(), theirs, "C's push stands");

        assert!(sync(&remote, &a, NOW).unwrap().pushed);
        let published: Vec<String> = remote_records(&remote).into_iter().map(|r| r.id).collect();
        assert_eq!(published, vec!["1", "2", "3"]);
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
            Err(Error::Other(msg)) => assert_eq!(msg, foreign_vault_error(Provider::GoogleDrive)),
            other => panic!("expected the foreign-vault error, got {other:?}"),
        }
        assert_eq!(remote.uploads(), 0);
    }

    // The sentence names the provider, in the words the Drive copy always used.
    #[test]
    fn the_foreign_vault_error_reads_as_it_always_did_for_drive() {
        assert_eq!(
            foreign_vault_error(Provider::GoogleDrive),
            format!("Google Drive holds a vault from a different {APP_NAME} install")
        );
        assert_eq!(
            foreign_vault_error(Provider::Dropbox),
            format!("Dropbox holds a vault from a different {APP_NAME} install")
        );
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
        records_from_snapshot(&unpacked.snapshot, &key_bytes(), &tmp_dir())
            .unwrap()
            .attachments
    }

    // --- the audit log -----------------------------------------------------
    //
    // Log rows ride the same pack and merge as a union; the two-device shape
    // (both ways, verification on each side) is in
    // `commands::password_sync::two_devices`. What is this engine's own is
    // retention: a device reclaims every chain's aged rows as it packs, and
    // notes the cut in its own chain alone. Desktop only, as the writer is:
    // the broker that writes rows is not linked on iOS (see `Cargo.toml`), so
    // there is nothing there to write a chain with.

    #[cfg(desktop)]
    fn remote_audit(remote: &FakeRemote) -> Vec<AuditRecord> {
        let unpacked = pack::unpack(&remote.content().unwrap()).unwrap();
        records_from_snapshot(&unpacked.snapshot, &key_bytes(), &tmp_dir())
            .unwrap()
            .audit
    }

    #[cfg(desktop)]
    fn used() -> audit::Draft {
        audit::Draft {
            item_id: Some("1".into()),
            target: Some("https://example.com/login".into()),
            ..audit::Draft::new(audit::ActorKind::Agent, audit::Event::Used)
        }
    }

    #[cfg(desktop)]
    #[test]
    fn audit_rows_past_retention_leave_the_devices_own_chain_at_pack_time() {
        let mut a = Device::seeded(&[record("1", 200, b"one")]);
        a.writer = Some("device-a".into());
        let writer = audit::Writer::new("device-a", &a.store);
        writer.record(used(), NOW - 400 * DAY_MS).unwrap();
        writer.record(used(), NOW - 370 * DAY_MS).unwrap();
        let kept = writer.record(used(), NOW - DAY_MS).unwrap();
        // Another device's chain, one row aged and one not: the aged one goes
        // too, and that chain gets no note — only its own writer could add one.
        let other = audit::Writer::new("device-b", &a.store);
        other.record(used(), NOW - 400 * DAY_MS).unwrap();
        let other_kept = other.record(used(), NOW - DAY_MS).unwrap();
        let remote = FakeRemote::default();

        assert!(sync(&remote, &a, NOW).unwrap().pushed);

        let rows = a.store.audit_rows("device-a").unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], kept);
        let note = &rows[1];
        assert_eq!((note.seq, note.ts_ms), (4, NOW));
        assert_eq!(
            (note.event.as_str(), note.actor_kind.as_str()),
            ("pruned_through", "policy")
        );
        assert_eq!(
            note.detail.as_deref(),
            Some(format!("{{\"seq\":2,\"beforeMs\":{}}}", NOW - AUDIT_RETENTION_MS).as_str())
        );
        assert_eq!(audit::verify(&rows), Ok(()));
        let others = a.store.audit_rows("device-b").unwrap();
        assert_eq!(others, vec![other_kept]);
        assert_eq!(audit::verify(&others), Ok(()));

        // The pack carries the chains as pruned, and having pruned before it
        // packed, the device has nothing more to say.
        assert_eq!(remote_audit(&remote), a.store.audit_all().unwrap());
        assert!(!sync(&remote, &a, NOW).unwrap().pushed);
    }

    // A device that writes no log (iOS) still reclaims the aged rows it holds
    // — or its digest would never again match a device that did — and, having
    // no chain of its own, writes no note.
    #[cfg(desktop)]
    #[test]
    fn a_device_without_a_writer_reclaims_aged_rows_and_notes_nothing() {
        let a = Device::seeded(&[record("1", 200, b"one")]);
        let writer = audit::Writer::new("device-b", &a.store);
        writer.record(used(), NOW - 400 * DAY_MS).unwrap();
        let kept = writer.record(used(), NOW - DAY_MS).unwrap();
        let remote = FakeRemote::default();

        sync(&remote, &a, NOW).unwrap();

        assert_eq!(a.store.audit_all().unwrap(), vec![kept.clone()]);
        assert_eq!(remote_audit(&remote), vec![kept]);
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

    // --- key epochs ---------------------------------------------------------
    //
    // A password change moves a vault to a new key epoch: a new descriptor
    // (generation + 1) and a new key. These are the runs either side of one.

    fn epoch(byte: u8, gen: u32) -> (VaultKey, String) {
        let mut params = KdfParams::argon2id(&[byte; 32], 256, 1, 1);
        for _ in 0..gen {
            params = params.next_generation();
        }
        let key = VaultKey::Argon2 {
            master: Zeroizing::new(vec![byte; 32]),
        };
        (key, params.to_json().unwrap())
    }

    // A real entry, sealed under `key` for the row it is stored in.
    fn sealed(id: &str, updated_at: i64, title: &str, key: &VaultKey) -> Record {
        let entry = crate::models::Entry {
            id: id.into(),
            title: title.into(),
            ..Default::default()
        };
        Record {
            updated_at,
            ..crate::store::migrate::build_record(
                &entry,
                key.payload_cipher().seal(&entry).unwrap(),
            )
            .unwrap()
        }
    }

    fn title_of(device: &Device, id: &str) -> String {
        let record = device.store.get(id).unwrap().unwrap();
        device
            .key
            .payload_cipher()
            .unseal(id, &record.payload)
            .unwrap()
            .title
    }

    // The account's pack is under the epoch this device just left. It opens
    // with the retired key, its rows come across onto the current key, and the
    // pack goes back up under the current epoch — even though the merge leaves
    // nothing else to say.
    #[test]
    fn a_pack_under_a_retired_epoch_is_merged_and_pushed_under_the_current_one() {
        let (old_key, old_kdf) = epoch(1, 0);
        let (new_key, new_kdf) = epoch(2, 1);
        let slow = Device::keyed(old_key, old_kdf.clone());
        slow.store
            .import(&[sealed("1", 200, "from the slow device", &slow.key)])
            .unwrap();
        let remote = FakeRemote::with(slow.pack_bytes());

        let mut changed = Device::keyed(new_key, new_kdf.clone());
        changed.retired = vec![RetiredKey {
            kdf: old_kdf,
            material: Zeroizing::new(vec![1; 32]),
            retired_at: 0,
        }];

        let outcome = sync(&remote, &changed, NOW).unwrap();

        assert_eq!(outcome.merged, 1);
        assert!(outcome.pushed && outcome.rekeyed_remote);
        assert_eq!(title_of(&changed, "1"), "from the slow device");
        let up = pack::unpack(&remote.content().unwrap()).unwrap();
        assert_eq!(
            up.kdf_params_json, new_kdf,
            "the account is on the new epoch"
        );

        // The next run finds the current epoch and has nothing to say.
        let again = sync(&remote, &changed, NOW).unwrap();
        assert!(!again.pushed && !again.rekeyed_remote);
    }

    // The forced push is about the epoch, not the contents: a retired pack
    // whose rows this device already holds byte for byte still goes back up.
    #[test]
    fn a_retired_pack_with_nothing_new_is_still_resealed() {
        let (old_key, old_kdf) = epoch(1, 0);
        let (new_key, new_kdf) = epoch(2, 1);
        let mut changed = Device::keyed(new_key, new_kdf.clone());
        changed.retired = vec![RetiredKey {
            kdf: old_kdf.clone(),
            material: Zeroizing::new(vec![1; 32]),
            retired_at: 0,
        }];
        // An empty vault on both sides: equal digests, equal (absent) names.
        let remote = FakeRemote::with(Device::keyed(old_key, old_kdf).pack_bytes());

        let outcome = sync(&remote, &changed, NOW).unwrap();

        assert_eq!(outcome.merged, 0);
        assert!(outcome.pushed && outcome.rekeyed_remote);
        assert_eq!(
            pack::unpack(&remote.content().unwrap())
                .unwrap()
                .kdf_params_json,
            new_kdf
        );
    }

    // Another device changed the password: the account is a generation ahead.
    // Nothing is merged and nothing is pushed until this one adopts it.
    #[test]
    fn a_pack_a_generation_ahead_asks_for_the_new_password() {
        let (new_key, new_kdf) = epoch(2, 1);
        let ahead = Device::keyed(new_key, new_kdf);
        ahead
            .store
            .import(&[sealed("1", 200, "new", &ahead.key)])
            .unwrap();
        let remote = FakeRemote::with(ahead.pack_bytes());
        let (old_key, old_kdf) = epoch(1, 0);
        let behind = Device::keyed(old_key, old_kdf);
        behind
            .store
            .import(&[sealed("2", 300, "unsynced edit", &behind.key)])
            .unwrap();

        assert!(matches!(
            sync(&remote, &behind, NOW),
            Err(Error::PasswordChangedRemotely)
        ));
        assert_eq!(remote.uploads(), 0);
        assert_eq!(behind.ids(), vec!["2"], "the local edit is intact");
    }

    // Two devices each changed the password once: the same generation, other
    // salts. The only useful answer is the same prompt.
    #[test]
    fn a_concurrent_change_at_the_same_generation_asks_too() {
        let mine = Device::keyed(epoch(3, 1).0, epoch(3, 1).1);
        let remote = FakeRemote::with(Device::keyed(epoch(4, 1).0, epoch(4, 1).1).pack_bytes());
        assert!(matches!(
            sync(&remote, &mine, NOW),
            Err(Error::PasswordChangedRemotely)
        ));
        assert_eq!(remote.uploads(), 0);
    }

    // The account holds an epoch this device left behind and has no key for.
    // Pushing would overwrite whatever is in it, so the run stops.
    #[test]
    fn a_pack_under_an_epoch_this_device_no_longer_holds_is_left_alone() {
        let (old_key, old_kdf) = epoch(1, 0);
        let remote = FakeRemote::with(Device::keyed(old_key, old_kdf).pack_bytes());
        let (new_key, new_kdf) = epoch(2, 1);
        let changed = Device::keyed(new_key, new_kdf);

        assert!(matches!(
            sync(&remote, &changed, NOW),
            Err(Error::VaultUnderRetiredPassword)
        ));
        assert_eq!(remote.uploads(), 0);
    }

    // Two generation-0 descriptors that differ are what they always were: two
    // installs that never shared a vault.
    #[test]
    fn two_unchanged_vaults_that_differ_are_still_foreign() {
        assert_eq!(
            epoch_error(&epoch(1, 0).1, &epoch(2, 0).1, Provider::GoogleDrive).kind(),
            "other"
        );
        assert!(matches!(
            epoch_error("not json", &epoch(2, 1).1, Provider::GoogleDrive),
            Error::Other(_)
        ));
    }
}
