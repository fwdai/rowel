//! What a host holds about agents between a lock and the next.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::actions::{Client, ClientKind};
use crate::grant::{Grant, State};
use crate::policy::RateState;
use crate::signal::Signal;

/// How long an ended grant stays in the Settings list after its window.
pub const RETAIN_ENDED_MS: i64 = 60 * 60 * 1000;

/// What the host holds about agents between a lock and the next: the grants
/// it issued, how often each client has been asking, what it has to tell
/// each extension, and the switches as they stand for this session.
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
    /// the files; authoritative from then on, changed only by the host's
    /// one path for a change of either ([`super::change_under_locks`]),
    /// under the lock. `None` until then, and again after a seal — nothing
    /// is released on `None`.
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
    pub fn access_on(&self) -> Result<(), &'static str> {
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
    pub fn access_since(&self, epoch: u64) -> Result<(), &'static str> {
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

/// Milliseconds since the Unix epoch: the clock every grant, rule and row is
/// stamped with.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as i64)
}
