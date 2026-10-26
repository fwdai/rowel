//! The admin API (`docs/server-api.md`, "Admin API"): members and their
//! devices, lockboxes and who holds them, agent definitions and the trust
//! policies that name them, policy rules, the audit export and the tenant's
//! events. Every route is a member call under the same bearer, refused with
//! a 403 unless the member is an admin, and every change is written down as
//! a tenant event naming the admin who made it (`store::events`).

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::extract::rejection::QueryRejection;
use axum::extract::{FromRequestParts, Path, Query, State};
use axum::http::request::Parts;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, patch, post, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;

use rowel_broker::actions::ClientKind;
use rowel_broker::grant::CreatedBy;
use rowel_broker::policy::Rule;
use rowel_core::store::AuditRecord;

use crate::api::{Failure, JsonBody};
use crate::auth::Authed;
use crate::lockboxes;
use crate::state::{now_ms, AppState};
use crate::store::events::ServerEvent;
use crate::store::{
    workload_kind, Agent, AuditFilter, Device, DeviceState, Lockbox, LockboxMember, LockboxRole,
    Member, Role, Store, TrustPolicy, SERVER_ADMIN,
};

/// How many events one `GET /v1/admin/events` returns when it does not
/// say, and the most it may ask for.
const DEFAULT_EVENTS: usize = 100;
const MAX_EVENTS: usize = 1000;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v1/admin/members", get(list_members).post(add_member))
        .route("/v1/admin/members/{id}", patch(update_member))
        .route("/v1/admin/members/{id}/devices", get(member_devices))
        .route("/v1/admin/devices/{id}/approve", post(approve_device))
        .route("/v1/admin/devices/{id}/revoke", post(revoke_device))
        .route(
            "/v1/admin/lockboxes",
            get(list_lockboxes).post(create_lockbox),
        )
        .route("/v1/admin/lockboxes/{id}", patch(update_lockbox))
        .route(
            "/v1/admin/lockboxes/{id}/members/{member_id}",
            put(set_lockbox_member).delete(remove_lockbox_member),
        )
        .route("/v1/admin/agents", get(list_agents).post(add_agent))
        .route(
            "/v1/admin/agents/{id}",
            patch(update_agent).delete(remove_agent),
        )
        .route("/v1/admin/agents/{id}/trust", post(add_trust))
        .route("/v1/admin/trust/{id}", delete(remove_trust))
        .route("/v1/admin/rules", get(list_rules).post(add_rule))
        .route("/v1/admin/rules/{id}", delete(remove_rule))
        .route("/v1/admin/audit/export", get(audit_export))
        .route("/v1/admin/events", get(events))
}

/// A member session whose member is an admin: a 401 as `Authed` answers
/// it, then a 403 for anyone else. Taken first by every handler, so a
/// non-admin is refused before a path or a body is read.
pub struct Admin(pub Authed);

impl FromRequestParts<Arc<AppState>> for Admin {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let authed = Authed::from_request_parts(parts, state)
            .await
            .map_err(IntoResponse::into_response)?;
        if !authed.is_admin() {
            return Err(
                Failure::forbidden("the admin API is for members with the admin role")
                    .into_response(),
            );
        }
        Ok(Self(authed))
    }
}

impl Admin {
    /// Write down a change this admin made: `device_id` when it is about a
    /// device, `detail` naming everything else it touched.
    fn record(
        &self,
        store: &Store,
        event: &str,
        device_id: Option<&str>,
        detail: serde_json::Value,
    ) -> Result<(), Failure> {
        store.record_event(
            event,
            Some(&self.0.member.id),
            device_id,
            Some(&detail.to_string()),
        )?;
        Ok(())
    }
}

type Reply<T> = Result<T, Failure>;

fn required(value: &str, field: &str) -> Reply<String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(Failure::bad_request(format!("{field} is required")));
    }
    Ok(value.to_string())
}

// --- members and devices ------------------------------------------------------

async fn list_members(
    State(state): State<Arc<AppState>>,
    _admin: Admin,
) -> Reply<Json<Vec<Member>>> {
    Ok(Json(state.store.list_members()?))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NewMember {
    email: String,
    name: Option<String>,
    role: Role,
}

async fn add_member(
    State(state): State<Arc<AppState>>,
    admin: Admin,
    JsonBody(body): JsonBody<NewMember>,
) -> Reply<(StatusCode, Json<Member>)> {
    let email = required(&body.email, "email")?;
    if !email.contains('@') || email.contains(char::is_whitespace) {
        return Err(Failure::bad_request(format!(
            "{email} is not an email address"
        )));
    }
    if state.store.member_by_email(&email)?.is_some() {
        return Err(Failure::conflict(format!("{email} is already a member")));
    }
    let name = body
        .name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty());
    let member = state.store.add_member(&email, name, body.role)?;
    admin.record(
        &state.store,
        "member_added",
        None,
        json!({ "memberId": member.id, "email": member.email, "role": member.role }),
    )?;
    Ok((StatusCode::CREATED, Json(member)))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MemberChange {
    role: Option<Role>,
    disabled: Option<bool>,
}

/// Change a member's role, or disable them. Disabling is offboarding, in
/// the order that is safe to stop at any step: the member is marked first,
/// which ends every session at its next request (`Authed`), then the
/// sessions are revoked, then each device is revoked, which drops the
/// masters sealed to it.
async fn update_member(
    State(state): State<Arc<AppState>>,
    admin: Admin,
    Path(id): Path<String>,
    JsonBody(change): JsonBody<MemberChange>,
) -> Reply<Json<Member>> {
    let store = &state.store;
    let member = store
        .member(&id)?
        .ok_or_else(|| Failure::not_found(format!("no member {id}")))?;
    // An admin cannot take themselves out, so the tenant always keeps one
    // admin who can undo what another did.
    let self_demotion = change.role.is_some_and(|role| role != Role::Admin);
    if id == admin.0.member.id && (self_demotion || change.disabled == Some(true)) {
        return Err(Failure::conflict(
            "an admin does not demote or disable themselves; another admin does",
        ));
    }
    if change.disabled == Some(false) && member.disabled_at_ms.is_some() {
        return Err(Failure::conflict(
            "a disabled member is not enabled again: their devices were revoked with them",
        ));
    }
    if let Some(role) = change.role {
        store.set_member_role(&id, role)?;
    }
    let mut detail = json!({ "memberId": id, "role": change.role });
    if change.disabled == Some(true) && member.disabled_at_ms.is_none() {
        store.disable_member(&id)?;
        let sessions = store.revoke_sessions_of_member(&id)?;
        let mut devices = 0;
        for device in store.devices_of(&id)? {
            if device.state != DeviceState::Revoked {
                store.revoke_device(&device.id)?;
                devices += 1;
            }
        }
        detail["disabled"] = json!(true);
        detail["sessionsRevoked"] = json!(sessions);
        detail["devicesRevoked"] = json!(devices);
    }
    admin.record(store, "member_updated", None, detail)?;
    Ok(Json(store.member(&id)?.ok_or_else(|| {
        Failure::not_found(format!("no member {id}"))
    })?))
}

async fn member_devices(
    State(state): State<Arc<AppState>>,
    _admin: Admin,
    Path(id): Path<String>,
) -> Reply<Json<Vec<Device>>> {
    if state.store.member(&id)?.is_none() {
        return Err(Failure::not_found(format!("no member {id}")));
    }
    Ok(Json(state.store.devices_of(&id)?))
}

/// How a member's first device is let in: nothing of theirs is approved
/// yet to approve it.
async fn approve_device(
    State(state): State<Arc<AppState>>,
    admin: Admin,
    Path(id): Path<String>,
) -> Reply<Json<Device>> {
    let device = state
        .store
        .device(&id)?
        .ok_or_else(|| Failure::not_found(format!("no device {id}")))?;
    if device.state == DeviceState::Revoked {
        return Err(Failure::conflict(
            "a revoked device is not approved again; the member enrolls it afresh",
        ));
    }
    let device = state.store.approve_device(&id, &admin.0.member.id)?;
    admin.record(
        &state.store,
        "device_approved",
        Some(&id),
        json!({ "memberId": device.member_id }),
    )?;
    Ok(Json(device))
}

async fn revoke_device(
    State(state): State<Arc<AppState>>,
    admin: Admin,
    Path(id): Path<String>,
) -> Reply<StatusCode> {
    state.store.revoke_device(&id)?;
    admin.record(&state.store, "device_revoked", Some(&id), json!({}))?;
    Ok(StatusCode::NO_CONTENT)
}

// --- lockboxes ----------------------------------------------------------------

#[derive(Serialize)]
struct LockboxView {
    #[serde(flatten)]
    lockbox: Lockbox,
    members: Vec<LockboxMember>,
}

fn lockbox_view(store: &Store, lockbox: Lockbox) -> Reply<LockboxView> {
    let members = store.lockbox_members(&lockbox.id)?;
    Ok(LockboxView { lockbox, members })
}

fn existing_lockbox(store: &Store, id: &str) -> Reply<Lockbox> {
    store
        .lockbox(id)?
        .ok_or_else(|| Failure::not_found(format!("no lockbox {id}")))
}

async fn list_lockboxes(
    State(state): State<Arc<AppState>>,
    _admin: Admin,
) -> Reply<Json<Vec<LockboxView>>> {
    let store = &state.store;
    let views = store
        .list_lockboxes()?
        .into_iter()
        .map(|lockbox| lockbox_view(store, lockbox))
        .collect::<Reply<_>>()?;
    Ok(Json(views))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NewLockbox {
    name: String,
}

async fn create_lockbox(
    State(state): State<Arc<AppState>>,
    admin: Admin,
    JsonBody(body): JsonBody<NewLockbox>,
) -> Reply<(StatusCode, Json<LockboxView>)> {
    let name = required(&body.name, "name")?;
    let lockbox = lockboxes::create(&state.store, &*state.root, &state.data_dir, &name)?;
    admin.record(
        &state.store,
        "lockbox_created",
        None,
        json!({ "lockboxId": lockbox.id, "name": lockbox.name }),
    )?;
    Ok((
        StatusCode::CREATED,
        Json(lockbox_view(&state.store, lockbox)?),
    ))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LockboxChange {
    frozen: Option<bool>,
    agents: Option<bool>,
}

async fn update_lockbox(
    State(state): State<Arc<AppState>>,
    admin: Admin,
    Path(id): Path<String>,
    JsonBody(change): JsonBody<LockboxChange>,
) -> Reply<Json<LockboxView>> {
    existing_lockbox(&state.store, &id)?;
    if let Some(frozen) = change.frozen {
        // Through the vault, under its lock (`Vaults::set_frozen`): a push or
        // a grant in flight finishes first and none lands after this answers.
        // That lock may be held for a whole merge, so not on this task.
        let (held, lockbox) = (state.clone(), id.clone());
        tokio::task::spawn_blocking(move || held.vaults.set_frozen(&held, &lockbox, frozen))
            .await
            .map_err(|e| Failure::internal(e.to_string()))??;
    }
    let store = &state.store;
    if let Some(agents) = change.agents {
        store.set_agents(&id, agents)?;
    }
    admin.record(
        store,
        "lockbox_updated",
        None,
        json!({ "lockboxId": id, "frozen": change.frozen, "agents": change.agents }),
    )?;
    Ok(Json(lockbox_view(store, existing_lockbox(store, &id)?)?))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Holding {
    role: LockboxRole,
}

/// Give a member a role on a lockbox, replacing the one they had. The
/// master reaches their approved devices at their next listing, sealed to
/// each (`lockboxes::listing_for`).
async fn set_lockbox_member(
    State(state): State<Arc<AppState>>,
    admin: Admin,
    Path((id, member_id)): Path<(String, String)>,
    JsonBody(body): JsonBody<Holding>,
) -> Reply<StatusCode> {
    let store = &state.store;
    existing_lockbox(store, &id)?;
    if store.member(&member_id)?.is_none() {
        return Err(Failure::not_found(format!("no member {member_id}")));
    }
    store.add_lockbox_member(&id, &member_id, body.role)?;
    admin.record(
        store,
        "lockbox_member_set",
        None,
        json!({ "lockboxId": id, "memberId": member_id, "role": body.role }),
    )?;
    Ok(StatusCode::NO_CONTENT)
}

async fn remove_lockbox_member(
    State(state): State<Arc<AppState>>,
    admin: Admin,
    Path((id, member_id)): Path<(String, String)>,
) -> Reply<StatusCode> {
    let store = &state.store;
    existing_lockbox(store, &id)?;
    store.remove_lockbox_member(&id, &member_id)?;
    admin.record(
        store,
        "lockbox_member_removed",
        None,
        json!({ "lockboxId": id, "memberId": member_id }),
    )?;
    Ok(StatusCode::NO_CONTENT)
}

// --- agents and trust policies ---------------------------------------------------

#[derive(Serialize)]
struct AgentView {
    #[serde(flatten)]
    agent: Agent,
    trust: Vec<TrustPolicy>,
}

fn agent_view(store: &Store, agent: Agent) -> Reply<AgentView> {
    let trust = store.trust_policies_of(&agent.id)?;
    Ok(AgentView { agent, trust })
}

fn existing_agent(store: &Store, id: &str) -> Reply<Agent> {
    store
        .agent(id)?
        .ok_or_else(|| Failure::not_found(format!("no agent {id}")))
}

async fn list_agents(
    State(state): State<Arc<AppState>>,
    _admin: Admin,
) -> Reply<Json<Vec<AgentView>>> {
    let store = &state.store;
    let views = store
        .list_agents()?
        .into_iter()
        .map(|agent| agent_view(store, agent))
        .collect::<Reply<_>>()?;
    Ok(Json(views))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NewAgent {
    name: String,
    lockbox_id: String,
}

async fn add_agent(
    State(state): State<Arc<AppState>>,
    admin: Admin,
    JsonBody(body): JsonBody<NewAgent>,
) -> Reply<(StatusCode, Json<AgentView>)> {
    let name = required(&body.name, "name")?;
    if state.store.lockbox(&body.lockbox_id)?.is_none() {
        return Err(Failure::bad_request(format!(
            "lockboxId {} names no lockbox",
            body.lockbox_id
        )));
    }
    let agent = state.store.add_agent(&name, &body.lockbox_id)?;
    admin.record(
        &state.store,
        "agent_added",
        None,
        json!({ "agentId": agent.id, "name": agent.name, "lockboxId": agent.lockbox_id }),
    )?;
    Ok((StatusCode::CREATED, Json(agent_view(&state.store, agent)?)))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AgentChange {
    name: Option<String>,
    enabled: Option<bool>,
}

/// Rename or switch an agent definition. Switched off, its clients are
/// refused at their next request, since the host looks the definition up
/// on every one.
async fn update_agent(
    State(state): State<Arc<AppState>>,
    admin: Admin,
    Path(id): Path<String>,
    JsonBody(change): JsonBody<AgentChange>,
) -> Reply<Json<AgentView>> {
    let store = &state.store;
    existing_agent(store, &id)?;
    if let Some(name) = &change.name {
        store.rename_agent(&id, &required(name, "name")?)?;
    }
    if let Some(enabled) = change.enabled {
        store.set_agent_enabled(&id, enabled)?;
    }
    admin.record(
        store,
        "agent_updated",
        None,
        json!({ "agentId": id, "name": change.name, "enabled": change.enabled }),
    )?;
    Ok(Json(agent_view(store, existing_agent(store, &id)?)?))
}

/// Remove an agent definition; its trust policies and clients go with it.
async fn remove_agent(
    State(state): State<Arc<AppState>>,
    admin: Admin,
    Path(id): Path<String>,
) -> Reply<StatusCode> {
    state.store.remove_agent(&id)?;
    admin.record(
        &state.store,
        "agent_removed",
        None,
        json!({ "agentId": id }),
    )?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NewTrust {
    issuer: String,
    claims: BTreeMap<String, String>,
    #[serde(default)]
    fixed_audience: bool,
    /// What a matching token may associate as. An agent unless said
    /// otherwise; not `ClientKind`'s own default, which is the extension a
    /// policy never admits.
    #[serde(default = "agent_kind")]
    client_kind: ClientKind,
}

fn agent_kind() -> ClientKind {
    ClientKind::Agent
}

async fn add_trust(
    State(state): State<Arc<AppState>>,
    admin: Admin,
    Path(agent_id): Path<String>,
    JsonBody(body): JsonBody<NewTrust>,
) -> Reply<(StatusCode, Json<TrustPolicy>)> {
    existing_agent(&state.store, &agent_id)?;
    let issuer = required(&body.issuer, "issuer")?;
    if url::Url::parse(&issuer)
        .ok()
        .and_then(|url| url.host().map(|_| ()))
        .is_none()
    {
        return Err(Failure::bad_request(format!(
            "issuer {issuer} is not a URL"
        )));
    }
    // A policy with no claims would admit every token the issuer mints for
    // anyone — every repository on GitHub, every pod on the cluster.
    if body.claims.is_empty() {
        return Err(Failure::bad_request(
            "a trust policy names at least one claim to match",
        ));
    }
    let kind = workload_kind(body.client_kind).map_err(Failure::bad_request)?;
    let policy =
        state
            .store
            .add_trust_policy(&agent_id, &issuer, body.claims, body.fixed_audience, kind)?;
    admin.record(
        &state.store,
        "trust_added",
        None,
        json!({
            "trustId": policy.id,
            "agentId": agent_id,
            "issuer": policy.issuer,
            "clientKind": policy.kind,
        }),
    )?;
    Ok((StatusCode::CREATED, Json(policy)))
}

async fn remove_trust(
    State(state): State<Arc<AppState>>,
    admin: Admin,
    Path(id): Path<String>,
) -> Reply<StatusCode> {
    state.store.remove_trust_policy(&id)?;
    admin.record(
        &state.store,
        "trust_removed",
        None,
        json!({ "trustId": id }),
    )?;
    Ok(StatusCode::NO_CONTENT)
}

// --- rules ------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RuleEntry {
    #[serde(default)]
    lockbox_id: Option<String>,
    rule: Rule,
}

async fn list_rules(
    State(state): State<Arc<AppState>>,
    _admin: Admin,
) -> Reply<Json<Vec<RuleEntry>>> {
    Ok(Json(
        state
            .store
            .list_rules()?
            .into_iter()
            .map(|(lockbox_id, rule)| RuleEntry { lockbox_id, rule })
            .collect(),
    ))
}

/// Keep a rule for one lockbox, or for every lockbox without `lockboxId`.
/// It is the server's whatever the body says (`SERVER_ADMIN`, as the store
/// reads every rule back), stamped now.
async fn add_rule(
    State(state): State<Arc<AppState>>,
    admin: Admin,
    JsonBody(mut entry): JsonBody<RuleEntry>,
) -> Reply<(StatusCode, Json<RuleEntry>)> {
    let store = &state.store;
    let id = required(&entry.rule.id, "rule.id")?;
    if let Some(lockbox_id) = &entry.lockbox_id {
        if store.lockbox(lockbox_id)?.is_none() {
            return Err(Failure::bad_request(format!(
                "lockboxId {lockbox_id} names no lockbox"
            )));
        }
    }
    if store.list_rules()?.iter().any(|(_, rule)| rule.id == id) {
        return Err(Failure::conflict(format!("a rule {id} is already kept")));
    }
    entry.rule.id = id;
    entry.rule.created_at_ms = now_ms();
    entry.rule.created_by = CreatedBy::Admin(SERVER_ADMIN.into());
    store.add_rule(entry.lockbox_id.as_deref(), &entry.rule)?;
    admin.record(
        store,
        "rule_added",
        None,
        json!({ "ruleId": entry.rule.id, "lockboxId": entry.lockbox_id }),
    )?;
    Ok((StatusCode::CREATED, Json(entry)))
}

async fn remove_rule(
    State(state): State<Arc<AppState>>,
    admin: Admin,
    Path(id): Path<String>,
) -> Reply<StatusCode> {
    state.store.remove_rule(&id)?;
    admin.record(&state.store, "rule_removed", None, json!({ "ruleId": id }))?;
    Ok(StatusCode::NO_CONTENT)
}

// --- the audit export and the tenant's events --------------------------------------

#[derive(Deserialize)]
struct ExportQuery {
    since: Option<i64>,
    until: Option<i64>,
    lockbox: Option<String>,
    actor: Option<String>,
}

/// One exported row: the lockbox's audit record as its chain holds it,
/// with the lockbox it came from and the hashes in hex, so an export
/// verifies per `(lockboxId, writerId)` chain as the lockbox does.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExportRow<'a> {
    lockbox_id: &'a str,
    id: &'a str,
    writer_id: &'a str,
    seq: i64,
    ts_ms: i64,
    prev_hash: String,
    hash: String,
    actor_kind: &'a str,
    actor_id: Option<&'a str>,
    actor_name: Option<&'a str>,
    run_id: Option<&'a str>,
    acting_for: Option<&'a str>,
    grant_id: Option<&'a str>,
    item_id: Option<&'a str>,
    target: Option<&'a str>,
    mode: Option<&'a str>,
    approval: Option<&'a str>,
    approver: Option<&'a str>,
    event: &'a str,
    detail: Option<&'a str>,
}

impl<'a> ExportRow<'a> {
    fn of(lockbox_id: &'a str, row: &'a AuditRecord) -> Self {
        Self {
            lockbox_id,
            id: &row.id,
            writer_id: &row.writer_id,
            seq: row.seq,
            ts_ms: row.ts_ms,
            prev_hash: hex::encode(row.prev_hash),
            hash: hex::encode(row.hash),
            actor_kind: &row.actor_kind,
            actor_id: row.actor_id.as_deref(),
            actor_name: row.actor_name.as_deref(),
            run_id: row.run_id.as_deref(),
            acting_for: row.acting_for.as_deref(),
            grant_id: row.grant_id.as_deref(),
            item_id: row.item_id.as_deref(),
            target: row.target.as_deref(),
            mode: row.mode.as_deref(),
            approval: row.approval.as_deref(),
            approver: row.approver.as_deref(),
            event: &row.event,
            detail: row.detail.as_deref(),
        }
    }
}

/// The audit mirror as JSON Lines, one row per line, filtered by time
/// (`since` inclusive, `until` exclusive, both ms), lockbox and actor.
async fn audit_export(
    State(state): State<Arc<AppState>>,
    _admin: Admin,
    query: Result<Query<ExportQuery>, QueryRejection>,
) -> Reply<Response> {
    let Query(query) = query.map_err(|rejection| Failure::bad_request(rejection.body_text()))?;
    let rows = state.store.audit_export(&AuditFilter {
        since_ms: query.since,
        until_ms: query.until,
        lockbox_id: query.lockbox,
        actor_id: query.actor,
    })?;
    let mut body = Vec::new();
    for (lockbox_id, row) in &rows {
        serde_json::to_writer(&mut body, &ExportRow::of(lockbox_id, row))
            .map_err(Failure::internal)?;
        body.push(b'\n');
    }
    Ok(([(header::CONTENT_TYPE, "application/x-ndjson")], body).into_response())
}

#[derive(Deserialize)]
struct EventsQuery {
    since: Option<i64>,
    limit: Option<usize>,
}

async fn events(
    State(state): State<Arc<AppState>>,
    _admin: Admin,
    query: Result<Query<EventsQuery>, QueryRejection>,
) -> Reply<Json<Vec<ServerEvent>>> {
    let Query(query) = query.map_err(|rejection| Failure::bad_request(rejection.body_text()))?;
    let limit = query.limit.unwrap_or(DEFAULT_EVENTS).min(MAX_EVENTS);
    Ok(Json(
        state.store.events_since(query.since.unwrap_or(0), limit)?,
    ))
}
