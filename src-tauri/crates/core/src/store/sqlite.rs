//! SQLite + SQLCipher backend for [`VaultStore`]. A single `Mutex<Connection>`
//! guards a WAL-mode, whole-file-encrypted database. No pool, no async, no ORM.

use std::fs;
use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension, Row, Statement};
use rusqlite_migration::{Migrations, M};
use zeroize::Zeroizing;

use super::hash::{derived_rank, record_hash, state_digest};
use super::{now_ms, EntryMeta, Record, Result, StoreError, VaultStore};

// Ordered schema migrations, versioned via SQLite's `user_version` pragma. This
// is the DDL history — append (never edit) an `M::up` for each future schema
// change (Phase 2 KDF columns, Phase 3 sync columns). The `meta` k/v table holds
// APP data (KDF descriptor, biometric flag, settings), not schema versioning.
fn migrations() -> Migrations<'static> {
    Migrations::new(migration_list())
}

// After running all migrations, `user_version` equals the list length. A DB
// above that was written by a newer build — refuse with a dedicated error
// instead of letting the failure masquerade as a wrong key (see `open`).
fn schema_version() -> i64 {
    migration_list().len() as i64
}

fn migration_list() -> Vec<M<'static>> {
    vec![
        M::up(
            "CREATE TABLE meta (
           key   TEXT PRIMARY KEY,
           value TEXT
         );
         CREATE TABLE entries (
           id         TEXT PRIMARY KEY,
           kind       TEXT,
           title      TEXT,
           tags       TEXT,
           url_host   TEXT,
           created_at INTEGER,
           updated_at INTEGER,
           deleted_at INTEGER,
           payload    BLOB
         );",
        ),
        // Card network slug ("visa", …), derived from the number at save time
        // so listings show brand marks without touching the payload. NULL =
        // not yet derived (backfilled once on unlock).
        M::up("ALTER TABLE entries ADD COLUMN card_brand TEXT;"),
        // The user's star. Pre-existing rows default to unstarred, which is the
        // truth for every vault written before this column existed.
        M::up("ALTER TABLE entries ADD COLUMN favorite INTEGER NOT NULL DEFAULT 0;"),
        // Whether the entry holds a passkey, derived from the payload at save
        // time so the list can mark the row without unsealing it. Pre-existing
        // rows default to 0 and are corrected by a one-time unlock backfill —
        // there is no NULL here to mark the derivation as pending, since 0 is
        // also the truth for every entry that simply has no passkey.
        M::up("ALTER TABLE entries ADD COLUMN has_passkey INTEGER NOT NULL DEFAULT 0;"),
        // An env file's name and variable count, derived from the payload at
        // save time so the list can subtitle the row without unsealing it. Both
        // nullable like `card_brand`: NULL `var_count` = not yet derived
        // (backfilled once on unlock). NULL `file_name` is also simply the truth
        // for a pasted file, so the count is the marker the backfill reads.
        M::up(
            "ALTER TABLE entries ADD COLUMN file_name TEXT;
             ALTER TABLE entries ADD COLUMN var_count INTEGER;",
        ),
        // The website-favicon cache. It used to be loose files named after the
        // host under the app-data dir, which put the vault's host list — live
        // and deleted — in the clear beside the encrypted database, where any
        // file-level backup picked it up. In here it gets what the rest of the
        // metadata gets: SQLCipher at rest. `uri` NULL is a recorded miss (the
        // host has no icon), which is why the presence of the row and the
        // presence of an icon are two different questions; `fetched_at` is what
        // ages a miss out.
        M::up(
            "CREATE TABLE favicons (
           host       TEXT PRIMARY KEY,
           uri        TEXT,
           fetched_at INTEGER NOT NULL
         );",
        ),
        // A login's username. NULL = not yet derived (backfilled on unlock).
        M::up("ALTER TABLE entries ADD COLUMN username TEXT;"),
    ]
}

// New columns are appended last so pre-existing column indexes stay put.
const COLS: &str = "id, kind, title, tags, url_host, created_at, updated_at, deleted_at, payload, card_brand, favorite, has_passkey, file_name, var_count, username";
const META_COLS: &str = "id, kind, title, tags, url_host, created_at, updated_at, deleted_at, card_brand, favorite, has_passkey, file_name, var_count, username";

const META_UPSERT: &str = "INSERT INTO meta (key, value) VALUES (?1, ?2)
     ON CONFLICT(key) DO UPDATE SET value = excluded.value";

/// Namespace for every device-local sync bookkeeping key in `meta` (the dirty
/// flag, and the last-sync state the engine records). Grouped under one prefix
/// so a restore can scrub the lot without enumerating them — a snapshot carries
/// the *source* device's bookkeeping, which is meaningless on the target.
pub const SYNC_META_PREFIX: &str = "sync_";

pub struct SqliteStore {
    conn: Mutex<Connection>,
}

impl SqliteStore {
    /// Open (creating if absent) an encrypted DB at `path`, keyed by the raw
    /// `key` bytes. The key is used directly (no passphrase KDF); the caller
    /// derives it. Opening an existing DB with the wrong key fails here.
    pub fn open(path: &Path, key: &[u8]) -> Result<Self> {
        let existed = path.metadata().map(|m| m.len() > 0).unwrap_or(false);
        if let Some(parent) = path.parent() {
            create_private_dir(parent)?;
        }
        // Before SQLite touches the path, so the file — and the `-wal`/`-shm`
        // it will create beside it — is never on disk under the umask's mode.
        restrict(path)?;

        let mut conn = Connection::open(path)?;
        // Raw-key pragma: the hex bytes are the key, not a passphrase. Must run
        // before any other pragma that touches the (encrypted) file.
        conn.execute_batch(&key_pragma("key", key))?;
        // Connection hygiene: WAL for crash-safe per-row writes; NORMAL is the
        // durable/fast pairing for WAL; temp_store=MEMORY keeps sort/temp data
        // (plaintext metadata) off disk; busy_timeout absorbs the brief lock a
        // second connection (e.g. a snapshot) can hold; secure_delete zeroes
        // what a DELETE or a shrinking UPDATE frees, so a purged payload's
        // ciphertext does not sit in a free page until that page is reused —
        // which is what "delete forever" has to mean to anyone who later gets
        // the database key. No foreign_keys pragma: the schema has no
        // relations. SQLCipher has no such default pragmas.
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA temp_store = MEMORY;
             PRAGMA busy_timeout = 5000;
             PRAGMA secure_delete = ON;",
        )
        .map_err(|e| wrong_key_or(existed, e))?;

        // Force key verification on an existing DB (a wrong key errors only on
        // read). Same mapping as above: only SQLCipher's "not a database" is the
        // key's fault; a busy lock or an I/O error here keeps its own cause.
        if existed {
            conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| {
                r.get::<_, i64>(0)
            })
            .map_err(|e| wrong_key_or(existed, e))?;
        }

        // A vault stamped by a newer build must surface as "update the app",
        // never as a key failure — for a password manager, a fake "wrong
        // password" is the worst possible misdiagnosis.
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version > schema_version() {
            return Err(StoreError::SchemaNewer);
        }

        // Apply pending schema migrations on the decrypted DB (tracked via
        // `user_version`; idempotent — a no-op once the DB is at the latest).
        migrations()
            .to_latest(&mut conn)
            .map_err(|e| StoreError::Other(e.to_string()))?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Write a consistent, still-encrypted snapshot of the live DB to `dest`
    /// using SQLite's online-backup API. Unlike `fs::copy` of a WAL-mode file,
    /// this reads *through* the connection, so it always captures committed WAL
    /// frames a plain copy could miss (or tear). `dest` is keyed with the same
    /// `key`, so the snapshot is as encrypted as the source — never a plaintext
    /// leak — and reopens as a normal store.
    pub fn snapshot_to(&self, dest: &Path, key: &[u8]) -> Result<()> {
        restrict(dest)?;
        let mut dst = Connection::open(dest)?;
        dst.execute_batch(&key_pragma("key", key))?;
        let src = self.lock();
        let backup = rusqlite::backup::Backup::new(&src, &mut dst)?;
        backup.run_to_completion(100, std::time::Duration::from_millis(50), None)?;
        Ok(())
    }

    /// Re-encrypt the whole database in place under `new_key` (SQLCipher
    /// `PRAGMA rekey`). The open connection stays valid and keyed with the new
    /// key afterwards. Used by change-master-password after the payloads have
    /// been re-encrypted under the new app key.
    pub fn rekey(&self, new_key: &[u8]) -> Result<()> {
        let conn = self.lock();
        conn.execute_batch(&key_pragma("rekey", new_key))?;
        // `PRAGMA rekey` is an ordinary write transaction that rewrites every
        // page, so in WAL mode the re-encrypted pages land in `vault.db-wal`
        // and the main file still opens under the OLD password until something
        // checkpoints. A password change must not leave old-key pages behind in
        // either file, so fold the WAL back and truncate it here — and *check*
        // that it happened. A checkpoint this one cannot complete would leave
        // old-key pages behind while the caller went on to record the new KDF
        // params and drop its rollback snapshot, so the failure is propagated
        // and the caller (see `auth::rekey`) restores the pre-change vault.
        checkpoint_truncate(&conn)?;
        Ok(())
    }

    /// Test seam: stamp the DB as if a future build had migrated it further.
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_user_version(&self, version: i64) -> Result<()> {
        self.lock()
            .execute_batch(&format!("PRAGMA user_version = {version}"))?;
        Ok(())
    }

    /// Fetch one row's metadata by id **including tombstones**, unlike
    /// [`VaultStore::list`] which hides them. Save, restore, favorite and purge
    /// all report back the row they just wrote, and restore and purge act on
    /// rows the live read cannot see. None of them wants the payload, so this
    /// does not read it: the whole point of a metadata projection is that
    /// reporting a write never decrypts anything.
    pub fn row_meta(&self, id: &str) -> Result<Option<EntryMeta>> {
        let conn = self.lock();
        let sql = format!("SELECT {META_COLS} FROM entries WHERE id = ?1");
        Ok(conn.query_row(&sql, params![id], row_to_meta).optional()?)
    }

    /// Set the derived card-network slug without stamping `updated_at` — it is
    /// derived metadata, not a user edit (used by the one-time unlock backfill).
    pub fn set_card_brand(&self, id: &str, brand: &str) -> Result<()> {
        self.lock().execute(
            "UPDATE entries SET card_brand = ?1 WHERE id = ?2",
            params![brand, id],
        )?;
        Ok(())
    }

    /// Set the derived passkey flag without stamping `updated_at`, for the same
    /// reason [`SqliteStore::set_card_brand`] does not: it is a projection of
    /// the payload the row already holds, not a user edit.
    pub fn set_has_passkey(&self, id: &str, has_passkey: bool) -> Result<()> {
        self.lock().execute(
            "UPDATE entries SET has_passkey = ?1 WHERE id = ?2",
            params![has_passkey, id],
        )?;
        Ok(())
    }

    /// Stamp an env row's derived file name and variable count without touching
    /// `updated_at`, for the same reason [`SqliteStore::set_card_brand`] does
    /// not: the payload already holds them, this only writes them down.
    pub fn set_env_meta(&self, id: &str, file_name: Option<&str>, var_count: i64) -> Result<()> {
        self.lock().execute(
            "UPDATE entries SET file_name = ?1, var_count = ?2 WHERE id = ?3",
            params![file_name, var_count, id],
        )?;
        Ok(())
    }

    /// Stamp a login row's derived username without touching `updated_at`.
    pub fn set_username(&self, id: &str, username: &str) -> Result<()> {
        self.lock().execute(
            "UPDATE entries SET username = ?1 WHERE id = ?2",
            params![username, id],
        )?;
        Ok(())
    }

    /// Merge foreign records into the vault, last-writer-wins per id, in one
    /// transaction. Returns how many rows were written.
    ///
    /// A record wins when its `updated_at` is strictly newer, or — on an exact
    /// timestamp tie — when more of its derived columns are stamped (see
    /// [`derived_rank`]), or failing that when its [`record_hash`] sorts higher
    /// bytewise. That tie-break is the whole reason this is not a plain "newer
    /// or keep local":
    /// "keep local" is not commutative, so on a tie two devices each keep their
    /// own row, every sync sees a difference, and they ping-pong pushes forever
    /// without ever converging. Ordering by content hash makes the merge a true
    /// join — idempotent, commutative, associative — so both sides
    /// independently pick the same winner and the vaults converge. Equal
    /// timestamp *and* equal hash is the same content, so nothing is written.
    ///
    /// Tombstones take part like any other row (they are looked up, and they
    /// can win or lose), which is what makes "deleted on one device" and
    /// "edited on another" resolve by time rather than by which side ran first.
    ///
    /// The one exception is a purged shell (see [`VaultStore::purge`]), which is
    /// **absorbing**: it beats every incoming row and no incoming row can leave
    /// it. Under plain last-writer-wins a peer that edited the entry *after* the
    /// purge would hand the sealed payload straight back, so "Delete forever"
    /// would not be forever. Making the shell the top of the lattice keeps the
    /// merge a join — idempotent, commutative, associative — while making the
    /// purge final: both sides of a conflict land on the shell.
    ///
    /// Winners are written verbatim: no timestamp is stamped here, or the
    /// merged row would immediately look newer than its source everywhere else.
    pub fn merge_records(&self, recs: &[Record]) -> Result<usize> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        let mut changed = 0usize;
        {
            // Tombstones must be visible here, so this reads the raw row rather
            // than going through `get` (which hides them).
            let mut find = tx.prepare(&format!("SELECT {COLS} FROM entries WHERE id = ?1"))?;
            let mut write = tx.prepare(&verbatim_upsert())?;

            for incoming in recs {
                let local: Option<Record> = find
                    .query_row(params![incoming.id], row_to_record)
                    .optional()?;

                let wins = match &local {
                    None => true,
                    // Absorbing in both directions, so the two devices agree:
                    // a purge cannot be undone, and it always propagates.
                    Some(local) if is_purged(local) => false,
                    Some(_) if is_purged(incoming) => true,
                    Some(local) => rank(incoming) > rank(local),
                };

                if wins {
                    exec_record(&mut write, incoming)?;
                    changed += 1;
                }
            }
        }
        tx.commit()?;
        Ok(changed)
    }

    /// Fingerprint of the whole entry table, tombstones included. Two vaults
    /// hold the same entry state exactly when their digests match, which is the
    /// sync engine's push decision: push when the digests differ, full stop.
    pub fn state_digest(&self) -> Result<[u8; 32]> {
        Ok(state_digest(&self.export_for_sync()?))
    }

    /// How many live entries the vault holds: what the list shows, so the
    /// same filter as `list`, and every tombstone (archived or purged) left out.
    pub fn count_live(&self) -> Result<u32> {
        Ok(self.lock().query_row(
            "SELECT count(*) FROM entries WHERE deleted_at IS NULL",
            [],
            |r| r.get(0),
        )?)
    }

    /// Delete every `meta` row whose key starts with `prefix`, returning how
    /// many were removed.
    ///
    /// Matched with `substr`, not `LIKE`: `_` is a LIKE wildcard, so the literal
    /// prefix `sync_` would also match `syncX…` — a silent over-delete.
    pub fn meta_delete_prefix(&self, prefix: &str) -> Result<usize> {
        Ok(self.lock().execute(
            "DELETE FROM meta WHERE substr(key, 1, length(?1)) = ?1",
            params![prefix],
        )?)
    }

    /// Hard-delete tombstones deleted before `cutoff_ms`, returning the number
    /// of rows reclaimed.
    ///
    /// Tombstones cannot be kept forever, and dropping one is not free: a device
    /// that has been offline since before the cutoff still holds the row as
    /// live, sees no tombstone to tell it otherwise, and re-pushes it as a new
    /// record — the entry resurrects. A ~90-day cutoff is the trade: only a
    /// device dormant for a full quarter can resurrect anything, and the vault
    /// does not grow without bound. Deciding *when* to call this (never before
    /// the tombstone has been pushed at least once, or the delete is lost
    /// instead of propagated) belongs to the sync engine, not the store.
    pub fn purge_tombstones_before(&self, cutoff_ms: i64) -> Result<usize> {
        let conn = self.lock();
        let reclaimed = conn.execute(
            "DELETE FROM entries WHERE deleted_at IS NOT NULL AND deleted_at < ?1",
            params![cutoff_ms],
        )?;
        if reclaimed > 0 {
            drop_wal_history(&conn);
        }
        Ok(reclaimed)
    }

    /// The cached favicon for `host`, if the cache still answers for it:
    ///
    /// - `None` — nothing usable. Either the host was never looked up, or the
    ///   only thing recorded is a miss older than `miss_ttl_ms`, which is due
    ///   to be retried.
    /// - `Some(None)` — a fresh miss: the host was looked up and has no icon.
    /// - `Some(Some(uri))` — the cached icon.
    ///
    /// A hit never ages out: an icon that was right once stays rendered, and
    /// re-fetching every site's artwork on a timer would be a periodic
    /// broadcast of the vault's host list. Only misses expire, so a site that
    /// gains an icon (or was offline when we asked) is asked again. The TTL is
    /// the caller's policy, not the store's.
    pub fn get_favicon(&self, host: &str, miss_ttl_ms: i64) -> Result<Option<Option<String>>> {
        let row: Option<(Option<String>, i64)> = self
            .lock()
            .query_row(
                "SELECT uri, fetched_at FROM favicons WHERE host = ?1",
                params![host],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        Ok(match row {
            Some((Some(uri), _)) => Some(Some(uri)),
            Some((None, fetched_at)) => {
                (now_ms().saturating_sub(fetched_at) < miss_ttl_ms).then_some(None)
            }
            None => None,
        })
    }

    /// Record the lookup for `host`: its icon, or `None` for "has none".
    /// Re-stamps `fetched_at`, so a repeated miss restarts its TTL rather than
    /// being retried on every launch once the first one aged out.
    pub fn put_favicon(&self, host: &str, uri: Option<&str>) -> Result<()> {
        self.lock().execute(
            "INSERT INTO favicons (host, uri, fetched_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(host) DO UPDATE SET uri = excluded.uri, fetched_at = excluded.fetched_at",
            params![host, uri, now_ms()],
        )?;
        Ok(())
    }

    /// Test seam: put the DB back the way the build before the favicon cache
    /// left it — no `favicons` table, no columns added since, stamped at the
    /// version that preceded it — so a test can watch the migrations land on an
    /// existing vault. Keep in step with `migration_list`.
    #[cfg(test)]
    pub(crate) fn drop_favicons_for_test(&self) -> Result<()> {
        let previous = schema_version() - 2;
        self.lock().execute_batch(&format!(
            "ALTER TABLE entries DROP COLUMN username;
             DROP TABLE favicons;
             PRAGMA user_version = {previous};"
        ))?;
        Ok(())
    }

    /// Test seam: make writing one `meta` key fail, so a test can watch a batch
    /// that trips over it take the keys written before it back down.
    #[cfg(test)]
    pub(crate) fn refuse_meta_key_for_test(&self, key: &str) -> Result<()> {
        self.lock().execute_batch(&format!(
            "CREATE TRIGGER refuse_meta BEFORE INSERT ON meta WHEN NEW.key = '{key}'
             BEGIN SELECT RAISE(ABORT, 'refused'); END;"
        ))?;
        Ok(())
    }

    /// Test seam: the current value of an integer pragma.
    #[cfg(test)]
    pub(crate) fn pragma_i64(&self, name: &str) -> Result<i64> {
        Ok(self
            .lock()
            .query_row(&format!("PRAGMA {name}"), [], |r| r.get(0))?)
    }
}

/// Fold the WAL back into the main file and truncate it. `secure_delete`
/// zeroes freed content inside a page, but the page's *previous* image is
/// still a frame in the WAL until a checkpoint overwrites it, so a purge
/// checkpoints as well. Best effort: a snapshot connection holding a read
/// lock makes the checkpoint partial, and the next one finishes the job.
fn drop_wal_history(conn: &Connection) {
    let _ = checkpoint_truncate(conn);
}

/// The same checkpoint, but verified. `PRAGMA wal_checkpoint` reports a
/// blocked or partial checkpoint in its result row rather than as a statement
/// error, so running it with `execute_batch` succeeds even when nothing was
/// folded back. The row is (busy, log, checkpointed): `busy = 1` means another
/// connection's lock stopped it, and `log != checkpointed` means only part of
/// the WAL made it into the main file. `log = -1` is "not in WAL mode" — there
/// is no write-ahead log to fold, which is a complete checkpoint by definition.
fn checkpoint_truncate(conn: &Connection) -> Result<()> {
    let (busy, log, checkpointed): (i64, i64, i64) =
        conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?;
    if busy != 0 || (log >= 0 && log != checkpointed) {
        return Err(StoreError::Other(format!(
            "WAL checkpoint did not complete (busy={busy}, log={log}, \
             checkpointed={checkpointed}); the write-ahead log may still hold \
             pages of the previous database"
        )));
    }
    Ok(())
}

impl VaultStore for SqliteStore {
    fn meta_get(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .lock()
            .query_row("SELECT value FROM meta WHERE key = ?1", params![key], |r| {
                r.get(0)
            })
            .optional()?)
    }

    fn meta_set(&self, key: &str, value: &str) -> Result<()> {
        self.lock().execute(META_UPSERT, params![key, value])?;
        Ok(())
    }

    fn meta_set_many(&self, pairs: &[(&str, &str)]) -> Result<()> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        {
            let mut write = tx.prepare(META_UPSERT)?;
            for (key, value) in pairs {
                write.execute(params![key, value])?;
            }
        }
        // Returning early above drops the transaction unread, which rolls back
        // whatever of the batch had already been written.
        tx.commit()?;
        Ok(())
    }

    fn list(&self) -> Result<Vec<EntryMeta>> {
        let conn = self.lock();
        let sql = format!("SELECT {META_COLS} FROM entries WHERE deleted_at IS NULL ORDER BY id");
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], row_to_meta)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn list_deleted(&self) -> Result<Vec<EntryMeta>> {
        let conn = self.lock();
        // A purged row keeps its tombstone (sync needs it) but has no payload
        // left, so an empty payload is exactly "already permanently deleted".
        let sql = format!(
            "SELECT {META_COLS} FROM entries
             WHERE deleted_at IS NOT NULL AND length(payload) > 0
             ORDER BY deleted_at DESC, id"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], row_to_meta)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn get(&self, id: &str) -> Result<Option<Record>> {
        let conn = self.lock();
        let sql = format!("SELECT {COLS} FROM entries WHERE id = ?1 AND deleted_at IS NULL");
        Ok(conn
            .query_row(&sql, params![id], row_to_record)
            .optional()?)
    }

    fn upsert(&self, rec: &Record) -> Result<()> {
        let now = now_ms();
        let conn = self.lock();
        // created_at is set only on insert; updated_at is always stamped to now.
        // `favorite` is deliberately absent from the update set: the star is not
        // part of the entry the editor round-trips, so an ordinary save must
        // leave whatever `set_favorite` last wrote alone. `has_passkey` is the
        // opposite case and IS updated: it is derived from the payload being
        // written, so a save that drops the last passkey has to clear it.
        conn.execute(
            &format!(
                "INSERT INTO entries ({COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)
                 ON CONFLICT(id) DO UPDATE SET
                   kind=excluded.kind, title=excluded.title, tags=excluded.tags,
                   url_host=excluded.url_host, updated_at=excluded.updated_at,
                   deleted_at=excluded.deleted_at, payload=excluded.payload,
                   card_brand=excluded.card_brand, has_passkey=excluded.has_passkey,
                   file_name=excluded.file_name, var_count=excluded.var_count,
                   username=excluded.username"
            ),
            params![
                rec.id,
                rec.kind,
                rec.title,
                rec.tags,
                rec.url_host,
                if rec.created_at == 0 {
                    now
                } else {
                    rec.created_at
                },
                now,
                rec.deleted_at,
                rec.payload,
                rec.card_brand,
                rec.favorite,
                rec.has_passkey,
                rec.file_name,
                rec.var_count,
                rec.username,
            ],
        )?;
        Ok(())
    }

    fn delete(&self, id: &str) -> Result<()> {
        let now = now_ms();
        let conn = self.lock();
        conn.execute(
            "UPDATE entries SET deleted_at = ?1, updated_at = ?1 WHERE id = ?2",
            params![now, id],
        )?;
        Ok(())
    }

    fn restore(&self, id: &str) -> Result<()> {
        let now = now_ms();
        self.lock().execute(
            "UPDATE entries SET deleted_at = NULL, updated_at = ?1 WHERE id = ?2",
            params![now, id],
        )?;
        Ok(())
    }

    /// Discard a tombstoned record's contents for good.
    ///
    /// A bare `DELETE` would not survive sync: every push is the whole state and
    /// [`SqliteStore::merge_records`] re-inserts any id it does not already hold,
    /// so a peer still carrying the tombstone would hand the row — sealed payload
    /// included — straight back on the next merge. Emptying the row in place
    /// instead makes the purge *win* that merge, which destroys the payload on
    /// every device rather than only this one. What is left is a content-free
    /// tombstone, reclaimed later by [`SqliteStore::purge_tombstones_before`]
    /// like any other.
    ///
    /// The stamped `updated_at` is not what makes the purge win — that shell is
    /// absorbing in [`SqliteStore::merge_records`], so a peer's *newer* edit of
    /// the same id cannot resurrect it either. The stamp only keeps the row
    /// ordered sensibly for anything else reading timestamps.
    fn purge(&self, id: &str) -> Result<()> {
        let now = now_ms();
        let conn = self.lock();
        let purged = conn.execute(
            "UPDATE entries
             SET payload = x'', title = '', tags = '[]', url_host = '',
                 card_brand = NULL, favorite = 0, has_passkey = 0,
                 file_name = NULL, var_count = NULL, username = NULL,
                 updated_at = ?1
             WHERE id = ?2 AND deleted_at IS NOT NULL",
            params![now, id],
        )?;
        // The old payload is zeroed in place by `secure_delete`; this drops the
        // WAL frame that still holds the page as it was before the update.
        if purged > 0 {
            drop_wal_history(&conn);
        }
        Ok(())
    }

    fn set_favorite(&self, id: &str, favorite: bool) -> Result<()> {
        // `updated_at` moves because the sync merge is last-writer-wins on it: a
        // star that left the clock alone would lose to (or coin-flip against, via
        // the hash tie-break) every peer's copy and silently revert.
        let now = now_ms();
        // Tombstones are excluded like they are in `purge`: a trashed entry has
        // no star to set, and the `updated_at` bump would hand a deleted row a
        // fresh stamp for the sync merge to carry around.
        self.lock().execute(
            "UPDATE entries SET favorite = ?1, updated_at = ?2
             WHERE id = ?3 AND deleted_at IS NULL",
            params![favorite, now, id],
        )?;
        Ok(())
    }

    fn export_for_sync(&self) -> Result<Vec<Record>> {
        let conn = self.lock();
        let sql = format!("SELECT {COLS} FROM entries ORDER BY id");
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], row_to_record)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn import(&self, recs: &[Record]) -> Result<()> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        {
            // Bulk sync-in: timestamps are preserved verbatim (no stamping), and
            // every incoming row clobbers unconditionally — unlike merge_records,
            // this is a user restoring a file, not a peer proposing changes.
            let mut stmt = tx.prepare(&verbatim_upsert())?;
            for r in recs {
                exec_record(&mut stmt, r)?;
            }
        }
        tx.commit()?;
        Ok(())
    }
}

// One full-row write that keeps the record's own timestamps — the shape both
// sync-in paths (import, merge_records) need.
fn verbatim_upsert() -> String {
    format!(
        "INSERT INTO entries ({COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)
         ON CONFLICT(id) DO UPDATE SET
           kind=excluded.kind, title=excluded.title, tags=excluded.tags,
           url_host=excluded.url_host, created_at=excluded.created_at,
           updated_at=excluded.updated_at, deleted_at=excluded.deleted_at,
           payload=excluded.payload, card_brand=excluded.card_brand,
           favorite=excluded.favorite, has_passkey=excluded.has_passkey,
           file_name=excluded.file_name, var_count=excluded.var_count,
           username=excluded.username"
    )
}

// The total order the merge picks winners by; see `merge_records`.
fn rank(r: &Record) -> (i64, u8, [u8; 32]) {
    (r.updated_at, derived_rank(r), record_hash(r))
}

// A row [`VaultStore::purge`] has emptied. An ordinary tombstone keeps its
// payload (`delete` only stamps `deleted_at`), so "tombstoned and content-free"
// is exactly "permanently deleted" — the same test `list_deleted` filters on.
fn is_purged(rec: &Record) -> bool {
    rec.deleted_at.is_some() && rec.payload.is_empty()
}

fn exec_record(stmt: &mut Statement, r: &Record) -> rusqlite::Result<usize> {
    stmt.execute(params![
        r.id,
        r.kind,
        r.title,
        r.tags,
        r.url_host,
        r.created_at,
        r.updated_at,
        r.deleted_at,
        r.payload,
        r.card_brand,
        r.favorite,
        r.has_passkey,
        r.file_name,
        r.var_count,
        r.username,
    ])
}

fn row_to_record(row: &Row) -> rusqlite::Result<Record> {
    Ok(Record {
        id: row.get(0)?,
        kind: row.get(1)?,
        title: row.get(2)?,
        tags: row.get(3)?,
        url_host: row.get(4)?,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
        deleted_at: row.get(7)?,
        payload: row.get(8)?,
        card_brand: row.get(9)?,
        favorite: row.get(10)?,
        has_passkey: row.get(11)?,
        file_name: row.get(12)?,
        var_count: row.get(13)?,
        username: row.get(14)?,
    })
}

fn row_to_meta(row: &Row) -> rusqlite::Result<EntryMeta> {
    Ok(EntryMeta {
        id: row.get(0)?,
        kind: row.get(1)?,
        title: row.get(2)?,
        tags: row.get(3)?,
        url_host: row.get(4)?,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
        deleted_at: row.get(7)?,
        card_brand: row.get(8)?,
        favorite: row.get(9)?,
        has_passkey: row.get(10)?,
        file_name: row.get(11)?,
        var_count: row.get(12)?,
        username: row.get(13)?,
    })
}

// SQLCipher cannot tell a wrongly-keyed file from a corrupt one: both come back
// as SQLITE_NOTADB from the first read of the header. On a database that
// already existed, a key we chose ourselves is by far the likelier cause, so it
// is named as such — and on a fresh file the header is ours, so it is not.
// Every other failure keeps its own error and stays out of "wrong password".
fn wrong_key_or(existed: bool, e: rusqlite::Error) -> StoreError {
    let not_a_db = matches!(
        &e,
        rusqlite::Error::SqliteFailure(err, _) if err.code == rusqlite::ErrorCode::NotADatabase
    );
    if existed && not_a_db {
        StoreError::WrongKey
    } else {
        StoreError::Sqlite(e)
    }
}

// `PRAGMA key` / `PRAGMA rekey` with the raw key spelled as hex. The statement
// *is* the key in another alphabet, so it is zeroized when dropped rather than
// left on the heap for the allocator to hand out later — as is the hex it is
// assembled from.
fn key_pragma(pragma: &str, key: &[u8]) -> Zeroizing<String> {
    use std::fmt::Write;
    let mut hex = Zeroizing::new(String::with_capacity(key.len() * 2));
    for byte in key {
        // Writing into a `String` cannot fail.
        let _ = write!(hex, "{byte:02x}");
    }
    Zeroizing::new(format!("PRAGMA {pragma} = \"x'{}'\";", *hex))
}

// Make the database file owner-only *before* SQLite opens it. A path with
// nothing at it is created empty — which SQLite reads as a new database — with
// the mode already in force; a file that exists is tightened in place. The
// `-wal` and `-shm` siblings need nothing of their own: SQLite creates them
// with the main file's permissions, so they inherit these.
//
// This replaces a `chmod` that used to run after the open and the migrations,
// by which point the file (and, in WAL mode, its log) had already spent the
// whole open under the umask's mode.
#[cfg(unix)]
fn restrict(path: &Path) -> Result<()> {
    match crate::owner_only::create_new(path) {
        Ok(_created) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            set_mode(path, 0o600);
            Ok(())
        }
        Err(e) => Err(e.into()),
    }
}

/// Create `path` (and its parents) as a directory only the current user may
/// enter. The vault directory and every workspace directory are made this way.
pub fn create_private_dir(path: &Path) -> std::io::Result<()> {
    fs::create_dir_all(path)?;
    set_mode(path, 0o700);
    Ok(())
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode));
}

// TODO: tighten Windows ACLs to the current user. `owner_only.rs` already
// builds the owner-only security descriptor for plaintext exports; handing the
// vault to SQLite born with it (the way `restrict` does on Unix) wants a
// Windows machine to verify the user keeps access before it ships.
#[cfg(not(unix))]
fn restrict(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) {}

// Lives here rather than in `store::tests` because it drives a second raw
// connection at the same key, which needs this module's private helpers.
#[cfg(test)]
mod checkpoint_tests {
    use super::*;

    const KEY: &[u8] = &[0x11; 32];

    /// A checkpoint that cannot complete must fail the rekey, not be swallowed:
    /// the caller records the new KDF params and drops its rollback snapshot on
    /// success, so a silent partial checkpoint would leave a vault whose main
    /// file still opens under the previous password.
    #[test]
    fn rekey_fails_when_the_checkpoint_cannot_complete() {
        let dir = std::env::temp_dir().join(format!("rowel-checkpoint-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        create_private_dir(&dir).unwrap();
        let path = dir.join("vault.db");
        let store = SqliteStore::open(&path, KEY).unwrap();
        store.meta_set("k", "v").unwrap();
        // Report the blocked checkpoint at once instead of sitting out the
        // connection's five-second busy timeout first.
        store
            .lock()
            .execute_batch("PRAGMA busy_timeout = 0;")
            .unwrap();

        // A second connection parked in a read transaction: in WAL mode its
        // read-mark keeps the log from being reset, so the TRUNCATE checkpoint
        // reports itself busy instead of erroring out.
        let reader = Connection::open(&path).unwrap();
        reader.execute_batch(&key_pragma("key", KEY)).unwrap();
        reader
            .execute_batch("BEGIN; SELECT count(*) FROM meta;")
            .unwrap();

        let err = store.rekey(&[0x22; 32]).unwrap_err();
        assert!(
            err.to_string().contains("WAL checkpoint did not complete"),
            "expected a checkpoint failure, got: {err}"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
