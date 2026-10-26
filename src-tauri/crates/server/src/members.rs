//! The member API (`docs/server-api.md`, "Member API"): the app's sign-in,
//! the member and their devices, and the org lockboxes they hold, in the
//! shapes `src-tauri/src/org/client.rs` already speaks.
//!
//! Sign-in is two OAuth flows back to back. Towards the app this server is
//! the authorization server: the app is the public client `rowel-app` with
//! PKCE and a loopback redirect. Towards the company's identity provider it
//! is a relying party (`identity::members`): `authorize` starts a login
//! there, the IdP comes back to `callback`, and only then does the app's
//! redirect get its code. Between the two the app's half of the request —
//! where to send the code, its `state`, its challenge — is held here under
//! the IdP login's `state`, in memory, for as long as the IdP login lives.
//!
//! The approvals stream and the answers to it are `approvals`'.

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};

use axum::extract::rejection::FormRejection;
use axum::extract::{Form, Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::json;
use zeroize::Zeroizing;

use crate::api::{Failure, JsonBody};
use crate::auth::Authed;
use crate::config::Config;
use crate::identity::members::{allowed, LoginError, LOGIN_TTL_MS};
use crate::lockboxes;
use crate::state::{now_ms, AppState};
use crate::store::{DeviceState, Role, StoreError};

/// The one client this server signs in: the Rowel app.
pub const CLIENT_ID: &str = "rowel-app";

/// The most app sign-ins held between `authorize` and `callback`, as many
/// as the IdP client holds logins: starting one is unauthenticated, so the
/// map is bounded, and past this the oldest is dropped.
const MAX_APP_LOGINS: usize = 10_000;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v1/members/auth/authorize", get(authorize))
        .route("/v1/members/auth/callback", get(callback))
        .route("/v1/members/auth/token", post(token))
        .route("/v1/members/me", get(me))
        .route("/v1/members/enroll", post(enroll))
        .route("/v1/members/me/devices", get(devices))
        .route("/v1/members/devices/{id}/approve", post(approve))
        .route("/v1/members/me/lockboxes", get(lockboxes))
        .layer(Extension(Arc::new(AppLogins::default())))
}

// --- sign-in ---------------------------------------------------------------

/// The app's half of a sign-in in flight.
struct AppLogin {
    redirect_uri: String,
    state: String,
    code_challenge: String,
    expires_at_ms: i64,
}

#[derive(Default)]
struct AppLogins(Mutex<HashMap<String, AppLogin>>);

impl AppLogins {
    fn hold(&self, idp_state: String, login: AppLogin, now_ms: i64) {
        let mut held = self.0.lock().unwrap_or_else(|e| e.into_inner());
        held.retain(|_, login| login.expires_at_ms > now_ms);
        if held.len() >= MAX_APP_LOGINS {
            if let Some(oldest) = held
                .iter()
                .min_by_key(|(_, login)| login.expires_at_ms)
                .map(|(state, _)| state.clone())
            {
                held.remove(&oldest);
            }
        }
        held.insert(idp_state, login);
    }

    /// The login held under `idp_state`, once: a callback replayed finds
    /// nothing.
    fn take(&self, idp_state: &str, now_ms: i64) -> Option<AppLogin> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(idp_state)
            .filter(|login| login.expires_at_ms > now_ms)
    }
}

/// A `302 Found` to `location`, the redirect RFC 6749 names; axum's
/// `Redirect::to` is a 303.
fn found(location: &str) -> Response {
    (StatusCode::FOUND, [(header::LOCATION, location)]).into_response()
}

async fn authorize(
    State(state): State<Arc<AppState>>,
    Extension(logins): Extension<Arc<AppLogins>>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Response, Failure> {
    let field = |name: &str| {
        query
            .get(name)
            .map(String::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| Failure::bad_request(format!("{name} is required")))
    };
    if field("response_type")? != "code" {
        return Err(Failure::bad_request("response_type must be code"));
    }
    if field("client_id")? != CLIENT_ID {
        return Err(Failure::bad_request(format!(
            "client_id must be {CLIENT_ID}: this server signs in the Rowel app"
        )));
    }
    if field("code_challenge_method")? != "S256" {
        return Err(Failure::bad_request("code_challenge_method must be S256"));
    }
    let code_challenge = field("code_challenge")?;
    if !is_s256_challenge(code_challenge) {
        return Err(Failure::bad_request(
            "code_challenge must be an S256 challenge: 43 characters of base64url",
        ));
    }
    let app_state = field("state")?;
    let redirect_uri = field("redirect_uri")?;
    loopback_redirect(redirect_uri)?;
    let idp = state.idp.as_ref().ok_or_else(|| {
        Failure::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "member sign-in is not configured on this server",
        )
    })?;

    let now = now_ms();
    let pending = idp.start_login(now);
    logins.hold(
        pending.state,
        AppLogin {
            redirect_uri: redirect_uri.into(),
            state: app_state.into(),
            code_challenge: code_challenge.into(),
            expires_at_ms: now + LOGIN_TTL_MS,
        },
        now,
    );
    Ok(found(&pending.authorize_url))
}

/// `BASE64URL(SHA-256(verifier))` is always 43 characters of the base64url
/// alphabet; anything else could never match a verifier at the token
/// endpoint, so it is refused while the person can still be told.
fn is_s256_challenge(challenge: &str) -> bool {
    challenge.len() == 43 && URL_SAFE_NO_PAD.decode(challenge).is_ok()
}

/// The app's redirect, accepted only on this machine: `http://127.0.0.1:
/// <port>/…` or `http://localhost:<port>/…`. The app is a native public
/// client that takes the code on a listener it opened for this sign-in
/// (RFC 8252 §7.3), and no redirect is registered with this server, so any
/// other URI would let a link crafted by anyone send a member's code —
/// redeemable with the crafter's own PKCE verifier — to a site of their
/// choosing.
fn loopback_redirect(uri: &str) -> Result<url::Url, Failure> {
    let parsed =
        url::Url::parse(uri).map_err(|_| Failure::bad_request("redirect_uri is not a URL"))?;
    let loopback = match parsed.host() {
        Some(url::Host::Ipv4(addr)) => addr == Ipv4Addr::LOCALHOST,
        Some(url::Host::Domain(name)) => name.eq_ignore_ascii_case("localhost"),
        _ => false,
    };
    if parsed.scheme() != "http"
        || !loopback
        || parsed.port().is_none()
        || parsed.fragment().is_some()
    {
        return Err(Failure::bad_request(
            "redirect_uri must be the app's listener on this machine: \
             http://127.0.0.1:<port>/… or http://localhost:<port>/…",
        ));
    }
    Ok(parsed)
}

#[derive(Deserialize)]
struct CallbackQuery {
    state: Option<String>,
    code: Option<String>,
    error: Option<String>,
}

/// Why a sign-in the app started did not give it a code: an RFC 6749
/// §4.1.2.1 error code and the reason in words.
struct Refusal {
    error: &'static str,
    description: String,
}

fn denied(description: impl Into<String>) -> Refusal {
    Refusal {
        error: "access_denied",
        description: description.into(),
    }
}

impl From<StoreError> for Refusal {
    fn from(error: StoreError) -> Self {
        Refusal {
            error: "server_error",
            description: Failure::internal(error).reason,
        }
    }
}

/// Where the IdP sends the browser back. Once the app's half of the login
/// is found, every outcome goes back to the app's redirect with its
/// `state` — the code, or an RFC 6749 error — because the app's listener
/// waits for exactly that and would otherwise sit until its own timeout
/// while the browser showed the reason.
async fn callback(
    State(state): State<Arc<AppState>>,
    Extension(logins): Extension<Arc<AppLogins>>,
    Query(query): Query<CallbackQuery>,
) -> Response {
    let now = now_ms();
    let Some(idp_state) = query.state.clone() else {
        return Failure::bad_request("the identity provider sent no state").into_response();
    };
    let Some(login) = logins.take(&idp_state, now) else {
        return Failure::bad_request(
            "this sign-in is not one in progress, or took longer than ten minutes; \
             start it again from the Rowel app",
        )
        .into_response();
    };
    // The URI was checked when it was held; it parses.
    let Ok(mut back) = url::Url::parse(&login.redirect_uri) else {
        return Failure::internal("a held redirect does not parse").into_response();
    };
    match sign_in(&state, &login, &idp_state, query, now).await {
        Ok(code) => {
            back.query_pairs_mut().append_pair("code", &code);
        }
        Err(refusal) => {
            back.query_pairs_mut()
                .append_pair("error", refusal.error)
                .append_pair("error_description", &refusal.description);
        }
    }
    back.query_pairs_mut().append_pair("state", &login.state);
    found(back.as_str())
}

/// Finish the IdP login, find or provision the member, and issue the app's
/// code bound to the challenge and redirect it came with.
async fn sign_in(
    state: &AppState,
    login: &AppLogin,
    idp_state: &str,
    query: CallbackQuery,
    now: i64,
) -> Result<Zeroizing<String>, Refusal> {
    if query.error.is_some() {
        return Err(denied("the identity provider did not complete the sign-in"));
    }
    let code = query
        .code
        .ok_or_else(|| denied("the identity provider sent no code"))?;
    let (Some(idp), Some(oidc)) = (&state.idp, &state.config.members.oidc) else {
        return Err(denied("member sign-in is not configured on this server"));
    };
    let identity = idp
        .finish_login(idp_state, &code, now)
        .await
        .map_err(|e| match e {
            LoginError::Exchange(_) => Refusal {
                error: "temporarily_unavailable",
                description: e.to_string(),
            },
            other => denied(other.to_string()),
        })?;
    let email = &identity.email;
    let domains = &oidc.allowed_domains;
    if !allowed(email, domains) {
        return Err(denied(format!(
            "{email} is not in a domain this server admits"
        )));
    }
    let member = match state.store.member_by_email(email)? {
        Some(member) => member,
        // A server that names the domains its people come from takes anyone
        // the IdP vouches for in them as a colleague, and makes them a
        // member at their first sign-in. With no list, anyone with an
        // account at the IdP — which may be a public one — would become a
        // member just by signing in, so there an admin adds each member
        // first.
        None if !domains.is_empty() => {
            state
                .store
                .add_member(email, identity.name.as_deref(), Role::Member)?
        }
        None => {
            return Err(denied(format!(
                "{email} is not a member of this server; ask an admin to add you"
            )))
        }
    };
    if member.disabled_at_ms.is_some() {
        return Err(denied(format!("{email} is disabled on this server")));
    }
    let code = state
        .store
        .issue_code(&member.id, &login.code_challenge, &login.redirect_uri)?;
    state
        .store
        .record_event("signed_in", Some(&member.id), None, None)?;
    Ok(code)
}

// --- the token endpoint ----------------------------------------------------

#[derive(Serialize)]
struct TokenReply<'a> {
    access_token: &'a str,
    refresh_token: &'a str,
    expires_in: i64,
    token_type: &'static str,
}

/// An RFC 6749 §5.2 error body.
fn oauth_error(status: StatusCode, error: &'static str) -> Response {
    (status, Json(json!({ "error": error }))).into_response()
}

/// The access token's lifetime from the config, kept well inside `i64` so
/// the store's `now + ttl` cannot overflow whatever the file says.
fn session_ttl_ms(config: &Config) -> i64 {
    config.session_ttl_ms.min(u64::MAX >> 2) as i64
}

/// The token endpoint, RFC 6749's form in and out. Its replies are never
/// cached (§5.1), and nothing it fails with names the code or the token
/// presented.
async fn token(
    State(state): State<Arc<AppState>>,
    form: Result<Form<HashMap<String, String>>, FormRejection>,
) -> Response {
    let Ok(Form(form)) = form else {
        return oauth_error(StatusCode::BAD_REQUEST, "invalid_request");
    };
    let field = |name: &str| {
        form.get(name)
            .map(String::as_str)
            .filter(|value| !value.is_empty())
    };
    if field("client_id") != Some(CLIENT_ID) {
        return oauth_error(StatusCode::BAD_REQUEST, "invalid_request");
    }
    let ttl = session_ttl_ms(&state.config);
    let issued = match field("grant_type") {
        Some("authorization_code") => {
            let (Some(code), Some(verifier), Some(redirect_uri)) =
                (field("code"), field("code_verifier"), field("redirect_uri"))
            else {
                return oauth_error(StatusCode::BAD_REQUEST, "invalid_request");
            };
            let member_id = match state.store.redeem_code(code, verifier, redirect_uri) {
                Ok(Some(member_id)) => member_id,
                Ok(None) => return oauth_error(StatusCode::BAD_REQUEST, "invalid_grant"),
                Err(e) => return Failure::internal(e).into_response(),
            };
            state.store.create_session(&member_id, None, ttl)
        }
        Some("refresh_token") => {
            let Some(refresh) = field("refresh_token") else {
                return oauth_error(StatusCode::BAD_REQUEST, "invalid_request");
            };
            // A token the server no longer holds is a 401, the status the
            // app reads as "sign in again". That includes one spent before:
            // presenting it again revokes the whole session it belonged to
            // (`Store::refresh_session`), since either the member or a thief
            // holds a copy.
            match state.store.refresh_session(refresh, ttl) {
                Ok(Some(issued)) => Ok(issued),
                Ok(None) => return oauth_error(StatusCode::UNAUTHORIZED, "invalid_grant"),
                Err(e) => Err(e),
            }
        }
        Some(_) => return oauth_error(StatusCode::BAD_REQUEST, "unsupported_grant_type"),
        None => return oauth_error(StatusCode::BAD_REQUEST, "invalid_request"),
    };
    let (_session, access, refresh) = match issued {
        Ok(issued) => issued,
        Err(e) => return Failure::internal(e).into_response(),
    };
    (
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::PRAGMA, "no-cache"),
        ],
        Json(TokenReply {
            access_token: &access,
            refresh_token: &refresh,
            expires_in: ttl / 1000,
            token_type: "Bearer",
        }),
    )
        .into_response()
}

// --- the member and their devices -------------------------------------------

async fn me(
    State(state): State<Arc<AppState>>,
    authed: Authed,
) -> Result<Json<serde_json::Value>, Failure> {
    let tenant = state.store.tenant()?;
    let member = &authed.member;
    Ok(Json(json!({
        "member": { "id": member.id, "email": member.email, "name": member.name },
        "tenant": { "name": tenant.name },
    })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Enroll {
    device_pubkey: Option<String>,
    device_name: Option<String>,
}

/// Enroll the calling app's device, pending, and bind the session to it:
/// from now on the listing seals masters to this device, once approved.
async fn enroll(
    State(state): State<Arc<AppState>>,
    authed: Authed,
    JsonBody(body): JsonBody<Enroll>,
) -> Result<Json<serde_json::Value>, Failure> {
    let public_key = body
        .device_pubkey
        .ok_or_else(|| Failure::bad_request("devicePubkey is required"))?;
    let name = body
        .device_name
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| Failure::bad_request("deviceName is required"))?;
    // Checked here rather than when a master is first sealed to it, which
    // would fail the member's listing long after the enrollment that
    // caused it.
    let is_x25519 = STANDARD
        .decode(public_key.trim())
        .is_ok_and(|bytes| bytes.len() == 32);
    if !is_x25519 {
        return Err(Failure::bad_request(
            "devicePubkey must be an X25519 public key: 32 bytes of base64",
        ));
    }
    let device = state
        .store
        .enroll_device(&authed.member.id, &name, &public_key)?;
    state
        .store
        .bind_session_device(&authed.session.id, &device.id)?;
    state.store.record_event(
        "device_enrolled",
        Some(&authed.member.id),
        Some(&device.id),
        None,
    )?;
    Ok(Json(
        json!({ "deviceId": device.id, "state": device.state }),
    ))
}

/// A device as the member's own list shows it (`docs/server-api.md`).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DeviceView {
    id: String,
    name: String,
    state: DeviceState,
    public_key: String,
    created_at_ms: i64,
}

async fn devices(
    State(state): State<Arc<AppState>>,
    authed: Authed,
) -> Result<Json<Vec<DeviceView>>, Failure> {
    Ok(Json(
        state
            .store
            .devices_of(&authed.member.id)?
            .into_iter()
            .map(|device| DeviceView {
                id: device.id,
                name: device.name,
                state: device.state,
                public_key: device.public_key,
                created_at_ms: device.created_at_ms,
            })
            .collect(),
    ))
}

/// Approve a pending device of the caller's: from one of their approved
/// devices, or by an admin for any member's. Another member's device is
/// as unknown to a member as one that does not exist.
async fn approve(
    State(state): State<Arc<AppState>>,
    authed: Authed,
    Path(id): Path<String>,
) -> Result<StatusCode, Failure> {
    let approver = authed
        .device
        .as_ref()
        .filter(|device| device.state == DeviceState::Approved)
        .filter(|device| device.member_id == authed.member.id);
    if approver.is_none() && !authed.is_admin() {
        return Err(Failure::forbidden(
            "a device is approved from one of your devices that is itself approved",
        ));
    }
    let target = state
        .store
        .device(&id)?
        .filter(|device| authed.is_admin() || device.member_id == authed.member.id)
        .ok_or_else(|| Failure::not_found(format!("no device {id}")))?;
    if target.state == DeviceState::Revoked {
        return Err(Failure::conflict(
            "a revoked device is not approved again; enroll it afresh",
        ));
    }
    let by = approver.map_or(&authed.member.id, |device| &device.id);
    state.store.approve_device(&target.id, by)?;
    state.store.record_event(
        "device_approved",
        Some(&authed.member.id),
        Some(&target.id),
        Some(&json!({ "by": by }).to_string()),
    )?;
    Ok(StatusCode::NO_CONTENT)
}

/// The member's lockboxes; the master sealed to the session's device only
/// when that device is approved (`lockboxes::listing_for`).
async fn lockboxes(
    State(state): State<Arc<AppState>>,
    authed: Authed,
) -> Result<Json<Vec<lockboxes::LockboxListing>>, Failure> {
    Ok(Json(lockboxes::listing_for(
        &state.store,
        &*state.root,
        &authed.member,
        authed.device.as_ref(),
    )?))
}
