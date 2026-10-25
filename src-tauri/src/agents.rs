//! Agent grants on this device: the ones in flight, what ends them, and the
//! rules the user wrote by answering the dialog.
//!
//! Grants live in memory, on `AppState`, and nowhere else. A grant is good for
//! minutes and only while the vault is open — every lock clears them, so
//! nothing is ever served from a locked vault — and the Settings list keeps
//! an ended one for an hour so the user can see what just happened. Standing
//! permissions are not grants but rules (`rowel_broker::policy::Rule`), sealed
//! under the lockbox's `meta` like the extension associations: a rule names a
//! client key that exists only on this device, so there is nothing in it
//! another device could use, and the sync merge ignores `meta` anyway.
//!
//! Every state change writes an audit row into the open lockbox's `audit`
//! table through the broker's chained writer, under the session lock, with
//! this install's id (`storage::install_id`) as the writer id — the chain is
//! per writer, and the workspace id is `default` on every device. Nothing
//! secret goes in a row: the target is reduced to scheme, host and path
//! before it is written.
//!
//! Two switches gate all of it: the app's Agents switch (`settings`) and the
//! open lockbox's agent access (the registry). The files hold them between
//! launches; while a session is open, what counts is the copy on
//! `AgentState` ([`AccessState`]), read from the files once, by the first
//! request of the session ([`initialise`]), and from then on changed only
//! under the agents lock, in the one section that also acts on the change
//! and writes the file ([`change_access`]). A release looks nowhere else
//! ([`AgentState::access_on`]), so a release and a change of a switch cannot
//! interleave: whichever takes the lock second sees the other whole. The
//! epoch moves with every change, so a request that decided under one and
//! settles under another is refused whatever the switches say by then. A
//! lock or a lockbox switch resets all of it ([`clear`]).
//!
//! Lock order, everywhere: `workspace_lock` → session → `AppState::agents`
//! → settings — the registry's lock is the outer one for the lockbox's
//! switch, written and first read under it; settings are read and written
//! innermost. The consent slot (`browser::CONSENT`) is taken under the
//! session lock at most (a seal refuses the ask up) and before the agents
//! lock (a grant ask checks the epoch under the slot as it goes up), never
//! after it: a change of the switches refuses the ask only once it has let
//! the agents lock go.

use std::collections::HashMap;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Manager};

use rowel_broker::actions::ClientKind;
use rowel_broker::audit::{ActorKind, Draft, Event, Writer};
use rowel_broker::grant::{Grant, ItemSelector, Mode, State, Target};
use rowel_broker::policy::{RateState, Rule};
use rowel_broker::Signal;

use crate::browser::{self, Client};
use crate::error::Result;
use crate::state::AppState;
use crate::store::{SqliteStore, VaultStore};
use crate::{events, session, settings, storage, workspace};

/// How long an ended grant stays in the Settings list after its window.
pub const RETAIN_ENDED_MS: i64 = 60 * 60 * 1000;

/// The `meta` key the lockbox's own rules are sealed under.
const RULES_META: &str = "agent_rules";

/// What the app holds about agents between a lock and the next: the grants it
/// issued, how often each client has been asking, what it has to tell each
/// extension, and the switches as they stand for this session.
#[derive(Default)]
pub struct AgentState {
    pub grants: Vec<Grant>,
    pub rates: HashMap<String, RateState>,
    /// What each extension is to be told the next time it asks
    /// (`get-signals`), by client key: an agent's fill grant it may have the
    /// page for, a grant that ended and whose site it signs out of. Queued
    /// under the agents lock, in the same section as the change it reports,
    /// and handed over whole by `take_signals`.
    pub signals: HashMap<String, Vec<Signal>>,
    /// The switches, once the session's first request has read them from
    /// the files; authoritative from then on, changed only by
    /// [`change_access`], under the lock. `None` until then, and again
    /// after a seal — nothing is released on `None`.
    pub access: Option<AccessState>,
    /// Moved by every change of either switch, and by every seal, under the
    /// locks a release is made under: a request that decided under one epoch
    /// settles under no other.
    pub access_epoch: u64,
}

/// The two switches as the session holds them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AccessState {
    /// The app's Agents switch.
    pub enabled: bool,
    /// The open lockbox's agent access.
    pub lockbox_on: bool,
}

impl AgentState {
    pub fn rate(&mut self, client_key: &str) -> &mut RateState {
        self.rates.entry(client_key.to_string()).or_default()
    }

    /// Whether a credential may be released right now: both switches on, as
    /// the session holds them. Otherwise the reason the `denied` row names:
    /// `switch` for the app's (or for switches not read yet, which no
    /// request has had the chance to), `lockbox` for the open lockbox's.
    pub fn access_on(&self) -> std::result::Result<(), &'static str> {
        match self.access {
            None => Err("switch"),
            Some(AccessState { enabled: false, .. }) => Err("switch"),
            Some(AccessState {
                lockbox_on: false, ..
            }) => Err("lockbox"),
            Some(_) => Ok(()),
        }
    }

    /// [`AgentState::access_on`], and nothing changed since `epoch`: what a
    /// request decided under one epoch is settled under no other, whatever
    /// the switches say by then — a switch off and on again is a change.
    pub fn access_since(&self, epoch: u64) -> std::result::Result<(), &'static str> {
        self.access_on()?;
        if self.access_epoch != epoch {
            return Err("switch");
        }
        Ok(())
    }

    /// The switches for this session, if no request has read them yet:
    /// `initial` is what the files say now. What the session already holds
    /// is authoritative and left as it is.
    pub fn initialise(&mut self, initial: AccessState) -> AccessState {
        *self.access.get_or_insert(initial)
    }

    /// A switch changed: what decided before this settles no more.
    pub fn bump_access(&mut self) {
        self.access_epoch = self.access_epoch.wrapping_add(1);
    }

    /// The session ended: nothing it held survives it — the grants, the
    /// rates, the switches as it read them — and what decided under it
    /// settles no more, so the epoch moves too.
    pub fn reset(&mut self) {
        self.bump_access();
        self.access = None;
        self.clear();
    }

    pub fn find_mut(&mut self, id: &str) -> Option<&mut Grant> {
        self.grants.iter_mut().find(|g| g.id == id)
    }

    /// Flip every active grant whose window has closed, and hand back the
    /// ones this call flipped, for their `ended` rows.
    pub fn expire_due(&mut self, now_ms: i64) -> Vec<Grant> {
        self.grants
            .iter_mut()
            .filter_map(|g| g.expire_if_due(now_ms).then(|| g.clone()))
            .collect()
    }

    /// Forget the grants that ended long enough ago for the list to let go.
    /// Returns whether anything went.
    pub fn prune(&mut self, now_ms: i64) -> bool {
        let before = self.grants.len();
        self.grants
            .retain(|g| g.state == State::Active || now_ms < retention_end(g));
        self.grants.len() != before
    }

    /// The next moment something here changes on its own: an active grant
    /// expiring, or an ended one falling off the list.
    pub fn next_wake_ms(&self) -> Option<i64> {
        self.grants
            .iter()
            .map(|g| {
                if g.state == State::Active {
                    g.expires_at_ms
                } else {
                    retention_end(g)
                }
            })
            .min()
    }

    pub fn clear(&mut self) {
        self.grants.clear();
        self.rates.clear();
        self.signals.clear();
    }

    /// Queue `signal` for every extension among `clients`: an extension is
    /// the only kind that fills, so the only kind a fill grant or a sign-out
    /// concerns, and the host cannot tell which browser profile has the page
    /// open, so each of them hears it.
    pub fn signal_extensions(&mut self, clients: &[Client], signal: &Signal) {
        for client in clients.iter().filter(|c| c.kind == ClientKind::Extension) {
            self.signals
                .entry(client.key.clone())
                .or_default()
                .push(signal.clone());
        }
    }
}

// An ended grant leaves the list an hour after its window would have closed
// — or, for one ended early, an hour after that moment would have been; the
// window is the only clock a grant carries, and it is close enough.
fn retention_end(grant: &Grant) -> i64 {
    grant.expires_at_ms.saturating_add(RETAIN_ENDED_MS)
}

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

// --- the switches, as the session holds them ----------------------------------------

/// The session's first look at the switches, from the files, before
/// anything is decided on them. Every `Host` method that reaches the agents'
/// state begins with this call — a request, a check of a grant, a listing
/// of what an agent may see, a signal drained — so no path reads the
/// switches unset and refuses what the files allow; it costs one lock and
/// returns at once once the session holds them. The open lockbox's access
/// comes from the registry, under
/// `workspace_lock` — the lockbox writer's outer lock, so the read and what
/// the session then keeps cannot straddle a write — and the app's switch
/// from the settings, under the agents lock, where the settings writer
/// changes it. A session that already holds them is left with what it
/// holds: that is the authoritative copy, and the files are not read again
/// until the next session.
pub fn initialise(app: &AppHandle) {
    let state = app.state::<AppState>();
    if state.agents.lock().unwrap().access.is_some() {
        return;
    }
    let root = storage::root_dir(app).ok();
    let _paths = state.workspace_lock.lock().unwrap();
    let active = state.active_workspace.lock().unwrap().clone();
    let lockbox_on = root
        .as_deref()
        .is_some_and(|root| workspace::agents_enabled_locked(root, &active));
    let mut held = state.agents.lock().unwrap();
    if held.access.is_none() {
        let enabled = settings::current(app).browser.agents.enabled;
        held.initialise(AccessState {
            enabled,
            lockbox_on,
        });
    }
}

/// The one path a change of either switch goes through: `settings::set` for
/// the app's, `workspace::set_agents` for a lockbox's (under its
/// `workspace_lock`). Under the session and agents locks, `persist` writes
/// the file and applies the change to the switches the session holds (left
/// alone until a request has read them), and says whether the switch
/// changed at all; if it did, in that same section, the epoch moves — any
/// change, on or off, of any lockbox's access, so a request caught in
/// between is refused once and asks again — and, when the open lockbox is
/// cut off by it, every active grant ends with a `revoked` row naming
/// `reason`. Then, with the locks let go, the grant ask up on screen is
/// refused through the consent slot and its dialog told to close: its settle
/// would find the epoch moved, and a dialog the user can no longer complete
/// should not stay up. A `persist` that fails changes nothing: the file and
/// the session's copy never disagree.
pub fn change_access<T>(
    app: &AppHandle,
    reason: &'static str,
    persist: impl FnOnce(&mut Option<AccessState>) -> Result<(T, bool)>,
) -> Result<T> {
    let state = app.state::<AppState>();
    let now = now_ms();
    let writer_id = storage::install_id(app);
    let (value, changed) = {
        let session = state.session.lock().unwrap();
        let mut held = state.agents.lock().unwrap();
        change_under_locks(
            session.store().ok(),
            &mut held,
            &writer_id,
            reason,
            now,
            persist,
        )?
    };
    if changed {
        if let Some(ask_id) = browser::refuse_grant_ask() {
            events::agent_grant_closed(app, &ask_id);
        }
        events::agents_changed(app);
        rearm(app);
        browser::server::notify_signals();
    }
    Ok(value)
}

/// [`change_access`]'s one section, over what the locks guard: `persist`,
/// then — for a change — [`switch_changed`].
pub(crate) fn change_under_locks<T>(
    store: Option<&SqliteStore>,
    held: &mut AgentState,
    writer_id: &str,
    reason: &str,
    now: i64,
    persist: impl FnOnce(&mut Option<AccessState>) -> Result<(T, bool)>,
) -> Result<(T, bool)> {
    let (value, changed) = persist(&mut held.access)?;
    if changed {
        switch_changed(store, held, writer_id, reason, now);
    }
    Ok((value, changed))
}

/// What a change of the switches does to what the session holds, once the
/// change is in `held.access` (`store` is the open lockbox's, if one is
/// open): the epoch moves, and — only when the open lockbox is cut off now
/// ([`AgentState::access_on`]) — every active grant ends with a `revoked`
/// row naming `reason`.
pub(crate) fn switch_changed(
    store: Option<&SqliteStore>,
    held: &mut AgentState,
    writer_id: &str,
    reason: &str,
    now: i64,
) {
    held.bump_access();
    if held.access_on().is_ok() {
        return;
    }
    let detail = format!(r#"{{"reason":"{reason}"}}"#);
    let revoked: Vec<Grant> = held
        .grants
        .iter_mut()
        .filter(|g| g.state == State::Active)
        .map(|grant| {
            grant.state = State::Revoked;
            grant.clone()
        })
        .collect();
    for grant in &revoked {
        ended(store, held, writer_id, grant, Event::Revoked, &detail, now);
    }
}

/// A grant is over, whichever way (`event`: expired, revoked, exposed), and
/// its state already says so: the row goes on the lockbox's log and every
/// extension of the lockbox is told to forget it and sign out of its site
/// (`Signal::GrantEnded`). The one place both happen, so no way of ending a
/// grant can leave a browser signed in under it. With no lockbox open
/// (`store` is `None`) there is no log to write to and no extension list to
/// signal — and no session for a signal to outlive.
pub(crate) fn ended(
    store: Option<&SqliteStore>,
    held: &mut AgentState,
    writer_id: &str,
    grant: &Grant,
    event: Event,
    detail: &str,
    now: i64,
) {
    let Some(store) = store else {
        return;
    };
    let clients = browser::clients_in(store);
    record(
        store,
        writer_id,
        about(
            draft(event, &grant.client_key, &clients),
            grant,
            Some(detail),
        ),
        now,
    );
    held.signal_extensions(
        &clients,
        &Signal::GrantEnded {
            grant_id: grant.id.clone(),
            origins: origin_of(&grant.targets)
                .map(site_origin)
                .into_iter()
                .collect(),
        },
    );
}

// --- expiry -----------------------------------------------------------------------

/// Point the one expiry timer at the next deadline, or stop it when there is
/// nothing left to end. Call after every change to the grants.
pub fn rearm(app: &AppHandle) {
    let state = app.state::<AppState>();
    let next = state.agents.lock().unwrap().next_wake_ms();
    match next {
        Some(at) => {
            let delay = Duration::from_millis(at.saturating_sub(now_ms()).max(0) as u64);
            let handle = app.clone();
            state.agent_expiry.arm(delay, move || on_expiry(&handle));
        }
        None => state.agent_expiry.disarm(),
    }
}

// The timer came due: expire what is due, write an `ended` row for each
// grant that flipped and tell the extensions, let the lists know, and arm
// for the next one.
fn on_expiry(app: &AppHandle) {
    let state = app.state::<AppState>();
    let now = now_ms();
    let writer_id = storage::install_id(app);
    let changed = {
        let session = state.session.lock().unwrap();
        let mut agents = state.agents.lock().unwrap();
        let flipped = expire_due(session.store().ok(), &mut agents, &writer_id, now);
        let pruned = agents.prune(now);
        flipped || pruned
    };
    if changed {
        events::agents_changed(app);
        browser::server::notify_signals();
    }
    rearm(app);
}

/// Flip every grant whose window has closed, with the row and the signal
/// each flip owes ([`ended`]): what the timer does when it fires, and what a
/// read of a grant's state does first, so the state it reads is the one the
/// log says. Whether anything flipped.
pub(crate) fn expire_due(
    store: Option<&SqliteStore>,
    held: &mut AgentState,
    writer_id: &str,
    now: i64,
) -> bool {
    let flipped = held.expire_due(now);
    for grant in &flipped {
        ended(
            store,
            held,
            writer_id,
            grant,
            Event::Ended,
            r#"{"reason":"expired"}"#,
            now,
        );
    }
    !flipped.is_empty()
}

/// Every seal ends here — a lock, the auto-lock, a lockbox switch
/// (`session::sealed`, with the session lock let go): nothing is served from
/// a locked vault, the switches are read afresh by the next session's first
/// request, a client that was being slowed down starts afresh, and what
/// decided under the session settles no more ([`AgentState::reset`]). The
/// grant ask up on screen is refused through the slot, after the agents lock
/// is let go, and its dialog told to close: a sealed vault leaves no dialog
/// up and no request waiting.
pub fn clear(app: &AppHandle) {
    let state = app.state::<AppState>();
    state.agents.lock().unwrap().reset();
    state.agent_expiry.disarm();
    if let Some(ask_id) = browser::refuse_grant_ask() {
        events::agent_grant_closed(app, &ask_id);
    }
    events::agents_changed(app);
}

/// The user ended grant `id` from Settings.
pub fn end_grant(app: &AppHandle, id: &str) -> Result<()> {
    if end_grant_by(app, id, None, r#"{"reason":"user"}"#)? {
        Ok(())
    } else {
        Err(crate::error::Error::NotFound)
    }
}

/// End grant `id` before its window closes, with `detail` on its `revoked`
/// row: from Settings, which may end any grant (`owner` is `None`), or by
/// the client that holds it, which may end its own and hears of no other —
/// a grant id is a capability, and whether one exists is not for whoever has
/// heard of it. Whether the grant was found (and, for an owner, was theirs);
/// a grant that had already ended is left as it ended, and is still found.
pub(crate) fn end_grant_by(
    app: &AppHandle,
    id: &str,
    owner: Option<&str>,
    detail: &str,
) -> Result<bool> {
    let state = app.state::<AppState>();
    let now = now_ms();
    let writer_id = storage::install_id(app);
    {
        let session = state.session.lock().unwrap();
        let store = session.store()?;
        let mut agents = state.agents.lock().unwrap();
        let Some(grant) = agents
            .find_mut(id)
            .filter(|g| owner.is_none_or(|key| g.client_key == key))
        else {
            return Ok(false);
        };
        if !matches!(grant.state, State::Active | State::Pending) {
            return Ok(true);
        }
        grant.state = State::Revoked;
        let grant = grant.clone();
        ended(
            Some(store),
            &mut agents,
            &writer_id,
            &grant,
            Event::Revoked,
            detail,
            now,
        );
    }
    events::agents_changed(app);
    rearm(app);
    browser::server::notify_signals();
    Ok(true)
}

// --- rules --------------------------------------------------------------------------

fn rules_in(store: &SqliteStore) -> Vec<Rule> {
    store
        .meta_get(RULES_META)
        .ok()
        .flatten()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

pub fn write_rules(store: &SqliteStore, rules: &[Rule]) -> Result<()> {
    store
        .meta_set(RULES_META, &serde_json::to_string(rules)?)
        .map_err(session::store_err)
}

/// The lockbox's own rules that still apply at `now_ms`. A rule past its end
/// is dropped here and the shorter list written back, because the evaluator
/// has no clock and Settings should not list what no longer counts.
pub fn live_rules(store: &SqliteStore, now_ms: i64) -> Vec<Rule> {
    let rules = rules_in(store);
    let live: Vec<Rule> = rules
        .iter()
        .filter(|r| r.is_live(now_ms))
        .cloned()
        .collect();
    if live.len() != rules.len() {
        if let Err(e) = write_rules(store, &live) {
            log::warn!("agents: could not drop the expired rules: {e}");
        }
    }
    live
}

/// The user removed rule `id` from Settings.
pub fn revoke_rule(app: &AppHandle, id: &str) -> Result<()> {
    let state = app.state::<AppState>();
    let now = now_ms();
    let writer_id = storage::install_id(app);
    {
        let session = state.session.lock().unwrap();
        let store = session.store()?;
        let mut rules = rules_in(store);
        let Some(at) = rules.iter().position(|r| r.id == id) else {
            return Err(crate::error::Error::NotFound);
        };
        let rule = rules.remove(at);
        write_rules(store, &rules)?;
        let clients = browser::clients_in(store);
        let client_key = rule_client(&rule).unwrap_or_default();
        let mut draft = draft(Event::RuleRemoved, &client_key, &clients);
        draft.item_id = single_id(&rule.items);
        draft.target = origin_of(&rule.targets).map(audit_target);
        draft.mode = rule.modes.first().copied();
        draft.approval = Some(rule.approval);
        draft.detail = Some(format!(r#"{{"ruleId":"{}"}}"#, rule.id));
        record(store, &writer_id, draft, now);
    }
    events::agents_changed(app);
    Ok(())
}

// --- what Settings lists --------------------------------------------------------

/// A standing rule as Settings shows it: who may use what, where, until when.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RuleRow {
    pub id: String,
    pub client_key: String,
    pub client_name: String,
    pub item_id: Option<String>,
    pub item_title: Option<String>,
    pub origin: Option<String>,
    /// `None` is until removed.
    pub expires_at_ms: Option<i64>,
    pub created_at_ms: i64,
}

/// A grant as Settings shows it.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GrantRow {
    pub id: String,
    pub client_key: String,
    pub client_name: String,
    pub item_id: Option<String>,
    pub item_title: Option<String>,
    pub origin: Option<String>,
    pub state: State,
    pub expires_at_ms: i64,
    pub uses: u32,
}

/// The open lockbox's own rules — not the baseline, which is not stored —
/// with the names Settings needs beside the ids.
pub fn list_rules(app: &AppHandle) -> Result<Vec<RuleRow>> {
    let state = app.state::<AppState>();
    let session = state.session.lock().unwrap();
    let store = session.store()?;
    let now = now_ms();
    let clients = browser::clients_in(store);
    let titles = titles_in(store);
    Ok(live_rules(store, now)
        .into_iter()
        .map(|rule| {
            let client_key = rule_client(&rule).unwrap_or_default();
            let item_id = single_id(&rule.items);
            RuleRow {
                client_name: client_label(&clients, &client_key),
                client_key,
                item_title: item_id.as_ref().and_then(|id| titles.get(id).cloned()),
                item_id,
                origin: origin_of(&rule.targets).map(str::to_string),
                expires_at_ms: rule.expires_at_ms,
                created_at_ms: rule.created_at_ms,
                id: rule.id,
            }
        })
        .collect())
}

/// Every grant still held: the active ones, and the ones that ended within
/// the last hour. Empty while locked.
pub fn list_grants(app: &AppHandle) -> Vec<GrantRow> {
    let state = app.state::<AppState>();
    let session = state.session.lock().unwrap();
    let (clients, titles) = match session.store() {
        Ok(store) => (browser::clients_in(store), titles_in(store)),
        Err(_) => return Vec::new(),
    };
    let now = now_ms();
    let mut agents = state.agents.lock().unwrap();
    // What the list says should be what is true now, not what was true when
    // the timer last fired.
    agents.expire_due(now);
    agents.prune(now);
    agents
        .grants
        .iter()
        .map(|grant| {
            let item_id = single_id(&grant.items);
            GrantRow {
                id: grant.id.clone(),
                client_name: client_label(&clients, &grant.client_key),
                client_key: grant.client_key.clone(),
                item_title: item_id.as_ref().and_then(|id| titles.get(id).cloned()),
                item_id,
                origin: origin_of(&grant.targets).map(str::to_string),
                state: grant.state,
                expires_at_ms: grant.expires_at_ms,
                uses: grant.uses,
            }
        })
        .collect()
}

fn titles_in(store: &SqliteStore) -> HashMap<String, String> {
    store
        .list()
        .map(|metas| {
            metas
                .into_iter()
                .map(|m| (m.id, m.title))
                .collect::<HashMap<_, _>>()
        })
        .unwrap_or_default()
}

fn rule_client(rule: &Rule) -> Option<String> {
    match &rule.agents {
        rowel_broker::policy::AgentSelector::Ids(ids) => ids.first().cloned(),
        _ => None,
    }
}

/// The one item a selector names, when it names exactly one.
pub fn single_id(selector: &ItemSelector) -> Option<String> {
    match selector {
        ItemSelector::Ids(ids) if ids.len() == 1 => ids.first().cloned(),
        _ => None,
    }
}

/// The origin among `targets`, when there is one.
pub fn origin_of(targets: &[Target]) -> Option<&str> {
    targets.iter().find_map(|t| match t {
        Target::Origin(origin) => Some(origin.as_str()),
        _ => None,
    })
}

// --- audit ----------------------------------------------------------------------

/// A row about what client `key` did, as the kind of client the lockbox's
/// record says it is. A key the lockbox never named — a stale association —
/// is written as an extension, the one kind there was before there were
/// kinds, and the only kind that reaches the host without a record.
pub fn draft(event: Event, key: &str, clients: &[Client]) -> Draft {
    let client = clients.iter().find(|c| c.key == key);
    let kind = client.map_or(ClientKind::Extension, |c| c.kind);
    let mut draft = Draft::new(actor_kind(kind), event);
    draft.actor_id = Some(key.to_string());
    draft.actor_name = Some(client.map_or_else(|| key_fingerprint(key), |c| c.name.clone()));
    draft
}

/// The log's word for a client of `kind`.
pub fn actor_kind(kind: ClientKind) -> ActorKind {
    match kind {
        ClientKind::Extension => ActorKind::Extension,
        ClientKind::Agent => ActorKind::Agent,
        ClientKind::Sidecar => ActorKind::Sidecar,
    }
}

/// `draft`, about `grant`: its id, the item when it names one, the target
/// reduced to what the log may hold, the mode, the gate and who passed it,
/// and the agent's own words for what it is doing (`run_id`, `acting_for`).
pub fn about(mut draft: Draft, grant: &Grant, detail: Option<&str>) -> Draft {
    draft.grant_id = Some(grant.id.clone());
    draft.run_id = grant.run_id.clone();
    draft.acting_for = grant.acting_for.clone();
    draft.item_id = single_id(&grant.items);
    draft.target = origin_of(&grant.targets).map(audit_target);
    draft.mode = grant.modes.first().copied().or(Some(Mode::Fill));
    draft.approval = Some(grant.approval);
    draft.approver = grant.approver.clone();
    draft.detail = detail.map(str::to_string);
    draft
}

/// Chain `draft` onto the lockbox's log. A row that will not write is logged
/// and not fatal: the decision it describes has been made, and refusing the
/// fill over a bookkeeping failure would only teach the user to turn the
/// bookkeeping off.
pub fn record(store: &SqliteStore, writer_id: &str, draft: Draft, now_ms: i64) {
    if let Err(e) = Writer::new(writer_id, store).record(draft, now_ms) {
        log::warn!("agents: could not write the audit row: {e}");
    }
}

/// A URL as the log may hold it: scheme, host and path, never the query or
/// the fragment, which is where a page puts tokens and the like.
pub fn audit_target(url: &str) -> String {
    url.split(['?', '#']).next().unwrap_or_default().to_string()
}

/// The origin of the page a request named — scheme, host and port — which is
/// what a standing rule is written for and what the dialog shows: a rule is
/// about a site, not about the page's path, and never about its query
/// string. A URL that will not parse keeps what the log would keep.
pub fn site_origin(url: &str) -> String {
    match url::Url::parse(url).map(|u| u.origin()) {
        Ok(origin @ url::Origin::Tuple(..)) => origin.ascii_serialization(),
        _ => audit_target(url),
    }
}

/// What to call a client: the name the user gave it, or — for a key the open
/// lockbox never let in, which a stale association can be — its two ends, as
/// Settings shows a key.
pub fn client_label(clients: &[Client], key: &str) -> String {
    clients
        .iter()
        .find(|c| c.key == key)
        .map(|c| c.name.clone())
        .unwrap_or_else(|| key_fingerprint(key))
}

fn key_fingerprint(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    if chars.len() <= 12 {
        return key.to_string();
    }
    let head: String = chars[..8].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}…{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use rowel_broker::grant::{Approval, CreatedBy, GrantRequest, Terms};

    fn active(id: &str, now_ms: i64, ttl_ms: i64) -> Grant {
        let req = GrantRequest {
            client_key: "ext".into(),
            agent_id: None,
            acting_for: None,
            items: ItemSelector::Ids(vec!["gh".into()]),
            targets: vec![Target::Origin(
                "https://github.com/login?next=/x#top".into(),
            )],
            modes: vec![Mode::Fill],
            run_id: None,
            reason: None,
        };
        let terms = Terms {
            approval: Approval::Ask,
            ttl_ms,
            max_uses: None,
            reason_required: false,
            created_by: CreatedBy::User,
        };
        Grant {
            id: id.into(),
            state: State::Active,
            approver: Some("user".into()),
            ..Grant::from_request("lb", req, terms, now_ms)
        }
    }

    // The timer is armed to the earliest end; firing flips what is due and
    // says which, and the flipped ones stay listed for an hour.
    #[test]
    fn the_state_expires_what_is_due_and_keeps_it_an_hour() {
        let mut state = AgentState::default();
        state.grants.push(active("a", 0, 1_000));
        state.grants.push(active("b", 0, 5_000));
        assert_eq!(state.next_wake_ms(), Some(1_000));

        let flipped = state.expire_due(1_000);
        assert_eq!(
            flipped.iter().map(|g| g.id.as_str()).collect::<Vec<_>>(),
            ["a"]
        );
        assert_eq!(state.find_mut("a").unwrap().state, State::Expired);
        assert!(state.expire_due(1_000).is_empty(), "flipped once");
        assert_eq!(state.next_wake_ms(), Some(5_000), "the live one is next");

        assert!(!state.prune(1_000 + RETAIN_ENDED_MS - 1));
        state.expire_due(5_000);
        assert_eq!(
            state.next_wake_ms(),
            Some(1_000 + RETAIN_ENDED_MS),
            "then the hour's end"
        );
        assert!(state.prune(1_000 + RETAIN_ENDED_MS));
        assert!(state.find_mut("a").is_none());
        assert!(state.find_mut("b").is_some());
    }

    // A seal forgets everything the session held — the grants, the rates,
    // the switches as it read them — and moves the epoch, so what decided
    // under the session settles no more.
    #[test]
    fn a_reset_forgets_what_the_session_held_and_moves_the_epoch() {
        let mut state = AgentState::default();
        state.initialise(AccessState {
            enabled: true,
            lockbox_on: true,
        });
        state.grants.push(active("a", 0, 1_000));
        state.rate("ext").record_denial(0);
        let before = state.access_epoch;
        state.reset();
        assert!(state.grants.is_empty() && state.rates.is_empty());
        assert_eq!(state.access, None);
        assert_eq!(state.next_wake_ms(), None);
        assert_eq!(state.access_epoch, before + 1);
        assert_eq!(state.access_since(before), Err("switch"));
    }

    // The switches as the session holds them say whether a release may go,
    // and why not: unread, or the app's switch off, is `switch`; the open
    // lockbox's access off is `lockbox`. And a request decided under one
    // epoch settles under no other.
    #[test]
    fn the_switches_the_session_holds_say_whether_a_release_may_go() {
        let mut held = AgentState::default();
        assert_eq!(held.access_on(), Err("switch"), "not read yet");
        held.initialise(AccessState {
            enabled: true,
            lockbox_on: true,
        });
        assert_eq!(held.access_on(), Ok(()));
        let epoch = held.access_epoch;
        assert_eq!(held.access_since(epoch), Ok(()));

        held.access.as_mut().unwrap().lockbox_on = false;
        assert_eq!(held.access_on(), Err("lockbox"));
        held.access.as_mut().unwrap().enabled = false;
        assert_eq!(
            held.access_on(),
            Err("switch"),
            "the app's switch is the outer one"
        );

        held.access = Some(AccessState {
            enabled: true,
            lockbox_on: true,
        });
        held.bump_access();
        assert_eq!(held.access_on(), Ok(()), "on, for a new request");
        assert_eq!(
            held.access_since(epoch),
            Err("switch"),
            "not for one decided before the change"
        );
        held.access.as_mut().unwrap().lockbox_on = false;
        assert_eq!(
            held.access_since(epoch),
            Err("lockbox"),
            "the switch that is off, when one is"
        );
    }

    fn client(name: &str, key: &str, kind: ClientKind) -> Client {
        Client {
            name: name.into(),
            key: key.into(),
            kind,
            platform: None,
            created_at_ms: 0,
            last_seen_at_ms: None,
            agent_id: None,
            driven_by_agent: false,
        }
    }

    // What a row says about a grant, and what it must never say: the page's
    // query string and fragment are gone, the rest of the URL is kept.
    #[test]
    fn a_row_about_a_grant_names_it_without_the_query_string() {
        let grant = active("g1", 0, 1_000);
        let clients = vec![client("Work Chrome", "ext", ClientKind::Extension)];
        let row = about(draft(Event::Used, "ext", &clients), &grant, Some("{}"));
        assert_eq!(row.actor_kind, ActorKind::Extension);
        assert_eq!(row.actor_name.as_deref(), Some("Work Chrome"));
        assert_eq!(row.grant_id.as_deref(), Some("g1"));
        assert_eq!(row.item_id.as_deref(), Some("gh"));
        assert_eq!(row.target.as_deref(), Some("https://github.com/login"));
        assert_eq!(row.mode, Some(Mode::Fill));
        assert_eq!(row.approval, Some(Approval::Ask));
        assert_eq!(row.approver.as_deref(), Some("user"));
        assert_eq!(row.run_id, None);

        assert_eq!(audit_target("https://a.b/c#frag"), "https://a.b/c");
        assert_eq!(audit_target("https://a.b"), "https://a.b");
    }

    // The actor is what the lockbox's record says the client is, and an
    // agent's own words for its run go on the row with the grant.
    #[test]
    fn a_row_names_the_actor_by_the_clients_kind_and_carries_the_run() {
        let clients = vec![
            client("Work Chrome", "ext", ClientKind::Extension),
            client("Claude Code", "agent", ClientKind::Agent),
            client("CLI", "side", ClientKind::Sidecar),
        ];
        assert_eq!(
            draft(Event::Requested, "agent", &clients).actor_kind,
            ActorKind::Agent
        );
        assert_eq!(
            draft(Event::Requested, "side", &clients).actor_kind,
            ActorKind::Sidecar
        );
        assert_eq!(
            draft(Event::Requested, "nobody", &clients).actor_kind,
            ActorKind::Extension,
            "a key the lockbox never named is an extension's"
        );

        let grant = Grant {
            client_key: "agent".into(),
            run_id: Some("run-7".into()),
            acting_for: Some("alice".into()),
            ..active("g2", 0, 1_000)
        };
        let row = about(draft(Event::Granted, "agent", &clients), &grant, None);
        assert_eq!(row.actor_kind, ActorKind::Agent);
        assert_eq!(row.actor_name.as_deref(), Some("Claude Code"));
        assert_eq!(row.run_id.as_deref(), Some("run-7"));
        assert_eq!(row.acting_for.as_deref(), Some("alice"));
    }

    // Every way a grant ends goes through `ended`: the row, and the sign-out
    // signal to every extension — and to nothing else.
    #[test]
    fn an_ended_grant_is_signalled_to_every_extension_and_no_agent() {
        let (_dir, store, _cipher) = rowel_broker::testing::vault();
        store
            .meta_set(
                "browser_clients",
                &serde_json::to_string(&[
                    client("Work Chrome", "ext", ClientKind::Extension),
                    client("Home Firefox", "ext2", ClientKind::Extension),
                    client("Claude Code", "agent", ClientKind::Agent),
                ])
                .unwrap(),
            )
            .unwrap();
        let mut held = AgentState::default();
        let grant = Grant {
            client_key: "agent".into(),
            state: State::Revoked,
            ..active("g1", 0, 1_000)
        };

        ended(
            Some(&store),
            &mut held,
            "dev",
            &grant,
            Event::Revoked,
            r#"{"reason":"agent"}"#,
            5,
        );

        let rows = store.audit_rows("dev").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].event, "revoked");
        assert_eq!(rows[0].actor_kind, "agent");
        assert_eq!(rows[0].detail.as_deref(), Some(r#"{"reason":"agent"}"#));
        let expected = Signal::GrantEnded {
            grant_id: "g1".into(),
            origins: vec!["https://github.com".into()],
        };
        assert_eq!(held.signals.get("ext"), Some(&vec![expected.clone()]));
        assert_eq!(held.signals.get("ext2"), Some(&vec![expected]));
        assert!(!held.signals.contains_key("agent"));

        // No lockbox open: nowhere to write, nobody to tell.
        let mut none = AgentState::default();
        ended(None, &mut none, "dev", &grant, Event::Ended, "{}", 5);
        assert!(none.signals.is_empty());

        // A seal forgets what was queued with the rest.
        held.reset();
        assert!(held.signals.is_empty());
    }

    // A rule is written for the site, with the port when one was named, and
    // without the path a page happened to be on.
    #[test]
    fn a_standing_rule_is_for_the_sites_origin() {
        assert_eq!(
            site_origin("https://github.com/login?next=/x#top"),
            "https://github.com"
        );
        assert_eq!(
            site_origin("http://localhost:8080/admin"),
            "http://localhost:8080"
        );
        assert_eq!(site_origin("not a url?x=1"), "not a url");
    }

    // A key the lockbox never named is shown by its ends, as Settings does.
    #[test]
    fn an_unknown_client_is_named_by_its_key_fingerprint() {
        assert_eq!(
            client_label(&[], "AAAAbbbbCCCCddddEEEEffffGGGGhhhhIIIIjjjjKKK="),
            "AAAAbbbb…KKK="
        );
        assert_eq!(client_label(&[], "short"), "short");
    }
}
