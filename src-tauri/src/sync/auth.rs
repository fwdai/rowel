//! Google OAuth2 with PKCE (port of `gdrive/auth.js`), in two shapes.
//!
//! Desktop talks to a Desktop-app client: the consent URL opens in the browser
//! and a one-shot loopback listener on 127.0.0.1 captures the auth code, so the
//! whole flow is one blocking call. The listener takes whichever port the OS
//! hands it (Google allows any loopback port for a Desktop-app client) and
//! gives up after [`CONSENT_TIMEOUT`], so a consent tab the user closes
//! without answering cannot wedge a fixed port — or the "pending" state the
//! frontend shows — until the app restarts.
//!
//! iOS talks to an iOS client, which is public (no secret, PKCE mandatory) and
//! is registered against a redirect URI on its own URL scheme. Safari takes the
//! screen and the app is suspended behind it, so the flow has to be cut in half
//! — [`begin`] opens the consent page, [`complete`] runs when iOS reopens the
//! app with the redirect. Everything between the two (the PKCE pair, the auth
//! URL, the token exchange, the token file) is shared.
//!
//! Every request carries a random `state` that the redirect has to echo back
//! ([`parse_redirect`]). On desktop that is belt-and-braces over a loopback port
//! only this process listens on; on iOS it is the whole defence — any app can
//! open a URL on our scheme, and without the nonce one could pass off a stray
//! URL as Google's answer to a flow it never saw.
//!
//! Tokens are stored encrypted via the vault cryptor either way.

#[cfg(desktop)]
use std::io::{BufRead, BufReader, Write};
#[cfg(desktop)]
use std::net::TcpListener;
#[cfg(desktop)]
use std::time::{Duration, Instant};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::RngCore;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;
use url::Url;

#[cfg(desktop)]
use crate::app::APP_NAME;
use crate::error::{Error, Result};
use crate::{crypto::Cryptor, storage};

#[cfg(desktop)]
const HOST: &str = "127.0.0.1";
#[cfg(desktop)]
const CALLBACK: &str = "/auth/callback";
/// How long the loopback listener waits for the browser to come back with an
/// answer. Long enough to read a consent screen and pick an account; short
/// enough that an abandoned tab frees the flow within the session.
#[cfg(desktop)]
const CONSENT_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// How often the listener checks the deadline while nothing has connected.
#[cfg(desktop)]
const ACCEPT_POLL: Duration = Duration::from_millis(100);
/// How long one connection gets to send its request line.
#[cfg(desktop)]
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const SCOPE: &str = "https://www.googleapis.com/auth/drive.file";
const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

// Supply your own Google OAuth client at build or run time.
const CLIENT_ID_PLACEHOLDER: &str = "YOUR_GOOGLE_OAUTH_CLIENT_ID";

/// A Google iOS client is addressed by its *reversed* client id, which is also
/// the URL scheme the app registers: client id `123-abc.apps.googleusercontent
/// .com` <-> scheme `com.googleusercontent.apps.123-abc`.
#[cfg(any(mobile, test))]
const IOS_SCHEME_PREFIX: &str = "com.googleusercontent.apps.";
#[cfg(any(mobile, test))]
const IOS_CLIENT_ID_SUFFIX: &str = ".apps.googleusercontent.com";
/// One slash, not two: a reversed-client-id scheme has no authority for a
/// second one to separate. This is the exact string Google registers.
#[cfg(any(mobile, test))]
const IOS_REDIRECT_PATH: &str = ":/oauth2redirect";

/// What a redirect URL turned out to be, once checked against the request that
/// is waiting for one.
#[derive(Debug, PartialEq, Eq)]
pub enum Redirect {
    /// Google's answer to *our* request: the code to exchange.
    Code(String),
    /// Google's answer to our request, and the answer was no (or nothing).
    Denied(String),
    /// Not an answer to our request at all: the `state` is missing or belongs
    /// to some other flow. Whatever is pending must not be touched on its
    /// account.
    Foreign,
}

/// The OAuth grant. `Debug` is hand-written below: both tokens are bearer
/// credentials for the user's Drive, and a derived impl would print them into
/// whatever `{:?}` they reached.
#[derive(Default, Clone, Serialize, Deserialize)]
pub struct Tokens {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
}

impl std::fmt::Debug for Tokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Presence, never the value: that is all a log line needs to tell a
        // connected account from a bare one.
        let redacted = |token: &Option<String>| token.as_ref().map(|_| "<redacted>");
        f.debug_struct("Tokens")
            .field("access_token", &redacted(&self.access_token))
            .field("refresh_token", &redacted(&self.refresh_token))
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
}

fn other<E: std::fmt::Display>(e: E) -> Error {
    Error::Other(e.to_string())
}

// --- which client this build talks to ---

/// The OAuth client and redirect this platform uses. Resolved per call rather
/// than cached: it is three string reads, and a cache would only add a way for
/// the two halves of the mobile flow to disagree.
struct Credentials {
    client_id: String,
    redirect_uri: String,
    /// Desktop clients ship a (non-confidential) secret. An iOS client is a
    /// public client and must never send one, so this stays `None` there.
    secret: Option<String>,
}

fn env_client_id() -> Option<String> {
    std::env::var("GOOGLE_OAUTH_CLIENT_ID")
        .ok()
        .or_else(|| option_env!("GOOGLE_OAUTH_CLIENT_ID").map(String::from))
        .filter(|id| !id.is_empty() && id != CLIENT_ID_PLACEHOLDER)
}

fn no_client() -> Error {
    Error::Other("Google OAuth client not configured; set GOOGLE_OAUTH_CLIENT_ID".into())
}

#[cfg(desktop)]
impl Credentials {
    /// The client alone. The redirect is left empty because it depends on the
    /// port the consent flow's listener lands on ([`Credentials::with_loopback`]);
    /// a token refresh, the other caller, never sends one.
    fn resolve(_app: &AppHandle) -> Result<Self> {
        Ok(Self {
            client_id: env_client_id().ok_or_else(no_client)?,
            redirect_uri: String::new(),
            secret: std::env::var("GOOGLE_OAUTH_CLIENT_SECRET")
                .ok()
                .or_else(|| option_env!("GOOGLE_OAUTH_CLIENT_SECRET").map(String::from))
                .filter(|s| !s.is_empty()),
        })
    }

    /// Address the redirect at the loopback listener bound to `port`.
    fn with_loopback(mut self, port: u16) -> Self {
        self.redirect_uri = format!("http://{HOST}:{port}{CALLBACK}");
        self
    }
}

#[cfg(mobile)]
impl Credentials {
    /// The committed deep-link scheme is the source of truth (iOS client ids
    /// are public, and the scheme is what actually registers the app for the
    /// redirect). `GOOGLE_OAUTH_CLIENT_ID` overrides it for a build that is
    /// given the id from a secret — and the redirect is then derived back from
    /// that same id, so the two can never disagree at runtime.
    fn resolve(app: &AppHandle) -> Result<Self> {
        let client_id = match env_client_id() {
            Some(id) => id,
            None => client_id_from_scheme(&deep_link_scheme(app)?).ok_or_else(no_client)?,
        };
        let scheme = scheme_from_client_id(&client_id).ok_or_else(no_client)?;
        Ok(Self {
            redirect_uri: format!("{scheme}{IOS_REDIRECT_PATH}"),
            client_id,
            secret: None,
        })
    }
}

/// `com.googleusercontent.apps.123-abc` -> `123-abc.apps.googleusercontent.com`
#[cfg(any(mobile, test))]
fn client_id_from_scheme(scheme: &str) -> Option<String> {
    let id = scheme.strip_prefix(IOS_SCHEME_PREFIX)?;
    (!id.is_empty()).then(|| format!("{id}{IOS_CLIENT_ID_SUFFIX}"))
}

/// The inverse of [`client_id_from_scheme`].
#[cfg(any(mobile, test))]
fn scheme_from_client_id(client_id: &str) -> Option<String> {
    let id = client_id.strip_suffix(IOS_CLIENT_ID_SUFFIX)?;
    (!id.is_empty()).then(|| format!("{IOS_SCHEME_PREFIX}{id}"))
}

/// The reversed-client-id scheme out of `plugins.deep-link.mobile` (see
/// `tauri.ios.conf.json`). Other schemes may be configured for other purposes,
/// so this picks the one that looks like a Google client rather than the first.
#[cfg(mobile)]
fn deep_link_scheme(app: &AppHandle) -> Result<String> {
    app.config()
        .plugins
        .0
        .get("deep-link")
        .and_then(|c| c.get("mobile")?.as_array())
        .and_then(|domains| {
            domains.iter().find_map(|d| {
                d.get("scheme")?
                    .as_array()?
                    .iter()
                    .filter_map(|s| s.as_str())
                    .find(|s| s.starts_with(IOS_SCHEME_PREFIX))
                    .map(str::to_owned)
            })
        })
        .ok_or_else(no_client)
}

/// Whether a deep link is *this build's* OAuth redirect — the configured
/// scheme and path exactly — rather than some other URL the app was opened
/// with. Says nothing about which request it answers; [`parse_redirect`] does.
#[cfg(mobile)]
pub fn redirect_matches(app: &AppHandle, url: &Url) -> bool {
    Credentials::resolve(app).is_ok_and(|c| matches_redirect_uri(url, &c.redirect_uri))
}

/// `url` is `redirect_uri` plus, at most, a query: same scheme (schemes are
/// case-insensitive, and the `url` crate lowercases the parsed one) and the same
/// path. A prefix match on the scheme would accept every Google client's
/// redirect, not just ours.
#[cfg(any(mobile, test))]
fn matches_redirect_uri(url: &Url, redirect_uri: &str) -> bool {
    Url::parse(redirect_uri).is_ok_and(|expected| {
        url.scheme().eq_ignore_ascii_case(expected.scheme()) && url.path() == expected.path()
    })
}

// --- token persistence (encrypted `auth/gdrive.swftx`) ---

/// The encrypted token file, and the connection generation that says which
/// account the tokens in it belong to.
///
/// The two move together — a disconnect deletes and bumps, a password change's
/// re-seal rewrites and bumps — and a refresh's write-back is only allowed if
/// the generation it read is still the one on the file. So they are paired
/// here, and every transition takes the generation's mutex for its whole
/// length: a bare compare would leave a gap between it and the write for a
/// disconnect to land in. On a plain path rather than an `AppHandle`, so the
/// transitions can be exercised against a scratch directory.
struct TokenFile<'a> {
    path: std::path::PathBuf,
    /// `AppState::sync_generation` in the app; a bare mutex in a test.
    generation: &'a std::sync::Mutex<u64>,
}

impl<'a> TokenFile<'a> {
    fn new(path: std::path::PathBuf, generation: &'a std::sync::Mutex<u64>) -> Self {
        Self { path, generation }
    }

    /// The active workspace's token file and the app's connection generation.
    fn of(app: &'a AppHandle) -> Result<Self> {
        Ok(Self::new(
            storage::gdrive_path(app)?,
            &app_state(app).inner().sync_generation,
        ))
    }

    fn read(&self, cryptor: &Cryptor) -> Option<Tokens> {
        read_sealed(&self.path, cryptor)
    }

    /// Seal the tokens under `cryptor` and write them. Atomic, so a crash
    /// mid-write leaves the previous grant rather than a truncated file that
    /// reads as "not connected" — and owner-only from creation.
    fn write(&self, cryptor: &Cryptor, tokens: &Tokens) -> Result<()> {
        let json = serde_json::to_string(tokens)?;
        storage::atomic_write_private(&self.path, cryptor.encrypt(&json)?.as_bytes())
    }

    /// The tokens together with the generation they belong to, read as one:
    /// the pair is what a write-back is later judged against, and reading them
    /// separately would let a disconnect slip between and hand back tokens
    /// stamped with a generation that never held them.
    fn snapshot(&self, cryptor: &Cryptor) -> Result<(u64, Tokens)> {
        let generation = self.generation.lock().unwrap();
        let tokens = self.read(cryptor).ok_or(Error::SyncNotConfigured)?;
        Ok((*generation, tokens))
    }

    /// Bump the generation and delete the file under one hold of the guard, so
    /// a refresh's write-back and a re-seal (which take the same guard) either
    /// see the old generation and finish before this runs, or see the new one
    /// and leave the file gone.
    fn disconnect(&self) -> Result<()> {
        let mut generation = self.generation.lock().unwrap();
        *generation += 1;
        storage::remove_if_present(&self.path)
    }

    /// Read with `old` and write back with `new`, under the guard so a
    /// disconnect cannot land between them and have its delete undone by the
    /// write.
    ///
    /// The generation moves too, exactly as [`TokenFile::disconnect`] moves it:
    /// a sync run that captured the *old* cryptor can be awaiting a token
    /// refresh right now, and its write-back would re-seal the file under the
    /// key the change just retired — leaving credentials no later unlock could
    /// decrypt. Bumping here makes that write-back's
    /// [`TokenFile::persist_if_current`] return `Ok(false)`, so the in-flight
    /// run winds down and leaves the freshly re-sealed file alone.
    ///
    /// No file is no connection to re-seal: nothing is written and the
    /// generation stands.
    fn reseal(&self, old: &Cryptor, new: &Cryptor) -> Result<()> {
        let mut generation = self.generation.lock().unwrap();
        let Some(tokens) = self.read(old) else {
            return Ok(());
        };
        self.write(new, &tokens)?;
        *generation += 1;
        Ok(())
    }

    /// Write the tokens if the connection they belong to is still
    /// `generation`, and say whether it was. `false` means the token file
    /// moved out from under `generation` — disconnected, or re-sealed by a
    /// password change — and nothing was written; whatever the tokens were for
    /// is the caller's to wind down.
    fn persist_if_current(
        &self,
        cryptor: &Cryptor,
        tokens: &Tokens,
        generation: u64,
    ) -> Result<bool> {
        let current = self.generation.lock().unwrap();
        if *current != generation {
            return Ok(false);
        }
        self.write(cryptor, tokens)?;
        Ok(true)
    }
}

/// The tokens sealed in the file at `path`, if there are any to read. An
/// absent, empty or undecryptable file all mean the same thing — not connected
/// — so none of them is an error.
fn read_sealed(path: &std::path::Path, cryptor: &Cryptor) -> Option<Tokens> {
    let blob = storage::read_file(path).ok().filter(|b| !b.is_empty())?;
    let json = cryptor.decrypt(&blob).ok()?;
    serde_json::from_str(&json).ok()
}

pub fn read_tokens(app: &AppHandle, cryptor: &Cryptor) -> Option<Tokens> {
    TokenFile::of(app).ok()?.read(cryptor)
}

/// [`read_tokens`] from a named workspace directory rather than the active
/// workspace's — the twin of [`write_tokens_in`], for a caller acting on a
/// workspace that is very likely locked (`commands::workspace::workspace_delete`).
///
/// Strict where [`read_sealed`] shrugs: `Ok(None)` is no token file at all, and
/// every other failure is an error. That caller is about to delete the only
/// copy of these credentials, and a file it could not read is not a workspace
/// that never connected — folding the two together would leave the vault's pack
/// live on Drive and take the workspace needed to retry the delete with it.
pub fn read_tokens_in(dir: &std::path::Path, cryptor: &Cryptor) -> Result<Option<Tokens>> {
    let path = dir.join(storage::GDRIVE_FILE);
    let blob = match std::fs::read_to_string(&path) {
        Ok(blob) => blob,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    // An empty file is a write that did not finish, not an absent one: the
    // grant it was meant to hold may still be live at Google.
    if blob.trim().is_empty() {
        return Err(Error::Other(unreadable_tokens_error()));
    }
    let json = cryptor
        .decrypt(&blob)
        .map_err(|_| Error::Other(unreadable_tokens_error()))?;
    let tokens =
        serde_json::from_str(&json).map_err(|_| Error::Other(unreadable_tokens_error()))?;
    Ok(Some(tokens))
}

// One message for every way the file can be there and unusable: the user can
// do the same thing about all of them, and none of them says which vault key
// or which byte was wrong.
fn unreadable_tokens_error() -> String {
    "this lockbox's saved Google account could not be read".into()
}

/// Seal the tokens under `cryptor` and write them.
///
/// Crate-visible because first-run onboarding holds tokens in memory long
/// before a vault key exists to seal them with, and persists them only once the
/// restore or create it is driving has produced one.
pub fn write_tokens(app: &AppHandle, cryptor: &Cryptor, tokens: &Tokens) -> Result<()> {
    TokenFile::of(app)?.write(cryptor, tokens)
}

/// [`write_tokens`] into a named workspace directory rather than the active
/// workspace's, for a vault being added beside the open one.
pub fn write_tokens_in(dir: &std::path::Path, cryptor: &Cryptor, tokens: &Tokens) -> Result<()> {
    let json = serde_json::to_string(tokens)?;
    storage::write_gdrive_in(dir, &cryptor.encrypt(&json)?)
}

pub fn is_configured(app: &AppHandle, cryptor: &Cryptor) -> bool {
    read_tokens(app, cryptor).is_some_and(|t| t.access_token.is_some() || t.refresh_token.is_some())
}

/// Forget the account: the token file goes, so nothing survives to mint a new
/// access token with. Keeping the refresh token here (as the legacy client did)
/// meant the next unlock read the file back as "configured" and auto-sync
/// uploaded to an account the user had just disconnected.
///
/// Local only: the grant at Google is left alone, because revoking it would
/// retire every token for the account and client — other workspaces', other
/// devices' — and a workspace cannot know who else holds one (see
/// `commands::sync::sync_disconnect`).
///
/// A failed delete is an error, not a shrug: the refresh token is still on disk
/// and still usable, so the only honest answer is that the account is *not*
/// disconnected.
///
/// The generation and the file move together ([`TokenFile::disconnect`]), so
/// an in-flight refresh cannot put the file back.
pub fn disconnect(app: &AppHandle) -> Result<()> {
    TokenFile::of(app)?.disconnect()
}

/// Re-seal the token file under `new` — a password change moved the vault key
/// it was sealed with. See [`TokenFile::reseal`]: the connection generation
/// moves with it, so a refresh that captured the old cryptor cannot write the
/// file back under the key the change just retired.
pub fn reseal_tokens(app: &AppHandle, old: &Cryptor, new: &Cryptor) -> Result<()> {
    TokenFile::of(app)?.reseal(old, new)
}

// --- OAuth flow ---

/// One consent request in flight: the PKCE verifier the code will be redeemed
/// with, and the `state` nonce the redirect has to echo to be believed.
pub struct Started {
    pub verifier: String,
    pub state: String,
}

// Send the user to Google's consent page, and hand back what the redirect will
// need to be recognised and redeemed.
fn open_consent(app: &AppHandle, credentials: &Credentials) -> Result<Started> {
    let started = Started {
        verifier: gen_nonce(),
        state: gen_nonce(),
    };
    let url = auth_url(credentials, &challenge(&started.verifier), &started.state)?;
    app.opener().open_url(url, None::<&str>).map_err(other)?;
    Ok(started)
}

/// Desktop: open the browser and block on the loopback listener until Google
/// redirects to it, then exchange the code — handing the tokens back rather
/// than writing them.
///
/// Every connect is keyless: onboarding has no vault yet, and a sync connect
/// on an open vault first asks what the account holds before the tokens are
/// sealed under it (`commands::sync::sync_adopt_pending`). Blocking (the
/// loopback listener): call it off the main thread.
#[cfg(desktop)]
pub fn obtain_tokens(app: &AppHandle) -> Result<Tokens> {
    // Bound before the browser opens, on a port the OS picks: the redirect
    // then names a port this process actually holds, and two attempts (or two
    // apps) can never fight over one.
    let listener = TcpListener::bind((HOST, 0)).map_err(other)?;
    let port = listener.local_addr()?.port();
    let credentials = Credentials::resolve(app)?.with_loopback(port);
    let started = open_consent(app, &credentials)?;
    let code = listen_for_code(&listener, &started.state)?;
    let client = super::http_client();
    tauri::async_runtime::block_on(exchange_code(
        &client,
        &credentials,
        &code,
        &started.verifier,
    ))
}

/// Mobile, first half: open the consent page and hand back what the caller
/// must keep until the redirect arrives.
#[cfg(mobile)]
pub fn begin(app: &AppHandle) -> Result<Started> {
    open_consent(app, &Credentials::resolve(app)?)
}

/// Mobile, second half: exchange a code [`parse_redirect`] accepted, handing
/// the tokens back rather than writing them — the twin of [`obtain_tokens`].
/// Async: this runs off the URL-open callback, not on it.
#[cfg(mobile)]
pub async fn exchange_for_tokens(app: &AppHandle, code: &str, verifier: &str) -> Result<Tokens> {
    let credentials = Credentials::resolve(app)?;
    exchange_code(&super::http_client(), &credentials, code, verifier).await
}

/// Read a redirect URL as the answer to the request identified by `state`.
///
/// The nonce is checked before anything else is believed: a URL without it, or
/// with someone else's, is [`Redirect::Foreign`] no matter what code or error
/// it carries.
pub fn parse_redirect(url: &Url, state: &str) -> Redirect {
    let mut code = None;
    let mut error = None;
    let mut echoed = None;
    for (key, value) in url.query_pairs() {
        match &*key {
            "code" => code = Some(value.into_owned()),
            "error" => error = Some(value.into_owned()),
            "state" => echoed = Some(value.into_owned()),
            _ => {}
        }
    }
    if echoed.as_deref() != Some(state) {
        return Redirect::Foreign;
    }
    match code {
        Some(code) => Redirect::Code(code),
        None => Redirect::Denied(
            error.unwrap_or_else(|| "Authorization was cancelled or failed".into()),
        ),
    }
}

// Return a valid access token, refreshing it if expired.
pub async fn access_token(client: &Client, app: &AppHandle, cryptor: &Cryptor) -> Result<String> {
    // Which connection these tokens belong to, read before they are. A refresh
    // awaits a network round trip between the read and the write-back, and no
    // lock is held across it (this codebase never holds one across a network
    // call — see `AppState::workspace_lock`), so a disconnect can delete the
    // token file in that window; the generation is what lets the write-back
    // notice.
    let (generation, mut tokens) = TokenFile::of(app)?.snapshot(cryptor)?;
    // Asked before the call, because that is what says whether the file on
    // disk is now out of date — afterwards the tokens look fresh either way.
    let refreshing = needs_refresh(&tokens);
    let token = fresh_access_token(client, app, &mut tokens).await?;
    // Skipping the write is the whole point: it would undo the disconnect that
    // deleted the file, or re-seal it under the key a password change just
    // retired. The caller still gets this token for the request it is in the
    // middle of, which is harmless — either way nothing after this refreshes
    // against the stale file again.
    if refreshing && !persisted_if_current(app, cryptor, &tokens, generation)? {
        log::info!(
            "the Drive connection moved on mid-refresh; not writing the refreshed tokens back"
        );
    }
    Ok(token)
}

/// [`TokenFile::persist_if_current`] on the active workspace's token file:
/// write the tokens if the connection they belong to is still `generation`,
/// and say whether it was.
pub(crate) fn persisted_if_current(
    app: &AppHandle,
    cryptor: &Cryptor,
    tokens: &Tokens,
    generation: u64,
) -> Result<bool> {
    TokenFile::of(app)?.persist_if_current(cryptor, tokens, generation)
}

fn app_state(app: &AppHandle) -> tauri::State<'_, crate::state::AppState> {
    use tauri::Manager;
    app.state::<crate::state::AppState>()
}

/// A valid access token for `tokens`, refreshing them *in place* if the one
/// they hold has expired.
///
/// Where they came from and whether they are ever written back is the caller's
/// business: [`access_token`] reads and rewrites the encrypted token file,
/// while onboarding holds the only copy in memory and has nowhere to write it
/// until a vault exists.
pub async fn fresh_access_token(
    client: &Client,
    app: &AppHandle,
    tokens: &mut Tokens,
) -> Result<String> {
    if needs_refresh(tokens) {
        let refresh_token = tokens
            .refresh_token
            .clone()
            .ok_or(Error::SyncNotConfigured)?;
        let fresh = refresh(client, &Credentials::resolve(app)?, &refresh_token).await?;
        tokens.access_token = fresh.access_token;
        tokens.expires_at = fresh.expires_at;
        // Google only re-issues a refresh token sometimes; keeping the old one
        // otherwise is what stops a refresh from disconnecting the account.
        if fresh.refresh_token.is_some() {
            tokens.refresh_token = fresh.refresh_token;
        }
    }
    tokens.access_token.clone().ok_or(Error::SyncNotConfigured)
}

fn needs_refresh(tokens: &Tokens) -> bool {
    match tokens.expires_at {
        _ if tokens.access_token.is_none() => true,
        Some(expires_at) => expires_at <= chrono::Utc::now().timestamp() + 60,
        None => true,
    }
}

fn auth_url(credentials: &Credentials, challenge: &str, state: &str) -> Result<String> {
    let mut url = Url::parse(AUTH_URL).map_err(other)?;
    url.query_pairs_mut()
        .append_pair("client_id", &credentials.client_id)
        .append_pair("redirect_uri", &credentials.redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("scope", SCOPE)
        .append_pair("access_type", "offline")
        .append_pair("prompt", "consent")
        .append_pair("state", state)
        .append_pair("code_challenge", challenge)
        .append_pair("code_challenge_method", "S256");
    Ok(url.into())
}

async fn exchange_code(
    client: &Client,
    credentials: &Credentials,
    code: &str,
    verifier: &str,
) -> Result<Tokens> {
    let form = vec![
        ("code", code.to_string()),
        ("redirect_uri", credentials.redirect_uri.clone()),
        ("grant_type", "authorization_code".to_string()),
        ("code_verifier", verifier.to_string()),
    ];
    post_token(client, credentials, form).await
}

async fn refresh(
    client: &Client,
    credentials: &Credentials,
    refresh_token: &str,
) -> Result<Tokens> {
    let form = vec![
        ("refresh_token", refresh_token.to_string()),
        ("grant_type", "refresh_token".to_string()),
    ];
    post_token(client, credentials, form).await
}

async fn post_token(
    client: &Client,
    credentials: &Credentials,
    mut form: Vec<(&str, String)>,
) -> Result<Tokens> {
    form.push(("client_id", credentials.client_id.clone()));
    if let Some(secret) = &credentials.secret {
        form.push(("client_secret", secret.clone()));
    }
    let resp = client
        .post(TOKEN_URL)
        .form(&form)
        .send()
        .await
        .map_err(other)?;
    let status = resp.status();
    let body = resp.text().await.map_err(other)?;
    if !status.is_success() {
        return Err(Error::Other(format!("OAuth token error {status}: {body}")));
    }
    let parsed: TokenResponse = serde_json::from_str(&body).map_err(other)?;
    Ok(Tokens {
        access_token: parsed.access_token,
        refresh_token: parsed.refresh_token,
        expires_at: parsed
            .expires_in
            .map(|s| chrono::Utc::now().timestamp() + s),
    })
}

// --- PKCE ---

// 48 random bytes, url-safe: within the 43-128 characters PKCE allows a
// verifier, and more than enough for the `state` nonce.
fn gen_nonce() -> String {
    let mut bytes = [0u8; 48];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

// --- loopback listener (desktop) ---

// Block until the browser hits the callback, returning the auth code — or
// until `CONSENT_TIMEOUT` passes with no answer, which is an error like any
// other refusal and frees whatever the caller had marked pending.
//
// `std` has no accept-with-timeout, so the listener is non-blocking and polled
// against the deadline. Browsers also open connections that are not the
// answer (a favicon request, a speculative connection that sends nothing), so
// anything that is not this request's answer — another path, or a callback
// that does not echo our `state` — is answered 404 and the wait goes on.
#[cfg(desktop)]
fn listen_for_code(listener: &TcpListener, state: &str) -> Result<String> {
    listen_for_code_until(listener, state, Instant::now() + CONSENT_TIMEOUT)
}

#[cfg(desktop)]
fn listen_for_code_until(listener: &TcpListener, state: &str, deadline: Instant) -> Result<String> {
    listener.set_nonblocking(true)?;
    loop {
        // Checked on every pass, connection or not: a local process that keeps
        // connecting without ever sending a request could otherwise hold the
        // flow open past the deadline, one read timeout at a time. The same
        // remaining time also bounds each read, so the last connection cannot
        // stretch the wait either.
        let Some(remaining) = deadline
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
        else {
            return Err(Error::Other(
                "Google sign-in took too long; try again".into(),
            ));
        };
        let (mut stream, _) = match listener.accept() {
            Ok(connection) => connection,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(ACCEPT_POLL.min(remaining));
                continue;
            }
            Err(e) => return Err(e.into()),
        };
        // Some platforms hand the accepted socket the listener's non-blocking
        // flag; the read below wants a plain deadline instead.
        stream.set_nonblocking(false)?;
        stream.set_read_timeout(Some(REQUEST_TIMEOUT.min(remaining)))?;

        let mut reader = BufReader::new(&stream);
        let mut request_line = String::new();
        if reader.read_line(&mut request_line).is_err() {
            continue;
        }

        // "GET /auth/callback?code=... HTTP/1.1"
        let path = request_line.split_whitespace().nth(1).unwrap_or("");
        if !path.starts_with(CALLBACK) {
            let _ = stream.write_all(NOT_FOUND.as_bytes());
            continue;
        }
        let Ok(url) = Url::parse(&format!("http://{HOST}{path}")) else {
            let _ = stream.write_all(NOT_FOUND.as_bytes());
            continue;
        };
        let code = match parse_redirect(&url, state) {
            Redirect::Code(code) => Ok(code),
            Redirect::Denied(error) => Err(Error::Other(error)),
            // Not Google's answer to *our* request. Anything that can reach
            // this port — another local process, or a page in the same
            // browser — could otherwise end the sign-in with a bare hit on the
            // callback path, leaving the real redirect to a dead port. Treated
            // like any other unknown path: answered, and the wait goes on
            // until the real answer or the deadline.
            Redirect::Foreign => {
                let _ = stream.write_all(NOT_FOUND.as_bytes());
                continue;
            }
        };

        let _ = stream.write_all(response_html(code.as_ref().err()).as_bytes());
        return code;
    }
}

#[cfg(desktop)]
const NOT_FOUND: &str = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

// The error shown on the page comes off the redirect's query string, so it is
// Google's text but not ours: escaped rather than trusted.
#[cfg(desktop)]
fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(desktop)]
fn response_html(error: Option<&Error>) -> String {
    let (status, body) = match error {
        None => (
            "200 OK",
            format!(
                "<h2>You've successfully connected!</h2><p>You may now close this window and return to {APP_NAME}.</p>"
            ),
        ),
        Some(error) => (
            "400 Bad Request",
            format!(
                "<h2>Failed to connect your Google Drive account.</h2><p>{}</p>",
                escape_html(&error.to_string())
            ),
        ),
    };
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html\r\nConnection: close\r\n\r\n\
         <!doctype html><html><body style=\"font-family:sans-serif;text-align:center;padding:64px\">{body}</body></html>"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_is_url_safe_sha256() {
        // Known RFC 7636 test vector.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(
            challenge(verifier),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn verifier_length_within_pkce_bounds() {
        let v = gen_nonce();
        assert!((43..=128).contains(&v.len()));
    }

    // The redirect follows the listener, not the other way round: whatever
    // port the OS hands out is the one Google is told to come back to.
    #[cfg(desktop)]
    #[test]
    fn the_desktop_redirect_names_the_listeners_port() {
        let credentials = Credentials {
            client_id: "1-a.apps.googleusercontent.com".into(),
            redirect_uri: String::new(),
            secret: None,
        }
        .with_loopback(51234);
        assert_eq!(
            credentials.redirect_uri,
            "http://127.0.0.1:51234/auth/callback"
        );
    }

    // An answer that never comes is an error, not a thread parked forever.
    #[cfg(desktop)]
    #[test]
    fn a_listener_nobody_connects_to_gives_up() {
        let listener = TcpListener::bind((HOST, 0)).unwrap();
        // The real deadline is minutes; the loop's shape is what this checks,
        // so run it against a deadline that has already passed.
        let started = Instant::now();
        let result = listen_for_code_until(&listener, "nonce", started);
        assert!(result.is_err());
        assert!(started.elapsed() < CONSENT_TIMEOUT);
    }

    // A local process that keeps connecting but never sends a request must
    // not hold the flow open: each silent connection is bounded by what is
    // left of the deadline, not by a fresh read timeout of its own.
    #[cfg(desktop)]
    #[test]
    fn silent_connections_cannot_stretch_the_deadline() {
        let listener = TcpListener::bind((HOST, 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let pest = {
            let stop = stop.clone();
            std::thread::spawn(move || {
                let mut held = Vec::new();
                while !stop.load(std::sync::atomic::Ordering::SeqCst) {
                    if let Ok(stream) = std::net::TcpStream::connect(addr) {
                        held.push(stream);
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
            })
        };

        let deadline = Duration::from_millis(300);
        let started = Instant::now();
        let result = listen_for_code_until(&listener, "nonce", started + deadline);
        stop.store(true, std::sync::atomic::Ordering::SeqCst);
        pest.join().unwrap();

        assert!(result.is_err());
        // Well inside one `REQUEST_TIMEOUT`: the deadline, not the per-socket
        // read timeout, is what ended the wait.
        assert!(started.elapsed() < deadline + Duration::from_secs(2));
    }

    // A hit on the callback path that is not an answer to this request must
    // not end the flow: anything on the machine can reach the loopback port,
    // and the real redirect is still on its way.
    #[cfg(desktop)]
    #[test]
    fn a_foreign_callback_leaves_the_flow_waiting() {
        let listener = TcpListener::bind((HOST, 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        let browser = std::thread::spawn(move || {
            let mut stray = std::net::TcpStream::connect(addr).unwrap();
            stray
                .write_all(b"GET /auth/callback?code=stray&state=someone-elses HTTP/1.1\r\n\r\n")
                .unwrap();
            // The server closes it after the 404, which is what ends this read.
            let mut answer = String::new();
            std::io::Read::read_to_string(&mut stray, &mut answer).unwrap();
            assert!(answer.starts_with("HTTP/1.1 404"), "{answer}");

            let mut real = std::net::TcpStream::connect(addr).unwrap();
            real.write_all(b"GET /auth/callback?code=4/abc&state=nonce HTTP/1.1\r\n\r\n")
                .unwrap();
            let mut answer = String::new();
            std::io::Read::read_to_string(&mut real, &mut answer).unwrap();
            assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
        });

        let code = listen_for_code_until(&listener, "nonce", Instant::now() + REQUEST_TIMEOUT);
        browser.join().unwrap();
        assert_eq!(code.unwrap(), "4/abc");
    }

    // Google's error text lands in the page as text, not as markup.
    #[cfg(desktop)]
    #[test]
    fn the_failure_page_escapes_the_error() {
        let page = response_html(Some(&Error::Other(
            "<script>alert('x' & \"y\")</script>".into(),
        )));
        assert!(
            page.contains("&lt;script&gt;alert(&#39;x&#39; &amp; &quot;y&quot;)&lt;/script&gt;")
        );
        assert!(!page.contains("<script>"));
    }

    #[test]
    fn the_consent_url_carries_the_state() {
        let credentials = Credentials {
            client_id: "1-a.apps.googleusercontent.com".into(),
            redirect_uri: "com.googleusercontent.apps.1-a:/oauth2redirect".into(),
            secret: None,
        };
        let url = Url::parse(&auth_url(&credentials, "chal", "nonce-1").unwrap()).unwrap();
        let state = url
            .query_pairs()
            .find(|(k, _)| k == "state")
            .map(|(_, v)| v.into_owned());
        assert_eq!(state.as_deref(), Some("nonce-1"));
    }

    #[test]
    fn only_the_exact_redirect_matches() {
        let ours = "com.googleusercontent.apps.1-a:/oauth2redirect";
        let matches = |s: &str| matches_redirect_uri(&Url::parse(s).unwrap(), ours);
        assert!(matches(
            "com.googleusercontent.apps.1-a:/oauth2redirect?code=x&state=s"
        ));
        // Schemes are case-insensitive.
        assert!(matches("COM.googleusercontent.apps.1-a:/oauth2redirect"));
        // Another Google client's scheme shares the prefix and is not ours.
        assert!(!matches(
            "com.googleusercontent.apps.2-b:/oauth2redirect?code=x"
        ));
        // Our scheme, some other path.
        assert!(!matches("com.googleusercontent.apps.1-a:/anything?code=x"));
        assert!(!matches("rowel:/oauth2redirect"));
    }

    #[test]
    fn ios_scheme_and_client_id_are_inverses() {
        let scheme = "com.googleusercontent.apps.123456-abcdef";
        let client_id = "123456-abcdef.apps.googleusercontent.com";
        assert_eq!(client_id_from_scheme(scheme).as_deref(), Some(client_id));
        assert_eq!(scheme_from_client_id(client_id).as_deref(), Some(scheme));
        assert_eq!(
            format!("{scheme}{IOS_REDIRECT_PATH}"),
            "com.googleusercontent.apps.123456-abcdef:/oauth2redirect"
        );
    }

    #[test]
    fn a_foreign_scheme_yields_no_client_id() {
        assert!(client_id_from_scheme("rowel").is_none());
        // The prefix alone names no client.
        assert!(client_id_from_scheme("com.googleusercontent.apps.").is_none());
        assert!(scheme_from_client_id("123-abc.example.com").is_none());
    }

    const REDIRECT: &str = "com.googleusercontent.apps.1-a:/oauth2redirect";

    #[test]
    fn a_redirect_with_our_state_yields_its_code() {
        let url = Url::parse(&format!("{REDIRECT}?code=4/abc&scope=x&state=nonce-1")).unwrap();
        assert_eq!(
            parse_redirect(&url, "nonce-1"),
            Redirect::Code("4/abc".into())
        );
    }

    #[test]
    fn a_denied_redirect_yields_googles_error() {
        let url = Url::parse(&format!("{REDIRECT}?error=access_denied&state=nonce-1")).unwrap();
        assert_eq!(
            parse_redirect(&url, "nonce-1"),
            Redirect::Denied("access_denied".into())
        );
    }

    #[test]
    fn a_redirect_with_neither_is_a_cancellation() {
        let url = Url::parse(&format!("{REDIRECT}?state=nonce-1")).unwrap();
        match parse_redirect(&url, "nonce-1") {
            Redirect::Denied(why) => assert!(why.contains("cancelled")),
            other => panic!("{other:?}"),
        }
    }

    // The whole point of the nonce: a code — or an error — on our scheme that
    // does not echo our state is nobody's business of ours, and must not end a
    // flow that is still waiting for the real answer.
    #[test]
    fn a_redirect_with_the_wrong_or_no_state_is_foreign() {
        let wrong = Url::parse(&format!("{REDIRECT}?code=4/abc&state=someone-elses")).unwrap();
        assert_eq!(parse_redirect(&wrong, "nonce-1"), Redirect::Foreign);
        let missing = Url::parse(&format!("{REDIRECT}?code=4/abc")).unwrap();
        assert_eq!(parse_redirect(&missing, "nonce-1"), Redirect::Foreign);
        let denied = Url::parse(&format!("{REDIRECT}?error=access_denied")).unwrap();
        assert_eq!(parse_redirect(&denied, "nonce-1"), Redirect::Foreign);
    }

    // --- the token file's guarded transitions ---

    use crate::crypto::hash_secret;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;

    fn tmp_dir() -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "rowel-auth-tokens-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn tmp_token_path() -> PathBuf {
        tmp_dir().join("gdrive.swftx")
    }

    // A workspace directory, with the subfolder the token file lives in — as
    // `workspace::dir_of` hands one to a delete.
    fn tmp_workspace_dir() -> PathBuf {
        let dir = tmp_dir();
        std::fs::create_dir_all(dir.join(storage::GDRIVE_FILE).parent().unwrap()).unwrap();
        dir
    }

    fn cryptor(password: &str) -> Cryptor {
        Cryptor::new(&hash_secret(password))
    }

    fn tokens(access: &str) -> Tokens {
        Tokens {
            access_token: Some(access.into()),
            refresh_token: Some("refresh-1".into()),
            expires_at: Some(4_102_444_800),
        }
    }

    // The re-seal a password change runs has to take the connection with it:
    // a refresh that read the file under the old key is awaiting Google right
    // now, and its write-back would seal the tokens under the key the change
    // just retired — leaving a file no later unlock could open.
    #[test]
    fn a_refresh_holding_the_old_generation_cannot_write_after_a_reseal() {
        let (old, new) = (cryptor("old-pw"), cryptor("new-pw"));
        let generation = Mutex::new(7);
        let file = TokenFile::new(tmp_token_path(), &generation);
        file.write(&old, &tokens("access-1")).unwrap();

        let (held_generation, mut held) = file.snapshot(&old).unwrap();
        file.reseal(&old, &new).unwrap();
        held.access_token = Some("access-2-refreshed".into());

        assert!(!file
            .persist_if_current(&old, &held, held_generation)
            .unwrap());
        assert!(file.read(&old).is_none());
        let survivor = file.read(&new).expect("re-sealed under the new key");
        assert_eq!(survivor.access_token.as_deref(), Some("access-1"));
    }

    // The same guard against the other transition: a disconnect deleted the
    // file, and the refresh it raced must not write it back into existence.
    #[test]
    fn a_refresh_holding_the_old_generation_cannot_resurrect_a_disconnected_file() {
        let cryptor = cryptor("pw");
        let generation = Mutex::new(0);
        let file = TokenFile::new(tmp_token_path(), &generation);
        file.write(&cryptor, &tokens("access-1")).unwrap();

        let (held_generation, mut held) = file.snapshot(&cryptor).unwrap();
        file.disconnect().unwrap();
        held.access_token = Some("access-2-refreshed".into());

        assert!(!file
            .persist_if_current(&cryptor, &held, held_generation)
            .unwrap());
        assert!(!file.path.exists());
    }

    // Nothing moved in the meantime, so the refreshed tokens land — otherwise
    // the guard would quietly cost every refresh its write-back.
    #[test]
    fn a_refresh_holding_the_current_generation_writes_back() {
        let cryptor = cryptor("pw");
        let generation = Mutex::new(3);
        let file = TokenFile::new(tmp_token_path(), &generation);
        file.write(&cryptor, &tokens("access-1")).unwrap();

        let (held_generation, mut held) = file.snapshot(&cryptor).unwrap();
        held.access_token = Some("access-2-refreshed".into());

        assert!(file
            .persist_if_current(&cryptor, &held, held_generation)
            .unwrap());
        let written = file.read(&cryptor).unwrap();
        assert_eq!(written.access_token.as_deref(), Some("access-2-refreshed"));
    }

    // A workspace's own token file, read for a delete that is about to remove
    // it: only a file that is not there means "this workspace never connected".
    // Anything present and unusable has to stop the delete, because the grant it
    // was written from may still be live at Google and the pack it reaches still
    // up there.
    #[test]
    fn a_workspaces_tokens_tell_an_absent_file_from_an_unreadable_one() {
        let dir = tmp_workspace_dir();
        let key = cryptor("pw");
        let path = dir.join(storage::GDRIVE_FILE);

        assert!(read_tokens_in(&dir, &key).unwrap().is_none());

        write_tokens_in(&dir, &key, &tokens("access-1")).unwrap();
        let read = read_tokens_in(&dir, &key).unwrap().unwrap();
        assert_eq!(read.access_token.as_deref(), Some("access-1"));

        // A write that never finished, bytes that are not a sealed blob, and a
        // seal this vault's key does not open.
        std::fs::write(&path, "").unwrap();
        assert!(read_tokens_in(&dir, &key).is_err());
        std::fs::write(&path, "not a sealed token file").unwrap();
        assert!(read_tokens_in(&dir, &key).is_err());
        write_tokens_in(&dir, &cryptor("another-pw"), &tokens("access-1")).unwrap();
        assert!(read_tokens_in(&dir, &key).is_err());
    }

    // No token file is no connection to re-seal: a password change on a vault
    // that never connected Drive is a no-op, and must not spend a generation
    // some in-flight refresh is holding.
    #[test]
    fn a_reseal_with_no_token_file_moves_nothing() {
        let (old, new) = (cryptor("old-pw"), cryptor("new-pw"));
        let generation = Mutex::new(5);
        let file = TokenFile::new(tmp_token_path(), &generation);

        file.reseal(&old, &new).unwrap();

        assert_eq!(*generation.lock().unwrap(), 5);
        assert!(!file.path.exists());
    }
}
