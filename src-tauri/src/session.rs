//! The unlocked vault: the in-memory session, and the vault-opening helpers
//! every command reaches for.

use std::sync::Mutex;

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
///
/// It is also how the webview tells the order of an open and a lock apart:
/// every unlock result says which session it opened (`UnlockResult::session`)
/// and every `vault:locked` the epoch the lock left (`events::vault_locked`).
/// The response and the event travel apart and can land in either order; a
/// result older than a lock the webview has heard of is not entered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Epoch(u64);

impl From<Epoch> for u64 {
    fn from(epoch: Epoch) -> u64 {
        epoch.0
    }
}

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

/// Taken before the slow part of anything that opens a session from locked —
/// an unlock deriving its key, a first run creating or restoring its vault, a
/// switch opening the next workspace — and presented to [`Session::open`].
///
/// Every lock advances the epoch, whether or not it found anything open
/// ([`lock`]), so an open whose ticket predates a lock is refused. Without
/// it a lock that landed while the key was still being derived — the OS
/// locking a second after the password was submitted — found nothing to
/// seal and left no trace, and the unlock then opened the vault behind it.
/// The same rule as a lease's [`Claim`], for an open that starts from nothing.
#[derive(Clone, Copy, Debug)]
pub struct Ticket {
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
    // iOS: the app is in the background, and the store is to hold no
    // connection between uses (`set_background`). About the app, not the
    // session: a lock and an unlock leave it as it is.
    backgrounded: bool,
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

    /// A lease is out: the key and store are away with a whole-vault
    /// operation, to be handed back (`adopt`, `restore`). What the iOS
    /// background asks, since a store that is away cannot be parked until it
    /// comes back. Only iOS has a caller (`background`).
    #[cfg_attr(not(target_os = "ios"), allow(dead_code))]
    pub fn is_held_out(&self) -> bool {
        self.held_out
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

    /// What an open about to start its slow part takes (see [`Ticket`]).
    pub fn ticket(&self) -> Ticket {
        Ticket {
            epoch: self.epoch(),
        }
    }

    /// Install the derived key and open store as a new session — unless the
    /// vault was locked, or changed hands, since `ticket` was taken. Then the
    /// lock wins: the key and store are dropped here and the vault stays as
    /// the lock left it. The session it opened, for the result to carry
    /// (see [`Epoch`]); `None` when it did not, and an open that did not must
    /// not go on as if it had.
    ///
    /// The state change alone: [`open`] is the open, with what it tells.
    #[must_use]
    fn open(
        &mut self,
        ticket: Ticket,
        key: VaultKey,
        store: SqliteStore,
        sync_configured: bool,
    ) -> Option<Epoch> {
        if self.epoch() != ticket.epoch {
            return None;
        }
        self.install(key, store, sync_configured);
        Some(self.epoch())
    }

    /// [`open`](Self::open) with nothing to race: a test's vault, set up
    /// before anything could lock it. Production code opens with a ticket.
    #[cfg(test)]
    pub fn set(&mut self, key: VaultKey, store: SqliteStore, sync_configured: bool) {
        self.install(key, store, sync_configured);
    }

    fn install(&mut self, key: VaultKey, store: SqliteStore, sync_configured: bool) {
        self.advance();
        self.key = Some(key);
        self.store = Some(store);
        self.sync_configured = sync_configured;
        self.place();
    }

    /// iOS: the app went to the background (`on`) or came back. In the
    /// background the store keeps no connection between uses
    /// (`SqliteStore::park`), so a suspended process holds no lock on a file
    /// in the App Group container — which iOS ends it for (`0xdead10cc`),
    /// and would again whenever the AutoFill extension opened the vault
    /// behind it. The key stays: the vault is as unlocked as it was, and the
    /// auto-lock's clock is what ends that. A store that arrives while
    /// backgrounded — an unlock finishing as the user swipes away, a lease
    /// coming back — is parked as it arrives (`place`).
    ///
    /// Only iOS has a caller (`background`); the rule itself is platform-free
    /// and tested on every one.
    #[cfg_attr(not(target_os = "ios"), allow(dead_code))]
    pub fn set_background(&mut self, on: bool) {
        self.backgrounded = on;
        self.place();
    }

    // The held store, parked or not as the background state says.
    fn place(&self) {
        let (Some(key), Some(store)) = (&self.key, &self.store) else {
            return;
        };
        if self.backgrounded {
            store.park(&*key.sqlcipher_key());
        } else {
            store.unpark();
        }
    }

    // Drop the in-memory key and close the store. Every lock path ends here,
    // including one that lands while a lease is out: the lease then has nothing
    // to come back to. The state change alone: [`lock`] and [`end_in`] are the
    // ends, with what they tell.
    fn clear(&mut self) {
        self.advance();
        self.key = None;
        self.store = None;
        self.sync_configured = false;
    }

    /// [`clear`](Self::clear), for a test elsewhere standing in for a lock.
    #[cfg(test)]
    pub fn clear_for_test(&mut self) {
        self.clear();
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
        self.place();
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
/// The check and the clear happen under one guard ([`seal_in`]). Checked first
/// and cleared later, an idle timer coming due in the gap between an unlock's
/// check and its open would clear the session that unlock had just opened and
/// send the user straight back to the lock screen. Returns whether anything
/// was sealed; a vault already locked has nothing to announce — but the lock
/// is still recorded, so an unlock on its way in is refused (see [`Ticket`]).
pub fn lock(app: &AppHandle) -> bool {
    end(app, false, |_| true)
}

/// The lock the user asked for (the `lock` command): announced even when there
/// was nothing left to seal — the user asked for the lock screen, and a webview
/// that somehow still shows the vault gets it.
pub fn lock_on_request(app: &AppHandle) {
    end(app, true, |_| true);
}

/// A lock decided earlier and taken now — the idle timer's deadline coming due,
/// a return to a window left for longer than the timeout — taken only if that
/// decision still holds ([`lock_applies`]). Whether it sealed.
///
/// [`lock`] is a lock *now*, whatever is open, and it is recorded even with
/// nothing open, so an unlock on its way in loses to it (`Ticket`): the OS
/// locking, the user asking. A decision made earlier is not that. It may
/// have been overtaken by the time it runs — activity re-armed the clock, a
/// session opened — and then it must lock nothing, and leave no trace for
/// an unlock on its way in to be refused by.
pub fn lock_if(app: &AppHandle, still: impl FnOnce(&Session) -> bool) -> bool {
    let state = app.state::<AppState>();
    end(app, false, |session| {
        lock_applies(session, &state.keyring, still)
    })
}

/// Whether a lock decided earlier applies, asked under the session guard
/// (`session`) — where every open, every end and every arming of the idle
/// clock is made, so nothing can change between this answer and the seal it
/// allows. `still` is the decision's own question, and it is always asked,
/// so what it consumes is consumed either way (a blur, say); it applies only
/// over something open to seal: a session, or a ring of keys.
pub(crate) fn lock_applies(
    session: &Session,
    keyring: &Mutex<crate::appkey::Keyring>,
    still: impl FnOnce(&Session) -> bool,
) -> bool {
    let holds = still(session);
    let open = session.is_live() || !keyring.lock().unwrap().is_empty();
    holds && open
}

// `applies`, asked under the guard before anything is sealed: always yes for
// a lock now, the decision's own question for a lock decided earlier.
fn end(app: &AppHandle, announce: bool, applies: impl FnOnce(&Session) -> bool) -> bool {
    let workspace = crate::workspace::active_id(app);
    let state = app.state::<AppState>();
    let (sealed, ended) = {
        let mut session = state.session.lock().unwrap();
        if !applies(&session) {
            return false;
        }
        let sealed = seal_in(&mut session, &state.keyring);
        let ended = (sealed.live || sealed.held).then(|| announce_ended(app));
        (sealed, ended)
    };
    if let Some(count) = sealed.count {
        crate::workspace::record_item_count(app, &workspace, count);
    }
    let was_open = ended.is_some();
    if let Some(ended) = ended {
        ended.finish(app);
    }
    // With the epoch read under the guard that sealed: read any later, it
    // could be an open's that landed since, and the webview would take the
    // result of that open for one this lock overtook.
    if was_open || announce {
        events::vault_locked(app, sealed.epoch);
    }
    was_open
}

/// Open a session from locked: the key and store installed — unless a lock
/// landed since `ticket` was taken ([`Ticket`]) — and, under the same guard,
/// everything that follows the session told ([`announce_opened`]). The only
/// way a session opens; the session it opened, for the result to carry.
#[must_use]
pub fn open(
    app: &AppHandle,
    ticket: Ticket,
    key: VaultKey,
    store: SqliteStore,
    sync_configured: bool,
) -> Option<Epoch> {
    let state = app.state::<AppState>();
    let mut session = state.session.lock().unwrap();
    let opened = session.open(ticket, key, store, sync_configured)?;
    announce_opened(app);
    Some(opened)
}

/// Continue the session a lease was taken from with another vault — a
/// workspace created or restored beside the one that was open — told as an
/// open, since it is a different vault now. `None` when a lock landed while
/// the lease was out: the lock wins (`Session::adopt`).
#[must_use]
pub fn continue_lease(
    app: &AppHandle,
    claim: Claim,
    key: VaultKey,
    store: SqliteStore,
    sync_configured: bool,
) -> Option<Epoch> {
    let state = app.state::<AppState>();
    let mut session = state.session.lock().unwrap();
    if !session.adopt(claim, key, store, sync_configured) {
        return None;
    }
    announce_opened(app);
    Some(session.epoch())
}

/// End the session under the caller's guard — a switch, a deletion, which
/// read and change more under it — and tell it ([`announce_ended`]). What is
/// left for after the guard comes back; the caller finishes it.
pub fn end_in(app: &AppHandle, session: &mut Session) -> Ended {
    session.clear();
    announce_ended(app)
}

/// Say `vault:locked` for a session ended other than by [`lock`] — a switch
/// landing on the lock screen, a deletion, an open its ticket refused — at
/// the epoch the session stands at now. Only under a step (`begin_step`):
/// every open holds one too, so none can land between that end and this, and
/// the epoch said is never a later open's.
pub fn announce_locked(app: &AppHandle) {
    let epoch = app.state::<AppState>().session.lock().unwrap().epoch();
    events::vault_locked(app, epoch);
}

/// The answer of an open its ticket refused: the vault is locked, by the lock
/// that landed while the open was on its way. That lock has already said so,
/// or — with nothing open to seal — had nothing to say.
pub fn locked_meanwhile() -> Error {
    log::info!("the vault was locked while it was being opened; it stays locked");
    Error::Locked
}

/// What a lock found under the session guard.
struct Sealed {
    /// A session was open, or out on a lease.
    live: bool,
    /// The ring held keys: the app was open at its own level.
    held: bool,
    /// The size the vault was left at, for the lock screen to show.
    count: Option<u32>,
    /// Where the lock left the session: later than every session opened
    /// before it, earlier than every one opened after.
    epoch: Epoch,
}

/// The state change of a lock, under the session guard the caller holds: the
/// session and the ring cleared, in one step.
fn seal_in(session: &mut Session, keyring: &Mutex<crate::appkey::Keyring>) -> Sealed {
    let live = session.is_live();
    // The size the vault is left at, for the lock screen to show while it is
    // sealed — read while the store is still open, recorded once the session
    // guard is let go (the registry's lock is taken before it, never inside).
    // A store out on a lease has nothing to count, and keeps its last one.
    let count = session
        .store()
        .ok()
        .and_then(|store| store.count_live().ok());
    // Open or not: clearing advances the epoch, and that is the lock's record.
    // An unlock still deriving its key when the lock lands — the OS locking a
    // moment after the password went in — has nothing here to seal yet, and
    // its ticket is what finds out it lost (`Session::open`).
    session.clear();
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
    let held = keyring.lock().unwrap().clear();
    Sealed {
        live,
        held,
        count,
        epoch: session.epoch(),
    }
}

/// [`seal_in`] for a test, with the guard taken here.
#[cfg(test)]
fn seal(state: &AppState) -> Sealed {
    seal_in(&mut state.session.lock().unwrap(), &state.keyring)
}

// What a change of session tells everything that has to follow it: the idle
// timer, the browser extensions (`database-unlocked`, `database-locked`), the
// approvals a Rowel Server routes here, the agents' grants and the clipboard.
//
// Told under the guard that made the change, as part of it. Told after the
// guard was let go, as it used to be, two changes could be told in the other
// order: a lock landing a moment after an open told the extensions
// `database-locked`, and the open's late `database-unlocked` followed it over
// a vault that was locked; a late disarm could stop the next session's idle
// clock. Under the guard the telling is in the order of the changes, because
// the changes are. So it is memory only — a signal queued, a flag set, a
// timer re-pointed, a list emptied — and nothing here takes the session lock
// (the agents' lock comes after it, as everywhere). What is not memory, the
// clipboard, is left for after the guard, bounded by what was decided under
// it (`Ended`).

/// A session opened, or another vault took its place.
fn announce_opened(app: &AppHandle) {
    crate::autolock::arm(app);
    #[cfg(desktop)]
    {
        crate::browser::server::hub(app).notify_unlocked();
        crate::org::approvals::start(app);
    }
}

/// A session ended. A secret copied out of the vault does not outlive it; the
/// idle timer armed for it must not come due inside the next one; a browser
/// extension filling from it is told it has gone; the approvals a Rowel Server
/// routes here stop, and the grants agents held — and the grant ask up on
/// screen — go with it.
fn announce_ended(app: &AppHandle) -> Ended {
    crate::autolock::disarm(app);
    #[cfg(desktop)]
    {
        crate::browser::server::hub(app).notify_locked();
        crate::org::approvals::stop();
        crate::agents::clear(app);
    }
    Ended {
        copies: crate::commands::clipboard::copies(app),
    }
}

/// What an ended session leaves for after its guard: the clipboard, which is
/// the OS's to read and write and too slow for the guard. Only the copies
/// made before the end are cleared, so one made in a session opened since is
/// left alone, however late this runs.
#[must_use]
pub struct Ended {
    copies: u64,
}

impl Ended {
    pub fn finish(self, app: &AppHandle) {
        crate::commands::clipboard::clear_on_lock(app, self.copies);
    }
}

// Defined in the core, where the passkey store needs it too; the commands
// reach it here.
pub use crate::error::store_err;

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
// A login's `username` follows the same shape, and so does an identity's
// `doc_type` and `doc_country`, with the type as the marker.
pub(crate) fn backfill_derived_columns(store: &SqliteStore, key: &VaultKey) {
    let Ok(metas) = store.list() else { return };
    let cipher = key.payload_cipher();
    for meta in metas {
        let brand_missing = meta.kind == "card" && meta.card_brand.is_none();
        let passkey_unflagged = meta.kind == "login" && !meta.has_passkey;
        let env_unstamped = meta.kind == "env" && meta.var_count.is_none();
        let username_unstamped = meta.kind == "login" && meta.username.is_none();
        let doc_unstamped = meta.kind == "identity" && meta.doc_type.is_none();
        if !brand_missing
            && !passkey_unflagged
            && !env_unstamped
            && !username_unstamped
            && !doc_unstamped
        {
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
        if doc_unstamped {
            if let Some(doc_type) = crate::store::migrate::derived_doc_type(&entry) {
                let country = crate::store::migrate::derived_doc_country(&entry);
                let _ = store.set_doc_meta(&meta.id, &doc_type, country.as_deref());
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
/// build with the reason attached. The same for the other side of that rule:
/// with no guard held across the slow part of an open, the open takes a
/// [`Ticket`] before it, so a lock that lands during it still wins.
#[cfg(test)]
mod lock_scope {
    // Everything above the test module: the fakes below it borrow nothing from
    // the real session and would only confuse the search.
    // The test module, not the first `#[cfg(test)]`: a test-only item can sit
    // among the production code (`Session::set`).
    fn production(source: &str) -> &str {
        match source.find("#[cfg(test)]\nmod ") {
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
        // The first such body is `SessionVault`'s: `WorkspaceVault`'s, further
        // down, has no session to borrow.
        let pack = body_of(
            engine,
            "fn pack(&self, reclaim: Reclaim) -> Result<Vec<u8>> {",
        );
        at(&pack, "self.keys.pack(");
        // The whole-database copy is what `PackKeys::pack` is.
        at(
            &body_of(
                engine,
                "fn pack(&self, source: &SqliteStore) -> Result<Vec<u8>>",
            ),
            "pack_store",
        );
        assert!(
            !pack.contains("with_store") && !pack.contains("with_session"),
            "the pack copies the whole database; it must not run inside the \
             session guard — take the lock for the reclaim and again for the \
             key re-check instead"
        );

        // ...and the step that *does* take the lock stays short.
        let reclaim = body_of(engine, "fn reclaim_and_open(&self, reclaim: Reclaim)");
        at(&reclaim, "with_store");
        assert!(
            !reclaim.contains("pack_store"),
            "the reclaim runs under the guard, so the pack may not move into it"
        );
    }

    // Every open from locked takes its ticket before its slow part — the
    // derive, the biometric prompt, the download, the restore — and opens
    // with it, so a lock that lands during that part wins (`Ticket`).
    #[test]
    fn every_open_takes_its_ticket_before_the_slow_part() {
        let auth = include_str!("commands/auth.rs");
        let opens = [
            (body_of(auth, "pub async fn unlock("), "unlock_off_thread("),
            (
                body_of(auth, "pub async fn unlock_biometric("),
                ".retrieve(",
            ),
            (
                body_of(
                    include_str!("commands/workspace.rs"),
                    "pub async fn workspace_select(",
                ),
                "open_off_thread(",
            ),
        ];
        for (body, slow) in &opens {
            assert!(
                at(body, ".ticket()") < at(body, slow),
                "the ticket is taken before `{slow}`, which a lock can land during"
            );
            let squeezed: String = body.split_whitespace().collect();
            at(&squeezed, "session::open(&app,ticket,");
        }

        let setup = include_str!("commands/setup.rs");
        for (signature, slow) in [
            ("pub async fn setup_restore_from_account(", "download("),
            ("pub async fn setup_restore_from_file(", "blocking("),
            ("pub async fn setup_restore_from_legacy(", "blocking("),
            ("pub async fn setup_create(", "create_off_thread("),
        ] {
            let body = body_of(setup, signature);
            assert!(
                at(&body, ".ticket()") < at(&body, slow),
                "{signature}: the ticket is taken before `{slow}`"
            );
            // Without whitespace: rustfmt lays a long call out one argument
            // to a line.
            let squeezed: String = body.split_whitespace().collect();
            at(&squeezed, "adopt(&app,&state,ticket,");
        }
        let adopt: String = body_of(setup, "fn adopt(").split_whitespace().collect();
        at(&adopt, "session::open(app,ticket,");
    }

    // Every change of session tells what follows it — the timer, the
    // extensions, the approvals, the grants — under the guard that made the
    // change, so two changes are never told in the other order (a stale
    // `database-unlocked` after the lock that ended it). The raw changes are
    // private to this module (`Session::open`, `Session::clear`), so these
    // are the only ways a session opens or ends.
    #[test]
    fn every_change_of_session_is_told_under_its_guard() {
        let source = include_str!("session.rs");
        for (signature, change, told) in [
            ("\npub fn open(", "session.open(", "announce_opened(app)"),
            (
                "\npub fn continue_lease(",
                "session.adopt(",
                "announce_opened(app)",
            ),
            ("\npub fn end_in(", "session.clear()", "announce_ended(app)"),
            ("\nfn end(", "seal_in(&mut session", "announce_ended(app)"),
        ] {
            let body = body_of(source, signature);
            assert!(
                at(&body, change) < at(&body, told),
                "{signature}: the change is told after it is made"
            );
            assert!(
                !body.contains("drop(session)"),
                "{signature}: and before its guard is let go"
            );
        }
        // `end` lets its guard go with the block it took it in, and only then
        // finishes what is not memory.
        let end = body_of(source, "\nfn end(");
        assert!(at(&end, "announce_ended(app)") < at(&end, "ended.finish(app)"));

        // The idle clock's own arming checks for a session under the same
        // guard, and the approvals' restart stops and starts under it.
        let touch = body_of(include_str!("autolock.rs"), "pub fn touch(");
        assert!(at(&touch, "session.lock()") < at(&touch, "arm(app)"));
        let restart = body_of(include_str!("org/approvals.rs"), "fn restart(");
        assert!(at(&restart, "session.lock()") < at(&restart, "stop()"));
        assert!(at(&restart, "stop()") < at(&restart, "start(app)"));
    }

    // A lock decided earlier — the idle deadline, a return to a window left
    // too long — is asked again under the guard (`lock_if`), never taken as
    // a lock now (`lock`): by the time it runs it may have been overtaken,
    // and the timer cannot call back a deadline already coming due.
    #[test]
    fn a_lock_decided_earlier_is_asked_again_under_the_guard() {
        let autolock = include_str!("autolock.rs");
        let idle = body_of(autolock, "fn idle(");
        at(&idle, "session::lock_if(");
        assert!(!idle.contains("session::lock("));
        at(&body_of(autolock, "pub fn arm("), "idle(&app, arming)");
        let events = body_of(autolock, "pub fn handle_event(");
        at(&events, "session::lock_if(");
        assert!(!events.contains("session::lock(") && !events.contains(" lock(app)"));
    }

    #[test]
    fn a_password_change_derives_and_re_keys_outside_the_guard() {
        let change = body_of(
            include_str!("commands/auth.rs"),
            "pub async fn change_master_password(",
        );
        assert!(
            !change.contains("session.lock()"),
            "the change itself takes no session lock"
        );
        assert!(
            at(&change, "derive_both") < at(&change, "rekey_open_workspace"),
            "both Argon2id derives belong before the lock is taken"
        );
        let body = body_of(
            include_str!("commands/auth.rs"),
            "pub(crate) async fn rekey_open_workspace(",
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
            at(&body, "derive_key") < at(&body, "session.lock()"),
            "the password's Argon2id derive belongs before the lock is taken"
        );
        // The unseal's own hop, as against the derive's before the lock.
        let unseal = "let (bytes, ext) = super::blocking(";
        assert!(
            at(&body, "session.lock()") < at(&body, unseal),
            "the rows and the cipher come out under the lock; the unseal does not"
        );
        assert!(
            at(&body, unseal) < at(&body, "to_imported"),
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

    // iOS: the background parks the store that is held, and the one that
    // arrives while it lasts; the foreground unparks what is held.
    #[test]
    fn the_background_parks_the_store_held_and_the_store_that_arrives() {
        let dir = tempfile::tempdir().unwrap();
        let mut session = unlocked(&dir);
        assert!(!session.store().unwrap().is_parked());

        session.set_background(true);
        assert!(session.store().unwrap().is_parked());
        assert!(session.is_unlocked(), "the key stays");
        // Uses go on working while parked.
        assert!(session.store().unwrap().count_live().is_ok());

        session.set_background(false);
        assert!(!session.store().unwrap().is_parked());

        // An unlock that lands in the background.
        session.clear();
        session.set_background(true);
        let key = key("first");
        let store = store_in(&dir, &key);
        session.set(key, store, true);
        assert!(session.store().unwrap().is_parked());

        // A lease that comes back in the background stays parked; one that
        // comes back in the foreground is unparked.
        let lease = session.take_out().unwrap();
        assert!(session.restore(lease));
        assert!(session.store().unwrap().is_parked());
        session.set_background(false);
        let lease = session.take_out().unwrap();
        assert!(session.restore(lease));
        assert!(!session.store().unwrap().is_parked());
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

    // The unlock as the commands run it: the ticket first, the key derived
    // with no guard held, the open last.
    #[test]
    fn an_undisturbed_unlock_opens() {
        let state = AppState::default();
        let ticket = state.session.lock().unwrap().ticket();

        let dir = tempfile::tempdir().unwrap();
        let key = key("first");
        let store = store_in(&dir, &key);

        assert!(state
            .session
            .lock()
            .unwrap()
            .open(ticket, key, store, false)
            .is_some());
        assert!(state.session.lock().unwrap().is_unlocked());
    }

    // The regression: the password is in, the key is being derived, and the
    // OS locks. Nothing is open yet, so the lock has nothing to seal — and it
    // used to leave no trace either, so the unlock then opened the vault
    // behind the locked OS. The lock now wins over the open still on its way.
    #[test]
    fn a_lock_while_an_unlock_derives_wins_with_nothing_open() {
        let state = AppState::default();
        let ticket = state.session.lock().unwrap().ticket();

        let sealed = seal(&state);
        assert!(!sealed.live && !sealed.held, "nothing was open to seal");

        let dir = tempfile::tempdir().unwrap();
        let key = key("first");
        let store = store_in(&dir, &key);
        let mut session = state.session.lock().unwrap();
        assert!(session.open(ticket, key, store, false).is_none());
        assert!(!session.is_live(), "the vault stays as the lock left it");
    }

    // The app open at its own level — a ring kept across a switch — with no
    // session behind it: the lock ends the ring and refuses the open too.
    #[test]
    fn a_lock_while_a_switch_opens_wins_over_the_ring_it_opened_from() {
        let state = AppState::default();
        state.keyring.lock().unwrap().insert("w2", b"material");
        let ticket = state.session.lock().unwrap().ticket();

        let sealed = seal(&state);
        assert!(sealed.held && !sealed.live);
        assert!(state.keyring.lock().unwrap().is_empty());

        let dir = tempfile::tempdir().unwrap();
        let key = key("first");
        let store = store_in(&dir, &key);
        assert!(state
            .session
            .lock()
            .unwrap()
            .open(ticket, key, store, false)
            .is_none());
    }

    // A lock is a lock however often it lands: a ticket taken after it is
    // good, so an unlock started once the OS is unlocked again opens.
    #[test]
    fn a_ticket_taken_after_a_lock_opens() {
        let state = AppState::default();
        seal(&state);
        let ticket = state.session.lock().unwrap().ticket();

        let dir = tempfile::tempdir().unwrap();
        let key = key("first");
        let store = store_in(&dir, &key);
        assert!(state
            .session
            .lock()
            .unwrap()
            .open(ticket, key, store, false)
            .is_some());
    }

    // What the webview reads off the numbers (`UnlockResult::session` against
    // `vault:locked`'s epoch): a lock is later than every session opened
    // before it and earlier than every one opened after, so a result is stale
    // exactly when a lock the webview has heard of is later than it — in
    // whatever order the result and the lock's event reached it.
    #[test]
    fn a_lock_is_numbered_between_the_session_before_it_and_the_one_after() {
        let state = AppState::default();
        let open = |dir: &tempfile::TempDir| {
            let key = key("first");
            let store = store_in(dir, &key);
            let mut session = state.session.lock().unwrap();
            let ticket = session.ticket();
            session.open(ticket, key, store, false).unwrap()
        };

        let before = tempfile::tempdir().unwrap();
        let opened_before = open(&before);
        let lock = seal(&state).epoch;
        let after = tempfile::tempdir().unwrap();
        let opened_after = open(&after);

        assert!(lock > opened_before, "a result the lock overtook is older");
        assert!(opened_after > lock, "a result opened since is newer");
    }
}
