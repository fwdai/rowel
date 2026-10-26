//! A Rowel Server as a sync provider: where an org lockbox's pack lives.
//!
//! There are no folders. A lockbox is a resource the server owns, its pack
//! one endpoint under it, and the member's listing says which lockboxes there
//! are:
//!
//! ```text
//! GET  /v1/members/me/lockboxes          the listing (`layout::vault_file_name(id)` each)
//! HEAD /v1/lockboxes/{id}/pack           ETag only — the revision
//! GET  /v1/lockboxes/{id}/pack           the bytes, with their ETag
//! PUT  /v1/lockboxes/{id}/pack           If-None-Match: *  → create, or 412
//!                                        If-Match: <ETag>  → update, or 412
//! ```
//!
//! The vault id *is* the server's lockbox id, so a pack's name in the account
//! is the one `layout` mints from it and the handle the core round-trips is
//! the id itself. The revision is the ETag verbatim, quotes included: the
//! server compares what it issued, and the adapter never reads it.
//!
//! The PUT is a compare-and-swap, as Dropbox's is. The server merges on its
//! side too — an agent's `save_login` lands between two device pushes — so
//! every server-side write moves the ETag, a device's stale write fails with
//! 412, the run fails with the retry message, and the next run pulls and
//! merges. A deletion marker is never found: a device never deletes an org
//! lockbox, the server does, and a device that asks is refused.
//!
//! The credential is not in the workspace's token file. That file names the
//! server ([`Tokens::account`]); the member session for it is the install's,
//! sealed under the app key (`org::connections`), and every call here looks
//! it up through a [`SessionResolver`] and carries it as a bearer. A 401
//! refreshes the session once through the server's token endpoint, keeps the
//! refreshed one, and retries; a second 401 is the session ending.

use reqwest::header::{CONTENT_LENGTH, ETAG, IF_MATCH, IF_NONE_MATCH};
use reqwest::{Response, StatusCode};

use crate::error::{Error, Result};
use crate::org::client::checked;
use crate::org::connections::SessionResolver;
use crate::sync::account::Auth;
use crate::sync::store::{Created, RemoteEntry, VaultStore};
use crate::sync::{http_client, layout, read_capped, Provider};

/// Worded for the sync status line, as Dropbox's is: the one failure two
/// devices (or a device and an agent) on one org lockbox will actually hit.
const CONFLICT_MESSAGE: &str =
    "the lockbox changed on the Rowel Server during the upload; sync will retry";

fn other<E: std::fmt::Display>(e: E) -> Error {
    Error::Other(e.to_string())
}

pub struct Store {
    /// The account the store was built from, handed back after a run
    /// (`AnyStore::into_tokens`); `None` for a store built over a server URL
    /// alone, which has no token file to hand anything back to.
    auth: Option<Auth>,
    /// The server the workspace's token file names ([`Tokens::account`]).
    server: Option<String>,
    session: SessionResolver,
    client: reqwest::Client,
}

impl Store {
    /// The store for a workspace's account: the server its token file names,
    /// with the member session the running app holds for it.
    pub fn new(auth: Auth) -> Self {
        Self {
            server: auth.bound_account(),
            session: SessionResolver::App(auth.app().clone()),
            client: http_client(),
            auth: Some(auth),
        }
    }

    /// A store over the server at `server` with whatever `session` resolves,
    /// and no workspace behind it: the tests', which have no token file to
    /// build an [`Auth`] from.
    #[cfg(test)]
    pub(crate) fn over(server: &str, session: SessionResolver) -> Result<Self> {
        Ok(Self {
            auth: None,
            server: Some(crate::org::connections::normalize_url(server)?),
            session,
            client: http_client(),
        })
    }

    pub fn into_auth(self) -> Option<Auth> {
        self.auth
    }

    fn server(&self) -> Result<&str> {
        self.server.as_deref().ok_or_else(|| {
            Error::Other("this lockbox's token file names no Rowel Server to sync through".into())
        })
    }

    fn pack_url(server: &str, id: &str) -> String {
        format!("{server}/v1/lockboxes/{id}/pack")
    }

    /// Send the request `build` makes, as the member: with the session's
    /// bearer, and once more with a refreshed one if the first answer is a
    /// 401. The refreshed session is kept before the retry, so a run that
    /// fails later still leaves the install signed in.
    async fn send(
        &self,
        build: impl Fn(&reqwest::Client, &str, &str) -> reqwest::RequestBuilder,
    ) -> Result<Response> {
        let server = self.server()?;
        let mut connection = self.session.connection(server)?;
        let resp = build(&self.client, server, &connection.access_token)
            .send()
            .await
            .map_err(other)?;
        if resp.status() != StatusCode::UNAUTHORIZED {
            return Ok(resp);
        }
        connection = self.session.refresh(&connection).await?;
        build(&self.client, server, &connection.access_token)
            .send()
            .await
            .map_err(other)
    }
}

/// The ETag the server issued, verbatim.
fn etag_of(resp: &Response) -> Option<String> {
    resp.headers()
        .get(ETAG)
        .and_then(|v| v.to_str().ok())
        .map(String::from)
}

/// The lockbox `id`'s pack as the core sees it: the id is the handle, the
/// ETag the revision. No modified time: the server does not say, and the
/// probe that orders by it never lists a server (`Provider::available`).
fn entry(id: &str, revision: Option<String>, size: Option<u64>) -> RemoteEntry {
    RemoteEntry {
        handle: id.to_string(),
        name: layout::vault_file_name(id),
        size,
        modified: String::new(),
        revision,
    }
}

impl VaultStore for Store {
    fn provider(&self) -> Provider {
        Provider::RowelServer
    }

    // The member's lockboxes that have a pack: one that has never been pushed
    // is not in the account yet, and the first run on it creates it.
    async fn list_vaults(&self) -> Result<Vec<RemoteEntry>> {
        let resp = self
            .send(|client, server, bearer| {
                client
                    .get(format!("{server}/v1/members/me/lockboxes"))
                    .bearer_auth(bearer)
            })
            .await?;
        let mut resp = checked(resp).await?;
        let body = read_capped(&mut resp, crate::sync::pack::MAX_PACK_BYTES).await?;
        let lockboxes: Vec<crate::org::client::Lockbox> =
            serde_json::from_slice(&body).map_err(other)?;
        Ok(lockboxes
            .into_iter()
            .filter(|lockbox| lockbox.etag.is_some())
            .map(|lockbox| entry(&lockbox.id, lockbox.etag, None))
            .collect())
    }

    // A `HEAD` of the pack named, never from a cache. A name that is not a
    // pack's — the deletion marker the core asks about — is nothing here.
    async fn stat(&self, name: &str) -> Result<Option<RemoteEntry>> {
        let Some(id) = layout::vault_id_of(name) else {
            return Ok(None);
        };
        let resp = self
            .send(|client, server, bearer| {
                client.head(Self::pack_url(server, id)).bearer_auth(bearer)
            })
            .await?;
        if resp.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let resp = checked(resp).await?;
        let size = resp
            .headers()
            .get(CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok());
        Ok(Some(entry(id, etag_of(&resp), size)))
    }

    async fn read(&self, handle: &str, max_bytes: usize) -> Result<Vec<u8>> {
        let resp = self
            .send(|client, server, bearer| {
                client
                    .get(Self::pack_url(server, handle))
                    .bearer_auth(bearer)
            })
            .await?;
        if resp.status() == StatusCode::NOT_FOUND {
            return Err(Error::NotFound);
        }
        let mut resp = checked(resp).await?;
        if resp
            .content_length()
            .is_some_and(|len| len > max_bytes as u64)
        {
            return Err(Error::FileTooLarge);
        }
        read_capped(&mut resp, max_bytes).await
    }

    // `If-None-Match: *`: a pack another device created since the core's
    // `stat` fails rather than being replaced, as every adapter's create does.
    async fn create(&self, name: &str, bytes: &[u8]) -> Result<Created> {
        let id = layout::vault_id_of(name)
            .ok_or_else(|| Error::Other(format!("not a lockbox pack name: {name}")))?;
        let resp = self
            .send(|client, server, bearer| {
                client
                    .put(Self::pack_url(server, id))
                    .bearer_auth(bearer)
                    .header(IF_NONE_MATCH, "*")
                    .body(bytes.to_vec())
            })
            .await?;
        if resp.status() == StatusCode::PRECONDITION_FAILED {
            return Err(Error::Other(
                "another device created this lockbox's pack on the Rowel Server while this one \
                 was syncing; sync will retry"
                    .into(),
            ));
        }
        let resp = checked(resp).await?;
        Ok(Created {
            handle: id.to_string(),
            revision: etag_of(&resp),
        })
    }

    // `If-Match` on the revision the run pulled, always: a pack rewritten
    // since — by a device or by the server's own merge of an agent's write —
    // is a 412, and the run fails with the retry message rather than
    // overwrite a write it never merged. An entry with no revision to match
    // on is refused here, before anything is sent: a PUT without the
    // precondition would be exactly that overwrite, and the engine's next
    // run pulls a revision to match. A pack that is gone is an error, never
    // a create.
    async fn update(&self, entry: &RemoteEntry, bytes: &[u8]) -> Result<Option<String>> {
        let revision = entry
            .revision
            .as_deref()
            .filter(|rev| !rev.is_empty())
            .ok_or_else(|| {
                Error::Other(
                    "the lockbox's pack on the Rowel Server has no revision to update against; \
                     sync will retry"
                        .into(),
                )
            })?;
        let resp = self
            .send(|client, server, bearer| {
                client
                    .put(Self::pack_url(server, &entry.handle))
                    .bearer_auth(bearer)
                    .header(IF_MATCH, revision)
                    .body(bytes.to_vec())
            })
            .await?;
        match resp.status() {
            StatusCode::PRECONDITION_FAILED => Err(Error::Other(CONFLICT_MESSAGE.into())),
            StatusCode::NOT_FOUND => Err(Error::NotFound),
            _ => Ok(etag_of(&checked(resp).await?)),
        }
    }

    async fn delete(&self, _handle: &str) -> Result<()> {
        Err(Error::Unsupported(
            "an org lockbox is removed at the Rowel Server, never from a device".into(),
        ))
    }
}
