//! Google Drive: finds (or creates) `Rowel/Vaults/` and reads and writes the
//! files inside it. Every name it uses comes from [`layout`] or from the core.
//!
//! What is Drive's alone stays here: `q`-escaping, pagination and multipart
//! uploads ([`api`]), `appProperties` ([`share`]), and **oldest-wins** among
//! same-name siblings — Drive permits two files of one name in one folder, and
//! the trait boundary is what keeps that from being anyone else's problem.

pub mod api;
pub mod share;

use std::sync::Mutex;

#[cfg(mobile)]
use tauri::AppHandle;

use crate::error::Result;
use crate::sync::account::{Auth, OAuthConfig};
use crate::sync::store::{RemoteEntry, VaultStore};
use crate::sync::{http_client, layout, Provider};
use api::DriveFile;

const SCOPE: &str = "https://www.googleapis.com/auth/drive.file";

// Supply your own Google OAuth client at build or run time.
const CLIENT_ID_PLACEHOLDER: &str = "YOUR_GOOGLE_OAUTH_CLIENT_ID";

/// Google's OAuth client, as the shared flow in `sync::account` drives it.
pub static OAUTH: OAuthConfig = OAuthConfig {
    provider: Provider::GoogleDrive,
    auth_url: "https://accounts.google.com/o/oauth2/v2/auth",
    token_url: "https://oauth2.googleapis.com/token",
    consent_params: &[
        ("scope", SCOPE),
        ("access_type", "offline"),
        ("prompt", "consent"),
    ],
    // A Desktop-app client accepts any loopback port.
    #[cfg(desktop)]
    loopback_ports: &[],
    #[cfg(desktop)]
    client_id: env_client_id,
    #[cfg(desktop)]
    client_secret: env_client_secret,
    no_client: "Google OAuth client not configured; set GOOGLE_OAUTH_CLIENT_ID",
    #[cfg(mobile)]
    mobile_client,
    // A refusal comes back on the redirect itself, as `?error=...`.
    #[cfg(mobile)]
    mobile_cancel_uri: |_| None,
};

// On both platforms: the desktop client's id, and the phone's override of the
// id its deep-link scheme implies (`mobile_client`).
fn env_client_id() -> Option<String> {
    std::env::var("GOOGLE_OAUTH_CLIENT_ID")
        .ok()
        .or_else(|| option_env!("GOOGLE_OAUTH_CLIENT_ID").map(String::from))
        .filter(|id| !id.is_empty() && id != CLIENT_ID_PLACEHOLDER)
}

// An iOS client is public and never sends one, so only the desktop client
// reads it.
#[cfg(desktop)]
fn env_client_secret() -> Option<String> {
    std::env::var("GOOGLE_OAUTH_CLIENT_SECRET")
        .ok()
        .or_else(|| option_env!("GOOGLE_OAUTH_CLIENT_SECRET").map(String::from))
        .filter(|s| !s.is_empty())
}

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

/// The committed deep-link scheme is the source of truth (iOS client ids are
/// public, and the scheme is what actually registers the app for the
/// redirect). `GOOGLE_OAUTH_CLIENT_ID` overrides it for a build that is given
/// the id from a secret — and the redirect is then derived back from that same
/// id, so the two can never disagree at runtime.
#[cfg(mobile)]
fn mobile_client(app: &AppHandle) -> Result<(String, String)> {
    let no_client = || crate::sync::account::no_client(&OAUTH);
    let client_id = match env_client_id() {
        Some(id) => id,
        None => client_id_from_scheme(&deep_link_scheme(app)?).ok_or_else(no_client)?,
    };
    let scheme = scheme_from_client_id(&client_id).ok_or_else(no_client)?;
    Ok((client_id, format!("{scheme}{IOS_REDIRECT_PATH}")))
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
/// `tauri.ios.conf.json`). Other schemes may be configured for other purposes
/// (Dropbox's among them), so this picks the one that looks like a Google
/// client rather than the first.
#[cfg(mobile)]
fn deep_link_scheme(app: &AppHandle) -> Result<String> {
    super::deep_link_scheme(app, IOS_SCHEME_PREFIX)
        .ok_or_else(|| crate::sync::account::no_client(&OAUTH))
}

/// `Rowel/Vaults`, with either folder created if this account has never had it.
/// The root id comes back too, so a caller that caches the pair
/// ([`Store::ensure_vaults`]) does not list it a second time.
async fn ensure_vaults_path(client: &reqwest::Client, token: &str) -> Result<(String, String)> {
    let root = match api::folder_id(client, token, layout::ROOT_FOLDER).await? {
        Some(id) => id,
        None => api::create_folder(client, token, layout::ROOT_FOLDER).await?,
    };
    let vaults = match api::folder_id_in(client, token, layout::VAULTS_FOLDER, &root).await? {
        Some(id) => id,
        None => api::create_folder_in(client, token, layout::VAULTS_FOLDER, Some(&root)).await?,
    };
    Ok((root, vaults))
}

/// [`VaultStore`] over the connected account's `Rowel/Vaults`.
///
/// Both folder ids are resolved once per instance (one instance per run)
/// because the selection rule is deterministic and re-listing them would only
/// cost round trips. A *file* is re-listed on every call: `head_revision`
/// exists precisely to observe a change another device made, so it must never
/// answer from a cache.
pub struct Store {
    auth: Auth,
    client: reqwest::Client,
    root: Mutex<Option<String>>,
    vaults: Mutex<Option<String>>,
}

impl Store {
    pub fn new(auth: Auth) -> Self {
        Self {
            auth,
            client: http_client(),
            root: Mutex::new(None),
            vaults: Mutex::new(None),
        }
    }

    pub fn into_auth(self) -> Auth {
        self.auth
    }

    async fn token(&self) -> Result<String> {
        self.auth.access_token(&self.client).await
    }

    // Resolve the Rowel folder, remembering it for the rest of the run.
    // `None` means it does not exist yet — which is also "no remote vault".
    async fn root(&self, token: &str) -> Result<Option<String>> {
        if let Some(id) = self.root.lock().unwrap().clone() {
            return Ok(Some(id));
        }
        let found = api::folder_id(&self.client, token, layout::ROOT_FOLDER).await?;
        if let Some(id) = &found {
            *self.root.lock().unwrap() = Some(id.clone());
        }
        Ok(found)
    }

    // `Rowel/Vaults`, the same way.
    async fn vaults(&self, token: &str) -> Result<Option<String>> {
        if let Some(id) = self.vaults.lock().unwrap().clone() {
            return Ok(Some(id));
        }
        let Some(root) = self.root(token).await? else {
            return Ok(None);
        };
        let found = api::folder_id_in(&self.client, token, layout::VAULTS_FOLDER, &root).await?;
        if let Some(id) = &found {
            *self.vaults.lock().unwrap() = Some(id.clone());
        }
        Ok(found)
    }

    // [`Store::vaults`], for a create: both folders are made when this is the
    // first thing this account has ever had put in it.
    async fn ensure_vaults(&self, token: &str) -> Result<String> {
        if let Some(id) = self.vaults(token).await? {
            return Ok(id);
        }
        let (root, vaults) = ensure_vaults_path(&self.client, token).await?;
        *self.root.lock().unwrap() = Some(root);
        *self.vaults.lock().unwrap() = Some(vaults.clone());
        Ok(vaults)
    }
}

/// A Drive file as the core sees it. The file id is the handle.
fn entry(file: DriveFile) -> RemoteEntry {
    RemoteEntry {
        handle: file.id,
        name: file.name,
        size: file.size,
        modified: file.modified_time,
        revision: file.head_revision,
    }
}

impl VaultStore for Store {
    fn provider(&self) -> Provider {
        Provider::GoogleDrive
    }

    // A missing `Rowel/` or `Vaults/` is an account nothing has ever synced to,
    // which is the same answer as an empty folder.
    async fn list_vaults(&self) -> Result<Vec<RemoteEntry>> {
        let token = self.token().await?;
        let Some(vaults) = self.vaults(&token).await? else {
            return Ok(Vec::new());
        };
        let files = api::list_folder(&self.client, &token, &vaults).await?;
        Ok(files.into_iter().map(entry).collect())
    }

    async fn stat(&self, name: &str) -> Result<Option<RemoteEntry>> {
        let token = self.token().await?;
        let Some(vaults) = self.vaults(&token).await? else {
            return Ok(None);
        };
        Ok(api::find_file(&self.client, &token, name, &vaults)
            .await?
            .map(entry))
    }

    async fn read(&self, handle: &str, max_bytes: usize) -> Result<Vec<u8>> {
        let token = self.token().await?;
        api::read_file(&self.client, &token, handle, max_bytes).await
    }

    async fn create(&self, name: &str, bytes: &[u8]) -> Result<Option<String>> {
        let token = self.token().await?;
        let vaults = self.ensure_vaults(&token).await?;
        Ok(api::create_file(&self.client, &token, name, &vaults, bytes)
            .await?
            .head_revision)
    }

    async fn update(&self, handle: &str, bytes: &[u8]) -> Result<Option<String>> {
        let token = self.token().await?;
        api::update_file(&self.client, &token, handle, bytes).await
    }

    async fn delete(&self, handle: &str) -> Result<()> {
        let token = self.token().await?;
        api::delete_file(&self.client, &token, handle).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    // A listed Drive file reaches the core with its id as the handle and every
    // field the probe shows carried across.
    #[test]
    fn a_drive_file_becomes_an_entry_keyed_by_its_id() {
        let file = DriveFile {
            id: "f1".into(),
            name: "a1b2.rowel".into(),
            created_time: "2024-01-01T00:00:00.000Z".into(),
            modified_time: "2024-05-04T10:11:12.000Z".into(),
            size: Some(2048),
            head_revision: Some("r1".into()),
            app_properties: Default::default(),
        };
        assert_eq!(
            entry(file),
            RemoteEntry {
                handle: "f1".into(),
                name: "a1b2.rowel".into(),
                size: Some(2048),
                modified: "2024-05-04T10:11:12.000Z".into(),
                revision: Some("r1".into()),
            }
        );
    }
}
