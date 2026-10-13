//! Dropbox, with **App folder** access.
//!
//! The app is registered as an App folder app, so Dropbox gives it one folder,
//! `/Apps/<app name>/` in the user's account, and every path the API takes or
//! returns is relative to it. The app cannot see, list or touch anything
//! outside. That folder stands in for `layout::ROOT_FOLDER`, so the layout
//! inside it is just:
//!
//! ```text
//! /Vaults/<vault-id>.rowel      the live pack (layout::vault_file_name)
//! /Vaults/<vault-id>.deleted    the deletion marker (layout::deleted_marker_name)
//! ```
//!
//! `/Vaults` is read as `/Lockboxes` once a later release has renamed it, by
//! the core's rule ([`crate::sync::store::pick_packs_folder`]); a fresh
//! account only ever gets `/Vaults`.
//!
//! Dropbox paths are unique and case-insensitive, so unlike Drive there are no
//! same-name siblings to choose between. Entries also carry a stable `id:...`
//! that every call here accepts in place of a path; it is the handle the core
//! holds ([`RemoteEntry::handle`]).
//!
//! - [`api`]: the v2 REST calls (list, metadata, download, upload, delete).
//! - [`oauth`]: endpoints, consent parameters, app key and redirect scheme.

pub mod api;
pub mod oauth;

use std::future::Future;
use std::sync::Mutex;

#[cfg(mobile)]
use tauri::AppHandle;

use crate::error::{Error, Result};
use crate::sync::account::{Auth, OAuthConfig};
use crate::sync::store::{ensure_packs_folder, find_packs_folder, RemoteEntry, VaultStore};
use crate::sync::{http_client, Provider};
use api::{DropboxEntry, WriteMode};

/// The app folder as the user finds it in their Dropbox. Dropbox names it
/// after the app, which is registered as `layout::ROOT_FOLDER`.
pub const APP_FOLDER: &str = "Apps/Rowel";

/// Dropbox's OAuth client, as the shared flow in `sync::account` drives it.
/// A public client: an app key, PKCE, and never a secret.
pub static OAUTH: OAuthConfig = OAuthConfig {
    provider: Provider::Dropbox,
    auth_url: oauth::AUTH_URL,
    token_url: oauth::TOKEN_URL,
    consent_params: oauth::CONSENT_PARAMS,
    // Dropbox matches the redirect exactly, port included: only these.
    #[cfg(desktop)]
    loopback_ports: &oauth::LOOPBACK_PORTS,
    #[cfg(desktop)]
    client_id: oauth::app_key,
    #[cfg(desktop)]
    client_secret: || None,
    no_client: "Dropbox app key not configured; set DROPBOX_APP_KEY",
    #[cfg(mobile)]
    mobile_client,
    #[cfg(mobile)]
    mobile_cancel_uri,
};

/// The app key as this phone build knows it, and so the scheme the redirect
/// comes back on. The deep-link scheme (`db-<APP_KEY>`) is the source of truth,
/// as Google's reversed client id is: it is what registers the app for the
/// redirect, and on iOS it is the only way the key reaches the binary at all —
/// `tauri ios build` compiles with an environment `option_env!` never sees, so
/// the release build injects the scheme instead (`scripts/release-ios.sh`).
/// The environment is the fallback, for a build that registers no scheme.
#[cfg(mobile)]
fn mobile_app_key(app: &AppHandle) -> Option<String> {
    super::deep_link_scheme(app, oauth::MOBILE_SCHEME_PREFIX)
        .and_then(|scheme| app_key_from_scheme(&scheme))
        .or_else(oauth::app_key)
}

/// The phone's client: the app key, and `db-<APP_KEY>://2/token`.
#[cfg(mobile)]
fn mobile_client(app: &AppHandle) -> Result<(String, String)> {
    let key = mobile_app_key(app).ok_or_else(|| crate::sync::account::no_client(&OAUTH))?;
    let redirect = oauth::mobile_redirect_uri(&key);
    Ok((key, redirect))
}

/// `db-<APP_KEY>://2/cancel`, where a consent the user cancelled lands.
#[cfg(mobile)]
fn mobile_cancel_uri(app: &AppHandle) -> Option<String> {
    mobile_app_key(app).map(|key| oauth::mobile_cancel_uri(&key))
}

/// `db-abc123` -> `abc123`. `None` for any other scheme, or the bare prefix.
#[cfg(any(mobile, test))]
fn app_key_from_scheme(scheme: &str) -> Option<String> {
    scheme
        .strip_prefix(oauth::MOBILE_SCHEME_PREFIX)
        .filter(|key| !key.is_empty())
        .map(String::from)
}

/// A folder at the top of the app folder, by path: `/Vaults`. A folder's
/// handle is its path; only files are addressed by `id:`.
fn folder_path(name: &str) -> String {
    format!("/{name}")
}

/// `name` inside the packs folder `folder`: `/Vaults/<name>`.
fn entry_path(folder: &str, name: &str) -> String {
    format!("{folder}/{name}")
}

/// The folder `name` at the top of the app folder, as its path, or `None`
/// when there is none. Something there that is not a folder is not one.
async fn folder_at(client: &reqwest::Client, token: &str, name: &str) -> Result<Option<String>> {
    let found = api::get_metadata(client, token, &folder_path(name)).await?;
    Ok(found.filter(|e| e.is_folder).map(|_| folder_path(name)))
}

/// The packs folder (`/Vaults`, or `/Lockboxes` after the cutover), if the
/// account has one, settled by the core's rule over `lookup`.
async fn packs_folder<F, Fut>(lookup: F) -> Result<Option<String>>
where
    F: FnMut(&'static str) -> Fut,
    Fut: Future<Output = Result<Option<String>>>,
{
    find_packs_folder(Provider::Dropbox, lookup).await
}

/// [`packs_folder`], or `/Vaults` when the account has none. Nothing is
/// created here: the upload that follows creates its parent folders.
async fn packs_folder_for_create<F, Fut>(lookup: F) -> Result<String>
where
    F: FnMut(&'static str) -> Fut,
    Fut: Future<Output = Result<Option<String>>>,
{
    ensure_packs_folder(Provider::Dropbox, lookup, |name| {
        std::future::ready(Ok(folder_path(name)))
    })
    .await
}

/// [`VaultStore`] over the connected account's app folder.
///
/// The packs folder is resolved once per instance (one instance per run), as
/// Drive resolves its folder ids: the rule is deterministic and asking again
/// would only cost round trips. A *file* is never cached: every `stat` asks the
/// server, which is what `head_revision` relies on to see another device's
/// write.
pub struct Store {
    auth: Auth,
    client: reqwest::Client,
    packs: Mutex<Option<String>>,
}

impl Store {
    pub fn new(auth: Auth) -> Self {
        Self {
            auth,
            client: http_client(),
            packs: Mutex::new(None),
        }
    }

    pub fn into_auth(self) -> Auth {
        self.auth
    }

    async fn token(&self) -> Result<String> {
        self.auth.access_token(&self.client).await
    }

    fn cached_packs(&self) -> Option<String> {
        self.packs.lock().unwrap().clone()
    }

    fn cache_packs(&self, path: &str) {
        *self.packs.lock().unwrap() = Some(path.to_string());
    }

    // The packs folder, remembered for the rest of the run once found. `None`
    // means there is none yet — which is also "no remote vault".
    async fn packs(&self, token: &str) -> Result<Option<String>> {
        if let Some(path) = self.cached_packs() {
            return Ok(Some(path));
        }
        let found = packs_folder(|name| folder_at(&self.client, token, name)).await?;
        if let Some(path) = &found {
            self.cache_packs(path);
        }
        Ok(found)
    }

    // [`Store::packs`], for a create: `/Vaults` when the account has none.
    async fn packs_for_create(&self, token: &str) -> Result<String> {
        if let Some(path) = self.cached_packs() {
            return Ok(path);
        }
        let path = packs_folder_for_create(|name| folder_at(&self.client, token, name)).await?;
        self.cache_packs(&path);
        Ok(path)
    }
}

/// A handle as this adapter hands them out: a Dropbox `id:...`. Anything else
/// is refused rather than passed on as a path — the handle round-trips through
/// the frontend, and only ever names an entry a listing or `stat` returned.
fn id_path(handle: &str) -> Result<&str> {
    if handle.starts_with("id:") && handle.len() > 3 {
        Ok(handle)
    } else {
        Err(Error::Other(format!("not a Dropbox file id: {handle}")))
    }
}

/// A Dropbox file as the core sees it. The `id:` value is the handle, `rev` the
/// revision, `server_modified` the time the probe orders by.
fn entry(file: DropboxEntry) -> RemoteEntry {
    RemoteEntry {
        handle: file.id,
        name: file.name,
        size: file.size,
        modified: file.server_modified,
        revision: file.rev,
    }
}

/// The files of a listing, as entries. A folder inside `Vaults/` is nothing the
/// app wrote and nothing it can read as a pack, so it is not reported.
fn file_entries(listing: Vec<DropboxEntry>) -> Vec<RemoteEntry> {
    listing
        .into_iter()
        .filter(|e| !e.is_folder)
        .map(entry)
        .collect()
}

/// How an update writes over the entry the core located.
///
/// With its rev, a server-side compare-and-swap ([`WriteMode::Update`]): the
/// upload lands only if the file is still exactly what the `stat` saw. A plain
/// overwrite would not do, even by `id:` — Dropbox keeps a deleted file's id,
/// so an overwrite racing a "delete everywhere" (marker written, pack deleted)
/// would put the pack back beside its marker, and a fetch only consults the
/// marker when the pack is missing: the deleted vault would come back on every
/// device. It would also let another device's write between the `stat` and
/// this upload be lost, a window the engine's head check cannot close.
///
/// Without one (the listing gave none, which Dropbox does not do for a file)
/// there is nothing to compare against, so the overwrite by `id:` is the best
/// left: still never a new file under a new name.
fn update_mode(revision: Option<&str>) -> WriteMode {
    match revision.filter(|rev| !rev.is_empty()) {
        Some(rev) => WriteMode::Update(rev.to_string()),
        None => WriteMode::Overwrite,
    }
}

impl VaultStore for Store {
    fn provider(&self) -> Provider {
        Provider::Dropbox
    }

    // No packs folder is an account nothing has ever synced to, which is the
    // same answer as an empty one.
    async fn list_vaults(&self) -> Result<Vec<RemoteEntry>> {
        let token = self.token().await?;
        let Some(folder) = self.packs(&token).await? else {
            return Ok(Vec::new());
        };
        let listing = api::list_folder(&self.client, &token, &folder).await?;
        Ok(file_entries(listing))
    }

    async fn stat(&self, name: &str) -> Result<Option<RemoteEntry>> {
        let token = self.token().await?;
        let Some(folder) = self.packs(&token).await? else {
            return Ok(None);
        };
        let path = entry_path(&folder, name);
        let found = api::get_metadata(&self.client, &token, &path).await?;
        Ok(found.filter(|e| !e.is_folder).map(entry))
    }

    // The rev the download reports is dropped on purpose: the core records the
    // one from the `stat` that located the entry (see `VaultStore::read`).
    async fn read(&self, handle: &str, max_bytes: usize) -> Result<Vec<u8>> {
        let token = self.token().await?;
        let (bytes, _rev) =
            api::download(&self.client, &token, id_path(handle)?, max_bytes).await?;
        Ok(bytes)
    }

    // Parent folders are created by the upload itself. `api::upload` writes in
    // `overwrite` mode, so a pack another device created between the core's
    // `stat` and this call is replaced rather than duplicated — which a full-
    // state merge on that device's next run recovers from; Drive would have
    // made a same-name sibling instead.
    async fn create(&self, name: &str, bytes: &[u8]) -> Result<Option<String>> {
        let token = self.token().await?;
        let folder = self.packs_for_create(&token).await?;
        api::upload(
            &self.client,
            &token,
            &entry_path(&folder, name),
            &WriteMode::Overwrite,
            bytes,
        )
        .await
    }

    // A compare-and-swap on the rev the core's `stat` just saw
    // ([`update_mode`]), addressed by `id:`.
    async fn update(&self, entry: &RemoteEntry, bytes: &[u8]) -> Result<Option<String>> {
        let path = id_path(&entry.handle)?;
        let token = self.token().await?;
        let mode = update_mode(entry.revision.as_deref());
        api::upload(&self.client, &token, path, &mode, bytes).await
    }

    async fn delete(&self, handle: &str) -> Result<()> {
        let token = self.token().await?;
        api::delete(&self.client, &token, id_path(handle)?).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::layout;

    fn file(id: &str, name: &str) -> DropboxEntry {
        DropboxEntry {
            id: id.into(),
            name: name.into(),
            path_lower: format!("/vaults/{}", name.to_lowercase()),
            is_folder: false,
            size: Some(2048),
            rev: Some("015f9d2a3c1e".into()),
            server_modified: "2024-05-04T10:11:12Z".into(),
        }
    }

    // A listed Dropbox file reaches the core with its `id:` as the handle, its
    // rev as the revision and its server time as the one the probe orders by.
    #[test]
    fn a_dropbox_file_becomes_an_entry_keyed_by_its_id() {
        assert_eq!(
            entry(file("id:a4ayc_80_OEAAAAAAAAAXw", "a1b2.rowel")),
            RemoteEntry {
                handle: "id:a4ayc_80_OEAAAAAAAAAXw".into(),
                name: "a1b2.rowel".into(),
                size: Some(2048),
                modified: "2024-05-04T10:11:12Z".into(),
                revision: Some("015f9d2a3c1e".into()),
            }
        );
    }

    // Unknown stays unknown: no rev is `None`, never a made-up value.
    #[test]
    fn a_file_without_a_rev_has_no_revision() {
        let mut f = file("id:x", "a1b2.rowel");
        f.rev = None;
        f.size = None;
        let e = entry(f);
        assert_eq!(e.revision, None);
        assert_eq!(e.size, None);
    }

    #[test]
    fn folders_in_the_vaults_folder_are_not_entries() {
        let folder = DropboxEntry {
            id: "id:dir".into(),
            name: "old".into(),
            path_lower: "/vaults/old".into(),
            is_folder: true,
            size: None,
            rev: None,
            server_modified: String::new(),
        };
        let entries = file_entries(vec![
            file("id:a", "a1b2.rowel"),
            folder,
            file("id:b", "c3d4.deleted"),
        ]);
        let handles: Vec<&str> = entries.iter().map(|e| e.handle.as_str()).collect();
        assert_eq!(handles, ["id:a", "id:b"]);
    }

    // `api::list_folder` answers a missing `/Vaults` with no entries; the store
    // passes that through as an empty account, not an error.
    #[test]
    fn a_missing_vaults_folder_is_an_empty_account() {
        assert!(file_entries(Vec::new()).is_empty());
    }

    // --- which folder is `Vaults/` ---

    // The folders at the top of an app folder, as `folder_at` would find them:
    // each by its path.
    fn lookup_in(
        folders: &'static [&'static str],
    ) -> impl FnMut(&'static str) -> std::future::Ready<Result<Option<String>>> {
        move |name| std::future::ready(Ok(folders.contains(&name).then(|| folder_path(name))))
    }

    #[tokio::test]
    async fn an_app_folder_with_vaults_syncs_there() {
        let found = packs_folder(lookup_in(&[layout::VAULTS_FOLDER])).await;
        assert_eq!(found.unwrap().as_deref(), Some("/Vaults"));
    }

    #[tokio::test]
    async fn an_app_folder_renamed_by_the_cutover_syncs_in_lockboxes() {
        let found = packs_folder(lookup_in(&[layout::LOCKBOXES_FOLDER])).await;
        assert_eq!(found.unwrap().as_deref(), Some("/Lockboxes"));
    }

    #[tokio::test]
    async fn an_app_folder_with_neither_has_no_packs_folder() {
        assert_eq!(packs_folder(lookup_in(&[])).await.unwrap(), None);
    }

    // Named as the user finds them in their Dropbox, under `Apps/`.
    #[tokio::test]
    async fn an_app_folder_with_both_is_refused_naming_both() {
        let both = &[layout::LOCKBOXES_FOLDER, layout::VAULTS_FOLDER];
        let err = packs_folder(lookup_in(both)).await.unwrap_err();
        assert!(
            matches!(&err, Error::Other(m) if m.starts_with("this Dropbox holds both Apps/Rowel/Vaults and Apps/Rowel/Lockboxes;")),
            "{err:?}"
        );
        assert!(packs_folder_for_create(lookup_in(both)).await.is_err());
    }

    // A fresh account gets `/Vaults`, never `/Lockboxes`; one that has either
    // keeps it.
    #[tokio::test]
    async fn a_create_targets_vaults_unless_the_account_has_a_folder() {
        let fresh = packs_folder_for_create(lookup_in(&[])).await.unwrap();
        assert_eq!(fresh, "/Vaults");
        assert_eq!(entry_path(&fresh, "a1b2.rowel"), "/Vaults/a1b2.rowel");

        let renamed = packs_folder_for_create(lookup_in(&[layout::LOCKBOXES_FOLDER]))
            .await
            .unwrap();
        assert_eq!(entry_path(&renamed, "a1b2.rowel"), "/Lockboxes/a1b2.rowel");
    }

    #[test]
    fn the_app_folder_is_named_after_the_app() {
        assert_eq!(APP_FOLDER, format!("Apps/{}", layout::ROOT_FOLDER));
        assert_eq!(Provider::Dropbox.packs_root_label(), APP_FOLDER);
        assert_eq!(Provider::GoogleDrive.packs_root_label(), "Rowel");
    }

    // Only an `id:` is ever addressed by handle: a path smuggled in through the
    // frontend's `fileId` is refused before it reaches Dropbox.
    #[test]
    fn only_dropbox_ids_are_handles() {
        assert_eq!(id_path("id:abc").unwrap(), "id:abc");
        assert!(id_path("id:").is_err());
        assert!(id_path("/Vaults/a1b2.rowel").is_err());
        assert!(id_path("").is_err());
    }

    // An update is conditional on the rev the `stat` saw whenever there is
    // one; only an unknown rev falls back to a plain overwrite.
    #[test]
    fn an_update_is_a_compare_and_swap_on_the_known_rev() {
        assert_eq!(
            update_mode(Some("015f9d2a3c1e")),
            WriteMode::Update("015f9d2a3c1e".into())
        );
        assert_eq!(update_mode(None), WriteMode::Overwrite);
        assert_eq!(update_mode(Some("")), WriteMode::Overwrite);
    }

    #[test]
    fn the_oauth_client_is_dropboxs_public_client() {
        assert_eq!(OAUTH.provider, Provider::Dropbox);
        assert_eq!(OAUTH.auth_url, "https://www.dropbox.com/oauth2/authorize");
        assert_eq!(OAUTH.token_url, "https://api.dropboxapi.com/oauth2/token");
        assert!(OAUTH.no_client.contains(oauth::APP_KEY_ENV));
    }

    // The desktop client is the only one that could carry a secret, and it
    // does not: a PKCE public client never sends one.
    #[cfg(desktop)]
    #[test]
    fn the_desktop_client_sends_no_secret() {
        assert!((OAUTH.client_secret)().is_none());
    }

    #[test]
    fn consent_asks_for_offline_access_and_the_file_scopes() {
        assert!(OAUTH
            .consent_params
            .contains(&("token_access_type", "offline")));
        assert!(OAUTH.consent_params.contains(&("scope", oauth::SCOPE)));
    }

    // Dropbox has no any-port loopback rule: the desktop flow must be told the
    // registered ports, or it would bind one the App Console never heard of.
    #[cfg(desktop)]
    #[test]
    fn the_desktop_flow_uses_the_registered_loopback_ports() {
        assert!(!OAUTH.loopback_ports.is_empty());
        assert_eq!(OAUTH.loopback_ports, oauth::LOOPBACK_PORTS);
    }

    #[test]
    fn the_mobile_app_key_is_read_back_from_its_scheme() {
        assert_eq!(app_key_from_scheme("db-abc123").as_deref(), Some("abc123"));
        assert_eq!(app_key_from_scheme("db-"), None);
        assert_eq!(app_key_from_scheme("com.googleusercontent.apps.1-a"), None);
        let key = app_key_from_scheme("db-abc123").unwrap();
        assert_eq!(oauth::mobile_redirect_uri(&key), "db-abc123://2/token");
        assert_eq!(oauth::mobile_cancel_uri(&key), "db-abc123://2/cancel");
    }
}
