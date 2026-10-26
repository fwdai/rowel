//! Agent grants as the desktop app holds them: the broker's engine
//! (`rowel_broker::agents`) run under the app's locks. What is here is what
//! only the app has around that engine — the session and the lockbox
//! registry the switches are read from, the settings file they are written
//! to, the one expiry timer, the consent slot a grant ask waits in, and the
//! events that keep Settings' lists current. Each function takes the locks
//! in the engine's order (`workspace_lock` → session → `AppState::agents` →
//! settings), calls in, and does the follow-ups with the locks let go. The
//! state, the rows, the rules and what Settings lists are the engine's, and
//! are reached here under the names they had.

use std::time::Duration;

use tauri::{AppHandle, Manager};

pub use rowel_broker::agents::{
    change_under_locks, draft, ended, expire_due, now_ms, record, site_origin, AccessState,
    AgentState, GrantRow, RuleRow,
};
use rowel_broker::agents::{
    end_grant_in, list_grants_in, list_rules_in, revoke_rule_in, GrantEnd, Locked,
};
pub use rowel_broker::lockbox::titles_in;

use crate::browser;
use crate::error::Result;
use crate::state::AppState;
use crate::{events, settings, storage, workspace};

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
/// `reason` (`change_under_locks`). Then, with the locks let go, the grant
/// ask up on screen is refused through the consent slot and its dialog told
/// to close: its settle would find the epoch moved, and a dialog the user
/// can no longer complete should not stay up. A `persist` that fails changes
/// nothing: the file and the session's copy never disagree.
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
/// row (`end_grant_in`): from Settings, which may end any grant (`owner` is
/// `None`), or by the client that holds it, which may end its own and hears
/// of no other. Whether the grant was found (and, for an owner, was theirs);
/// the lists, the timer and the extensions follow only an end that changed
/// something.
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
        let ctx = Locked {
            store,
            held: &mut agents,
            writer: &writer_id,
            now,
        };
        match end_grant_in(ctx, id, owner, detail) {
            GrantEnd::NotFound => return Ok(false),
            GrantEnd::AlreadyOver => return Ok(true),
            GrantEnd::Ended => {}
        }
    }
    events::agents_changed(app);
    rearm(app);
    browser::server::notify_signals();
    Ok(true)
}

// --- rules --------------------------------------------------------------------------

/// The user removed rule `id` from Settings.
pub fn revoke_rule(app: &AppHandle, id: &str) -> Result<()> {
    let state = app.state::<AppState>();
    let now = now_ms();
    let writer_id = storage::install_id(app);
    {
        let session = state.session.lock().unwrap();
        revoke_rule_in(session.store()?, &writer_id, id, now)?;
    }
    events::agents_changed(app);
    Ok(())
}

// --- what Settings lists --------------------------------------------------------

/// The open lockbox's own rules — not the baseline, which is not stored —
/// with the names Settings needs beside the ids.
pub fn list_rules(app: &AppHandle) -> Result<Vec<RuleRow>> {
    let state = app.state::<AppState>();
    let session = state.session.lock().unwrap();
    Ok(list_rules_in(session.store()?, now_ms()))
}

/// Every grant still held: the active ones, and the ones that ended within
/// the last hour. Empty while locked.
pub fn list_grants(app: &AppHandle) -> Vec<GrantRow> {
    let state = app.state::<AppState>();
    let session = state.session.lock().unwrap();
    let Ok(store) = session.store() else {
        return Vec::new();
    };
    let mut agents = state.agents.lock().unwrap();
    list_grants_in(store, &mut agents, now_ms())
}
