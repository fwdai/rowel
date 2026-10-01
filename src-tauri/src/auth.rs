//! Master-password domain logic: the failed-unlock backoff, and the rekey saga
//! a password change runs.
//!
//! Beside `session.rs` for the same reason the vault-opening helpers are: both
//! are work, not wiring. Everything here is CPU- or disk-bound and only ever
//! runs on a blocking thread, which leaves `commands::auth` a set of thin entry
//! points that lock, hand off, and adopt the result.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::crypto::{KdfParams, PayloadCipher, VaultKey};
use crate::error::{Error, Result};
use crate::session::{backfill_derived_columns, record_kdf_meta, store_err};
use crate::state::AppState;
use crate::storage;
use crate::store::{Record, SqliteStore, StoreError, VaultStore};

// --- Failed-unlock backoff (T-AUTH-3) ---------------------------------------
//
// Defense-in-depth on top of Argon2id+SQLCipher: this only slows down repeated
// guessing *through the app*, it does not add cryptographic strength. State
// lives in a plaintext sidecar next to the DB (`storage::LOCKOUT_SIDECAR_FILE`)
// because a wrong password never opens the encrypted DB, so it cannot be kept
// in the `meta` table — the same reasoning as the KDF sidecar.
//
// First `FREE_ATTEMPTS` wrong tries are free (no delay). Every attempt beyond
// that doubles the wait: 2^(attempt - FREE_ATTEMPTS) seconds, capped at
// `MAX_DELAY_SECS`. With FREE_ATTEMPTS=3: attempt 4 -> 2s, 5 -> 4s, 6 -> 8s,
// ... 12+ -> capped at 300s (5 min).
const FREE_ATTEMPTS: u32 = 3;
const MAX_DELAY_SECS: u64 = 300;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockoutState {
    failed_attempts: u32,
    // Epoch millis; 0 means "not locked".
    pub locked_until_ms: i64,
}

impl LockoutState {
    // The workspace that is open — what an unlock throttles.
    pub fn load(app: &AppHandle) -> Result<Self> {
        Self::parse(storage::read_lockout_sidecar(app)?)
    }

    // Any workspace, by its directory. A delete proves the *target* workspace's
    // password, which may be a locked one, so its backoff is the one that counts.
    pub fn load_in(dir: &Path) -> Result<Self> {
        Self::parse(storage::read_lockout_sidecar_in(dir)?)
    }

    // Missing or unparseable sidecar reads as "no lockout" — this state is a
    // throttle, not a security boundary, so failing open here is fine.
    fn parse(json: Option<String>) -> Result<Self> {
        match json {
            Some(json) => Ok(serde_json::from_str(&json).unwrap_or_else(|e| {
                log::warn!("lockout sidecar is unreadable, resetting: {e}");
                Self::default()
            })),
            None => Ok(Self::default()),
        }
    }

    pub fn save(&self, app: &AppHandle) -> Result<()> {
        storage::write_lockout_sidecar(app, &serde_json::to_string(self)?)
    }

    pub fn save_in(&self, dir: &Path) -> Result<()> {
        storage::write_lockout_sidecar_in(dir, &serde_json::to_string(self)?)
    }
}

// The refusal owed to a caller that asked while `state` is still locked out, or
// `None` when the attempt may go ahead. Every password proof the app offers
// asks this first, so none of them is a way around the others' backoff.
pub fn locked_out(state: LockoutState, now_ms: i64) -> Option<Error> {
    (state.locked_until_ms > now_ms).then(|| Error::TooManyAttempts {
        retry_after_secs: retry_after_secs(state.locked_until_ms, now_ms),
    })
}

// One more wrong password: the state to persist, and the error the caller owes
// the user — `TooManyAttempts` once the escalation has started, so the UI can
// say how long, and a plain `InvalidPassword` while the attempts are still free.
pub fn penalize(state: LockoutState, now_ms: i64) -> (LockoutState, Error) {
    let updated = record_failed_attempt(state, now_ms);
    let refusal = locked_out(updated, now_ms).unwrap_or(Error::InvalidPassword);
    (updated, refusal)
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// Seconds remaining until `locked_until_ms`, rounded up so a caller never tells
// the UI to retry a moment too early. Callers only invoke this when locked
// (`locked_until_ms > now_ms`), so the difference is always positive.
pub fn retry_after_secs(locked_until_ms: i64, now_ms: i64) -> u64 {
    let remaining_ms = (locked_until_ms - now_ms).max(0) as u64;
    remaining_ms.div_ceil(1000)
}

// Exponential delay for the Nth failed attempt (1-indexed), 0 while still free.
fn backoff_delay_secs(failed_attempts: u32) -> u64 {
    if failed_attempts <= FREE_ATTEMPTS {
        return 0;
    }
    let exp = (failed_attempts - FREE_ATTEMPTS).min(63);
    2u64.saturating_pow(exp).min(MAX_DELAY_SECS)
}

// State transition for one more wrong password at time `now_ms`.
pub fn record_failed_attempt(mut state: LockoutState, now_ms: i64) -> LockoutState {
    state.failed_attempts += 1;
    let delay = backoff_delay_secs(state.failed_attempts);
    state.locked_until_ms = if delay > 0 {
        now_ms + delay as i64 * 1000
    } else {
        0
    };
    state
}

// --- The change-master-password saga ----------------------------------------

/// What a failed password change left behind.
///
/// The error alone is not enough for the caller: the store came *out* of the
/// session to be re-keyed, so whatever survived the rollback has to go back in
/// before the error is returned.
pub struct Rollback {
    pub error: Error,
    /// The vault, reopened under the unchanged old key. `None` when the
    /// rollback could not reopen it, or could not retire the recovery marker
    /// that would overwrite it at the next unlock; either way the session must
    /// be cleared rather than continue on state recovery is going to replace.
    pub restored: Option<(VaultKey, SqliteStore)>,
}

/// The files a password change touches and the snapshots that guard them, all
/// in one workspace directory.
///
/// Resolved once, up front, from a single workspace lookup: resolving each path
/// on demand re-reads the active workspace every time, and a switch landing
/// between two lookups would pair one vault's snapshot with another's database.
/// On plain paths (no `AppHandle`) so the saga and its recovery are testable.
pub struct RekeyPaths {
    pub db: PathBuf,
    /// The DB snapshot, whose presence is the "a change was in flight" marker.
    pub db_backup: PathBuf,
    /// Where the DB snapshot is built before it is published as `db_backup`.
    staging: PathBuf,
    pub sidecar: PathBuf,
    pub sidecar_backup: PathBuf,
}

impl RekeyPaths {
    pub fn in_dir(dir: &Path) -> Self {
        let db_backup = dir.join(storage::DB_REKEY_BACKUP_FILE);
        let mut staging = db_backup.clone().into_os_string();
        staging.push(".tmp");
        Self {
            db: dir.join(storage::DB_FILE),
            db_backup,
            staging: PathBuf::from(staging),
            sidecar: dir.join(storage::KDF_SIDECAR_FILE),
            sidecar_backup: dir.join(storage::KDF_SIDECAR_REKEY_BACKUP_FILE),
        }
    }

    /// The active workspace's paths. The lookup happens under the workspace
    /// lock so all five agree on which vault they belong to; the lock is
    /// released before any file work, per its documented discipline.
    pub fn resolve(app: &AppHandle, state: &AppState) -> Result<Self> {
        let dir = {
            let _paths = state.workspace_lock.lock().unwrap();
            storage::workspace_dir(app)?
        };
        Ok(Self::in_dir(&dir))
    }
}

/// The whole destructive half of a password change: snapshot, re-seal every
/// payload under `new_key`, re-key the database, rewrite the KDF sidecar.
/// Correct even if slow: touches every row once.
///
/// Crash-consistency: the destructive on-disk steps (import -> rekey -> sidecar)
/// are guarded by snapshots of *both* files that decide whether the vault opens —
/// the DB and its KDF sidecar — taken before the first of them. The presence of
/// the DB snapshot is itself the "a change was in flight" marker, so it is
/// published (renamed into place from a staging file, after the sidecar snapshot
/// is complete) as the last step before the sequence starts and removed as the
/// first step after it commits; [`recover_interrupted_rekey`] rolls the pair
/// back on the next unlock if a crash left it behind. An in-process failure
/// rolls back the same way, so either path leaves the vault open under the
/// unchanged current password.
///
/// The one accepted edge: a crash in the window between the sidecar write and
/// the removal of the snapshots rolls back a change that had actually completed.
/// The OLD password then works and the user repeats the change — a deliberate
/// trade for a single, simple recovery rule ("snapshot present ⇒ roll back")
/// over a commit marker nobody can test.
///
/// The invariant every exit keeps: **a store is handed back only when no marker
/// is on disk.** Recovery rolls the vault back to the snapshot at the next
/// unlock, and a snapshot that outlives a live session would take every edit
/// made in that session with it. So a marker that cannot be removed after the
/// commit undoes the change instead (see [`discard_snapshot`]), and a marker
/// that cannot be removed even then ends the session (`restored: None`) rather
/// than hand back a vault the next unlock is going to overwrite.
///
/// Blocking from end to end — a whole-vault re-seal plus two whole-file copies —
/// so it only ever runs on the blocking pool, never on a command thread. The
/// caller holds the setup step for the duration, and every unlock path takes
/// the same step, so recovery can never run against a change still in flight.
#[allow(clippy::result_large_err)]
pub fn rekey(
    store: SqliteStore,
    old_key: VaultKey,
    new_key: VaultKey,
    params: &KdfParams,
    paths: &RekeyPaths,
) -> std::result::Result<(VaultKey, SqliteStore), Rollback> {
    // Recovery point: snapshot the pre-change (old-keyed) DB and the sidecar
    // that names its key, and publish the pair.
    if let Err(error) = publish_snapshot(&store, &old_key, paths) {
        // Nothing destructive has run, so the store itself is fine to hand
        // back — but only under the invariant: whatever the failed publish left
        // behind must not stand as a recovery point over a live session.
        let _ = fs::remove_file(&paths.staging);
        let restored = match discard_snapshot(paths) {
            Ok(()) => Some((old_key, store)),
            Err(e) => {
                log::error!("a rekey snapshot could not be retired; locking the vault: {e}");
                None
            }
        };
        return Err(Rollback { error, restored });
    }

    // Destructive sequence. On any error, roll back to the snapshot.
    if let Err(error) = rekey_vault(&store, &old_key, &new_key, params, &paths.sidecar) {
        // Close the (possibly re-keyed) connection before the file it holds is
        // replaced. The current password is unchanged, so the restored pair
        // opens under the OLD key.
        drop(store);
        return Err(Rollback {
            error,
            restored: roll_back(old_key, paths),
        });
    }

    // Success: the change is committed on disk. Drop the recovery point — and
    // if the marker will not go, undo the change while nothing has been edited
    // since, instead of reporting a success the next unlock would revert.
    match discard_snapshot(paths) {
        Ok(()) => Ok((new_key, store)),
        Err(e) => {
            drop(store);
            Err(Rollback {
                error: Error::Other(format!(
                    "the password change could not be finalized and was undone; \
                     the previous password still applies ({e})"
                )),
                restored: roll_back(old_key, paths),
            })
        }
    }
}

/// The same change on a workspace that is not the open one: its own directory,
/// its own store, and no session on the other side of it.
///
/// Everything [`rekey`] guarantees holds here too — the change is atomic for
/// this workspace, and a failure leaves it on the password it had — but there
/// is nothing to hand back, so a rollback that reopened the vault only closes
/// it again. A key that does not open the database is reported as
/// [`Error::InvalidPassword`], which is what tells a caller working through
/// every workspace that this one is on a password of its own.
///
/// Blocking from end to end, like the saga it runs: callers put it on the
/// blocking pool.
pub fn rekey_workspace(
    dir: &Path,
    old_key: VaultKey,
    new_key: VaultKey,
    params: &KdfParams,
) -> Result<()> {
    let paths = RekeyPaths::in_dir(dir);
    let store = SqliteStore::open(&paths.db, &*old_key.sqlcipher_key()).map_err(|e| match e {
        StoreError::WrongKey => Error::InvalidPassword,
        StoreError::SchemaNewer => Error::VaultTooNew,
        e => Error::Other(format!("could not open the vault: {e}")),
    })?;
    rekey(store, old_key, new_key, params, &paths).map_err(|rollback| rollback.error)?;
    Ok(())
}

// Put the pre-change pair back and reopen it under the old key. The connection
// must already be closed. `None` — the session ends — when the pair could not
// be restored, or was restored but its marker could not be retired: either way
// there is no state a writable session could safely sit on top of, and the
// next unlock's recovery gets another go at the same snapshot.
fn roll_back(old_key: VaultKey, paths: &RekeyPaths) -> Option<(VaultKey, SqliteStore)> {
    match restore_rekey_backup(paths) {
        Ok(true) => reopen(paths, &old_key).map(|store| (old_key, store)),
        Ok(false) => {
            log::error!("the rekey snapshot vanished before the rollback; locking the vault");
            None
        }
        Err(e) => {
            log::error!(
                "could not roll the vault back to its rekey snapshot; locking the vault: {e}"
            );
            None
        }
    }
}

// Reopen the restored database from the paths the saga was given rather than
// from the app handle's: the vault being changed is not always the open one —
// a password change now re-keys every workspace that shares the password — and
// the two must not be resolved separately. Same shape as
// `session::open_with_key`, minus the metadata list a rollback has no use for.
fn reopen(paths: &RekeyPaths, key: &VaultKey) -> Option<SqliteStore> {
    let store = SqliteStore::open(&paths.db, &*key.sqlcipher_key()).ok()?;
    backfill_derived_columns(&store, key);
    Some(store)
}

// Build the recovery point and publish it in the one order a crash cannot
// misrepresent: the DB snapshot goes to a staging name and the sidecar snapshot
// is written whole (atomically), and only then is the DB snapshot renamed into
// place as the marker. Recovery keys off the marker alone, so a crash anywhere
// before the rename leaves at most a staging file it never looks at — never a
// half-copied "snapshot" it would copy over a healthy vault.
fn publish_snapshot(store: &SqliteStore, old_key: &VaultKey, paths: &RekeyPaths) -> Result<()> {
    let _ = fs::remove_file(&paths.staging);
    store
        .snapshot_to(&paths.staging, &*old_key.sqlcipher_key())
        .map_err(store_err)?;
    // Durable before the rename, so the marker never points at bytes that a
    // power loss could still take back. Opened for write, not read: on Windows
    // the flush behind `sync_all` (`FlushFileBuffers`) needs a write handle and
    // fails with access denied on a read-only one. `write` without `truncate`
    // leaves the bytes alone.
    fs::OpenOptions::new()
        .write(true)
        .open(&paths.staging)?
        .sync_all()?;
    // A vault created before sidecars existed has none; recovery reads the
    // absence of this snapshot as "restore to no sidecar".
    match fs::read_to_string(&paths.sidecar) {
        Ok(json) => storage::atomic_write_file(&paths.sidecar_backup, &json)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let _ = fs::remove_file(&paths.sidecar_backup);
        }
        Err(e) => return Err(e.into()),
    }
    fs::rename(&paths.staging, &paths.db_backup)?;
    Ok(())
}

// Retire the recovery point, marker first. The sidecar snapshot goes only once
// the marker is gone: recovery reads "marker without sidecar snapshot" as a
// vault that had no sidecar, and removing the sidecar snapshot from under a
// marker that stays would turn that reading into a rollback that pairs the old
// DB with the new sidecar. A marker that stays is an error the caller acts on
// (see [`rekey`]); the pair is left intact for the next unlock's recovery.
fn discard_snapshot(paths: &RekeyPaths) -> Result<()> {
    remove_marker(&paths.db_backup)?;
    let _ = fs::remove_file(&paths.sidecar_backup);
    Ok(())
}

// Removing the marker is the one file operation whose failure costs the user
// something (an undone change, or a locked vault), and the realistic cause is
// transient — a virus scanner holding a freshly written multi-megabyte file
// open on Windows. So it is retried, briefly, before it is reported. Already
// absent is success: gone is the goal.
const MARKER_REMOVE_ATTEMPTS: u32 = 5;
const MARKER_REMOVE_RETRY: std::time::Duration = std::time::Duration::from_millis(50);

fn remove_marker(marker: &Path) -> Result<()> {
    let mut attempt = 1;
    loop {
        match fs::remove_file(marker) {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) if attempt >= MARKER_REMOVE_ATTEMPTS => return Err(e.into()),
            Err(_) => {
                attempt += 1;
                std::thread::sleep(MARKER_REMOVE_RETRY);
            }
        }
    }
}

/// Roll back a master-password change a crash left half-applied, before anything
/// tries to open the vault. Called at the top of every unlock path, under the
/// same setup step a password change holds — so it never mistakes a change
/// still in flight (whose store is still open) for an interrupted one.
///
/// The rule is deliberately blunt: a leftover DB snapshot means [`rekey`] never
/// reached its cleanup, so whatever the DB and sidecar say now is discarded in
/// favour of the pre-change pair — which is the only pair guaranteed to open
/// together. Without it, a crash mid-sequence bricks the vault (a DB re-keyed
/// under params that only ever existed in memory) while a perfectly good
/// snapshot sits beside it.
///
/// One `metadata` call when there is nothing to do, which is every unlock but
/// the vanishingly rare one.
pub fn recover_interrupted_rekey(app: &AppHandle, state: &AppState) -> Result<()> {
    recover(&RekeyPaths::resolve(app, state)?)
}

/// [`recover_interrupted_rekey`] on a named workspace directory rather than the
/// active workspace's. A password change re-keys every workspace that shares the
/// password, so any of them can be the one a crash caught — and a workspace that
/// is opened from the ring rather than from a password (`workspace_select`)
/// never passes through an unlock that would have rolled it back.
///
/// Its callers take the same setup step an unlock does, for the same reason: a
/// change still in flight must never be read as an interrupted one.
pub fn recover_interrupted_rekey_in(dir: &Path) -> Result<()> {
    recover(&RekeyPaths::in_dir(dir))
}

fn recover(paths: &RekeyPaths) -> Result<()> {
    if restore_rekey_backup(paths)? {
        log::warn!(
            "rolled back an interrupted master-password change; the previous password applies"
        );
    }
    Ok(())
}

// The file-level half of the recovery. Reports whether anything was rolled
// back; the usual answer is "no", for the cost of the single `metadata` call
// below. A staging file without a marker (a crash mid-snapshot) is not a
// recovery point and is left alone; the next password change replaces it.
fn restore_rekey_backup(paths: &RekeyPaths) -> Result<bool> {
    if fs::metadata(&paths.db_backup).is_err() {
        return Ok(false);
    }
    restore_db_file(&paths.db, &paths.db_backup)?;
    // The marker is only ever published after the sidecar snapshot is complete
    // (see `publish_snapshot`), so its absence here has exactly one meaning: the
    // vault had no sidecar when the change began (legacy/dev), and the one the
    // interrupted change may have written names a key the restored DB does not
    // use. Restoring "no sidecar" is as much a part of the rollback as
    // restoring the old one.
    if paths.sidecar_backup.exists() {
        fs::copy(&paths.sidecar_backup, &paths.sidecar)?;
    } else {
        let _ = fs::remove_file(&paths.sidecar);
    }
    // Both go only once the pair is back in place: until then a second crash has
    // to find the marker still there and try again. A marker that will not go
    // is an error — the vault is on the restored pair, but must not open as a
    // writable session while recovery still has a snapshot to apply over it.
    remove_marker(&paths.db_backup)?;
    let _ = fs::remove_file(&paths.sidecar_backup);
    let _ = fs::remove_file(&paths.staging);
    Ok(true)
}

// The destructive on-disk sequence, isolated so a single `?` failure triggers the
// snapshot rollback in the caller. Sidecar (atomic) written last, after the rekey.
fn rekey_vault(
    store: &SqliteStore,
    old_key: &VaultKey,
    new_key: &VaultKey,
    params: &KdfParams,
    sidecar: &Path,
) -> Result<()> {
    let old_cipher = old_key.payload_cipher();
    let new_cipher = new_key.payload_cipher();
    // Re-seal every row's payload under the new payload key (timestamps + tombstones kept).
    let resealed: Vec<Record> = store
        .export_for_sync()
        .map_err(store_err)?
        .into_iter()
        .map(|r| reseal_record(r, &old_cipher, &new_cipher))
        .collect::<Result<_>>()?;
    store.import(&resealed).map_err(store_err)?;
    reseal_attachments(store, old_key, new_key)?;
    store.rekey(&*new_key.sqlcipher_key()).map_err(store_err)?;
    record_kdf_meta(store, params)?;
    storage::atomic_write_file(sidecar, &params.to_json()?)?;
    Ok(())
}

// Roll the DB file back to the pre-change snapshot. The connection must already
// be closed. Stale WAL/SHM sidecars are removed so they can't overlay the
// restored (old-keyed) file with new-keyed frames.
fn restore_db_file(db: &Path, backup: &Path) -> Result<()> {
    for suffix in ["-wal", "-shm"] {
        let mut sidecar = db.to_path_buf().into_os_string();
        sidecar.push(suffix);
        let _ = fs::remove_file(PathBuf::from(sidecar));
    }
    fs::copy(backup, db)?;
    Ok(())
}

// The attached files follow the payloads onto the new key: their subkey is
// derived from the same master. A purged shell has no blob to carry. A vault
// with nothing attached does nothing here — which is every legacy-keyed one,
// since those are refused attachments, so the `None` below is never reached
// with a file in hand.
fn reseal_attachments(store: &SqliteStore, old_key: &VaultKey, new_key: &VaultKey) -> Result<()> {
    let mut atts = store.export_attachments_for_sync().map_err(store_err)?;
    atts.retain(|a| !a.blob.is_empty());
    if atts.is_empty() {
        return Ok(());
    }
    let refused = || Error::Unsupported("this vault's key scheme cannot hold attachments".into());
    let old = old_key.attachment_cipher().ok_or_else(refused)?;
    let new = new_key.attachment_cipher().ok_or_else(refused)?;
    for a in &mut atts {
        let file = old.unseal(&a.meta.id, &a.blob)?;
        a.blob = new.seal(&a.meta.id, &file)?;
    }
    store.import_attachments(&atts).map_err(store_err)
}

// Unseal a record's payload under the old cipher and re-seal it under the new one,
// preserving all metadata (id/kind/title/tags/url_host/timestamps/tombstone).
fn reseal_record(mut r: Record, old: &PayloadCipher, new: &PayloadCipher) -> Result<Record> {
    let entry = old.unseal(&r.id, &r.payload)?;
    r.payload = new.seal(&entry)?;
    Ok(r)
}

#[cfg(test)]
mod lockout_tests {
    use super::*;

    // Deterministic "now" — never sleep in these tests.
    const NOW: i64 = 1_700_000_000_000;

    #[test]
    fn free_attempts_have_no_delay() {
        for n in 0..=FREE_ATTEMPTS {
            assert_eq!(backoff_delay_secs(n), 0, "attempt {n} should be free");
        }
    }

    #[test]
    fn delay_doubles_past_the_free_attempts() {
        assert_eq!(backoff_delay_secs(FREE_ATTEMPTS + 1), 2);
        assert_eq!(backoff_delay_secs(FREE_ATTEMPTS + 2), 4);
        assert_eq!(backoff_delay_secs(FREE_ATTEMPTS + 3), 8);
        assert_eq!(backoff_delay_secs(FREE_ATTEMPTS + 4), 16);
    }

    #[test]
    fn delay_is_capped_at_max_delay() {
        assert_eq!(backoff_delay_secs(FREE_ATTEMPTS + 20), MAX_DELAY_SECS);
        // Never overflows even for pathologically large attempt counts.
        assert_eq!(backoff_delay_secs(u32::MAX), MAX_DELAY_SECS);
    }

    #[test]
    fn a_free_attempt_never_locks() {
        let state = LockoutState::default();
        let after = record_failed_attempt(state, NOW);
        assert_eq!(after.failed_attempts, 1);
        assert_eq!(after.locked_until_ms, 0, "still within FREE_ATTEMPTS");
    }

    #[test]
    fn crossing_free_attempts_sets_a_lockout() {
        let mut state = LockoutState::default();
        for _ in 0..FREE_ATTEMPTS {
            state = record_failed_attempt(state, NOW);
        }
        assert_eq!(state.locked_until_ms, 0);

        // The next failure crosses the threshold and locks.
        let locked = record_failed_attempt(state, NOW);
        assert_eq!(locked.failed_attempts, FREE_ATTEMPTS + 1);
        assert_eq!(locked.locked_until_ms, NOW + 2_000);
    }

    #[test]
    fn retry_after_secs_rounds_up_to_the_next_second() {
        // 1500ms remaining must report 2s, never 1s (never tell the UI to
        // retry a moment too early).
        assert_eq!(retry_after_secs(NOW + 1_500, NOW), 2);
        assert_eq!(retry_after_secs(NOW + 2_000, NOW), 2);
        assert_eq!(retry_after_secs(NOW + 1, NOW), 1);
        assert_eq!(retry_after_secs(NOW, NOW), 0);
    }

    #[test]
    fn success_resets_lockout_state() {
        // Whatever the state was, a successful unlock always resets to the
        // zero value — mirrors the `unlock` command's success branch.
        let locked = LockoutState {
            failed_attempts: 9,
            locked_until_ms: NOW + 300_000,
        };
        assert_ne!(locked, LockoutState::default());
        let reset = LockoutState::default();
        assert_eq!(reset.failed_attempts, 0);
        assert_eq!(reset.locked_until_ms, 0);
    }

    #[test]
    fn lockout_state_serde_round_trips() {
        let state = LockoutState {
            failed_attempts: 5,
            locked_until_ms: NOW,
        };
        let json = serde_json::to_string(&state).unwrap();
        let back: LockoutState = serde_json::from_str(&json).unwrap();
        assert_eq!(state, back);
    }

    #[test]
    fn corrupt_sidecar_json_falls_back_to_default() {
        // Mirrors `LockoutState::load`'s parse-failure branch: unreadable state
        // fails open (no lockout) rather than erroring the whole unlock flow.
        let parsed: std::result::Result<LockoutState, _> = serde_json::from_str("not json");
        assert!(parsed.is_err());
    }

    // --- the policy both password proofs share ------------------------------

    #[test]
    fn a_live_wait_is_the_only_thing_that_refuses() {
        let free = LockoutState {
            failed_attempts: 2,
            locked_until_ms: 0,
        };
        assert!(locked_out(free, NOW).is_none());

        let expired = LockoutState {
            failed_attempts: 9,
            locked_until_ms: NOW - 1,
        };
        assert!(
            locked_out(expired, NOW).is_none(),
            "a wait that has run out"
        );

        let live = LockoutState {
            failed_attempts: 9,
            locked_until_ms: NOW + 4_000,
        };
        match locked_out(live, NOW) {
            Some(Error::TooManyAttempts { retry_after_secs }) => assert_eq!(retry_after_secs, 4),
            other => panic!("expected TooManyAttempts, got {other:?}"),
        }
    }

    #[test]
    fn a_free_failure_is_reported_as_a_wrong_password() {
        let (updated, refusal) = penalize(LockoutState::default(), NOW);
        assert_eq!(updated.failed_attempts, 1);
        assert!(matches!(refusal, Error::InvalidPassword));
    }

    // Past the free attempts the caller owes the wait, not a bare refusal — so
    // the UI can say how long rather than inviting an immediate retry.
    #[test]
    fn a_failure_that_escalates_is_reported_as_the_wait() {
        let mut state = LockoutState::default();
        for _ in 0..FREE_ATTEMPTS {
            state = record_failed_attempt(state, NOW);
        }
        let (updated, refusal) = penalize(state, NOW);
        assert_eq!(updated.failed_attempts, FREE_ATTEMPTS + 1);
        match refusal {
            Error::TooManyAttempts { retry_after_secs } => assert_eq!(retry_after_secs, 2),
            other => panic!("expected TooManyAttempts, got {other:?}"),
        }
    }

    // --- the sidecar on a directory of its own ------------------------------

    #[test]
    fn a_directorys_lockout_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let state = LockoutState {
            failed_attempts: 5,
            locked_until_ms: NOW,
        };
        state.save_in(dir.path()).unwrap();
        assert_eq!(LockoutState::load_in(dir.path()).unwrap(), state);
    }

    // A workspace nobody has guessed at yet, and one whose sidecar will not
    // parse, both read as "no lockout" rather than failing the command.
    #[test]
    fn a_missing_or_corrupt_sidecar_reads_as_no_lockout() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            LockoutState::load_in(dir.path()).unwrap(),
            LockoutState::default()
        );

        fs::write(dir.path().join(storage::LOCKOUT_SIDECAR_FILE), "not json").unwrap();
        assert_eq!(
            LockoutState::load_in(dir.path()).unwrap(),
            LockoutState::default()
        );
    }

    // What `workspace_delete` now does per wrong guess: the escalation persists
    // against that workspace's own directory, so the next attempt is refused
    // before any Argon2id runs.
    #[test]
    fn repeated_failures_against_a_directory_escalate() {
        let dir = tempfile::tempdir().unwrap();

        let mut state = LockoutState::load_in(dir.path()).unwrap();
        for _ in 0..=FREE_ATTEMPTS {
            let (updated, _) = penalize(state, NOW);
            updated.save_in(dir.path()).unwrap();
            state = LockoutState::load_in(dir.path()).unwrap();
        }

        assert_eq!(state.failed_attempts, FREE_ATTEMPTS + 1);
        assert!(
            locked_out(state, NOW).is_some(),
            "the next guess must be refused before the derive"
        );
    }

    // Two workspaces are throttled apart: guessing at one must not lock the
    // other, which is the whole reason the state lives beside each vault.
    #[test]
    fn one_workspaces_lockout_does_not_reach_another() {
        let guessed = tempfile::tempdir().unwrap();
        let untouched = tempfile::tempdir().unwrap();

        let (locked, _) = penalize(
            LockoutState {
                failed_attempts: FREE_ATTEMPTS,
                locked_until_ms: 0,
            },
            NOW,
        );
        locked.save_in(guessed.path()).unwrap();

        assert!(locked_out(LockoutState::load_in(guessed.path()).unwrap(), NOW).is_some());
        assert!(locked_out(LockoutState::load_in(untouched.path()).unwrap(), NOW).is_none());
    }
}

#[cfg(test)]
mod recovery_tests {
    use super::*;
    use crate::models::Entry;
    use crate::store::migrate;
    use tempfile::TempDir;

    // The paths recovery works on, in a fresh dir.
    fn paths() -> (TempDir, RekeyPaths) {
        let dir = tempfile::tempdir().unwrap();
        let paths = RekeyPaths::in_dir(dir.path());
        (dir, paths)
    }

    // A real (legacy-keyed, so no Argon2id cost) store holding one entry.
    fn store_with_one_entry(db: &Path, key: &VaultKey) -> SqliteStore {
        let entry = Entry {
            id: "1".into(),
            title: "before".into(),
            ..Default::default()
        };
        let store = SqliteStore::open(db, &*key.sqlcipher_key()).unwrap();
        let payload = key.payload_cipher().seal(&entry).unwrap();
        store
            .upsert(&migrate::build_record(&entry, payload).unwrap())
            .unwrap();
        store
    }

    fn title_under(db: &Path, key: &VaultKey) -> String {
        let store = SqliteStore::open(db, &*key.sqlcipher_key()).unwrap();
        let record = store.get("1").unwrap().unwrap();
        key.payload_cipher()
            .unseal(&record.id, &record.payload)
            .unwrap()
            .title
    }

    #[test]
    fn a_leftover_backup_pair_rolls_both_files_back() {
        let (_dir, p) = paths();
        fs::write(&p.db, "new-keyed").unwrap();
        fs::write(&p.db_backup, "old-keyed").unwrap();
        fs::write(&p.sidecar, "new-params").unwrap();
        fs::write(&p.sidecar_backup, "old-params").unwrap();
        // WAL/SHM from the interrupted run: new-keyed frames that must not
        // overlay the restored old-keyed file.
        let (wal, shm) = (p.db.with_extension("db-wal"), p.db.with_extension("db-shm"));
        fs::write(&wal, "frames").unwrap();
        fs::write(&shm, "index").unwrap();

        assert!(restore_rekey_backup(&p).unwrap());

        assert_eq!(fs::read_to_string(&p.db).unwrap(), "old-keyed");
        assert_eq!(fs::read_to_string(&p.sidecar).unwrap(), "old-params");
        assert!(!p.db_backup.exists() && !p.sidecar_backup.exists());
        assert!(!wal.exists() && !shm.exists());
    }

    #[test]
    fn a_sidecar_less_vault_rolls_back_to_no_sidecar() {
        // A legacy/dev vault had no sidecar to snapshot. The interrupted change
        // got as far as writing one, which names an Argon2id key the restored
        // (legacy-keyed) DB does not use — left in place, neither password
        // would open the vault.
        let (_dir, p) = paths();
        fs::write(&p.db, "new-keyed").unwrap();
        fs::write(&p.db_backup, "old-keyed").unwrap();
        fs::write(&p.sidecar, "new-params").unwrap();

        assert!(restore_rekey_backup(&p).unwrap());

        assert_eq!(fs::read_to_string(&p.db).unwrap(), "old-keyed");
        assert!(
            !p.sidecar.exists(),
            "the new sidecar must go with the new DB"
        );
        assert!(!p.db_backup.exists());
    }

    #[test]
    fn no_backup_is_a_no_op() {
        // The hot path: every unlock that did not follow a crashed change.
        let (_dir, p) = paths();
        fs::write(&p.db, "live").unwrap();
        fs::write(&p.sidecar, "params").unwrap();

        assert!(!restore_rekey_backup(&p).unwrap());

        assert_eq!(fs::read_to_string(&p.db).unwrap(), "live");
        assert_eq!(fs::read_to_string(&p.sidecar).unwrap(), "params");
    }

    #[test]
    fn a_crash_mid_snapshot_is_not_a_recovery_point() {
        // The process died while the DB snapshot was being built: a partial
        // staging file, no marker. The vault is untouched and healthy, and the
        // half-copied bytes must never be copied over it.
        let (_dir, p) = paths();
        fs::write(&p.db, "live").unwrap();
        fs::write(&p.sidecar, "params").unwrap();
        fs::write(&p.staging, "half a data").unwrap();
        // ...and the sidecar snapshot may or may not have landed by then.
        fs::write(&p.sidecar_backup, "params").unwrap();

        assert!(!restore_rekey_backup(&p).unwrap());

        assert_eq!(fs::read_to_string(&p.db).unwrap(), "live");
        assert_eq!(fs::read_to_string(&p.sidecar).unwrap(), "params");
    }

    #[test]
    fn a_published_snapshot_is_a_complete_pair_with_nothing_staged() {
        let (_dir, p) = paths();
        fs::write(&p.sidecar, "old-params").unwrap();
        let old = VaultKey::legacy_from_password("old-pw");
        let store = store_with_one_entry(&p.db, &old);

        publish_snapshot(&store, &old, &p).unwrap();

        assert!(!p.staging.exists(), "the staging file outlived the publish");
        assert_eq!(fs::read_to_string(&p.sidecar_backup).unwrap(), "old-params");
        assert_eq!(title_under(&p.db_backup, &old), "before");

        // A stale sidecar snapshot from an earlier change never survives into a
        // pair whose vault has no sidecar: recovery would restore it.
        fs::remove_file(&p.sidecar).unwrap();
        publish_snapshot(&store, &old, &p).unwrap();
        assert!(!p.sidecar_backup.exists());
        assert!(p.db_backup.exists());
    }

    #[test]
    fn discarding_the_snapshot_retires_the_whole_pair() {
        let (_dir, p) = paths();
        fs::write(&p.db_backup, "old-keyed").unwrap();
        fs::write(&p.sidecar_backup, "old-params").unwrap();

        discard_snapshot(&p).unwrap();

        assert!(!p.db_backup.exists() && !p.sidecar_backup.exists());
        // Idempotent: nothing to retire is not a failure.
        discard_snapshot(&p).unwrap();
    }

    // The marker is the one file whose removal failing has to be reported, not
    // shrugged off: a marker that outlives a live session rolls that session's
    // edits back at the next unlock. And the sidecar snapshot has to stay with
    // it, or recovery would read the lone marker as a sidecar-less vault.
    #[cfg(unix)]
    #[test]
    fn a_marker_that_will_not_go_is_an_error_and_keeps_its_pair() {
        use std::os::unix::fs::PermissionsExt;

        let (dir, p) = paths();
        fs::write(&p.db_backup, "old-keyed").unwrap();
        fs::write(&p.sidecar_backup, "old-params").unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o555)).unwrap();

        let result = discard_snapshot(&p);

        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();
        assert!(
            result.is_err(),
            "a read-only directory should refuse the removal"
        );
        assert!(p.db_backup.exists() && p.sidecar_backup.exists());
        // And the same marker blocks recovery from claiming the vault is clean.
        fs::write(&p.db, "new-keyed").unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o555)).unwrap();
        // The restore itself writes into the directory too, so the failure here
        // is the directory's; what matters is that it is an error, not `Ok`.
        let recovered = restore_rekey_backup(&p);
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();
        assert!(recovered.is_err());
    }

    #[test]
    fn a_crash_before_the_sidecar_write_leaves_the_old_password_working() {
        // The bricking window, end to end on a real store: payloads re-sealed
        // and the DB re-keyed, but the new params never reached disk, so they
        // died with the process. Recovery has to undo all of it.
        let (_dir, p) = paths();
        fs::write(&p.sidecar, "old-params").unwrap();
        let old = VaultKey::legacy_from_password("old-pw");
        let new = VaultKey::legacy_from_password("new-pw");

        {
            let store = store_with_one_entry(&p.db, &old);
            // The saga: snapshot the pair, re-seal every payload, re-key — then
            // "crash" instead of writing the sidecar.
            publish_snapshot(&store, &old, &p).unwrap();
            let resealed: Vec<Record> = store
                .export_for_sync()
                .unwrap()
                .into_iter()
                .map(|r| reseal_record(r, &old.payload_cipher(), &new.payload_cipher()).unwrap())
                .collect();
            store.import(&resealed).unwrap();
            store.rekey(&*new.sqlcipher_key()).unwrap();
        }

        assert!(restore_rekey_backup(&p).unwrap());

        assert_eq!(title_under(&p.db, &old), "before");
        assert_eq!(fs::read_to_string(&p.sidecar).unwrap(), "old-params");
    }

    #[test]
    fn a_failed_change_that_rolled_back_in_process_leaves_no_marker() {
        // The in-process rollback reopens the old-keyed vault and hands it back
        // as a live session. If the marker stayed, the next unlock would roll
        // back to this snapshot again and discard every edit made meanwhile.
        let (_dir, p) = paths();
        fs::write(&p.sidecar, "old-params").unwrap();
        let old = VaultKey::legacy_from_password("old-pw");
        let store = store_with_one_entry(&p.db, &old);
        publish_snapshot(&store, &old, &p).unwrap();
        drop(store);

        // The file half of `roll_back`: the pair goes back and the marker goes.
        assert!(restore_rekey_backup(&p).unwrap());

        assert!(!p.db_backup.exists() && !p.sidecar_backup.exists());
        assert!(!restore_rekey_backup(&p).unwrap());
        assert_eq!(title_under(&p.db, &old), "before");
        assert_eq!(fs::read_to_string(&p.sidecar).unwrap(), "old-params");
    }

    // Recovery on a directory rather than on the app's paths: what a switch to a
    // workspace the ring holds runs before it opens it, since a change that
    // crashed on a workspace nobody was looking at is never met by an unlock.
    #[test]
    fn a_named_directorys_interrupted_change_is_rolled_back_too() {
        let (dir, p) = paths();
        fs::write(&p.db, "new-keyed").unwrap();
        fs::write(&p.db_backup, "old-keyed").unwrap();
        fs::write(&p.sidecar, "new-params").unwrap();
        fs::write(&p.sidecar_backup, "old-params").unwrap();

        recover_interrupted_rekey_in(dir.path()).unwrap();

        assert_eq!(fs::read_to_string(&p.db).unwrap(), "old-keyed");
        assert_eq!(fs::read_to_string(&p.sidecar).unwrap(), "old-params");
        assert!(!p.db_backup.exists() && !p.sidecar_backup.exists());
    }

    // --- the change a workspace beside the open one gets ------------------------

    #[test]
    fn a_workspace_directory_changes_password_on_its_own_files() {
        let (dir, p) = paths();
        let old = VaultKey::legacy_from_password("old-pw");
        fs::write(&p.sidecar, "old-params").unwrap();
        drop(store_with_one_entry(&p.db, &old));

        // A salt of this workspace's own, and the key the new password derives
        // to under it — exactly what `commands::auth::rekey_one` builds.
        let params = KdfParams::default_argon2id();
        let new = VaultKey::Argon2 {
            master: crate::crypto::derive(b"new-pw", &params).unwrap(),
        };

        rekey_workspace(dir.path(), old, new, &params).unwrap();

        let new = VaultKey::Argon2 {
            master: crate::crypto::derive(b"new-pw", &params).unwrap(),
        };
        assert_eq!(title_under(&p.db, &new), "before");
        assert_eq!(
            fs::read_to_string(&p.sidecar).unwrap(),
            params.to_json().unwrap(),
            "the descriptor has to name the key the database now uses"
        );
        // The invariant every exit of the saga keeps: no marker on disk behind a
        // change that committed.
        assert!(!p.db_backup.exists() && !p.sidecar_backup.exists() && !p.staging.exists());
    }

    // An attached file is sealed under a subkey of the master, so a password
    // change has to carry it across like the payloads, or it would never open
    // again under the new password.
    #[test]
    fn a_password_change_carries_the_attached_files_across() {
        use crate::store::{Attachment, AttachmentMeta};

        let (dir, p) = paths();
        let old = VaultKey::Argon2 {
            master: zeroize::Zeroizing::new(vec![1u8; 32]),
        };
        fs::write(&p.sidecar, "old-params").unwrap();
        {
            let store = store_with_one_entry(&p.db, &old);
            let blob = old
                .attachment_cipher()
                .unwrap()
                .seal("f1", b"file")
                .unwrap();
            store
                .insert_attachment(&Attachment {
                    meta: AttachmentMeta {
                        id: "f1".into(),
                        entry_id: "1".into(),
                        name: "f.txt".into(),
                        mime: None,
                        size: 4,
                        created_at: 0,
                        updated_at: 0,
                        deleted_at: None,
                    },
                    blob,
                })
                .unwrap();
        }
        let params = KdfParams::argon2id(b"salt-0123456789012345", 256, 1, 1);
        let derive = || VaultKey::Argon2 {
            master: crate::crypto::derive(b"new-pw", &params).unwrap(),
        };

        rekey_workspace(dir.path(), old, derive(), &params).unwrap();

        let new = derive();
        let store = SqliteStore::open(&p.db, &*new.sqlcipher_key()).unwrap();
        let stored = store.get_attachment("f1").unwrap().unwrap();
        let file = new
            .attachment_cipher()
            .unwrap()
            .unseal("f1", &stored.blob)
            .unwrap();
        assert_eq!(&**file, b"file");
    }

    // The workspace is on a password of its own. Nothing is touched, and the
    // refusal is the one the caller reads as "leave this one alone" rather than
    // as a failure to report.
    #[test]
    fn a_key_that_does_not_open_the_workspace_changes_nothing() {
        let (dir, p) = paths();
        let old = VaultKey::legacy_from_password("old-pw");
        fs::write(&p.sidecar, "old-params").unwrap();
        drop(store_with_one_entry(&p.db, &old));

        let params = KdfParams::default_argon2id();
        let refusal = rekey_workspace(
            dir.path(),
            VaultKey::legacy_from_password("someone-elses-pw"),
            VaultKey::legacy_from_password("new-pw"),
            &params,
        );

        assert!(matches!(refusal, Err(Error::InvalidPassword)));
        assert_eq!(title_under(&p.db, &old), "before");
        assert_eq!(fs::read_to_string(&p.sidecar).unwrap(), "old-params");
        assert!(!p.db_backup.exists());
    }
}
