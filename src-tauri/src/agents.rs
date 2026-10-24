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
//! open lockbox's agent access (the registry). Neither is read where a
//! credential is released. Instead a request takes an [`Access`] snapshot —
//! the epoch first, then the switches — and every release checks it again
//! under the locks ([`Access::verify`]); a change of either switch moves the
//! epoch under those same locks ([`access_changed`]), so a release and a
//! switch-off cannot interleave: the release sees the old epoch and is
//! refused, or it came first and the change revokes what it released.
//!
//! Lock order, everywhere: the session lock first, then `AppState::agents`;
//! the consent slot (`browser::CONSENT`) before `AppState::agents`, never
//! after it — a grant ask checks the epoch under the slot as it goes up, and
//! a switch change refuses the ask through the slot only once it has let the
//! agents lock go.

use std::collections::HashMap;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Manager};

use rowel_broker::audit::{ActorKind, Draft, Event, Writer};
use rowel_broker::grant::{Grant, ItemSelector, Mode, State, Target};
use rowel_broker::policy::{RateState, Rule};

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
/// issued, how often each client has been asking, and which change of the
/// switches is the latest.
#[derive(Default)]
pub struct AgentState {
    pub grants: Vec<Grant>,
    pub rates: HashMap<String, RateState>,
    /// Moved by every change of either switch (`access_changed`), under the
    /// locks a release is made under. An [`Access`] read under one epoch
    /// releases nothing under another. Never reset: a lock clears the grants,
    /// not the history of the switches.
    pub access_epoch: u64,
}

impl AgentState {
    pub fn rate(&mut self, client_key: &str) -> &mut RateState {
        self.rates.entry(client_key.to_string()).or_default()
    }

    /// A switch changed: every [`Access`] read before this is stale.
    pub fn bump_access(&mut self) {
        self.access_epoch = self.access_epoch.wrapping_add(1);
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

// --- the switches, as a release checks them -----------------------------------------

/// The two switches as a request read them, and the epoch it read them
/// under. Taken before the locks a release is made under, checked again
/// inside them: see [`Access::verify`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Access {
    pub epoch: u64,
    /// The app's Agents switch.
    pub enabled: bool,
    /// The open lockbox's agent access.
    pub lockbox_on: bool,
}

/// The switches as they stand, under the epoch they stand under. The epoch
/// is read first, under the agents lock: a change that lands after it and
/// before the switches are read has moved it, and `verify` sees that; one
/// that landed before is in the switches themselves. The lockbox's switch
/// takes `workspace_lock`, which is why none of this runs under the session
/// lock.
pub fn access(app: &AppHandle) -> Access {
    let state = app.state::<AppState>();
    let epoch = state.agents.lock().unwrap().access_epoch;
    let enabled = settings::current(app).browser.agents.enabled;
    let lockbox_on = workspace::agents_enabled(app, &workspace::active_id(app));
    Access {
        epoch,
        enabled,
        lockbox_on,
    }
}

impl Access {
    /// Whether a credential may be released right now, under the agents lock
    /// (`held` is what the lock guards): both switches were on when read, and
    /// nothing has changed either since — the epoch is still the one they
    /// were read under. Otherwise the reason the `denied` row names: the
    /// switch that was read off, or `switch` for a change that landed after
    /// the read, whichever switch it was.
    pub fn verify(&self, held: &AgentState) -> std::result::Result<(), &'static str> {
        if !self.enabled {
            return Err("switch");
        }
        if !self.lockbox_on {
            return Err("lockbox");
        }
        if held.access_epoch != self.epoch {
            return Err("switch");
        }
        Ok(())
    }

    /// This snapshot's switches under `epoch` instead of its own: the
    /// switches as they stand now, held to the epoch a request was decided
    /// under, so a change in between still refuses and the row still names
    /// the switch that is off.
    pub fn since(self, epoch: u64) -> Self {
        Self { epoch, ..self }
    }

    /// Whether the open lockbox is cut off: the app's switch or its own
    /// access off. What a change of the switches revokes for, as against
    /// what it merely moves the epoch for.
    pub fn cut_off(&self) -> bool {
        !self.enabled || !self.lockbox_on
    }
}

/// The one path a change of either switch goes through, once it is written
/// (`settings::set` for the app's, `workspace::set_agents` for a lockbox's).
/// Any change — on or off, of any lockbox's access — comes here and moves
/// the epoch under the session and agents locks, so a release decided under
/// the old one is refused ([`Access::verify`]); one epoch for all of them
/// keeps the check one comparison, and a request caught in between is
/// refused once and asks again. Only a change that leaves the open lockbox
/// cut off ([`Access::cut_off`]: the app's switch off, or its own access
/// off) also ends every active grant, with a `revoked` row naming `reason`;
/// a switch turned on, or another lockbox's access either way, leaves them
/// be. Then, with the locks let go, the grant ask up on screen is refused
/// through the consent slot and its dialog told to close, whatever the
/// change: its settle would find the epoch moved, and a dialog the user can
/// no longer complete should not stay up.
pub fn access_changed(app: &AppHandle, reason: &'static str) {
    let state = app.state::<AppState>();
    let now = now_ms();
    let writer_id = storage::install_id(app);
    // Read before the locks: the lockbox's switch takes `workspace_lock`.
    let off = access(app).cut_off();
    {
        let session = state.session.lock().unwrap();
        let mut agents = state.agents.lock().unwrap();
        switch_changed(
            session.store().ok(),
            &mut agents,
            &writer_id,
            off,
            reason,
            now,
        );
    }
    // The slot, never under the agents lock: the ask going up takes them the
    // other way round. The refusal wakes the request thread, which settles
    // under the session lock and finds the epoch moved.
    if let Some(ask_id) = browser::refuse_grant_ask() {
        events::agent_grant_closed(app, &ask_id);
    }
    events::agents_changed(app);
    rearm(app);
}

/// What a change of the switches does under the locks (`store` is the open
/// lockbox's, if one is open; `held` what the agents lock guards): the epoch
/// moves, and — only when the change leaves the open lockbox cut off (`off`)
/// — every active grant ends with a `revoked` row naming `reason`.
pub(crate) fn switch_changed(
    store: Option<&SqliteStore>,
    held: &mut AgentState,
    writer_id: &str,
    off: bool,
    reason: &str,
    now: i64,
) {
    held.bump_access();
    if !off {
        return;
    }
    let detail = format!(r#"{{"reason":"{reason}"}}"#);
    let clients = store.map(browser::clients_in).unwrap_or_default();
    for grant in held.grants.iter_mut().filter(|g| g.state == State::Active) {
        grant.state = State::Revoked;
        if let Some(store) = store {
            let draft = about(
                draft(Event::Revoked, &grant.client_key, &clients),
                grant,
                Some(&detail),
            );
            record(store, writer_id, draft, now);
        }
    }
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
// grant that flipped, let the lists know, and arm for the next one.
fn on_expiry(app: &AppHandle) {
    let state = app.state::<AppState>();
    let now = now_ms();
    let writer_id = storage::install_id(app);
    let changed = {
        let session = state.session.lock().unwrap();
        let mut agents = state.agents.lock().unwrap();
        let flipped = agents.expire_due(now);
        let pruned = agents.prune(now);
        if let Ok(store) = session.store() {
            let clients = browser::clients_in(store);
            for grant in &flipped {
                let draft = about(
                    draft(Event::Ended, &grant.client_key, &clients),
                    grant,
                    Some(r#"{"reason":"expired"}"#),
                );
                record(store, &writer_id, draft, now);
            }
        }
        !flipped.is_empty() || pruned
    };
    if changed {
        events::agents_changed(app);
    }
    rearm(app);
}

/// Every lock ends here: nothing is served from a locked vault, and a client
/// that was being slowed down starts afresh with the next session.
pub fn clear(app: &AppHandle) {
    let state = app.state::<AppState>();
    state.agents.lock().unwrap().clear();
    state.agent_expiry.disarm();
    events::agents_changed(app);
}

/// The user ended grant `id` from Settings.
pub fn end_grant(app: &AppHandle, id: &str) -> Result<()> {
    let state = app.state::<AppState>();
    let now = now_ms();
    let writer_id = storage::install_id(app);
    {
        let session = state.session.lock().unwrap();
        let store = session.store()?;
        let mut agents = state.agents.lock().unwrap();
        let grant = agents.find_mut(id).ok_or(crate::error::Error::NotFound)?;
        if grant.state == State::Active {
            grant.state = State::Revoked;
            let clients = browser::clients_in(store);
            let draft = about(
                draft(Event::Revoked, &grant.client_key, &clients),
                grant,
                Some(r#"{"reason":"user"}"#),
            );
            record(store, &writer_id, draft, now);
        }
    }
    events::agents_changed(app);
    rearm(app);
    Ok(())
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

/// A row about what client `key` did, as the extension that it is.
pub fn draft(event: Event, key: &str, clients: &[Client]) -> Draft {
    let mut draft = Draft::new(ActorKind::Extension, event);
    draft.actor_id = Some(key.to_string());
    draft.actor_name = Some(client_label(clients, key));
    draft
}

/// `draft`, about `grant`: its id, the item when it names one, the target
/// reduced to what the log may hold, the mode, the gate and who passed it.
pub fn about(mut draft: Draft, grant: &Grant, detail: Option<&str>) -> Draft {
    draft.grant_id = Some(grant.id.clone());
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

    #[test]
    fn a_clear_forgets_grants_and_rate_history_alike() {
        let mut state = AgentState::default();
        state.grants.push(active("a", 0, 1_000));
        state.rate("ext").record_denial(0);
        state.bump_access();
        state.clear();
        assert!(state.grants.is_empty() && state.rates.is_empty());
        assert_eq!(state.next_wake_ms(), None);
        assert_eq!(
            state.access_epoch, 1,
            "the switches' history is not the session's"
        );
    }

    // An access snapshot releases only under the epoch it was read under and
    // with both switches on; the reason names the switch read off, or, for a
    // change that landed after the read, `switch`.
    #[test]
    fn an_access_snapshot_verifies_against_the_epoch_it_was_read_under() {
        let mut held = AgentState::default();
        let on = Access {
            epoch: held.access_epoch,
            enabled: true,
            lockbox_on: true,
        };
        assert_eq!(on.verify(&held), Ok(()));
        assert_eq!(
            Access {
                enabled: false,
                ..on
            }
            .verify(&held),
            Err("switch")
        );
        assert_eq!(
            Access {
                lockbox_on: false,
                ..on
            }
            .verify(&held),
            Err("lockbox")
        );

        // The switches changed after the snapshot: whichever it was.
        held.bump_access();
        assert_eq!(on.verify(&held), Err("switch"));
        // A fresh read sees the switch that is off, under the new epoch.
        let fresh = Access {
            epoch: held.access_epoch,
            enabled: true,
            lockbox_on: false,
        };
        assert_eq!(fresh.verify(&held), Err("lockbox"));
        // The switches as they stand now, held to an older epoch: the
        // change in between still refuses, and the row still names the
        // switch that is off.
        assert_eq!(fresh.since(on.epoch).verify(&held), Err("lockbox"));
        assert_eq!(
            Access {
                lockbox_on: true,
                ..fresh
            }
            .since(on.epoch)
            .verify(&held),
            Err("switch")
        );
    }

    // What a row says about a grant, and what it must never say: the page's
    // query string and fragment are gone, the rest of the URL is kept.
    #[test]
    fn a_row_about_a_grant_names_it_without_the_query_string() {
        let grant = active("g1", 0, 1_000);
        let clients = vec![Client {
            name: "Work Chrome".into(),
            key: "ext".into(),
        }];
        let row = about(draft(Event::Used, "ext", &clients), &grant, Some("{}"));
        assert_eq!(row.actor_kind, ActorKind::Extension);
        assert_eq!(row.actor_name.as_deref(), Some("Work Chrome"));
        assert_eq!(row.grant_id.as_deref(), Some("g1"));
        assert_eq!(row.item_id.as_deref(), Some("gh"));
        assert_eq!(row.target.as_deref(), Some("https://github.com/login"));
        assert_eq!(row.mode, Some(Mode::Fill));
        assert_eq!(row.approval, Some(Approval::Ask));
        assert_eq!(row.approver.as_deref(), Some("user"));

        assert_eq!(audit_target("https://a.b/c#frag"), "https://a.b/c");
        assert_eq!(audit_target("https://a.b"), "https://a.b");
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
