//! Agent grants on a host: the ones in flight, what ends them, and the
//! rules the user wrote by answering the dialog — the one engine every host
//! runs, the desktop app's `AppHost` and a headless broker alike, over the
//! open store and what the host holds for it. Nothing here knows a window, a
//! session or a timer: the host takes its locks, calls in, and does what
//! follows a change on its own side.
//!
//! Grants live in memory, on the host's [`AgentState`], and nowhere else. A
//! grant is good for minutes and only while the vault is open — every lock
//! clears them, so nothing is ever served from a locked vault — and the
//! Settings list keeps an ended one for an hour so the user can see what just
//! happened. Standing permissions are not grants but rules
//! (`policy::Rule`), sealed under the lockbox's `meta` like the extension
//! associations (`lockbox::RULES_META`): a rule names a client key that
//! exists only on this device, so there is nothing in it another device
//! could use, and the sync merge ignores `meta` anyway.
//!
//! Every state change writes an audit row into the open lockbox's `audit`
//! table through the chained writer (`audit::Writer`), under the session
//! lock, with the host's install id as the writer id — the chain is per
//! writer, and the workspace id is `default` on every device. Nothing secret
//! goes in a row: the target is reduced to scheme, host and path before it
//! is written. The log fails closed: a release — a grant going active, a
//! use, a standing rule — is made only once its row is in ([`record`]), and
//! a row that will not write refuses it with `AuditUnavailable`; a row about
//! something that only restricts ([`note`]) is logged when it fails, and the
//! restriction still happens.
//!
//! Two switches gate all of it: the app's Agents switch and the open
//! lockbox's agent access. The host's files hold them between launches;
//! while a session is open, what counts is the copy on [`AgentState`]
//! ([`AccessState`]), read from the files once, by the first request of the
//! session ([`AgentState::initialise`]), and from then on changed only under
//! the agents lock, in the one section that also acts on the change and
//! writes the file ([`change_under_locks`]). A release looks nowhere else
//! ([`AgentState::access_on`]), so a release and a change of a switch cannot
//! interleave: whichever takes the lock second sees the other whole. The
//! epoch moves with every change, so a request that decided under one and
//! settles under another is refused whatever the switches say by then. A
//! lock or a lockbox switch resets all of it ([`AgentState::reset`]).
//!
//! Lock order, everywhere a host takes them: the lockbox registry's lock →
//! session → the agents' state → settings — the registry's lock is the outer
//! one for the lockbox's switch, written and first read under it; settings
//! are read and written innermost. Everything here that takes a [`Locked`]
//! is called with the session and agents locks held, in that order. The
//! consent slot a grant ask waits in is taken under the session lock at most
//! (a seal refuses the ask up) and before the agents lock (a grant ask checks
//! the epoch under the slot as it goes up), never after it: a change of the
//! switches refuses the ask only once it has let the agents lock go.

mod audit;
mod engine;
mod state;
#[cfg(test)]
mod tests;

use serde::Serialize;

use rowel_core::error::{Error, Result};
use rowel_core::store::SqliteStore;

use crate::audit::Event;
use crate::grant::{Grant, Mode, State};
use crate::lockbox::{clients_in, live_rules, rules_in, titles_in, write_rules};
use crate::signal::Signal;

pub use audit::{
    about, actor_kind, audit_target, audit_target_of, client_label, draft, note, origin_of, record,
    record_with, resolve_target_text, single_id, site_origin, target_text,
};
use audit::{first_mode, rule_client};
pub use engine::{
    candidates_in, client_covered, covered, decide, deny_unasked, outcome, reply_for, resolved,
    settle, visible_access, visible_in, Asking, Candidate, GrantAnswer, Lasting, Lockbox, Locked,
    Prepared, Verdict,
};
pub use state::{now_ms, AccessState, AgentState, RETAIN_ENDED_MS};

// --- the switches, as the session holds them ----------------------------------------

/// The one section a change of either switch goes through, over what the
/// locks guard: `persist` writes the host's file and applies the change to
/// the switches the session holds (left alone until a request has read
/// them), and says whether the switch changed at all; if it did, in that
/// same section, [`switch_changed`]. A `persist` that fails changes nothing:
/// the file and the session's copy never disagree.
pub fn change_under_locks<T>(
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
/// open): the epoch moves — any change, on or off, of any lockbox's access,
/// so a request caught in between is refused once and asks again — and,
/// only when the open lockbox is cut off now ([`AgentState::access_on`]),
/// every active grant ends with a `revoked` row naming `reason`.
pub fn switch_changed(
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
/// its state already says so: the row goes on the lockbox's log and, for a
/// grant with a site, every extension of the lockbox is told to forget it
/// and sign out of that site (`Signal::GrantEnded`). The one place both
/// happen, so no way of ending a grant can leave a browser signed in under
/// it. A header or env grant signed nothing in, so its end wakes no
/// extension: a signal with no origins would rouse every service worker for
/// a sign-out of nowhere. With no lockbox open (`store` is `None`) there is
/// no log to write to and no extension list to signal — and no session for
/// a signal to outlive.
pub fn ended(
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
    let clients = clients_in(store);
    note(
        store,
        writer_id,
        about(
            draft(event, &grant.client_key, &clients),
            grant,
            Some(detail),
        ),
        now,
    );
    if let Some(origin) = origin_of(&grant.targets) {
        held.signal_extensions(
            &clients,
            &Signal::GrantEnded {
                grant_id: grant.id.clone(),
                origins: vec![site_origin(origin)],
            },
        );
    }
}

// --- expiry and ending -----------------------------------------------------------

/// Flip every grant whose window has closed, with the row and the signal
/// each flip owes ([`ended`]): what the host's timer does when it fires, and
/// what a read of a grant's state does first, so the state it reads is the
/// one the log says. The one way a grant leaves `Active` by time. Whether
/// anything flipped: the host then wakes the extensions for the signals and
/// re-arms its timer, with the locks let go.
pub fn expire_due(
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

/// What [`end_grant_in`] found: the grant, and whether it ended it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrantEnd {
    /// Ended now, with its row and its signal: the host's lists and timer
    /// have something to follow.
    Ended,
    /// Found, but already over: left as it ended, nothing changed.
    AlreadyOver,
    /// No such grant — or, for an owner, not theirs.
    NotFound,
}

/// End grant `id` before its window closes, with `detail` on its `revoked`
/// row: from Settings, which may end any grant (`owner` is `None`), or by
/// the client that holds it, which may end its own and hears of no other —
/// a grant id is a capability, and whether one exists is not for whoever has
/// heard of it. A grant that had already ended is left as it ended, and is
/// still found.
pub fn end_grant_in(ctx: Locked<'_>, id: &str, owner: Option<&str>, detail: &str) -> GrantEnd {
    end_held(
        ctx,
        id,
        |g| owner.is_none_or(|key| g.client_key == key),
        Event::Revoked,
        detail,
    )
}

/// The page showed what was filled under grant `id`, as `reporter` saw it:
/// the grant ends now with an `exposed` row and every extension signs out of
/// the site. Only the client the grant is for, or the extension that was
/// served under it, may say so — the one that filled is the one whose page
/// it is, and any other client of the lockbox could otherwise end any grant
/// it has heard the id of. A report on a grant already over changes nothing:
/// the first one wrote the row and sent the sign-out.
pub fn report_exposure_in(ctx: Locked<'_>, id: &str, reporter: &str) -> GrantEnd {
    end_held(
        ctx,
        id,
        |g| g.client_key == reporter || g.served_to.as_deref() == Some(reporter),
        Event::Exposed,
        r#"{"reason":"exposure"}"#,
    )
}

/// End grant `id` early if `may_end` lets the caller, through [`ended`]: the
/// one path an early end takes, whoever asks for it.
fn end_held(
    ctx: Locked<'_>,
    id: &str,
    may_end: impl Fn(&Grant) -> bool,
    event: Event,
    detail: &str,
) -> GrantEnd {
    let Locked {
        store,
        held,
        writer,
        now,
    } = ctx;
    let Some(grant) = held.find_mut(id).filter(|g| may_end(g)) else {
        return GrantEnd::NotFound;
    };
    if !matches!(grant.state, State::Active | State::Pending) {
        return GrantEnd::AlreadyOver;
    }
    grant.state = State::Revoked;
    let grant = grant.clone();
    ended(Some(store), held, writer, &grant, event, detail, now);
    GrantEnd::Ended
}

// --- rules --------------------------------------------------------------------------

/// The user removed rule `id` from Settings: the shorter list is written
/// back and the removal goes on the log, naming the rule as it was.
pub fn revoke_rule_in(store: &SqliteStore, writer_id: &str, id: &str, now: i64) -> Result<()> {
    let mut rules = rules_in(store);
    let Some(at) = rules.iter().position(|r| r.id == id) else {
        return Err(Error::NotFound);
    };
    let rule = rules.remove(at);
    write_rules(store, &rules)?;
    let clients = clients_in(store);
    let client_key = rule_client(&rule).unwrap_or_default();
    let mut draft = draft(Event::RuleRemoved, &client_key, &clients);
    draft.item_id = single_id(&rule.items);
    draft.target = rule.targets.first().map(audit_target_of);
    draft.mode = rule.modes.first().copied();
    draft.approval = Some(rule.approval);
    draft.detail = Some(format!(r#"{{"ruleId":"{}"}}"#, rule.id));
    note(store, writer_id, draft, now);
    Ok(())
}

// --- what Settings lists --------------------------------------------------------

/// A standing rule as Settings shows it: who may use what, where, how, until
/// when. `origin` is kept beside `target` for the fill rows that predate
/// secret grants; `target` is what the list shows, whichever kind of place
/// the rule names.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RuleRow {
    pub id: String,
    pub client_key: String,
    pub client_name: String,
    pub item_id: Option<String>,
    pub item_title: Option<String>,
    pub origin: Option<String>,
    pub target: Option<String>,
    /// The rule's first mode: what the item is used for.
    pub mode: Mode,
    /// `None` is until removed.
    pub expires_at_ms: Option<i64>,
    pub created_at_ms: i64,
}

/// A grant as Settings shows it; `origin`, `target` and `mode` as on a
/// [`RuleRow`].
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GrantRow {
    pub id: String,
    pub client_key: String,
    pub client_name: String,
    pub item_id: Option<String>,
    pub item_title: Option<String>,
    pub origin: Option<String>,
    pub target: Option<String>,
    pub mode: Mode,
    pub state: State,
    pub expires_at_ms: i64,
    pub uses: u32,
}

/// The open lockbox's own rules — not the baseline, which is not stored —
/// with the names Settings needs beside the ids.
pub fn list_rules_in(store: &SqliteStore, now: i64) -> Vec<RuleRow> {
    let clients = clients_in(store);
    let titles = titles_in(store);
    live_rules(store, now)
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
                target: rule.targets.first().map(target_text),
                mode: first_mode(&rule.modes),
                expires_at_ms: rule.expires_at_ms,
                created_at_ms: rule.created_at_ms,
                id: rule.id,
            }
        })
        .collect()
}

/// Every grant still held: the active ones, and the ones that ended within
/// the last hour — and whether listing them expired any, which the host
/// follows up as it does its timer's firing.
pub fn list_grants_in(ctx: Locked<'_>) -> (Vec<GrantRow>, bool) {
    let Locked {
        store,
        held,
        writer,
        now,
    } = ctx;
    let clients = clients_in(store);
    let titles = titles_in(store);
    // What the list says should be what is true now, not what was true when
    // the timer last fired — and a grant it shows as over has its row.
    let expired = expire_due(Some(store), held, writer, now);
    held.prune(now);
    let rows = held
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
                target: grant.targets.first().map(target_text),
                mode: first_mode(&grant.modes),
                state: grant.state,
                expires_at_ms: grant.expires_at_ms,
                uses: grant.uses,
            }
        })
        .collect();
    (rows, expired)
}
