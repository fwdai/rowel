//! The asks an agent's request is waiting on, routed to the members who may
//! answer them (spec `03 §4`, "Server routing"). Underneath is `rowel
//! serve`'s queue — one for the tenant: the connection's thread blocks on its
//! own ticket with no lock held, and an answer, a refusal or the deadline
//! wakes it. Around it, this module keeps what the queue does not know: the
//! `Approval` the members' apps are shown (`docs/server-api.md`), the
//! lockbox each ask is about, who it is routed to, the `pending_approvals`
//! row that records it, and a broadcast the approvals streams listen on.
//!
//! Routing (I7): an ask goes to the members holding its lockbox as
//! `approver` or `admin`, and to the member a request says it acts for when
//! that member holds the lockbox too. First answer wins: the queue's slot
//! takes one outcome and refuses every later one, and the row is settled by
//! whichever outcome landed — `answered` by a device, `expired` at the
//! deadline or when the lockbox was sealed or cut off under it.
//!
//! Lock order: the routing list and the queue are each taken alone, never
//! one inside the other, and both sit inside a vault's agents lock when an
//! ask is queued (`OrgHost::request_grant`), never outside it.

use std::collections::{HashMap, HashSet};
use std::convert::Infallible;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::{stream, Stream, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use rowel_broker::actions::Client;
use rowel_broker::agents::{site_origin, target_text, Asking, GrantAnswer};
use rowel_broker::grant::{Approval as Gate, Mode, Target, VisibleItem};
use rowel_serve::approvals::{self as queue, Answer, AskKind};

use crate::auth::Authed;
use crate::state::{now_ms, AppState};
use crate::store::{Agent, Device, DeviceState, Lockbox, PendingState, Store};

/// How often an approvals stream says it is alive. The app reads a stream
/// quiet for 60 s as dead and wants a keepalive at least every 30.
const KEEPALIVE: Duration = Duration::from_secs(20);

/// How long a settled ask is remembered for a late answer to be told it
/// was settled (409) rather than that there is no such ask (404).
const SETTLED_RETAIN_MS: i64 = 10 * 60 * 1000;

/// How many asks a stream may fall behind by before it is ended; the app
/// reconnects and is sent what is pending first.
const BROADCAST_CAPACITY: usize = 256;

/// Where an ask would let an item be used, as the app's grant dialog renders
/// it: the app's `events::AskTarget`, the same serde shape.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AskTarget {
    Origin {
        origin: String,
    },
    Http {
        host: String,
        path: String,
        methods: Vec<String>,
    },
    Command {
        line: String,
    },
}

impl From<&Target> for AskTarget {
    fn from(target: &Target) -> Self {
        match target {
            // The site, never the page's path or query, as the dialog says it.
            Target::Origin(origin) => Self::Origin {
                origin: site_origin(origin),
            },
            Target::Http {
                host,
                path,
                methods,
            } => Self::Http {
                host: host.clone(),
                path: path.clone(),
                methods: methods.clone(),
            },
            Target::Command(line) => Self::Command { line: line.clone() },
        }
    }
}

/// An ask as a member's app is shown it (`docs/server-api.md`). No value,
/// placeholder or field ever rides in one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Approval {
    pub id: String,
    pub lockbox_id: String,
    pub lockbox_name: String,
    pub agent_name: String,
    pub mode: Mode,
    pub targets: Vec<AskTarget>,
    pub candidates: Vec<VisibleItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub step_up: bool,
    pub expires_at_ms: i64,
}

/// What the approvals streams hear.
#[derive(Clone, Debug)]
pub enum ApprovalEvent {
    New(Approval),
}

/// Why an answer did not land.
#[derive(Debug, PartialEq, Eq)]
pub enum AnswerError {
    /// No ask under that id, or one settled long enough ago to be forgotten.
    NotFound,
    /// The device's member is not among those the ask is routed to, or the
    /// device is not approved.
    Forbidden,
    /// Answered already, or past its deadline: the first outcome stands.
    Settled,
}

struct Routed {
    approval: Approval,
    acting_for: Option<String>,
    /// Still waiting on an answer.
    open: bool,
}

pub struct Approvals {
    queue: queue::Approvals,
    routed: Mutex<Vec<Routed>>,
    events: broadcast::Sender<ApprovalEvent>,
}

impl Default for Approvals {
    fn default() -> Self {
        Self::new()
    }
}

/// A queued ask, waited on by the connection thread that queued it.
pub struct Ticket<'a> {
    inner: queue::Ticket<'a>,
    approvals: &'a Approvals,
    store: &'a Store,
}

impl Approvals {
    pub fn new() -> Self {
        Self {
            // Announced here, by `enqueue`, in the server's words: the
            // queue's own announcement names `rowel approve`, which is not
            // how a server's asks are answered.
            queue: queue::Approvals::new(Arc::new(|_| {})),
            routed: Mutex::new(Vec::new()),
            events: broadcast::channel(BROADCAST_CAPACITY).0,
        }
    }

    fn routed(&self) -> MutexGuard<'_, Vec<Routed>> {
        self.routed.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ApprovalEvent> {
        self.events.subscribe()
    }

    /// Queue the ask `asking` for `timeout`, record it, and tell the
    /// streams. A row that will not write is logged and the ask goes out
    /// anyway, as an audit row that will not write does not refuse a grant:
    /// the decision is made, and the members can still make it.
    pub fn enqueue<'a>(
        &'a self,
        store: &'a Store,
        lockbox: &Lockbox,
        agent: &Agent,
        client: &Client,
        asking: &Asking,
        timeout: Duration,
    ) -> Ticket<'a> {
        let grant = &asking.grant;
        let mode = grant.modes.first().copied().unwrap_or(Mode::Fill);
        let candidates: Vec<VisibleItem> = asking
            .candidates
            .iter()
            .map(|c| c.visible.clone())
            .collect();
        let step_up = grant.approval == Gate::StepUp;
        let inner = self.queue.enqueue(
            AskKind::Grant {
                client_name: asking.client_name.clone(),
                client_kind: client.kind,
                mode,
                targets: grant.targets.iter().map(target_text).collect(),
                reason: grant.reason.clone(),
                candidates: candidates.clone(),
                step_up,
                // *This run* is Tier 1's: a routed ask is not offered it.
                run: false,
            },
            timeout,
        );
        let now = now_ms();
        let approval = Approval {
            id: inner.id().to_string(),
            lockbox_id: lockbox.id.clone(),
            lockbox_name: lockbox.name.clone(),
            agent_name: agent.name.clone(),
            mode,
            targets: grant.targets.iter().map(AskTarget::from).collect(),
            candidates,
            reason: grant.reason.clone(),
            step_up,
            expires_at_ms: now.saturating_add(timeout.as_millis() as i64),
        };
        match serde_json::to_string(&approval) {
            Ok(json) => {
                if let Err(e) = store.record_pending(
                    &approval.id,
                    &lockbox.id,
                    Some(&agent.id),
                    &client.key,
                    &json,
                    approval.expires_at_ms,
                ) {
                    log::warn!("rowel server: could not record ask {}: {e}", approval.id);
                }
            }
            Err(e) => log::warn!("rowel server: ask {} not recorded: {e}", approval.id),
        }
        log::info!(
            "rowel server: ask {} from agent {} for lockbox {} routed to its approvers",
            approval.id,
            agent.id,
            lockbox.id
        );
        {
            let mut routed = self.routed();
            routed.retain(|r| r.open || r.approval.expires_at_ms + SETTLED_RETAIN_MS > now);
            routed.push(Routed {
                approval: approval.clone(),
                acting_for: grant.acting_for.clone(),
                open: true,
            });
        }
        // No stream listening is no error: the ask waits for one.
        let _ = self.events.send(ApprovalEvent::New(approval));
        Ticket {
            inner,
            approvals: self,
            store,
        }
    }

    /// The asks waiting that are routed to `member_id`, oldest first.
    pub fn pending_for(&self, store: &Store, member_id: &str) -> Vec<Approval> {
        let open: Vec<(Approval, Option<String>)> = self
            .routed()
            .iter()
            .filter(|r| r.open)
            .map(|r| (r.approval.clone(), r.acting_for.clone()))
            .collect();
        let mut approvers: HashMap<String, Vec<String>> = HashMap::new();
        open.into_iter()
            .filter(|(approval, acting_for)| {
                routes_to(
                    store,
                    &mut approvers,
                    &approval.lockbox_id,
                    acting_for.as_deref(),
                    member_id,
                )
            })
            .map(|(approval, _)| approval)
            .collect()
    }

    /// Whether the ask `id` is still waiting and routed to `member_id`.
    pub fn is_for(&self, store: &Store, id: &str, member_id: &str) -> bool {
        let Some((lockbox_id, acting_for)) = self
            .routed()
            .iter()
            .find(|r| r.open && r.approval.id == id)
            .map(|r| (r.approval.lockbox_id.clone(), r.acting_for.clone()))
        else {
            return false;
        };
        routes_to(
            store,
            &mut HashMap::new(),
            &lockbox_id,
            acting_for.as_deref(),
            member_id,
        )
    }

    /// `device`'s answer to the ask `id`: the item picked, and for how many
    /// days, or `None` to refuse. A step-up is the device's business — it
    /// verified the person before posting — and an approved device's answer
    /// is trusted as given.
    pub fn answer(
        &self,
        store: &Store,
        id: &str,
        device: &Device,
        answer: Option<GrantAnswer>,
    ) -> Result<(), AnswerError> {
        let (lockbox_id, acting_for, open) = self
            .routed()
            .iter()
            .find(|r| r.approval.id == id)
            .map(|r| (r.approval.lockbox_id.clone(), r.acting_for.clone(), r.open))
            .ok_or(AnswerError::NotFound)?;
        let routed = routes_to(
            store,
            &mut HashMap::new(),
            &lockbox_id,
            acting_for.as_deref(),
            &device.member_id,
        );
        if device.state != DeviceState::Approved || !routed {
            return Err(AnswerError::Forbidden);
        }
        if !open {
            return Err(AnswerError::Settled);
        }
        // The queue's slot is where the first outcome lands — an answer, a
        // refusal, the deadline — and every later one is turned away there.
        self.queue
            .answer(id, Answer::Grant(answer))
            .map_err(|_| AnswerError::Settled)?;
        settle_row(store, id, PendingState::Answered, Some(&device.id));
        Ok(())
    }

    /// Refuse every ask waiting on `lockbox_id`: it was sealed, frozen or
    /// cut off under them, and their settle would refuse them anyway.
    pub fn refuse_lockbox(&self, store: &Store, lockbox_id: &str) {
        let waiting: Vec<String> = self
            .routed()
            .iter()
            .filter(|r| r.open && r.approval.lockbox_id == lockbox_id)
            .map(|r| r.approval.id.clone())
            .collect();
        for id in waiting {
            if self.queue.answer(&id, Answer::Grant(None)).is_ok() {
                settle_row(store, &id, PendingState::Expired, None);
            }
        }
    }

    fn close(&self, id: &str) {
        if let Some(routed) = self.routed().iter_mut().find(|r| r.approval.id == id) {
            routed.open = false;
        }
    }
}

impl Ticket<'_> {
    pub fn id(&self) -> &str {
        self.inner.id()
    }

    /// Block until the ask is answered, refused or past its deadline. At
    /// the deadline the row is marked expired; an answer or a refusal has
    /// settled it already.
    pub fn wait(self) -> Option<Answer> {
        let id = self.inner.id().to_string();
        let outcome = self.inner.wait();
        self.approvals.close(&id);
        if outcome.is_none() {
            settle_row(self.store, &id, PendingState::Expired, None);
        }
        outcome
    }
}

fn settle_row(store: &Store, id: &str, state: PendingState, device: Option<&str>) {
    if let Err(e) = store.settle_pending(id, state, device) {
        log::warn!("rowel server: could not settle ask {id}: {e}");
    }
}

/// Whether an ask on `lockbox_id` is routed to `member_id`: an approver or
/// admin of the lockbox, or the member the request acts for when they hold
/// it. `acting_for` is the agent's own word until delegation proofs land
/// (spec `02`), so it routes to a member who could see the lockbox anyway
/// and grants nobody else a say. `approvers` caches the lookups of one pass.
fn routes_to(
    store: &Store,
    approvers: &mut HashMap<String, Vec<String>>,
    lockbox_id: &str,
    acting_for: Option<&str>,
    member_id: &str,
) -> bool {
    let of_lockbox = approvers
        .entry(lockbox_id.to_string())
        .or_insert_with(|| store.approvers_of(lockbox_id).unwrap_or_default());
    if of_lockbox.iter().any(|m| m == member_id) {
        return true;
    }
    acting_for == Some(member_id)
        && store
            .lockbox_members(lockbox_id)
            .is_ok_and(|members| members.iter().any(|m| m.member_id == member_id))
}

/// Asks a previous run of the server left pending: their connections are
/// gone and nothing can answer them, so they are on record as expired.
pub fn expire_orphans(store: &Store) {
    let ids: Vec<String> = match store.list_lockboxes() {
        Ok(lockboxes) => lockboxes.into_iter().map(|l| l.id).collect(),
        Err(e) => {
            log::warn!("rowel server: could not list lockboxes: {e}");
            return;
        }
    };
    match store.list_pending(&ids) {
        Ok(pending) => {
            for ask in pending {
                settle_row(store, &ask.id, PendingState::Expired, None);
            }
        }
        Err(e) => log::warn!("rowel server: could not list pending asks: {e}"),
    }
}

// --- the member API's two approval endpoints ----------------------------------------

/// `GET /v1/members/me/approvals/stream` and `POST /v1/approvals/{id}`.
pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v1/members/me/approvals/stream", get(stream_approvals))
        .route("/v1/approvals/{id}", post(answer_approval))
}

/// The session's device, when it is the member's own and approved: the
/// only kind that may see or answer an ask.
fn approved_device(authed: &Authed) -> Result<&Device, StatusCode> {
    authed
        .device
        .as_ref()
        .filter(|d| d.state == DeviceState::Approved && d.member_id == authed.member.id)
        .ok_or(StatusCode::FORBIDDEN)
}

fn event_of(approval: &Approval) -> Result<Event, Infallible> {
    let data = serde_json::to_string(approval).unwrap_or_default();
    Ok(Event::default().event("approval").data(data))
}

/// Every ask pending for the member first, then each new one routed to them
/// as it is queued, with a keepalive comment between. An ask queued while
/// the pending ones were read can arrive twice; the second is dropped. A
/// stream that falls too far behind ends, and the app's reconnect starts it
/// again from what is pending.
async fn stream_approvals(
    State(state): State<Arc<AppState>>,
    authed: Authed,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, StatusCode> {
    approved_device(&authed)?;
    let member_id = authed.member.id;
    let live = state.approvals.subscribe();
    let pending = state.approvals.pending_for(&state.store, &member_id);
    let sent: HashSet<String> = pending.iter().map(|a| a.id.clone()).collect();
    let first = stream::iter(pending.iter().map(event_of).collect::<Vec<_>>());
    let then = stream::unfold(
        (live, state, member_id, sent),
        |(mut live, state, member_id, sent)| async move {
            loop {
                match live.recv().await {
                    Ok(ApprovalEvent::New(approval)) => {
                        if !sent.contains(&approval.id)
                            && state
                                .approvals
                                .is_for(&state.store, &approval.id, &member_id)
                        {
                            let event = event_of(&approval);
                            return Some((event, (live, state, member_id, sent)));
                        }
                    }
                    Err(_) => return None,
                }
            }
        },
    );
    Ok(Sse::new(first.chain(then)).keep_alive(KeepAlive::new().interval(KEEPALIVE)))
}

#[derive(Deserialize)]
struct AnswerBody {
    #[serde(default)]
    answer: Option<GrantAnswer>,
}

async fn answer_approval(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    authed: Authed,
    Json(body): Json<AnswerBody>,
) -> StatusCode {
    let device = match approved_device(&authed) {
        Ok(device) => device,
        Err(status) => return status,
    };
    match state
        .approvals
        .answer(&state.store, &id, device, body.answer)
    {
        Ok(()) => StatusCode::NO_CONTENT,
        Err(AnswerError::NotFound) => StatusCode::NOT_FOUND,
        Err(AnswerError::Forbidden) => StatusCode::FORBIDDEN,
        Err(AnswerError::Settled) => StatusCode::CONFLICT,
    }
}
