//! Where a one-time share lives on Drive while it is outstanding: a `Shares`
//! subfolder of the same `Rowel` Drive folder sync already uses.
//!
//! The sender uploads sealed bytes and makes the file link-readable; the
//! recipient — who has no Google account — fetches it with a public API key.
//! Drive only ever holds ciphertext plus opaque bookkeeping (an entry UUID, a
//! kind, an expiry), so the folder listing tells the *sender's* UI what is
//! outstanding without telling Google what was shared.
//!
//! Like `sync`, this is only the transport: the Drive calls are async and are
//! driven with `block_on` behind the synchronous traits in
//! [`crate::share::remote`], which is safe because commands run them on
//! `spawn_blocking` threads, never on a runtime worker.

use std::sync::Mutex;

use reqwest::Client;
use tauri::{async_runtime::block_on, AppHandle};

use super::api::{self as drive, DriveFile};
use crate::crypto::Cryptor;
use crate::error::{Error, Result};
use crate::share::envelope::MAX_SHARE_BYTES;
use crate::share::remote::{
    PublicFetch, ShareFile, ShareRemote, PROP_ENTRY_ID, PROP_EXPIRES_AT, PROP_KIND, PROP_SHARE,
    PROP_SHARE_VALUE, PROP_VAULT_ID,
};
use crate::sync::layout::{ROOT_FOLDER, SHARES_FOLDER};
use crate::sync::{access_token, http_client};

/// How long the recipient waits on Drive before giving up.
const FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Read a listed Drive file as a share. Pure: every field is already in the
/// listing, so neither the UI nor the sweep needs a second round trip.
pub fn parse_share_file(file: &DriveFile) -> ShareFile {
    ShareFile {
        id: file.id.clone(),
        marked: file.app_properties.get(PROP_SHARE).map(String::as_str) == Some(PROP_SHARE_VALUE),
        entry_id: file.app_properties.get(PROP_ENTRY_ID).cloned(),
        kind: file.app_properties.get(PROP_KIND).cloned(),
        vault_id: file.app_properties.get(PROP_VAULT_ID).cloned(),
        created_ms: chrono::DateTime::parse_from_rfc3339(&file.created_time)
            .map(|t| t.timestamp_millis())
            .unwrap_or(0),
        expires_ms: file
            .app_properties
            .get(PROP_EXPIRES_AT)
            .and_then(|v| v.parse().ok()),
    }
}

/// [`ShareRemote`] over the `Rowel/Shares` folder of the connected account.
///
/// The folder id is resolved once per instance and cached: unlike the sync
/// pack, a share file is addressed by the id upload just returned, so nothing
/// here has to observe another device's writes mid-operation.
pub struct DriveShareRemote {
    app: AppHandle,
    cryptor: Cryptor,
    folder: Mutex<Option<String>>,
}

impl DriveShareRemote {
    pub fn new(app: AppHandle, cryptor: Cryptor) -> Self {
        Self {
            app,
            cryptor,
            folder: Mutex::new(None),
        }
    }

    async fn token(&self, client: &Client) -> Result<String> {
        access_token(client, &self.app, &self.cryptor).await
    }

    /// `Rowel/Shares`, if it exists. Never creates: every sync run sweeps, and
    /// most users never share, so a listing must not leave a folder behind in
    /// an account that has nothing to list.
    async fn find_folder(&self, client: &Client, token: &str) -> Result<Option<String>> {
        if let Some(id) = self.folder.lock().unwrap().clone() {
            return Ok(Some(id));
        }
        let Some(root) = drive::folder_id(client, token, ROOT_FOLDER).await? else {
            return Ok(None);
        };
        let shares = drive::folder_id_in(client, token, SHARES_FOLDER, &root).await?;
        if let Some(id) = &shares {
            *self.folder.lock().unwrap() = Some(id.clone());
        }
        Ok(shares)
    }

    /// Find-or-create `Rowel/Shares`, for the upload path only. Creating is the
    /// normal case the first time a user shares anything, including before
    /// they have ever synced.
    async fn ensure_folder(&self, client: &Client, token: &str) -> Result<String> {
        if let Some(id) = self.find_folder(client, token).await? {
            return Ok(id);
        }
        let root = match drive::folder_id(client, token, ROOT_FOLDER).await? {
            Some(id) => id,
            None => drive::create_folder(client, token, ROOT_FOLDER).await?,
        };
        let shares = drive::create_folder_in(client, token, SHARES_FOLDER, Some(&root)).await?;
        *self.folder.lock().unwrap() = Some(shares.clone());
        Ok(shares)
    }
}

impl ShareRemote for DriveShareRemote {
    fn upload(&self, name: &str, bytes: &[u8], properties: &[(&str, &str)]) -> Result<String> {
        block_on(async {
            let client = http_client();
            let token = self.token(&client).await?;
            let folder = self.ensure_folder(&client, &token).await?;
            let file = drive::create_file_with_properties(
                &client, &token, name, &folder, bytes, properties,
            )
            .await?;
            Ok(file.id)
        })
    }

    fn make_public(&self, id: &str) -> Result<()> {
        block_on(async {
            let client = http_client();
            let token = self.token(&client).await?;
            drive::share_with_anyone(&client, &token, id).await
        })
    }

    fn delete(&self, id: &str) -> Result<()> {
        block_on(async {
            let client = http_client();
            let token = self.token(&client).await?;
            drive::delete_file(&client, &token, id).await
        })
    }

    fn list(&self) -> Result<Vec<ShareFile>> {
        block_on(async {
            let client = http_client();
            let token = self.token(&client).await?;
            let files =
                drive::find_by_app_property(&client, &token, PROP_SHARE, PROP_SHARE_VALUE).await?;
            Ok(files.iter().map(parse_share_file).collect())
        })
    }

    fn get(&self, id: &str) -> Result<Option<ShareFile>> {
        block_on(async {
            let client = http_client();
            let token = self.token(&client).await?;
            let file = drive::get_file(&client, &token, id).await?;
            Ok(file.as_ref().map(parse_share_file))
        })
    }
}

/// The recipient's fetcher. Holds no account state — an API key is all a
/// link-shared download needs.
pub struct DrivePublicFetch;

impl PublicFetch for DrivePublicFetch {
    fn download(&self, file_id: &str) -> Result<Vec<u8>> {
        let key = api_key()?;
        block_on(async {
            // Not the shared client: this request is made on a stranger's
            // say-so, so on top of the shared stall deadlines it gets one for
            // the whole request, which a 2 MiB cap makes reasonable.
            let client = crate::sync::http_client_builder()
                .timeout(FETCH_TIMEOUT)
                .build()
                .map_err(|e| Error::Other(e.to_string()))?;
            drive::download_public(&client, &key, file_id, MAX_SHARE_BYTES).await
        })
    }
}

/// The Google API key this build downloads with, from the environment at run
/// time or baked in at build time. A public identifier, not a secret: it names
/// the project for quota and grants nothing on its own. Mirrors the OAuth
/// client resolution in [`super::OAUTH`].
fn api_key() -> Result<String> {
    std::env::var("GOOGLE_API_KEY")
        .ok()
        .or_else(|| option_env!("GOOGLE_API_KEY").map(String::from))
        .filter(|key| !key.is_empty())
        .ok_or_else(|| Error::Other("Google API key not configured; set GOOGLE_API_KEY".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn drive_file(created: &str, properties: &[(&str, &str)]) -> DriveFile {
        DriveFile {
            id: "f1".into(),
            name: "share.rowelshare".into(),
            created_time: created.into(),
            modified_time: String::new(),
            size: None,
            head_revision: None,
            app_properties: properties
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect::<BTreeMap<_, _>>(),
        }
    }

    #[test]
    fn a_share_file_is_read_out_of_the_listing_alone() {
        let parsed = parse_share_file(&drive_file(
            "2024-03-01T12:00:00.000Z",
            &[
                (PROP_SHARE, PROP_SHARE_VALUE),
                (PROP_ENTRY_ID, "entry-uuid"),
                (PROP_KIND, "login"),
                (PROP_EXPIRES_AT, "1709300000000"),
                (PROP_VAULT_ID, "a1b2"),
            ],
        ));

        assert_eq!(parsed.id, "f1");
        assert!(parsed.marked);
        assert_eq!(parsed.entry_id.as_deref(), Some("entry-uuid"));
        assert_eq!(parsed.kind.as_deref(), Some("login"));
        assert_eq!(parsed.vault_id.as_deref(), Some("a1b2"));
        assert_eq!(parsed.created_ms, 1_709_294_400_000);
        assert_eq!(parsed.expires_ms, Some(1_709_300_000_000));
    }

    // A file with no marker is not this app's share, whatever else it carries —
    // the one thing `revoke` has to be able to tell about an id it was handed.
    #[test]
    fn a_share_without_properties_reads_as_unknown_rather_than_failing() {
        let parsed = parse_share_file(&drive_file("2024-03-01T12:00:00.000Z", &[]));
        assert!(!parsed.marked);
        assert_eq!(parsed.entry_id, None);
        assert_eq!(parsed.kind, None);
        assert_eq!(parsed.vault_id, None);
        assert_eq!(parsed.expires_ms, None);
    }

    // A file the sweep cannot date must not read as "expired long ago".
    #[test]
    fn malformed_times_degrade_to_epoch_and_no_expiry() {
        let parsed = parse_share_file(&drive_file("not a time", &[(PROP_EXPIRES_AT, "soon")]));
        assert_eq!(parsed.created_ms, 0);
        assert_eq!(parsed.expires_ms, None);
    }
}
