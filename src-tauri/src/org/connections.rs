//! The Rowel Servers this install is signed in to, with the member session
//! and the device's standing at each, sealed under the app key in one
//! root-level file (`storage::SERVERS_FILE`).
//!
//! Install-level on purpose. A member signs in once and the server hands them
//! every org lockbox they may hold; the session is the member's, not any one
//! lockbox's, so it lives beside the enrollment key rather than in a
//! workspace's token file. The token file an org lockbox keeps only names the
//! server (`Tokens::account`), and the sync adapter looks the session up here
//! by that name ([`SessionResolver`]).
//!
//! Sealed under the app key as the workspace keys are: readable only with the
//! vault open, and bound to its own name so a copy under another does not
//! open there. The plaintext is JSON and holds bearer tokens, so it is scrubbed
//! on drop everywhere it is held.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use super::client::{session_over, DeviceState, ServerClient, Session};
use crate::crypto;
use crate::error::{Error, Result};
use crate::storage;

/// The id the sealed file is bound to (`crypto::seal_workspace_key`).
const SEAL_ID: &str = "servers";

/// One server this install is signed in to.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Connection {
    /// The server, as [`normalize_url`] spells it: the key every lookup uses.
    pub url: String,
    pub tenant_name: String,
    pub member_id: String,
    pub member_email: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub member_name: Option<String>,
    /// This device's id at the server, once enrolled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    /// Where this device stands with the server, as of the last call that
    /// said: what Settings shows while waiting on an approval.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_state: Option<DeviceState>,
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// When `access_token` stops working, ms since the epoch; 0 when the
    /// server did not say.
    #[serde(default)]
    pub expires_at_ms: i64,
}

// Presence, never the value, as `Tokens` prints: both are bearer credentials.
impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connection")
            .field("url", &self.url)
            .field("tenant_name", &self.tenant_name)
            .field("member_id", &self.member_id)
            .field("member_email", &self.member_email)
            .field("member_name", &self.member_name)
            .field("device_id", &self.device_id)
            .field("device_state", &self.device_state)
            .field("access_token", &"<redacted>")
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "<redacted>"),
            )
            .field("expires_at_ms", &self.expires_at_ms)
            .finish()
    }
}

impl Connection {
    /// Take on a session the token endpoint just issued, at `now_ms`. The
    /// refresh token is replaced only when the server sent one: a server
    /// that does not rotate them would otherwise sign the device out on the
    /// first refresh.
    pub(crate) fn adopt(&mut self, session: Session, now_ms: i64) {
        self.access_token = session.access_token;
        if session.refresh_token.is_some() {
            self.refresh_token = session.refresh_token;
        }
        self.expires_at_ms = session
            .expires_in
            .map(|secs| now_ms.saturating_add(secs.saturating_mul(1000)))
            .unwrap_or(0);
    }
}

/// `url` as every record and lookup spells a server: lowercase scheme and
/// host, the port only when it is not the scheme's default, the path with
/// no trailing slash, and nothing after it. Two spellings of one server then
/// find one connection, and a URL that is not a server (`mailto:`, a bare
/// word) is refused before anything is sent to it.
///
/// `https` always. `http` only to a loopback host — `localhost`,
/// `127.0.0.0/8`, `[::1]` — which is a server on this machine (a
/// developer's own, the tests' fake); a bearer token and a wrapped master
/// over plaintext to anywhere else is refused here, before a request is
/// built, rather than left to a server to redirect.
pub(crate) fn normalize_url(url: &str) -> Result<String> {
    let parsed =
        url::Url::parse(url.trim()).map_err(|e| Error::Other(format!("not a server URL: {e}")))?;
    let host = parsed
        .host()
        .ok_or_else(|| Error::Other("a Rowel Server URL names a host".into()))?;
    let loopback = match host {
        url::Host::Domain(name) => name.eq_ignore_ascii_case("localhost"),
        url::Host::Ipv4(addr) => addr.is_loopback(),
        url::Host::Ipv6(addr) => addr.is_loopback(),
    };
    match parsed.scheme() {
        "https" => {}
        "http" if loopback => {}
        "http" => {
            return Err(Error::Other(
                "a Rowel Server is reached over https; http is only for a server on this \
                 machine (localhost or 127.0.0.1)"
                    .into(),
            ))
        }
        _ => {
            return Err(Error::Other(
                "a Rowel Server URL starts with https://".into(),
            ))
        }
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| Error::Other("a Rowel Server URL names a host".into()))?;
    let mut normalized = format!("{}://{host}", parsed.scheme());
    if let Some(port) = parsed.port() {
        normalized.push_str(&format!(":{port}"));
    }
    normalized.push_str(parsed.path().trim_end_matches('/'));
    Ok(normalized)
}

fn path_in(root: &Path) -> PathBuf {
    root.join(storage::SERVERS_FILE)
}

/// Every connection the install holds; none when it has never signed in.
pub(crate) fn load(root: &Path, app_key: &[u8]) -> Result<Vec<Connection>> {
    let path = path_in(root);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let sealed = std::fs::read_to_string(&path)?;
    // The wrong-key verdict passes through as it is: it is the one failure a
    // caller may act on (`org::set_aside_stale`), and wrapping it would hide it.
    let json = crypto::open_workspace_key(app_key, SEAL_ID, &sealed).map_err(|e| match e {
        Error::SealedUnderAnotherKey => e,
        e => Error::Crypto(format!(
            "the server connections do not open under the app key: {e}"
        )),
    })?;
    Ok(serde_json::from_slice(&json)?)
}

/// Which record a change was worked out from, and so the only record it may
/// land on. A change is computed from a copy read before a round trip, and a
/// sign-in can replace the record at the same URL while that round trip is
/// out — another member, another device, or the same member signing in
/// again. A URL alone cannot tell those apart, so every write names what it
/// expects to find. No `Debug`: a session's expectation holds a refresh
/// token.
#[derive(Clone)]
pub(crate) enum Expect {
    /// The same member with the same device: what a note about the device's
    /// standing needs, since the listing it came from was about that device.
    SignIn {
        member_id: String,
        device_id: Option<String>,
    },
    /// That sign-in still under the session it was read under: what a
    /// refresh needs, since its answer replaces the very refresh token it
    /// spent. A newer session in its place — a re-sign-in, or a refresh that
    /// finished first — is not one this answer may overwrite.
    Session {
        member_id: String,
        device_id: Option<String>,
        refresh_token: Option<String>,
    },
}

impl Expect {
    pub(crate) fn sign_in(of: &Connection) -> Self {
        Self::SignIn {
            member_id: of.member_id.clone(),
            device_id: of.device_id.clone(),
        }
    }

    pub(crate) fn session(of: &Connection) -> Self {
        Self::Session {
            member_id: of.member_id.clone(),
            device_id: of.device_id.clone(),
            refresh_token: of.refresh_token.clone(),
        }
    }

    fn holds(&self, held: &Connection) -> bool {
        match self {
            Self::SignIn {
                member_id,
                device_id,
            } => held.member_id == *member_id && held.device_id == *device_id,
            Self::Session {
                member_id,
                device_id,
                refresh_token,
            } => {
                held.member_id == *member_id
                    && held.device_id == *device_id
                    && held.refresh_token == *refresh_token
            }
        }
    }
}

/// What an [`update`] came to, with the record as it now stands either way.
#[derive(Debug)]
pub(crate) enum Updated {
    /// The record was the one expected, and the change is written.
    Changed(Connection),
    /// Another sign-in or session holds the URL now; nothing was written.
    Superseded(Connection),
}

impl Updated {
    pub(crate) fn into_record(self) -> Connection {
        match self {
            Self::Changed(record) | Self::Superseded(record) => record,
        }
    }
}

/// Change the connection to the server at `url` in place, if it is still
/// the one `expect` names: `change` runs over the record as the file holds
/// it now, and the result is written back as one step. The one way a held
/// connection is ever written, so two writers — a refresh rotating the
/// tokens, a listing noting the device's standing — never put a snapshot of
/// theirs over the other's write, and neither lands on a sign-in that
/// replaced the one it was worked out from.
pub(crate) fn update(
    root: &Path,
    app_key: &[u8],
    url: &str,
    expect: &Expect,
    change: impl FnOnce(&mut Connection),
) -> Result<Updated> {
    let mut connections = load(root, app_key)?;
    let updated = update_in(&mut connections, url, expect, change)?;
    if matches!(updated, Updated::Changed(_)) {
        save(root, app_key, &connections)?;
    }
    Ok(updated)
}

// The compare-and-set itself, over the list wherever it is held: the sealed
// file's and the tests' memory decide alike.
fn update_in(
    connections: &mut [Connection],
    url: &str,
    expect: &Expect,
    change: impl FnOnce(&mut Connection),
) -> Result<Updated> {
    let url = normalize_url(url)?;
    let held = connections
        .iter_mut()
        .find(|c| c.url == url)
        .ok_or_else(|| not_signed_in(&url))?;
    if !expect.holds(held) {
        return Ok(Updated::Superseded(held.clone()));
    }
    change(held);
    Ok(Updated::Changed(held.clone()))
}

fn not_signed_in(url: &str) -> Error {
    Error::Other(format!("not signed in to the Rowel Server at {url}"))
}

/// A sign-in's record, written whole under the sidecars lock: the one write
/// that is not an [`update`], since the session it carries is new and the
/// record it replaces, if any, held a session this one supersedes.
#[cfg(desktop)]
pub(crate) fn record_sign_in(app: &tauri::AppHandle, connection: Connection) -> Result<()> {
    use tauri::Manager;
    let state = app.state::<crate::state::AppState>();
    let _sidecars = state.sidecars.lock().unwrap();
    let (root, app_key) = app_root_and_key(app)?;
    upsert(&root, &app_key, connection)
}

/// Write `connections` as the whole list, sealed under `app_key`. An empty
/// list removes the file: nothing is signed in, and the file saying so would
/// only be one more thing to re-seal.
pub(crate) fn save(root: &Path, app_key: &[u8], connections: &[Connection]) -> Result<()> {
    let path = path_in(root);
    if connections.is_empty() {
        return storage::remove_if_present(&path);
    }
    let json = Zeroizing::new(serde_json::to_vec(connections)?);
    let sealed = crypto::seal_workspace_key(app_key, SEAL_ID, &json)?;
    storage::atomic_write_private(&path, sealed.as_bytes())
}

/// Add `connection`, or replace the one for the same server: a sign-in's
/// write ([`record_sign_in`]), which is desktop's alone for now.
#[cfg(any(desktop, test))]
pub(crate) fn upsert(root: &Path, app_key: &[u8], connection: Connection) -> Result<()> {
    let mut connections = load(root, app_key)?;
    match connections.iter_mut().find(|c| c.url == connection.url) {
        Some(held) => *held = connection,
        None => connections.push(connection),
    }
    save(root, app_key, &connections)
}

/// Forget the server at `url`, saying whether there was one to forget.
pub(crate) fn remove(root: &Path, app_key: &[u8], url: &str) -> Result<bool> {
    let url = normalize_url(url)?;
    let mut connections = load(root, app_key)?;
    let before = connections.len();
    connections.retain(|c| c.url != url);
    if connections.len() == before {
        return Ok(false);
    }
    save(root, app_key, &connections)?;
    Ok(true)
}

/// The connection to the server at `url`, if the install has one.
pub(crate) fn find(root: &Path, app_key: &[u8], url: &str) -> Result<Option<Connection>> {
    let url = normalize_url(url)?;
    Ok(load(root, app_key)?.into_iter().find(|c| c.url == url))
}

/// The file sealed under `new` instead of `old`: a master-password change
/// replaced the app key (`appkey::replace_app_key`). Nothing to do when the
/// install has never signed in.
pub(crate) fn reseal(root: &Path, old: &[u8], new: &[u8]) -> Result<()> {
    if !path_in(root).exists() {
        return Ok(());
    }
    let connections = load(root, old)?;
    save(root, new, &connections)
}

/// The root and the app key of the running app, for whatever reads or writes
/// a file sealed under it; `Locked` until the primary has been opened this
/// session, since only its key is the app key.
pub(crate) fn app_root_and_key(app: &tauri::AppHandle) -> Result<(PathBuf, Zeroizing<Vec<u8>>)> {
    use tauri::Manager;
    let root = storage::root_dir(app)?;
    let app_key = app
        .state::<crate::state::AppState>()
        .keyring
        .lock()
        .unwrap()
        .app_key()
        .ok_or(Error::Locked)?;
    Ok((root, app_key))
}

/// Where the sync adapter for an org lockbox, and the approvals thread, find
/// the member session for a server, and where they put a session refreshed
/// on the way.
pub(crate) enum SessionResolver {
    /// The running app's: the root and the app key read from the ring at
    /// each call — so a lock, which clears the ring, ends every caller's
    /// access at its next call — and the write made under the sidecars lock,
    /// as every file sealed under the app key is written.
    App(tauri::AppHandle),
    /// Connections held in memory and shared with whoever holds the `Arc`:
    /// the tests', which have no app and no root.
    #[cfg(test)]
    Memory(Arc<Mutex<Vec<Connection>>>),
}

impl SessionResolver {
    /// The connection to `url`, or the error that says the member is not
    /// signed in to that server.
    pub(crate) fn connection(&self, url: &str) -> Result<Connection> {
        let url = normalize_url(url)?;
        let found = match self {
            Self::App(app) => {
                let (root, app_key) = app_root_and_key(app)?;
                find(&root, &app_key, &url)?
            }
            #[cfg(test)]
            Self::Memory(held) => held.lock().unwrap().iter().find(|c| c.url == url).cloned(),
        };
        found.ok_or_else(|| not_signed_in(&url))
    }

    /// [`update`] on the connection to `url`, if it is still the one `expect`
    /// names: the one way a held connection is written, as one
    /// compare-and-set under the sidecars lock.
    pub(crate) fn update(
        &self,
        url: &str,
        expect: &Expect,
        change: impl FnOnce(&mut Connection),
    ) -> Result<Updated> {
        match self {
            Self::App(app) => {
                use tauri::Manager;
                let state = app.state::<crate::state::AppState>();
                let _sidecars = state.sidecars.lock().unwrap();
                let (root, app_key) = app_root_and_key(app)?;
                update(&root, &app_key, url, expect, change)
            }
            #[cfg(test)]
            Self::Memory(held) => update_in(&mut held.lock().unwrap(), url, expect, change),
        }
    }

    /// A fresh session for the server at `url` from the refresh token its
    /// connection holds: what a 401 asks for. The answer is taken on only by
    /// the record it was asked for — the same member and device under the
    /// refresh token just spent — checked when the token endpoint has
    /// answered, not when the request went out. A record that moved on
    /// meanwhile (a re-sign-in, another member, a refresh that finished
    /// first) keeps what it has, this answer is dropped, and the record as it
    /// stands is handed back for the caller to retry under. Without a refresh
    /// token the session is simply over.
    ///
    /// One refresh at a time per server ([`refresh_turn`]). A refresh token
    /// is single-use, and a server that sees one spent twice takes it for a
    /// stolen copy and ends the session (RFC 6819 §5.2.2.3): sync and the
    /// approvals stream meeting a 401 together would otherwise both spend
    /// the token they read, and the second would sign the member out under
    /// the first's fresh session. In turn, each refresh reads the token the
    /// one before it stored; and one that finds the session already moved on
    /// while it waited has nothing left to do but use it.
    pub(crate) async fn refresh(&self, url: &str) -> Result<Connection> {
        let seen = self.connection(url)?.refresh_token.clone();
        let gate = refresh_turn(url);
        let _turn = gate.lock().await;
        let connection = self.connection(url)?;
        if connection.refresh_token != seen {
            return Ok(connection);
        }
        let refresh_token = connection.refresh_token.clone().ok_or_else(|| {
            Error::Other(format!(
                "the Rowel Server at {} no longer accepts this sign-in; sign in again",
                connection.url
            ))
        })?;
        let expect = Expect::session(&connection);
        let fresh = ServerClient::new(&connection.url)?
            .refresh(&refresh_token)
            .await?;
        let now_ms = chrono::Utc::now().timestamp_millis();
        match self.update(url, &expect, |held| held.adopt(fresh, now_ms))? {
            Updated::Changed(record) => Ok(record),
            Updated::Superseded(record) => {
                log::info!(
                    "a newer session at {} replaced the one being refreshed; its answer is dropped",
                    record.url
                );
                Ok(record)
            }
        }
    }

    /// `call` as the member signed in to `url`, and once more under a
    /// refreshed session if the first answer was that the session is over
    /// (`client::SESSION_OVER`). A second such answer is the sign-in ending,
    /// and reaches the caller as it is.
    pub(crate) async fn as_member<T, Fut>(
        &self,
        url: &str,
        call: impl Fn(ServerClient) -> Fut,
    ) -> Result<T>
    where
        Fut: std::future::Future<Output = Result<T>>,
    {
        let connection = self.connection(url)?;
        let client = ServerClient::with_session(&connection.url, &connection.access_token)?;
        match call(client).await {
            Err(e) if session_over(&e) => {
                let connection = self.refresh(url).await?;
                call(ServerClient::with_session(
                    &connection.url,
                    &connection.access_token,
                )?)
                .await
            }
            outcome => outcome,
        }
    }
}

/// The turn every refresh of the session at `url` waits for: one async lock
/// per server URL, for the life of the process (a handful of servers at
/// most). Async, because the refresh holds it across the token endpoint's
/// answer; process-wide, because sync, the approvals stream and the commands
/// each make their own resolver.
fn refresh_turn(url: &str) -> Arc<futures_util::lock::Mutex<()>> {
    static TURNS: std::sync::LazyLock<
        Mutex<std::collections::HashMap<String, Arc<futures_util::lock::Mutex<()>>>>,
    > = std::sync::LazyLock::new(Mutex::default);
    let key = normalize_url(url).unwrap_or_else(|_| url.to_string());
    TURNS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(key)
        .or_default()
        .clone()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    const APP_KEY: &[u8] = &[3u8; 32];

    /// A signed-in, approved connection to `url` under `access`: the fixture
    /// the app key tests seal under one key and open under another too.
    pub(crate) fn connection(url: &str, access: &str) -> Connection {
        Connection {
            url: normalize_url(url).unwrap(),
            tenant_name: "Acme".into(),
            member_id: "m1".into(),
            member_email: "pat@acme.example".into(),
            member_name: Some("Pat".into()),
            device_id: Some("d1".into()),
            device_state: Some(DeviceState::Approved),
            access_token: access.into(),
            refresh_token: Some(format!("{access}-refresh")),
            expires_at_ms: 1_700_000_000_000,
        }
    }

    #[test]
    fn a_server_url_has_one_spelling() {
        for (given, expected) in [
            ("https://Rowel.Example.com/", "https://rowel.example.com"),
            (
                "https://rowel.example.com:443/",
                "https://rowel.example.com",
            ),
            ("http://127.0.0.1:8080", "http://127.0.0.1:8080"),
            (
                " https://rowel.example.com/rowel/ ",
                "https://rowel.example.com/rowel",
            ),
            (
                "https://rowel.example.com/rowel?x=1#frag",
                "https://rowel.example.com/rowel",
            ),
        ] {
            assert_eq!(normalize_url(given).unwrap(), expected, "{given}");
        }
        assert!(normalize_url("rowel.example.com").is_err());
        assert!(normalize_url("mailto:pat@acme.example").is_err());
        assert!(normalize_url("ftp://rowel.example.com").is_err());
    }

    // Plaintext reaches a server on this machine and nowhere else.
    #[test]
    fn http_is_for_loopback_hosts_alone() {
        for loopback in [
            "http://localhost:8080",
            "http://LocalHost",
            "http://127.0.0.1",
            "http://127.1.2.3:9000",
            "http://[::1]:8080",
        ] {
            assert!(normalize_url(loopback).is_ok(), "{loopback}");
        }
        for elsewhere in [
            "http://rowel.example.com",
            "http://10.0.0.5:8080",
            "http://192.168.1.10",
            "http://[fe80::1]",
            "http://localhost.example.com",
        ] {
            match normalize_url(elsewhere) {
                Err(Error::Other(why)) => assert!(why.contains("https"), "{elsewhere}: {why}"),
                other => panic!("{elsewhere} must be refused, got {other:?}"),
            }
        }
        assert!(
            normalize_url("https://10.0.0.5").is_ok(),
            "https goes anywhere"
        );
    }

    // Every write to a held connection is one read-modify-write, so a change
    // never carries a stale copy of the fields it did not touch.
    #[test]
    fn an_update_changes_the_record_in_place() {
        let root = tempfile::tempdir().unwrap();
        let first = connection("https://a.example", "a1");
        upsert(root.path(), APP_KEY, first.clone()).unwrap();
        let rotated = update(
            root.path(),
            APP_KEY,
            "https://A.example/",
            &Expect::session(&first),
            |c| c.refresh_token = Some("rotated".into()),
        )
        .unwrap();
        let Updated::Changed(rotated) = rotated else {
            panic!("the expected session was there")
        };
        assert_eq!(rotated.refresh_token.as_deref(), Some("rotated"));
        // Noted against the sign-in, not the session: the rotation above is
        // the same member's same device, so the note still lands.
        let noted = update(
            root.path(),
            APP_KEY,
            "https://a.example",
            &Expect::sign_in(&first),
            |c| c.device_state = Some(DeviceState::Pending),
        )
        .unwrap()
        .into_record();
        assert_eq!(noted.refresh_token.as_deref(), Some("rotated"));
        assert_eq!(noted.device_state, Some(DeviceState::Pending));
        assert_eq!(
            find(root.path(), APP_KEY, "https://a.example")
                .unwrap()
                .unwrap(),
            noted
        );
        assert!(update(
            root.path(),
            APP_KEY,
            "https://b.example",
            &Expect::sign_in(&first),
            |_| {}
        )
        .is_err());
    }

    // A change worked out from one sign-in never lands on another at the
    // same URL: not a refresh's answer once the session it spent is gone,
    // and not a note about a device once another member or device holds it.
    #[test]
    fn a_change_from_a_replaced_sign_in_lands_nowhere() {
        let root = tempfile::tempdir().unwrap();
        let old = connection("https://a.example", "old");
        upsert(root.path(), APP_KEY, old.clone()).unwrap();
        // While a refresh is out, the member signs in as someone else.
        let mut new = connection("https://a.example", "new");
        new.member_id = "m2".into();
        new.device_id = Some("d2".into());
        upsert(root.path(), APP_KEY, new.clone()).unwrap();

        let answered = update(
            root.path(),
            APP_KEY,
            "https://a.example",
            &Expect::session(&old),
            |c| c.access_token = "old-refreshed".into(),
        )
        .unwrap();
        assert!(matches!(&answered, Updated::Superseded(held) if held == &new));
        let noted = update(
            root.path(),
            APP_KEY,
            "https://a.example",
            &Expect::sign_in(&old),
            |c| c.device_state = Some(DeviceState::Revoked),
        )
        .unwrap();
        assert!(matches!(noted, Updated::Superseded(_)));
        assert_eq!(
            find(root.path(), APP_KEY, "https://a.example").unwrap(),
            Some(new.clone()),
            "the new sign-in is untouched"
        );

        // The same member signing in again on the same device is a new
        // session too: a refresh of the old one may not overwrite it.
        let mut again = old.clone();
        again.access_token = "again".into();
        again.refresh_token = Some("again-refresh".into());
        upsert(root.path(), APP_KEY, again.clone()).unwrap();
        let answered = update(
            root.path(),
            APP_KEY,
            "https://a.example",
            &Expect::session(&old),
            |c| c.access_token = "old-refreshed".into(),
        )
        .unwrap();
        assert!(matches!(answered, Updated::Superseded(held) if held == again));
    }

    #[test]
    fn connections_round_trip_sealed_under_the_app_key() {
        let root = tempfile::tempdir().unwrap();
        assert!(load(root.path(), APP_KEY).unwrap().is_empty());

        upsert(root.path(), APP_KEY, connection("https://a.example/", "a1")).unwrap();
        upsert(root.path(), APP_KEY, connection("https://b.example", "b1")).unwrap();
        upsert(root.path(), APP_KEY, connection("https://A.example", "a2")).unwrap();
        let held = load(root.path(), APP_KEY).unwrap();
        assert_eq!(held.len(), 2, "the second a.example replaced the first");
        assert_eq!(held[0].access_token, "a2");
        assert_eq!(
            find(root.path(), APP_KEY, "https://b.example/")
                .unwrap()
                .unwrap()
                .access_token,
            "b1"
        );
        assert!(find(root.path(), APP_KEY, "https://c.example")
            .unwrap()
            .is_none());

        // The file is ciphertext, and only this app key opens it. Looked for
        // by the whole secrets: two characters turn up in base64 by chance.
        let raw = std::fs::read_to_string(root.path().join(storage::SERVERS_FILE)).unwrap();
        assert!(!raw.contains("a2-refresh") && !raw.contains("pat@acme.example"));
        assert!(load(root.path(), &[4u8; 32]).is_err());

        assert!(remove(root.path(), APP_KEY, "https://a.example").unwrap());
        assert!(!remove(root.path(), APP_KEY, "https://a.example").unwrap());
        assert!(remove(root.path(), APP_KEY, "https://b.example").unwrap());
        assert!(!root.path().join(storage::SERVERS_FILE).exists());
    }

    // A password change moves the file onto the new app key, and the old one
    // no longer opens it; the wrong-key verdict passes through `load` as it
    // is, for `org::set_aside_stale` to act on. A file too short to be a
    // sealed blob is malformed instead, and says nothing about the key.
    #[test]
    fn the_file_follows_the_app_key() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(storage::SERVERS_FILE);
        reseal(root.path(), APP_KEY, &[5u8; 32]).unwrap();
        assert!(!path.exists(), "nothing to re-seal is not an error");

        upsert(root.path(), APP_KEY, connection("https://a.example", "a1")).unwrap();
        reseal(root.path(), APP_KEY, &[5u8; 32]).unwrap();
        assert!(matches!(
            load(root.path(), APP_KEY),
            Err(Error::SealedUnderAnotherKey)
        ));
        assert_eq!(load(root.path(), &[5u8; 32]).unwrap()[0].access_token, "a1");

        let sealed = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, &sealed[..20]).unwrap();
        assert!(matches!(load(root.path(), APP_KEY), Err(Error::Crypto(_))));
    }

    #[test]
    fn a_refreshed_session_keeps_the_refresh_token_unless_rotated() {
        let mut held = connection("https://a.example", "old");
        held.adopt(
            Session {
                access_token: "new".into(),
                refresh_token: None,
                expires_in: Some(3600),
                token_type: "Bearer".into(),
            },
            1_000,
        );
        assert_eq!(held.access_token, "new");
        assert_eq!(held.refresh_token.as_deref(), Some("old-refresh"));
        assert_eq!(held.expires_at_ms, 3_601_000);
        held.adopt(
            Session {
                access_token: "newer".into(),
                refresh_token: Some("rotated".into()),
                expires_in: None,
                token_type: "Bearer".into(),
            },
            2_000,
        );
        assert_eq!(held.refresh_token.as_deref(), Some("rotated"));
        assert_eq!(held.expires_at_ms, 0);
    }

    // The in-memory resolver answers as the sealed file does, spelling
    // included, so a test run against it exercises what the app runs.
    #[test]
    fn the_memory_resolver_answers_like_the_sealed_file() {
        let resolver = SessionResolver::Memory(Arc::new(Mutex::new(vec![connection(
            "https://a.example",
            "a1",
        )])));
        let held = resolver.connection("https://A.example/").unwrap();
        assert_eq!(held.access_token, "a1");
        resolver
            .update("https://a.example", &Expect::sign_in(&held), |c| {
                c.access_token = "a2".into()
            })
            .unwrap();
        assert_eq!(
            resolver
                .connection("https://a.example")
                .unwrap()
                .access_token,
            "a2"
        );
        for unknown in [
            resolver.connection("https://b.example").err(),
            resolver
                .update("https://b.example", &Expect::sign_in(&held), |_| {})
                .err(),
        ] {
            match unknown {
                Some(Error::Other(why)) => assert!(why.contains("not signed in"), "{why}"),
                other => panic!("an unknown server is not signed in, got {other:?}"),
            }
        }
    }
}
