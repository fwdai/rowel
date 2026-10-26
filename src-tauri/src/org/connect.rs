//! Signing the install in to a Rowel Server, on desktop: the browser, a
//! loopback listener, and the device's enrollment.
//!
//! The same shape as a provider's consent flow (`sync::account`): PKCE, a
//! listener bound before the browser opens so the redirect names a port this
//! process holds, the code waited for off the command thread and reported
//! through events (`org:connect`), and a cancel that moves an attempt
//! counter so a flow the user walked away from finds itself stale at every
//! step. The server, not a cloud provider, is the authorization server here,
//! so the code is exchanged at its token endpoint and the session it issues
//! is the member's for the whole install (`connections`).
//!
//! A phone has no loopback listener; its twin of this flow is a deep-link
//! one and a later iteration, so the command refuses there.

use std::sync::atomic::{AtomicU64, Ordering};

use tauri::async_runtime::block_on;
use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;

use super::client::ServerClient;
use super::connections::{self, normalize_url, Connection};
use super::{approvals, enrollment};
use crate::error::{Error, Result};
use crate::sync::{self, Provider};
use crate::{commands, events};

/// Which sign-in is current: a cancel or a newer connect moves it, and the
/// flow under way gives up the next time it looks.
static ATTEMPT: AtomicU64 = AtomicU64::new(0);

pub(crate) const PENDING: &str = "pending";
pub(crate) const DONE: &str = "done";
pub(crate) const ERROR: &str = "error";

/// Start signing in to the server at `url`: the attempt's number comes back
/// as soon as the flow is scheduled, and `org:connect` reports the rest
/// under it. Refused before anything is opened for a URL that is not a
/// server's, and while the vault is not open at the app level — the
/// enrollment key and the record of the session are both sealed under the
/// app key, so without it there is nothing to enroll with and nowhere to
/// keep what the sign-in produces.
///
/// An attempt reports only while it is the current one. A cancel, or a
/// newer attempt, disowns it: its sign-in just stops, nothing it produced is
/// kept, and nothing — not even its stopping — is said of it, since the
/// section that asked has moved on and a late word from an old attempt
/// would land on the new one.
pub(crate) fn start(app: &AppHandle, url: &str) -> Result<u64> {
    let url = normalize_url(url)?;
    connections::app_root_and_key(app)?;
    let attempt = ATTEMPT.fetch_add(1, Ordering::SeqCst) + 1;
    events::org_connect(app, &url, attempt, PENDING, None);
    let app = app.clone();
    // A blocking-pool thread: the loopback listener is polled, the HTTP calls
    // are driven with `block_on`.
    commands::detached(move || {
        let outcome = sign_in(&app, &url, attempt);
        if superseded(attempt) {
            return;
        }
        match outcome {
            Ok(()) => {
                events::org_connect(&app, &url, attempt, DONE, None);
                events::org_changed(&app);
                approvals::restart(&app);
            }
            Err(e) => events::org_connect(&app, &url, attempt, ERROR, Some(&e.to_string())),
        }
    });
    Ok(attempt)
}

/// Disown the sign-in under way: its listener closes and nothing it produces
/// is kept or reported.
pub(crate) fn cancel() {
    ATTEMPT.fetch_add(1, Ordering::SeqCst);
}

/// Whether `attempt` is no longer the current sign-in.
fn superseded(attempt: u64) -> bool {
    ATTEMPT.load(Ordering::SeqCst) != attempt
}

fn sign_in(app: &AppHandle, url: &str, attempt: u64) -> Result<()> {
    let cancelled = || superseded(attempt);
    let listener = sync::bind_loopback(&[])?;
    let redirect_uri = sync::loopback_redirect_uri(listener.local_addr()?.port());
    let (verifier, state) = (sync::gen_nonce(), sync::gen_nonce());
    let anonymous = ServerClient::new(url)?;
    if cancelled() {
        return Err(Error::Cancelled);
    }
    let authorize = anonymous.authorize_url(&redirect_uri, &state, &sync::challenge(&verifier));
    app.opener()
        .open_url(authorize, None::<&str>)
        .map_err(|e| Error::Other(e.to_string()))?;
    let code = sync::listen_for_code(&listener, &state, Provider::RowelServer, cancelled)?;
    let session = block_on(anonymous.token(&code, &verifier, &redirect_uri))?;
    let client = ServerClient::with_session(url, &session.access_token)?;
    let me = block_on(client.me())?;

    // This device is enrolled once per server and member: a sign-in that
    // renews a session keeps the device the first one enrolled, and so its
    // approval. Another member signing in on the same install is another
    // member's device.
    let (root, app_key) = connections::app_root_and_key(app)?;
    let held = connections::find(&root, &app_key, url)?
        .filter(|c| c.member_id == me.member.id && c.device_id.is_some());
    let (device_id, device_state) = match held {
        Some(connection) => (connection.device_id, connection.device_state),
        None => {
            let key = enrollment::load_or_create_for(app)?;
            let enrolled = block_on(client.enroll(&key.public_key_b64(), &device_name()))?;
            (Some(enrolled.device_id), Some(enrolled.state))
        }
    };
    let mut connection = Connection {
        url: url.to_string(),
        tenant_name: me.tenant.name,
        member_id: me.member.id,
        member_email: me.member.email,
        member_name: me.member.name,
        device_id,
        device_state,
        access_token: String::new(),
        refresh_token: None,
        expires_at_ms: 0,
    };
    connection.adopt(session, chrono::Utc::now().timestamp_millis());
    // Nothing of a disowned flow is kept, device included: the server will
    // list it until an admin removes it, but this install holds no session
    // for it and the next sign-in enrolls afresh.
    if cancelled() {
        return Err(Error::Cancelled);
    }
    connections::record_sign_in(app, connection)
}

/// What this device is called in the member's device list: the machine's
/// host name, without the `.local` a Mac appends, or the platform when the
/// OS gives no usable name.
fn device_name() -> String {
    gethostname::gethostname()
        .to_str()
        .map(|name| name.trim().trim_end_matches(".local"))
        .filter(|name| !name.is_empty() && *name != "localhost")
        .map(String::from)
        .unwrap_or_else(|| format!("Rowel on {}", os_name()))
}

fn os_name() -> &'static str {
    match std::env::consts::OS {
        "macos" => "macOS",
        "windows" => "Windows",
        "linux" => "Linux",
        other => other,
    }
}
