//! The unlocked vault: the in-memory session, and the vault-opening helpers
//! every command reaches for.

use tauri::{AppHandle, Manager};

use crate::crypto::{self, Cryptor, KdfParams, PayloadCipher, VaultKey};
use crate::error::{Error, Result};
use crate::events;
use crate::models::EntryMetaDto;
use crate::state::AppState;
use crate::storage;
use crate::store::{EntryMeta, Record, SqliteStore, StoreError, VaultStore};

/// Which session the vault is in.
///
/// Advanced every time the key changes hands — an unlock, a lock, a password
/// change, a workspace switch, a whole-vault operation taking the store out —
/// so work prepared against one session can tell, at the moment it writes,
/// that the session it was prepared for is no longer the one in front of it.
/// The alternative, holding the session mutex from preparation to write, is
/// what stalled every single-row command behind an import or a rekey.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Epoch(u64);

/// The key and store, out of the session for a whole-vault operation (a
/// password change, a workspace being created beside it).
///
/// While a lease is out the session reads as locked to every other command
/// and as *live* to the auto-lock (`Session::is_live`), so a blur still arms
/// the timer and a lock still ends the session. Handing the lease back —
/// [`Session::restore`] as it was, or [`Session::adopt`] as what it became —
/// only succeeds if nothing ended the session in between; otherwise the vault
/// stays as the lock left it and the key is dropped, not re-installed.
pub struct Lease {
    pub key: VaultKey,
    pub store: SqliteStore,
    sync_configured: bool,
    claim: Claim,
}

/// Proof that a lease was taken from a particular session, kept by the
/// operation while the key and store themselves are away being worked on.
pub struct Claim {
    epoch: Epoch,
}

impl Lease {
    /// The key and store to work on, and the claim to hand the result back with.
    pub fn split(self) -> (VaultKey, SqliteStore, bool, Claim) {
        (self.key, self.store, self.sync_configured, self.claim)
    }
}

// In-memory session. The vault key never leaves Rust; the frontend only ever
// receives non-secret entry metadata for the list and one decrypted entry at a
// time (reveal). The open, encrypted store handle lives here — not a decrypted
// vault — so plaintext secrets are never all held in memory.
//
// The key and store are private on purpose: the only way to take them out of a
// live session is a [`Lease`], which is what lets a lock that lands while they
// are away win over the operation that took them.
#[derive(Default)]
pub struct Session {
    // The active vault key (Argon2id master or legacy secret). It owns its own
    // zeroize-on-drop, so the material is scrubbed on lock/clear/replace.
    key: Option<VaultKey>,
    // The open SQLCipher store. Dropped (connection closed) on lock.
    store: Option<SqliteStore>,
    pub sync_configured: bool,
    epoch: u64,
    // A lease is out. Reads as locked to commands, as live to the auto-lock.
    held_out: bool,
}

impl Session {
    pub fn is_unlocked(&self) -> bool {
        self.key.is_some()
    }

    /// Unlocked, or held out by an operation that means to hand it back: there
    /// is a session to lock. What the auto-lock asks, so a blur during a rekey
    /// still arms the timer, and a timer that fires still ends the session the
    /// rekey would otherwise have restored past its timeout.
    pub fn is_live(&self) -> bool {
        self.key.is_some() || self.held_out
    }

    pub fn epoch(&self) -> Epoch {
        Epoch(self.epoch)
    }

    // The key is changing hands: whatever was prepared against the old session
    // is stale, and any lease out is orphaned.
    fn advance(&mut self) {
        self.epoch += 1;
        self.held_out = false;
    }

    // The held vault key, or fail if locked.
    pub fn key(&self) -> Result<&VaultKey> {
        self.key.as_ref().ok_or(Error::Locked)
    }

    // The per-entry payload cipher for this session, or fail if locked.
    pub fn payload_cipher(&self) -> Result<PayloadCipher> {
        Ok(self.key()?.payload_cipher())
    }

    // The legacy Cryptor. Live: it seals the Drive token file, the shares this
    // vault publishes, and the `.swftx` backups it reads.
    pub fn cryptor(&self) -> Result<Cryptor> {
        Ok(self.key()?.cryptor())
    }

    // Borrow the open store, or fail if locked.
    pub fn store(&self) -> Result<&SqliteStore> {
        self.store.as_ref().ok_or(Error::Locked)
    }

    /// The open store, provided this is still the session `epoch` was read
    /// from. What a write that was prepared outside the lock — records sealed
    /// under a cipher captured earlier — asks for: a password change that
    /// landed in between re-keyed the vault, and rows sealed under the old key
    /// would be accepted by the new store and never open again.
    pub fn store_at(&self, epoch: Epoch) -> Result<&SqliteStore> {
        if self.epoch() != epoch {
            return Err(Error::StaleSession);
        }
        self.store()
    }

    // Adopt the derived key and open store for this session.
    pub fn set(&mut self, key: VaultKey, store: SqliteStore, sync_configured: bool) {
        self.advance();
        self.key = Some(key);
        self.store = Some(store);
        self.sync_configured = sync_configured;
    }

    // Drop the in-memory key and close the store. Every lock path ends here,
    // including one that lands while a lease is out: the lease then has nothing
    // to come back to.
    pub fn clear(&mut self) {
        self.advance();
        self.key = None;
        self.store = None;
        self.sync_configured = false;
    }

    /// Take the key and store out for a whole-vault operation. Fails if locked.
    pub fn take_out(&mut self) -> Result<Lease> {
        let key = self.key.take().ok_or(Error::Locked)?;
        let store = self.store.take().ok_or(Error::Locked)?;
        let sync_configured = self.sync_configured;
        self.advance();
        self.held_out = true;
        Ok(Lease {
            key,
            store,
            sync_configured,
            claim: Claim {
                epoch: self.epoch(),
            },
        })
    }

    /// Install `key` and `store` as the continuation of the session `claim` was
    /// taken from — a rekeyed vault, or a new one created beside it. Only if
    /// nothing ended that session in between: a lock that landed while the
    /// lease was out wins, the material is dropped here, and the vault stays
    /// locked. Returns whether it was adopted.
    pub fn adopt(
        &mut self,
        claim: Claim,
        key: VaultKey,
        store: SqliteStore,
        sync_configured: bool,
    ) -> bool {
        if !self.held_out || self.epoch() != claim.epoch {
            return false;
        }
        self.held_out = false;
        self.key = Some(key);
        self.store = Some(store);
        self.sync_configured = sync_configured;
        true
    }

    /// Put a lease back exactly as it was taken, under the same rule as `adopt`.
    pub fn restore(&mut self, lease: Lease) -> bool {
        let (key, store, sync_configured, claim) = lease.split();
        self.adopt(claim, key, store, sync_configured)
    }
}

/// End the session and say so, if there is one to end.
///
/// Sealing the vault and announcing it are one act: the `lock` command, the
/// inactivity auto-lock, the tray, the E2E reset and a failed rekey's rollback
/// all end here, and a workspace switch emits the same event once its paths
/// have moved. So `vault:locked` is the one signal the frontend reacts to.
/// Before this, only the auto-lock announced itself and each of the others left
/// the webview to guess it had happened — which is how the same "drop
/// everything and show the lock screen" ended up written three times over
/// there.
///
/// The check and the clear happen under one guard. Checked first and cleared
/// later, an idle timer coming due in the gap between an unlock's check and
/// its `set` would clear the session that unlock had just opened and send the
/// user straight back to the lock screen. Returns whether anything was sealed;
/// a vault already locked has nothing to announce.
pub fn lock(app: &AppHandle) -> bool {
    let state = app.state::<AppState>();
    let workspace = crate::workspace::active_id(app);
    let mut session = state.session.lock().unwrap();
    let live = session.is_live();
    // The size the vault is left at, for the lock screen to show while it is
    // sealed — read while the store is still open, recorded once the session
    // guard is let go (the registry's lock is taken before it, never inside).
    // A store out on a lease has nothing to count, and keeps its last one.
    let count = session
        .store()
        .ok()
        .and_then(|store| store.count_live().ok());
    if live {
        session.clear();
    }
    // The app-level unlock ends with every lock, whichever workspace was open
    // — and whether or not one was: a ring kept across a switch to a workspace
    // it did not hold (`workspace_select`) is a vault open at this level with
    // no session behind it, and a lock has to end that too.
    //
    // Cleared under the session guard, not after it. Everything that puts a
    // key into the ring, or opens a vault from one, takes the session lock
    // first and reads the ring under it (`appkey`, `workspace_select`) — so
    // this either runs wholly before such a step, which then finds the ring
    // empty and stands down, or wholly after it, which it then ends.
    let held = state.keyring.lock().unwrap().clear();
    drop(session);
    if let Some(count) = count {
        crate::workspace::record_item_count(app, &workspace, count);
    }
    if !live && !held {
        return false;
    }
    sealed(app);
    events::vault_locked(app);
    true
}

/// What follows every clear of a session, however it was ended — here, or by
/// a workspace switch that clears and repoints in one step. A secret copied
/// out of the vault does not outlive the vault being open, and the idle timer
/// armed for this session must not come due inside the next one. And a
/// browser extension that was filling from it is told it has gone.
pub fn sealed(app: &AppHandle) {
    crate::commands::clipboard::clear_on_lock(app);
    crate::autolock::disarm(app);
    #[cfg(desktop)]
    crate::browser::server::notify_locked();
}

pub fn store_err(e: StoreError) -> Error {
    Error::Other(e.to_string())
}

// Resolve the vault key for `password`: Argon2id when the KDF sidecar is present
// (the current scheme), or the legacy deterministic key when a DB exists without
// a sidecar (interim/dev vaults created before this wiring). Feeds the password
// **directly** to Argon2id — no `hash_secret` pre-hash on this path.
pub fn derive_key(app: &AppHandle, password: &str) -> Result<VaultKey> {
    match storage::read_kdf_sidecar(app)? {
        Some(json) => {
            let params = KdfParams::from_json(&json)?;
            Ok(VaultKey::Argon2 {
                master: crypto::derive(password.as_bytes(), &params)?,
            })
        }
        None => Ok(VaultKey::legacy_from_password(password)),
    }
}

// Open the existing store with `key` and return its handle + entry metadata.
// Only the store's own key-verification failure is reported as a wrong
// password.
pub fn open_with_key(app: &AppHandle, key: &VaultKey) -> Result<(SqliteStore, Vec<EntryMetaDto>)> {
    // An open fails for reasons that have nothing to do with the key — an I/O
    // error, a truncated file, a lock that outlasts `busy_timeout`, a
    // permissions problem — and `unlock` throttles on `InvalidPassword`. Saying
    // "wrong password" to someone whose disk is failing both misdiagnoses it
    // and locks them out for guessing right, so each cause keeps its own name.
    let store =
        SqliteStore::open(&storage::db_path(app)?, &*key.sqlcipher_key()).map_err(|e| match e {
            StoreError::WrongKey => Error::InvalidPassword,
            StoreError::SchemaNewer => Error::VaultTooNew,
            e => Error::Other(format!("could not open the vault: {e}")),
        })?;
    backfill_derived_columns(&store, key);
    let metas = list_metas(&store)?;
    // Every open lands here — password, biometrics, a switch the ring allows —
    // so the lock screen knows the size of any vault opened on this device.
    // No lock held: this runs on the opening thread, ahead of the session.
    crate::workspace::record_item_count(app, &crate::workspace::active_id(app), metas.len() as u32);
    Ok((store, metas))
}

// Open a second connection to the live database, keyed the same way, for one
// long read.
//
// The session's own store may not be borrowed for the length of a pack: a pack
// copies the whole database page by page, and holding the session mutex for that
// blocks every command that only wants a single row — which is what made
// `reveal_entry` stall behind a sync. SQLCipher is happy with a second reader
// (the database is in WAL mode), so the pack reads through one of its own and
// the guard is released the moment this returns.
//
// The caller took the key from the session under the lock, so the connection is
// keyed to the vault as it was at that instant; a re-key landing afterwards
// fails this connection's reads rather than producing a snapshot nobody can
// open.
pub fn open_snapshot_source(app: &AppHandle, key: &[u8]) -> Result<SqliteStore> {
    SqliteStore::open(&storage::db_path(app)?, key).map_err(store_err)
}

// Derivation for the metadata columns added after a row was written: unseal each
// candidate once and stamp what its payload says. Best-effort — a failure just
// leaves the row for the next unlock.
//
// `card_brand` is a candidate while NULL, so that work stops for good once every
// card is stamped. `has_passkey` has no such marker — the column is NOT NULL
// DEFAULT 0, and 0 is also the truth for every login that simply has no passkey
// — so an unflagged login is re-checked on each unlock. That is the same set of
// payloads the post-unlock audit unseals anyway, and it is self-healing: a row
// arriving from a peer still on an older build gets corrected here too.
//
// An env row's `file_name` and `var_count` follow the `card_brand` shape: the
// count is `Some` for every stamped env entry, so a NULL count is the marker,
// and a file that simply has no name stays NULL there without re-running.
// A login's `username` follows the same shape.
pub(crate) fn backfill_derived_columns(store: &SqliteStore, key: &VaultKey) {
    let Ok(metas) = store.list() else { return };
    let cipher = key.payload_cipher();
    for meta in metas {
        let brand_missing = meta.kind == "card" && meta.card_brand.is_none();
        let passkey_unflagged = meta.kind == "login" && !meta.has_passkey;
        let env_unstamped = meta.kind == "env" && meta.var_count.is_none();
        let username_unstamped = meta.kind == "login" && meta.username.is_none();
        if !brand_missing && !passkey_unflagged && !env_unstamped && !username_unstamped {
            continue;
        }
        let Ok(Some(record)) = store.get(&meta.id) else {
            continue;
        };
        let Ok(entry) = cipher.unseal(&record.id, &record.payload) else {
            continue;
        };
        if brand_missing {
            if let Some(brand) = crate::store::migrate::derived_card_brand(&entry) {
                let _ = store.set_card_brand(&meta.id, &brand);
            }
        }
        // Only ever raised here: a stale `true` cannot survive, since every
        // `upsert` recomputes the flag from the payload it is writing.
        if passkey_unflagged && crate::store::migrate::derived_has_passkey(&entry) {
            let _ = store.set_has_passkey(&meta.id, true);
        }
        if env_unstamped {
            if let Some(count) = crate::store::migrate::derived_var_count(&entry) {
                let name = crate::store::migrate::derived_file_name(&entry);
                let _ = store.set_env_meta(&meta.id, name.as_deref(), count);
            }
        }
        if username_unstamped {
            if let Some(username) = crate::store::migrate::derived_username(&entry) {
                let _ = store.set_username(&meta.id, &username);
            }
        }
    }
}

// Password unlock (run inside spawn_blocking): derive the key, then open + list.
// Fresh-start default: a legacy `vault.swftx` is never migrated here — importing
// one is an explicit, off-thread action (see `import_swftx`).
pub fn unlock_with_password(
    app: &AppHandle,
    password: &str,
) -> Result<(VaultKey, SqliteStore, Vec<EntryMetaDto>)> {
    let key = derive_key(app, password)?;
    let (store, metas) = open_with_key(app, &key)?;
    Ok((key, store, metas))
}

// Create a brand-new Argon2id-keyed vault: fresh params -> write the sidecar
// (authoritative) -> derive the master -> create the encrypted DB keyed with the
// SQLCipher subkey -> record the KDF descriptor in `meta` too (for reference).
pub fn create_vault(app: &AppHandle, password: &str) -> Result<(VaultKey, SqliteStore)> {
    let params = KdfParams::default_argon2id();
    let descriptor = params.to_json()?;
    // Sidecar first: a DB must never exist without the descriptor needed to open it.
    storage::write_kdf_sidecar(app, &descriptor)?;
    let key = VaultKey::Argon2 {
        master: crypto::derive(password.as_bytes(), &params)?,
    };
    let store =
        SqliteStore::open(&storage::db_path(app)?, &*key.sqlcipher_key()).map_err(store_err)?;
    record_kdf_meta(&store, &params)?;
    // Born with an identity: what names this vault's pack on Drive, wherever
    // it is later synced from (`store::identity`).
    crate::store::identity::assign_vault_id(&store).map_err(store_err)?;
    Ok((key, store))
}

// Mirror the KDF descriptor into `meta` (the sidecar stays authoritative for
// opening; `meta` is reference/integrity only).
pub fn record_kdf_meta(store: &SqliteStore, params: &KdfParams) -> Result<()> {
    store.meta_set("kdf", params.algo()).map_err(store_err)?;
    store
        .meta_set("kdf_params", &params.to_json()?)
        .map_err(store_err)?;
    Ok(())
}

// Look one row's metadata up by id (used by the commands that must report back
// the row they just wrote). Tombstones are visible here: restore and purge both
// need to read a row the live `get` hides.
pub fn meta_dto_of(store: &SqliteStore, id: &str) -> Result<EntryMetaDto> {
    store
        .row_meta(id)
        .map_err(store_err)?
        .map(|m| EntryMetaDto::from(&m))
        .ok_or(Error::NotFound)
}

pub fn list_metas(store: &SqliteStore) -> Result<Vec<EntryMetaDto>> {
    Ok(metas(store.list().map_err(store_err)?))
}

// Tombstoned entries' metadata — what the Trash lists.
pub fn list_deleted_metas(store: &SqliteStore) -> Result<Vec<EntryMetaDto>> {
    Ok(metas(store.list_deleted().map_err(store_err)?))
}

fn metas(rows: Vec<EntryMeta>) -> Vec<EntryMetaDto> {
    rows.iter().map(EntryMetaDto::from).collect()
}

// All live records (payloads included, tombstones excluded).
pub fn live_records(store: &SqliteStore) -> Result<Vec<Record>> {
    Ok(store
        .export_for_sync()
        .map_err(store_err)?
        .into_iter()
        .filter(|r| r.deleted_at.is_none())
        .collect())
}

/// Where the session guard may not be held — asserted on the source itself.
///
/// The rule this module exists for: [`Session`]'s mutex is held around memory
/// only, never across a KDF, a pack, a network call or a modal prompt. Every
/// command in this app shares that one lock, so a guard alive across any of them
/// stalls all of them — that is what made `reveal_entry` queue behind a sync's
/// VACUUM and behind Argon2id on an export.
///
/// Nothing in the type system says so: a guard is just a value, and holding one
/// too long is a shape, not an error. So these are tripwires rather than proofs.
/// They read the source of the four functions that had it wrong and assert the
/// order their statements are in, so re-introducing the old shape fails the
/// build with the reason attached.
#[cfg(test)]
mod lock_scope {
    // Everything above the test module: the fakes below it borrow nothing from
    // the real session and would only confuse the search.
    fn production(source: &str) -> &str {
        match source.find("#[cfg(test)]") {
            Some(at) => &source[..at],
            None => source,
        }
    }

    // The brace-matched body of the function `signature` introduces, with
    // full-line comments dropped so prose about a lock never reads as one.
    fn body_of(source: &str, signature: &str) -> String {
        let source = production(source);
        let start = source
            .find(signature)
            .unwrap_or_else(|| panic!("`{signature}` is gone — keep this guard in step with it"));
        let open = start + source[start..].find('{').expect("a function has a body");
        let mut depth = 0usize;
        let mut end = None;
        for (i, c) in source[open..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(open + i);
                        break;
                    }
                }
                _ => {}
            }
        }
        source[open..=end.expect("balanced braces")]
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    // Where `needle` occurs in `body`, or a failure naming what went missing.
    fn at(body: &str, needle: &str) -> usize {
        body.find(needle)
            .unwrap_or_else(|| panic!("`{needle}` is gone — keep this guard in step with the code"))
    }

    #[test]
    fn the_sync_pack_does_not_borrow_the_session_store() {
        let engine = include_str!("sync/engine.rs");
        // The trailing brace tells the implementation apart from the trait
        // method it implements, which is a declaration and has no body.
        let pack = body_of(
            engine,
            "fn pack(&self, cutoff_ms: i64) -> Result<Vec<u8>> {",
        );
        at(&pack, "pack_store");
        assert!(
            !pack.contains("with_store") && !pack.contains("with_session"),
            "the pack copies the whole database; it must not run inside the \
             session guard — take the lock for the reclaim and again for the \
             key re-check instead"
        );

        // ...and the step that *does* take the lock stays short.
        let reclaim = body_of(engine, "fn reclaim_and_open(&self, cutoff_ms: i64)");
        at(&reclaim, "with_store");
        assert!(
            !reclaim.contains("pack_store"),
            "the reclaim runs under the guard, so the pack may not move into it"
        );
    }

    #[test]
    fn a_password_change_derives_and_re_keys_outside_the_guard() {
        let body = body_of(
            include_str!("commands/auth.rs"),
            "pub async fn change_master_password(",
        );
        assert!(
            at(&body, "derive_both") < at(&body, "session.lock()"),
            "both Argon2id derives belong before the lock is taken"
        );
        assert!(
            at(&body, "session.lock()") < at(&body, "blocking("),
            "the lock is taken to check the key and take the store out, and \
             released before the saga runs"
        );
        assert!(
            at(&body, "blocking(") < at(&body, "auth::rekey"),
            "the rekey saga runs on the blocking pool, not on a worker"
        );
    }

    #[test]
    fn the_vault_export_derives_and_packs_outside_the_guard() {
        let body = body_of(
            include_str!("commands/vault.rs"),
            "pub async fn export_vault(",
        );
        assert!(
            at(&body, "derive_key") < at(&body, "session.lock()"),
            "the export's Argon2id derive belongs before the lock is taken"
        );
        assert!(
            at(&body, "session.lock()") < at(&body, "pack_store"),
            "the lock is taken to check the key and open a connection of the \
             pack's own; the pack itself runs after it is released"
        );
        at(&body, "open_snapshot_source");
    }

    #[test]
    fn the_entry_export_unseals_from_a_cipher_clone() {
        let import = include_str!("commands/import.rs");
        // The helper takes an owned cipher, which is what lets the unseal leave
        // the guard's scope with the records.
        assert!(
            production(import)
                .contains("fn to_imported(records: &[Record], cipher: &PayloadCipher)"),
            "the export helper must take a cipher, not a session or a guard"
        );

        let body = body_of(import, "pub async fn export_entries(");
        assert!(
            at(&body, "session.lock()") < at(&body, "super::blocking("),
            "the rows and the cipher come out under the lock; the unseal does not"
        );
        assert!(
            at(&body, "super::blocking(") < at(&body, "to_imported"),
            "unsealing every entry is a pass over the whole vault — off the worker"
        );
    }
}

// The rule every whole-vault operation relies on: a lock that lands while the
// key is out wins, and a write prepared against an earlier session is refused.
#[cfg(test)]
mod epoch_tests {
    use super::*;

    fn key(password: &str) -> VaultKey {
        VaultKey::legacy_from_password(password)
    }

    fn store_in(dir: &tempfile::TempDir, key: &VaultKey) -> SqliteStore {
        SqliteStore::open(&dir.path().join("vault.db"), &*key.sqlcipher_key()).unwrap()
    }

    fn unlocked(dir: &tempfile::TempDir) -> Session {
        let key = key("first");
        let store = store_in(dir, &key);
        let mut session = Session::default();
        session.set(key, store, true);
        session
    }

    #[test]
    fn an_undisturbed_lease_comes_back() {
        let dir = tempfile::tempdir().unwrap();
        let mut session = unlocked(&dir);
        let before = session.epoch();

        let lease = session.take_out().unwrap();
        assert!(
            !session.is_unlocked(),
            "reads as locked while the key is out"
        );
        assert!(session.is_live(), "but as live to the auto-lock");
        assert!(matches!(session.store(), Err(Error::Locked)));

        assert!(session.restore(lease));
        assert!(session.is_unlocked());
        assert!(session.sync_configured, "restored as it was");
        assert_ne!(
            session.epoch(),
            before,
            "taking the key out is a change of hands"
        );
    }

    #[test]
    fn a_lock_while_the_key_is_out_wins() {
        let dir = tempfile::tempdir().unwrap();
        let mut session = unlocked(&dir);
        let lease = session.take_out().unwrap();

        // The user, or the auto-lock, ends the session meanwhile.
        session.clear();
        assert!(!session.is_live());

        assert!(!session.restore(lease), "nothing to come back to");
        assert!(!session.is_unlocked());
        assert!(!session.is_live());
    }

    #[test]
    fn a_rekeyed_vault_continues_the_session_only_if_it_is_still_there() {
        let dir = tempfile::tempdir().unwrap();
        let mut session = unlocked(&dir);
        let (_, store, sync_configured, claim) = session.take_out().unwrap().split();
        let rekeyed = key("second");

        assert!(session.adopt(claim, rekeyed, store, sync_configured));
        assert_eq!(
            session.key().unwrap().sqlcipher_key(),
            key("second").sqlcipher_key()
        );

        let (_, store, sync_configured, claim) = session.take_out().unwrap().split();
        session.clear();
        assert!(!session.adopt(claim, key("third"), store, sync_configured));
        assert!(!session.is_unlocked());
    }

    #[test]
    fn a_write_prepared_against_an_earlier_session_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let mut session = unlocked(&dir);
        let prepared = session.epoch();
        assert!(session.store_at(prepared).is_ok());

        // A password change: the key goes out and a new one comes back.
        let (_, store, sync_configured, claim) = session.take_out().unwrap().split();
        session.adopt(claim, key("second"), store, sync_configured);

        assert!(matches!(
            session.store_at(prepared),
            Err(Error::StaleSession)
        ));
        assert!(session.store_at(session.epoch()).is_ok());

        // A lock and a fresh unlock are a new session too. (A different vault:
        // the file above is still keyed with "first", and SQLCipher will not
        // open it under another key.)
        let prepared = session.epoch();
        session.clear();
        let other = tempfile::tempdir().unwrap();
        let key = key("first");
        let store = store_in(&other, &key);
        session.set(key, store, false);
        assert!(matches!(
            session.store_at(prepared),
            Err(Error::StaleSession)
        ));
    }
}
