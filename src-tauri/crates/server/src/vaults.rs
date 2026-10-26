//! The org lockboxes open for agents: each one's master unwrapped on first
//! use, its file open, and the agents' state the grant engine runs over —
//! `HeadlessHost`'s session and agents, once per lockbox. A lockbox unused
//! for `keyIdleMs` is sealed by the tick: the file closed, the master wiped,
//! and its grants ended with it, which is what spec `07`'s "a quiet server
//! holds no plaintext keys" costs. The next request unwraps it again, and
//! every unwrap is a `key_unwrapped` row in the lockbox's own log.
//!
//! Locks. The map is held only to find or insert a vault, never while a
//! vault's own locks are taken. Per vault the order is `HeadlessHost`'s:
//! session → agents, and the approvals queue only ever inside agents (an ask
//! is queued under the agents lock) or with both let go; never agents →
//! session. The metadata store's connection is innermost everywhere.
//!
//! The epoch. A vault's session counter moves at every unwrap and every
//! seal, and a seal resets the agents' state, which moves the switches'
//! epoch too: a grant decided in one session settles in no other, whatever
//! happened in between.
//!
//! Freezing. A vault's file is open only while its row is not frozen, and
//! the row's `frozen` changes only under that vault's session lock
//! ([`Vaults::set_frozen`], which seals in the same section). Every way in,
//! [`Vaults::open`], reads the row again under the same lock, so whoever
//! holds an open session holds it for a lockbox that was not frozen when the
//! lock was taken, and a freeze waits for the write in flight to finish and
//! then ends every later one: a write never lands after a freeze returned.
//! (A member's pull of a frozen lockbox opens its file for that pull alone:
//! `packs`.)

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use rowel_broker::agents::{self, change_under_locks, expire_due, AccessState, AgentState};
use rowel_broker::audit::{ActorKind, Draft, Event};
use rowel_broker::protocol::Code;
use rowel_core::crypto::PayloadCipher;
use rowel_core::error::Result as CoreResult;
use rowel_core::store::SqliteStore;

use crate::lockboxes::{self, Opened};
use crate::state::{now_ms, AppState};

/// A lockbox file open, and the cipher its items are sealed under.
pub struct Open {
    pub opened: Opened,
    pub cipher: PayloadCipher,
}

/// What a vault's session lock guards: the open file, or nothing, and which
/// session it is.
pub struct Session {
    open: Option<Open>,
    epoch: u64,
}

impl Session {
    pub fn open(&self) -> Result<&Open, Code> {
        self.open.as_ref().ok_or(Code::DatabaseNotOpened)
    }

    pub fn store(&self) -> Result<&SqliteStore, Code> {
        self.open().map(|open| &open.opened.store)
    }

    /// The store, if it is still the session `epoch` names.
    pub fn store_at(&self, epoch: u64) -> Result<&SqliteStore, Code> {
        if self.epoch != epoch {
            return Err(Code::DatabaseNotOpened);
        }
        self.store()
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }
}

/// One org lockbox as the agents' side of the server holds it.
pub struct Vault {
    id: String,
    session: Mutex<Session>,
    agents: Mutex<AgentState>,
    last_used_ms: AtomicI64,
}

impl Vault {
    fn new(id: &str) -> Self {
        Self {
            id: id.to_string(),
            session: Mutex::new(Session {
                open: None,
                epoch: 0,
            }),
            agents: Mutex::new(AgentState::default()),
            last_used_ms: AtomicI64::new(now_ms()),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn session(&self) -> MutexGuard<'_, Session> {
        self.session.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn agents(&self) -> MutexGuard<'_, AgentState> {
        self.agents.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn is_open(&self) -> bool {
        self.session().open.is_some()
    }

    fn touch(&self) {
        self.last_used_ms.store(now_ms(), Ordering::Relaxed);
    }

    /// Unwrap the master and open the file if the session holds none, under
    /// the session lock the caller holds; the row and the full mirror pass
    /// that every unwrap owes go in the same section. The `key_unwrapped`
    /// row is the record that the server used a key it holds, so an unwrap
    /// whose row will not write is no open: the key is dropped with the file
    /// and the request refused (`AuditUnavailable`).
    ///
    /// The row is read here, under the lock, every time: a frozen lockbox is
    /// sealed if it was open and refused (`GrantDenied`), and a row that
    /// cannot be read is no licence to serve. Whatever a caller read before
    /// it took the lock decides nothing.
    fn ensure_open(&self, state: &AppState, session: &mut Session) -> Result<(), Code> {
        match state.store.lockbox(&self.id) {
            Ok(Some(row)) if !row.frozen => {}
            Ok(Some(_)) => {
                self.seal_under(state, session);
                return Err(Code::GrantDenied);
            }
            Ok(None) | Err(_) => {
                self.seal_under(state, session);
                return Err(Code::DatabaseNotOpened);
            }
        }
        if session.open.is_some() {
            return Ok(());
        }
        let opened = lockboxes::open(&state.store, &*state.root, &state.data_dir, &self.id)
            .map_err(|e| {
                log::warn!("rowel server: lockbox {} would not open: {e}", self.id);
                Code::DatabaseNotOpened
            })?;
        let cipher = opened.key.payload_cipher();
        let now = now_ms();
        agents::record(
            &opened.store,
            &state.writer_id,
            Draft::new(ActorKind::Server, Event::KeyUnwrapped),
            now,
        )?;
        // Every writer's rows, once: the file may carry rows the mirror has
        // not seen — a device's whose mirroring failed at its push — and from
        // here on another writer's rows arrive only through a push, which
        // mirrors them itself (`packs`). The unwrap's own row is new, so this
        // pass moves the revision too.
        match opened.store.audit_all() {
            Ok(rows) => mirror_rows(state, &self.id, &rows),
            Err(e) => log::warn!("rowel server: lockbox {} log unreadable: {e}", self.id),
        }
        session.open = Some(Open { opened, cipher });
        session.epoch += 1;
        Ok(())
    }

    /// Read the lockbox's `agents` switch into the session's view of it: the
    /// first read of a session sets it, and a change since goes through the
    /// broker's one path for a switch (`change_under_locks`), which moves
    /// the epoch and revokes the active grants when the lockbox is cut off.
    /// Whether it changed, for the caller to refuse the asks waiting once the
    /// locks are let go, as `HeadlessHost::refresh` does.
    pub fn sync_switch(&self, writer_id: &str, lockbox_on: bool) -> bool {
        let session = self.session();
        let mut held = self.agents();
        if held.access.is_none() {
            initialise(&mut held, lockbox_on);
            return false;
        }
        let outcome = change_under_locks(
            session.store().ok(),
            &mut held,
            writer_id,
            "lockbox",
            now_ms(),
            |access| -> CoreResult<((), bool)> {
                match access {
                    Some(access) if access.lockbox_on != lockbox_on => {
                        access.lockbox_on = lockbox_on;
                        Ok(((), true))
                    }
                    _ => Ok(((), false)),
                }
            },
        );
        outcome.is_ok_and(|(_, changed)| changed)
    }

    /// Copy this server's new rows in the lockbox into the metadata's audit
    /// mirror, which is what the export reads (spec `09`). Called after every
    /// request that can have written a row; a failure is logged and the next
    /// pass picks the rows up, since it starts from what the mirror holds.
    pub fn mirror(&self, state: &AppState) {
        let session = self.session();
        if let Ok(store) = session.store() {
            self.mirror_from(state, store);
        }
    }

    fn mirror_from(&self, state: &AppState, store: &SqliteStore) {
        let after = match state.store.audit_high_water(&self.id, &state.writer_id) {
            Ok(seq) => seq,
            Err(e) => {
                log::warn!("rowel server: audit mirror of {}: {e}", self.id);
                return;
            }
        };
        match store.audit_after(&state.writer_id, after) {
            Ok(rows) => mirror_rows(state, &self.id, &rows),
            Err(e) => log::warn!("rowel server: lockbox {} log unreadable: {e}", self.id),
        }
    }

    /// Close the file and wipe the master: nothing is served from this
    /// lockbox until the next unwrap, and every grant goes. Whether anything
    /// was open. The asks waiting on it are the caller's to refuse, with the
    /// locks let go.
    fn seal(&self, state: &AppState) -> bool {
        self.seal_under(state, &mut self.session())
    }

    /// [`Vault::seal`] under the session lock the caller already holds.
    fn seal_under(&self, state: &AppState, session: &mut Session) -> bool {
        // The last rows of the session, before the store goes with it.
        if let Ok(store) = session.store() {
            self.mirror_from(state, store);
        }
        let was_open = session.open.take().is_some();
        session.epoch += 1;
        // Under the session lock, as `HeadlessHost::seal` resets: no request
        // can decide against the old session and settle against the next.
        self.agents().reset();
        was_open
    }

    /// Expire what is due, with its rows, and let go of grants long ended.
    fn expire(&self, state: &AppState, now: i64) {
        {
            let session = self.session();
            let Ok(store) = session.store() else {
                return;
            };
            let mut held = self.agents();
            expire_due(Some(store), &mut held, &state.writer_id, now);
            held.prune(now);
        }
        self.mirror(state);
    }
}

/// The switches for a session's first request: the server is the agents
/// broker, so the app's Agents switch is implied on; the lockbox's is the
/// row's. What the session already holds is left as it is.
pub fn initialise(held: &mut AgentState, lockbox_on: bool) {
    held.initialise(AccessState {
        enabled: true,
        lockbox_on,
    });
}

/// Copy `rows` into the audit mirror, and move the lockbox's revision once if
/// any of them was new there. A row new to the mirror is one the server wrote
/// into the file since the last pass — a grant's, an association, an unwrap —
/// and the file a device would pull has changed with it: unless the revision
/// moves, a device's HEAD still matches, it never pulls the row, and its next
/// push is taken under a revision that predates it (spec `07`, "every
/// server-side write bumps the revision"). Moved, the push fails with 412 and
/// the device pulls and merges first: the compare-and-swap working as designed.
fn mirror_rows(state: &AppState, lockbox_id: &str, rows: &[rowel_core::store::AuditRecord]) {
    if rows.is_empty() {
        return;
    }
    match state.store.mirror_audit(lockbox_id, rows) {
        Ok(0) => {}
        Ok(_) => {
            if let Err(e) = state.store.bump_revision(lockbox_id) {
                log::warn!("rowel server: revision of {lockbox_id}: {e}");
            }
        }
        Err(e) => log::warn!("rowel server: audit mirror of {lockbox_id}: {e}"),
    }
}

/// Every org lockbox an agent has reached since the process started, by id;
/// open or sealed, each keeps its counter, so a vault is never replaced by
/// a fresh one with an epoch the old one has already used.
#[derive(Default)]
pub struct Vaults {
    vaults: Mutex<HashMap<String, Arc<Vault>>>,
}

impl Vaults {
    fn map(&self) -> MutexGuard<'_, HashMap<String, Arc<Vault>>> {
        self.vaults.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The vault of `id`, if an agent has reached it; not opened.
    pub fn get(&self, id: &str) -> Option<Arc<Vault>> {
        self.map().get(id).cloned()
    }

    /// The vault of `id`, created sealed if no request has reached it yet:
    /// the one object whose session lock every open and every freeze of the
    /// lockbox take.
    fn entry(&self, id: &str) -> Arc<Vault> {
        self.map()
            .entry(id.to_string())
            .or_insert_with(|| Arc::new(Vault::new(id)))
            .clone()
    }

    /// The vault of lockbox `id`, open: its master unwrapped on first use. A
    /// frozen lockbox is refused (`GrantDenied`), and sealed if it was open,
    /// by the row as it stands under the vault's lock (`ensure_open`), never
    /// by a row the caller read earlier.
    pub fn open(&self, state: &AppState, id: &str) -> Result<Arc<Vault>, Code> {
        let vault = self.entry(id);
        vault.touch();
        let opened = vault.ensure_open(state, &mut vault.session());
        if let Err(code) = opened {
            if code == Code::GrantDenied {
                state.approvals.refuse_lockbox(&state.store, id);
            }
            return Err(code);
        }
        Ok(vault)
    }

    /// Freeze or thaw lockbox `id`. The row is written under the vault's
    /// session lock, and a freeze seals in the same section: a request
    /// writing to the lockbox finishes first, and none starts after this
    /// returns. The asks waiting on a frozen lockbox are refused once the
    /// lock is let go, as every seal does.
    pub fn set_frozen(
        &self,
        state: &AppState,
        id: &str,
        frozen: bool,
    ) -> Result<(), crate::store::StoreError> {
        let vault = self.entry(id);
        {
            let mut session = vault.session();
            state.store.set_frozen(id, frozen)?;
            if frozen && vault.seal_under(state, &mut session) {
                log::info!("rowel server: lockbox {id} frozen and sealed");
            }
        }
        if frozen {
            state.approvals.refuse_lockbox(&state.store, id);
        }
        Ok(())
    }

    /// Seal the vault of `id` if it is open, and refuse the asks waiting on
    /// it.
    pub fn seal(&self, state: &AppState, id: &str) {
        let Some(vault) = self.get(id) else {
            return;
        };
        if vault.seal(state) {
            log::info!("rowel server: lockbox {id} sealed");
        }
        state.approvals.refuse_lockbox(&state.store, id);
    }

    /// What the timer does: seal what has been idle for `keyIdleMs`, or was
    /// frozen, or is gone; expire what is due in the rest.
    pub fn tick(&self, state: &AppState, now: i64) {
        let idle_ms = i64::try_from(state.config.key_idle_ms).unwrap_or(i64::MAX);
        let open: Vec<Arc<Vault>> = self
            .map()
            .values()
            .filter(|vault| vault.is_open())
            .cloned()
            .collect();
        for vault in open {
            // A row that cannot be read is no licence to keep the key.
            let servable = matches!(state.store.lockbox(&vault.id), Ok(Some(row)) if !row.frozen);
            let last = vault.last_used_ms.load(Ordering::Relaxed);
            if !servable || now.saturating_sub(last) >= idle_ms {
                self.seal(state, &vault.id);
            } else {
                vault.expire(state, now);
            }
        }
    }
}
