//! An in-process Rowel Server for the org tests: every member endpoint the
//! app speaks (`org::client`) and the pack endpoints the sync adapter speaks
//! (`sync::providers::rowel`), over in-memory state, on a loopback port.
//!
//! Faithful where the contract has teeth — PKCE is verified, a bearer is
//! required, a pending device is handed no master, the pack `PUT` is a
//! compare-and-swap — and short everywhere else: one tenant, one member, an
//! authorization page that redirects straight back with a code instead of
//! sending anyone to an identity provider. The helpers at the bottom are the
//! admin and the agent: they mint lockboxes, rotate masters, approve a first
//! device and queue approvals.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Form, Path, Query, State};
use axum::http::header::{ETAG, IF_MATCH, IF_NONE_MATCH};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use zeroize::Zeroizing;

use super::client::{
    Device, DeviceState, Enrolled, Lockbox, Me, Member, Session, Tenant, CLIENT_ID,
};
use super::enrollment::seal_to;
use crate::crypto::{random_hex_id, KdfParams};
use crate::sync::challenge;

/// One sign-in: the two tokens it holds, and the device it was bound to by
/// an enrollment made under it.
struct Grant {
    access: String,
    refresh: String,
    device: Option<String>,
}

struct LockboxRecord {
    id: String,
    name: String,
    kdf: String,
    master: Zeroizing<Vec<u8>>,
    pack: Option<Vec<u8>>,
    etag: u64,
    /// Move the ETag once more at the next `PUT`, before its precondition is
    /// read: another writer landing between a device's head check and its
    /// upload.
    bump_next_put: bool,
}

#[derive(Default)]
struct Inner {
    codes: HashMap<String, String>,
    grants: Vec<Grant>,
    devices: Vec<Device>,
    lockboxes: Vec<LockboxRecord>,
    #[cfg(desktop)]
    pending: Vec<super::client::Approval>,
    #[cfg(desktop)]
    answers: Vec<(String, Option<rowel_broker::agents::GrantAnswer>)>,
}

struct Shared {
    state: Mutex<Inner>,
    #[cfg(desktop)]
    approvals: tokio::sync::broadcast::Sender<super::client::Approval>,
}

type App = State<Arc<Shared>>;

pub(crate) const MEMBER_ID: &str = "m-pat";
pub(crate) const MEMBER_EMAIL: &str = "pat@acme.example";
pub(crate) const TENANT: &str = "Acme";

pub(crate) struct FakeServer {
    pub(crate) url: String,
    shared: Arc<Shared>,
    task: tauri::async_runtime::JoinHandle<()>,
}

impl Drop for FakeServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl FakeServer {
    pub(crate) fn start() -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let shared = Arc::new(Shared {
            state: Mutex::default(),
            #[cfg(desktop)]
            approvals: tokio::sync::broadcast::channel(64).0,
        });
        let router = router(shared.clone());
        let task = tauri::async_runtime::spawn(async move {
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            axum::serve(listener, router).await.unwrap();
        });
        Self { url, shared, task }
    }

    fn state(&self) -> MutexGuard<'_, Inner> {
        self.shared.state.lock().unwrap()
    }

    /// The admin creates a lockbox: a random master, an `External`
    /// descriptor at generation 1, and the master wrapped to every approved
    /// device at listing time.
    pub(crate) fn create_lockbox(&self, name: &str) -> String {
        let id = random_hex_id();
        let mut master = Zeroizing::new(vec![0u8; 32]);
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut master);
        self.state().lockboxes.push(LockboxRecord {
            id: id.clone(),
            name: name.into(),
            kdf: KdfParams::external(random_hex_id(), 1).to_json().unwrap(),
            master,
            pack: None,
            etag: 0,
            bump_next_put: false,
        });
        id
    }

    /// The admin rotates a lockbox's master: a new key under a new `kid` at
    /// the next generation. The pack on the server stays as the last device
    /// pushed it, under the old descriptor, until a device re-keys it.
    pub(crate) fn rotate_master(&self, id: &str) {
        let mut state = self.state();
        let lockbox = state.lockboxes.iter_mut().find(|l| l.id == id).unwrap();
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut lockbox.master);
        lockbox.kdf = KdfParams::from_json(&lockbox.kdf)
            .unwrap()
            .next_generation()
            .to_json()
            .unwrap();
    }

    pub(crate) fn master_of(&self, id: &str) -> Zeroizing<Vec<u8>> {
        let state = self.state();
        state
            .lockboxes
            .iter()
            .find(|l| l.id == id)
            .unwrap()
            .master
            .clone()
    }

    pub(crate) fn kdf_of(&self, id: &str) -> String {
        let state = self.state();
        state
            .lockboxes
            .iter()
            .find(|l| l.id == id)
            .unwrap()
            .kdf
            .clone()
    }

    /// The admin approves a device — how a member's first device is let in.
    pub(crate) fn admin_approve(&self, device_id: &str) {
        let mut state = self.state();
        state
            .devices
            .iter_mut()
            .find(|d| d.id == device_id)
            .unwrap()
            .state = DeviceState::Approved;
    }

    /// Every access token stops working; refresh tokens stay good. What a
    /// session's expiry looks like from the app.
    pub(crate) fn expire_sessions(&self) {
        for grant in &mut self.state().grants {
            grant.access = random_hex_id();
        }
    }

    /// Another write lands on lockbox `id` just before the next `PUT` is
    /// checked, so that `PUT`'s precondition fails.
    pub(crate) fn bump_on_next_put(&self, id: &str) {
        self.state()
            .lockboxes
            .iter_mut()
            .find(|l| l.id == id)
            .unwrap()
            .bump_next_put = true;
    }

    /// An agent asked: queue the approval for the member, and tell every
    /// open stream. Under the state lock with the broadcast, so a stream
    /// opening at the same moment sees it exactly once (see `approvals`).
    #[cfg(desktop)]
    pub(crate) fn push_approval(&self, approval: super::client::Approval) {
        let mut state = self.state();
        state.pending.push(approval.clone());
        let _ = self.shared.approvals.send(approval);
    }

    #[cfg(desktop)]
    pub(crate) fn answers(&self) -> Vec<(String, Option<rowel_broker::agents::GrantAnswer>)> {
        self.state().answers.clone()
    }
}

fn router(shared: Arc<Shared>) -> Router {
    let router = Router::new()
        .route("/v1/members/auth/authorize", get(authorize))
        .route("/v1/members/auth/token", post(token))
        .route("/v1/members/me", get(me))
        .route("/v1/members/enroll", post(enroll))
        .route("/v1/members/me/devices", get(devices))
        .route("/v1/members/devices/{id}/approve", post(approve_device))
        .route("/v1/members/me/lockboxes", get(lockboxes))
        .route("/v1/lockboxes/{id}/pack", get(get_pack).put(put_pack));
    #[cfg(desktop)]
    let router = router
        .route("/v1/members/me/approvals/stream", get(approvals))
        .route("/v1/approvals/{id}", post(answer));
    router
        .layer(DefaultBodyLimit::max(crate::sync::pack::MAX_PACK_BYTES + 1))
        .with_state(shared)
}

/// The grant the bearer in `headers` belongs to, or a 401.
fn grant<'a>(state: &'a Inner, headers: &HeaderMap) -> Result<&'a Grant, StatusCode> {
    let bearer = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or(StatusCode::UNAUTHORIZED)?;
    state
        .grants
        .iter()
        .find(|g| g.access == bearer)
        .ok_or(StatusCode::UNAUTHORIZED)
}

/// Whether the device the bearer is bound to is approved.
fn approved_device(state: &Inner, headers: &HeaderMap) -> Result<Option<Device>, StatusCode> {
    let device = grant(state, headers)?.device.clone();
    Ok(device
        .and_then(|id| state.devices.iter().find(|d| d.id == id))
        .filter(|d| d.state == DeviceState::Approved)
        .cloned())
}

async fn authorize(
    State(shared): App,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Redirect, StatusCode> {
    let field = |name: &str| query.get(name).ok_or(StatusCode::BAD_REQUEST);
    if field("response_type")? != "code"
        || field("client_id")? != CLIENT_ID
        || field("code_challenge_method")? != "S256"
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    let code = random_hex_id();
    shared
        .state
        .lock()
        .unwrap()
        .codes
        .insert(code.clone(), field("code_challenge")?.clone());
    let mut back = url::Url::parse(field("redirect_uri")?).map_err(|_| StatusCode::BAD_REQUEST)?;
    back.query_pairs_mut()
        .append_pair("code", &code)
        .append_pair("state", field("state")?);
    Ok(Redirect::to(back.as_str()))
}

async fn token(
    State(shared): App,
    Form(form): Form<HashMap<String, String>>,
) -> Result<Json<Session>, StatusCode> {
    let field = |name: &str| form.get(name).ok_or(StatusCode::BAD_REQUEST);
    if field("client_id")? != CLIENT_ID {
        return Err(StatusCode::BAD_REQUEST);
    }
    let mut state = shared.state.lock().unwrap();
    let device = match field("grant_type")?.as_str() {
        "authorization_code" => {
            let expected = state
                .codes
                .remove(field("code")?)
                .ok_or(StatusCode::BAD_REQUEST)?;
            if challenge(field("code_verifier")?) != expected {
                return Err(StatusCode::BAD_REQUEST);
            }
            None
        }
        "refresh_token" => {
            let refresh = field("refresh_token")?;
            let at = state
                .grants
                .iter()
                .position(|g| &g.refresh == refresh)
                .ok_or(StatusCode::UNAUTHORIZED)?;
            state.grants.remove(at).device
        }
        _ => return Err(StatusCode::BAD_REQUEST),
    };
    let session = Session {
        access_token: random_hex_id(),
        refresh_token: Some(random_hex_id()),
        expires_in: Some(3600),
        token_type: "Bearer".into(),
    };
    state.grants.push(Grant {
        access: session.access_token.clone(),
        refresh: session.refresh_token.clone().unwrap(),
        device,
    });
    Ok(Json(session))
}

async fn me(State(shared): App, headers: HeaderMap) -> Result<Json<Me>, StatusCode> {
    let state = shared.state.lock().unwrap();
    grant(&state, &headers)?;
    Ok(Json(Me {
        member: Member {
            id: MEMBER_ID.into(),
            email: MEMBER_EMAIL.into(),
            name: Some("Pat".into()),
        },
        tenant: Tenant {
            name: TENANT.into(),
        },
    }))
}

async fn enroll(
    State(shared): App,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<Enrolled>, StatusCode> {
    let mut state = shared.state.lock().unwrap();
    let access = grant(&state, &headers)?.access.clone();
    let field = |name: &str| {
        body.get(name)
            .and_then(|v| v.as_str())
            .map(String::from)
            .ok_or(StatusCode::BAD_REQUEST)
    };
    let device = Device {
        id: random_hex_id(),
        name: field("deviceName")?,
        state: DeviceState::Pending,
        public_key: field("devicePubkey")?,
        created_at_ms: chrono::Utc::now().timestamp_millis(),
    };
    let enrolled = Enrolled {
        device_id: device.id.clone(),
        state: device.state,
    };
    state.devices.push(device);
    let held = state
        .grants
        .iter_mut()
        .find(|g| g.access == access)
        .unwrap();
    held.device = Some(enrolled.device_id.clone());
    Ok(Json(enrolled))
}

async fn devices(State(shared): App, headers: HeaderMap) -> Result<Json<Vec<Device>>, StatusCode> {
    let state = shared.state.lock().unwrap();
    grant(&state, &headers)?;
    Ok(Json(state.devices.clone()))
}

async fn approve_device(
    State(shared): App,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let mut state = shared.state.lock().unwrap();
    if approved_device(&state, &headers)?.is_none() {
        return Err(StatusCode::FORBIDDEN);
    }
    let device = state
        .devices
        .iter_mut()
        .find(|d| d.id == id)
        .ok_or(StatusCode::NOT_FOUND)?;
    device.state = DeviceState::Approved;
    Ok(StatusCode::NO_CONTENT)
}

async fn lockboxes(
    State(shared): App,
    headers: HeaderMap,
) -> Result<Json<Vec<Lockbox>>, StatusCode> {
    let state = shared.state.lock().unwrap();
    let approved = approved_device(&state, &headers)?;
    let listed = state
        .lockboxes
        .iter()
        .map(|l| Lockbox {
            id: l.id.clone(),
            name: l.name.clone(),
            kdf: l.kdf.clone(),
            wrapped_master: approved
                .as_ref()
                .map(|d| seal_to(&d.public_key, &l.master).unwrap()),
            etag: l.pack.as_ref().map(|_| etag(l.etag)),
        })
        .collect();
    Ok(Json(listed))
}

fn etag(n: u64) -> String {
    format!("\"{n}\"")
}

async fn get_pack(
    State(shared): App,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, StatusCode> {
    let state = shared.state.lock().unwrap();
    grant(&state, &headers)?;
    let lockbox = state
        .lockboxes
        .iter()
        .find(|l| l.id == id)
        .ok_or(StatusCode::NOT_FOUND)?;
    let pack = lockbox.pack.clone().ok_or(StatusCode::NOT_FOUND)?;
    Ok(([(ETAG, etag(lockbox.etag))], pack).into_response())
}

async fn put_pack(
    State(shared): App,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Bytes,
) -> Result<Response, StatusCode> {
    let mut state = shared.state.lock().unwrap();
    grant(&state, &headers)?;
    let lockbox = state
        .lockboxes
        .iter_mut()
        .find(|l| l.id == id)
        .ok_or(StatusCode::NOT_FOUND)?;
    if std::mem::take(&mut lockbox.bump_next_put) {
        lockbox.etag += 1;
    }
    let header = |name| headers.get(name).and_then(|v| v.to_str().ok());
    let exists = lockbox.pack.is_some();
    if header(IF_NONE_MATCH) == Some("*") && exists {
        return Err(StatusCode::PRECONDITION_FAILED);
    }
    if let Some(expected) = header(IF_MATCH) {
        if !exists || expected != etag(lockbox.etag) {
            return Err(StatusCode::PRECONDITION_FAILED);
        }
    }
    lockbox.pack = Some(body.to_vec());
    lockbox.etag += 1;
    Ok(([(ETAG, etag(lockbox.etag))], StatusCode::OK).into_response())
}

/// The member's pending approvals, then every one queued from now on, as
/// `text/event-stream`: a keepalive comment first (so a reader has to skip
/// one), then `event: approval` frames. The subscription is taken and the
/// backlog read under the one lock `push_approval` holds while it both
/// queues and broadcasts, so no approval is seen twice or missed.
#[cfg(desktop)]
async fn approvals(State(shared): App, headers: HeaderMap) -> Result<Response, StatusCode> {
    use futures_util::StreamExt;
    let (receiver, backlog) = {
        let state = shared.state.lock().unwrap();
        grant(&state, &headers)?;
        (shared.approvals.subscribe(), state.pending.clone())
    };
    let head = futures_util::stream::iter(
        std::iter::once(Bytes::from(": keepalive\n\n"))
            .chain(backlog.into_iter().map(frame))
            .map(Ok::<_, std::convert::Infallible>),
    );
    let live = futures_util::stream::unfold(receiver, |mut receiver| async move {
        receiver
            .recv()
            .await
            .ok()
            .map(|approval| (Ok::<_, std::convert::Infallible>(frame(approval)), receiver))
    });
    Response::builder()
        .header(axum::http::header::CONTENT_TYPE, "text/event-stream")
        .body(axum::body::Body::from_stream(head.chain(live)))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

#[cfg(desktop)]
fn frame(approval: super::client::Approval) -> Bytes {
    Bytes::from(format!(
        "event: approval\ndata: {}\n\n",
        serde_json::to_string(&approval).unwrap()
    ))
}

#[cfg(desktop)]
async fn answer(
    State(shared): App,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<StatusCode, StatusCode> {
    let mut state = shared.state.lock().unwrap();
    grant(&state, &headers)?;
    let answer = match body.get("answer") {
        None | Some(serde_json::Value::Null) => None,
        Some(value) => {
            Some(serde_json::from_value(value.clone()).map_err(|_| StatusCode::BAD_REQUEST)?)
        }
    };
    state.pending.retain(|a| a.id != id);
    state.answers.push((id, answer));
    Ok(StatusCode::NO_CONTENT)
}
