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
#[cfg(test)]
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
pub(crate) fn normalize_url(url: &str) -> Result<String> {
    let parsed =
        url::Url::parse(url.trim()).map_err(|e| Error::Other(format!("not a server URL: {e}")))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(Error::Other(
            "a Rowel Server URL starts with https://".into(),
        ));
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
    let json = crypto::open_workspace_key(app_key, SEAL_ID, &sealed).map_err(|e| {
        Error::Crypto(format!(
            "the server connections do not open under the app key: {e}"
        ))
    })?;
    Ok(serde_json::from_slice(&json)?)
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

/// Add `connection`, or replace the one for the same server.
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

/// A file the current app key does not open is one sealed under an app key
/// this device no longer has — what deleting the primary leaves behind — and
/// nothing will ever open it again: it goes, with a note, and the member
/// signs in again. A file that is simply not there, or that opens, is left.
pub(crate) fn remove_if_stale(root: &Path, app_key: &[u8]) {
    let path = path_in(root);
    if !path.exists() || load(root, app_key).is_ok() {
        return;
    }
    log::warn!("the server connections do not open under the app key; removing them");
    let _ = storage::remove_if_present(&path);
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
        found.ok_or_else(|| Error::Other(format!("not signed in to the Rowel Server at {url}")))
    }

    /// Keep `connection` as it is now, over the record for the same server.
    pub(crate) fn persist(&self, connection: &Connection) -> Result<()> {
        match self {
            Self::App(app) => {
                use tauri::Manager;
                let state = app.state::<crate::state::AppState>();
                let _sidecars = state.sidecars.lock().unwrap();
                let (root, app_key) = app_root_and_key(app)?;
                upsert(&root, &app_key, connection.clone())
            }
            #[cfg(test)]
            Self::Memory(held) => {
                let mut held = held.lock().unwrap();
                match held.iter_mut().find(|c| c.url == connection.url) {
                    Some(slot) => *slot = connection.clone(),
                    None => held.push(connection.clone()),
                }
                Ok(())
            }
        }
    }

    /// A fresh session for the server at `url` from the refresh token its
    /// connection holds, kept over the record before it is handed back: what
    /// a 401 asks for. Without a refresh token the session is simply over.
    pub(crate) async fn refresh(&self, url: &str) -> Result<Connection> {
        let mut connection = self.connection(url)?;
        let refresh_token = connection.refresh_token.clone().ok_or_else(|| {
            Error::Other(format!(
                "the Rowel Server at {} no longer accepts this sign-in; sign in again",
                connection.url
            ))
        })?;
        let fresh = ServerClient::new(&connection.url)?
            .refresh(&refresh_token)
            .await?;
        connection.adopt(fresh, chrono::Utc::now().timestamp_millis());
        self.persist(&connection)?;
        Ok(connection)
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

        // The file is ciphertext, and only this app key opens it.
        let raw = std::fs::read_to_string(root.path().join(storage::SERVERS_FILE)).unwrap();
        assert!(!raw.contains("a2") && !raw.contains("acme"));
        assert!(load(root.path(), &[4u8; 32]).is_err());

        assert!(remove(root.path(), APP_KEY, "https://a.example").unwrap());
        assert!(!remove(root.path(), APP_KEY, "https://a.example").unwrap());
        assert!(remove(root.path(), APP_KEY, "https://b.example").unwrap());
        assert!(!root.path().join(storage::SERVERS_FILE).exists());
    }

    // A password change moves the file onto the new app key; a file left
    // under a key the device no longer has goes, and one that opens stays.
    #[test]
    fn the_file_follows_the_app_key_and_a_stale_one_is_removed() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(storage::SERVERS_FILE);
        reseal(root.path(), APP_KEY, &[5u8; 32]).unwrap();
        assert!(!path.exists(), "nothing to re-seal is not an error");

        upsert(root.path(), APP_KEY, connection("https://a.example", "a1")).unwrap();
        reseal(root.path(), APP_KEY, &[5u8; 32]).unwrap();
        assert!(load(root.path(), APP_KEY).is_err());
        assert_eq!(load(root.path(), &[5u8; 32]).unwrap()[0].access_token, "a1");

        remove_if_stale(root.path(), &[5u8; 32]);
        assert!(path.exists(), "it opens, so it stays");
        remove_if_stale(root.path(), APP_KEY);
        assert!(
            !path.exists(),
            "sealed under a key this device no longer has"
        );
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
        let resolver = SessionResolver::Memory(Arc::default());
        assert!(resolver.connection("https://a.example").is_err());
        resolver
            .persist(&connection("https://a.example", "a1"))
            .unwrap();
        let mut held = resolver.connection("https://A.example/").unwrap();
        assert_eq!(held.access_token, "a1");
        held.access_token = "a2".into();
        resolver.persist(&held).unwrap();
        assert_eq!(
            resolver
                .connection("https://a.example")
                .unwrap()
                .access_token,
            "a2"
        );
        match resolver.connection("https://b.example") {
            Err(Error::Other(why)) => assert!(why.contains("not signed in"), "{why}"),
            other => panic!("an unknown server is not signed in, got {other:?}"),
        }
    }
}
