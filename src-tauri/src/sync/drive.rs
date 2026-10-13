//! Drive v3 REST surface. All calls are authorized with a bearer access token.
//! Query values are escaped and passed as parameters rather than
//! string-concatenated (fixes the legacy `q` injection).
//!
//! Bodies are **bytes**, not strings: the artifact this module moves is a
//! `.rowel` pack — SQLCipher ciphertext with a binary header — and routing it
//! through `String` would either corrupt it or fail to decode.

use std::collections::BTreeMap;
use std::future::Future;

use reqwest::Client;
use serde_json::{json, Value};

use super::layout;
use crate::error::{Error, Result};

const FILES: &str = "https://www.googleapis.com/drive/v3/files";
const UPLOAD: &str = "https://www.googleapis.com/upload/drive/v3/files";
// `createdTime` drives the deterministic pick below; `headRevisionId` is the
// change token the engine uses to detect a push that landed under it.
// `nextPageToken` has to be named too: the mask filters the whole response, so
// one that only asks for `files(...)` gets the files and no token back, which
// silently turns every listing into its first page.
// `size` and `modifiedTime` are what first-run onboarding shows the user about
// a remote vault it has not downloaded yet ("2.1 MB, last changed yesterday").
const LIST_FIELDS: &str =
    "nextPageToken, files(id, name, createdTime, modifiedTime, size, headRevisionId, appProperties)";
const PAGE_SIZE: &str = "100";
const FILE_FIELDS: &str =
    "id, name, createdTime, modifiedTime, size, headRevisionId, appProperties";
// What an upload declares when it is not a vault pack — shares, which are
// opaque sealed blobs to everything but the recipient's app.
const FILE_MIME: &str = "application/octet-stream";
const FOLDER_MIME: &str = "application/vnd.google-apps.folder";

/// One Drive object, with the fields selection and race detection need.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriveFile {
    pub id: String,
    pub name: String,
    pub created_time: String,
    /// Last content change, RFC 3339 UTC. Empty when the caller's field mask
    /// did not ask for it.
    pub modified_time: String,
    /// Byte length. Drive only reports one for blob files (a folder or a Google
    /// Docs file has none), and sends it as a *string* — see [`parse_size`].
    pub size: Option<u64>,
    /// Absent on folders, and on a file Drive has not yet assigned a revision
    /// to. The engine treats an absent revision as "unknown", which only costs
    /// it a race check, never correctness.
    pub head_revision: Option<String>,
    /// `appProperties`: private per-file metadata, visible only to this OAuth
    /// client. Sharing stores its bookkeeping (entry id, kind, expiry) here so
    /// a listing alone answers what a share is, without downloading it.
    pub app_properties: BTreeMap<String, String>,
}

fn other<E: std::fmt::Display>(e: E) -> Error {
    Error::Other(e.to_string())
}

// Escape a value for use inside a Drive `q` string literal (`name = '...'`).
fn escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\'', "\\'")
}

/// The most of a Drive API JSON response — metadata, a listing page, an error
/// — we will hold. Generous by a wide margin: a full listing page is tens of
/// KiB. It is here so that no response body is ever buffered unbounded, the
/// same rule the file downloads follow (see [`read_capped`]).
const MAX_JSON_BYTES: usize = 4 * 1024 * 1024;

/// The error for a non-success response, carrying whatever of the body could be
/// read as the detail. A body that could *not* be read — oversized, or a
/// transport failure part way through — says so: collapsing that into an empty
/// string leaves `Drive API 500: ` and nothing to diagnose from.
fn api_error(status: reqwest::StatusCode, body: Result<Vec<u8>>) -> Error {
    let detail = match &body {
        Ok(body) => String::from_utf8_lossy(body).into_owned(),
        Err(e) => format!("<could not read body: {e}>"),
    };
    Error::Other(format!("Drive API {status}: {detail}"))
}

async fn check(mut resp: reqwest::Response) -> Result<Value> {
    let status = resp.status();
    if !status.is_success() {
        let body = read_capped(&mut resp, MAX_JSON_BYTES).await;
        return Err(api_error(status, body));
    }
    let body = read_capped(&mut resp, MAX_JSON_BYTES).await?;
    if body.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_slice(&body).map_err(other)
}

// Run a files.list query and return *every* match.
async fn find_all(client: &Client, token: &str, q: &str) -> Result<Vec<DriveFile>> {
    collect_pages(|page_token| async move {
        let resp = client
            .get(FILES)
            .bearer_auth(token)
            .query(&list_query(q, page_token.as_deref()))
            .send()
            .await
            .map_err(other)?;
        check(resp).await
    })
    .await
}

// The query string of one `files.list` request: the first page without a
// token, every later one with the token the page before handed back.
fn list_query<'a>(q: &'a str, page_token: Option<&'a str>) -> Vec<(&'static str, &'a str)> {
    let mut query = vec![("q", q), ("fields", LIST_FIELDS), ("pageSize", PAGE_SIZE)];
    if let Some(token) = page_token {
        query.push(("pageToken", token));
    }
    query
}

// Follow a listing to its last page. Drive pages whenever it likes, not only
// past `pageSize`, so one request is never the whole answer: a share left on a
// later page would be invisible to both the sweep and the revoke list. `fetch`
// makes one request, given the page token to send, and returns the raw page;
// keeping the transport out here is what lets the loop be tested end to end.
async fn collect_pages<F, Fut>(mut fetch: F) -> Result<Vec<DriveFile>>
where
    F: FnMut(Option<String>) -> Fut,
    Fut: Future<Output = Result<Value>>,
{
    let mut files = Vec::new();
    let mut page_token: Option<String> = None;
    loop {
        let (page, next) = parse_listing(&fetch(page_token).await?);
        files.extend(page);
        match next {
            Some(next) => page_token = Some(next),
            None => return Ok(files),
        }
    }
}

/// One page of a `files.list` response: its files and the token for the next
/// page, if Drive says there is one.
fn parse_listing(data: &Value) -> (Vec<DriveFile>, Option<String>) {
    let files = data["files"]
        .as_array()
        .map(|files| files.iter().filter_map(parse_file).collect())
        .unwrap_or_default();
    let next = data["nextPageToken"]
        .as_str()
        .filter(|t| !t.is_empty())
        .map(String::from);
    (files, next)
}

fn parse_file(value: &Value) -> Option<DriveFile> {
    Some(DriveFile {
        id: value["id"].as_str()?.to_string(),
        name: value["name"].as_str().unwrap_or("").to_string(),
        created_time: value["createdTime"].as_str().unwrap_or("").to_string(),
        modified_time: value["modifiedTime"].as_str().unwrap_or("").to_string(),
        size: parse_size(&value["size"]),
        head_revision: value["headRevisionId"].as_str().map(String::from),
        app_properties: parse_properties(&value["appProperties"]),
    })
}

/// Drive sends `size` as a decimal *string* (int64 does not survive JSON
/// numbers intact), so reading it as a number gets `None` for every real
/// response. A number is still accepted, because nothing about the caller
/// changes if Drive ever sends one.
fn parse_size(value: &Value) -> Option<u64> {
    match value {
        Value::String(s) => s.parse().ok(),
        other => other.as_u64(),
    }
}

// Drive declares appProperties as string->string, but a value that is not a
// string is simply dropped rather than stringified: a caller reading one back
// expects what it wrote, and `"1"` and `1` are not the same key to it.
fn parse_properties(value: &Value) -> BTreeMap<String, String> {
    value
        .as_object()
        .map(|map| {
            map.iter()
                .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

/// The oldest match, `id` breaking an exact tie.
///
/// Drive permits same-name siblings and `files.list` guarantees no particular
/// order, so "take the first result" let two devices settle on *different*
/// folders or vault files and sync past each other forever — a split brain that
/// looks exactly like sync silently not working. Oldest-first is the one rule
/// every device can evaluate identically: whichever file the pair created
/// first is the one they both keep using, and a duplicate created later is
/// simply ignored (the merge self-heals whatever landed in it once a device
/// pulls it, and nothing is lost meanwhile because pushes are full-state).
///
/// `createdTime` is RFC 3339 UTC from Drive, a fixed-width format in which
/// lexicographic order *is* chronological order — no date parsing needed.
fn oldest(files: Vec<DriveFile>) -> Option<DriveFile> {
    files.into_iter().min_by(|a, b| {
        a.created_time
            .cmp(&b.created_time)
            .then_with(|| a.id.cmp(&b.id))
    })
}

pub async fn folder_id(client: &Client, token: &str, name: &str) -> Result<Option<String>> {
    let q = format!(
        "name = '{}' and mimeType = '{FOLDER_MIME}' and trashed = false",
        escape(name)
    );
    Ok(oldest(find_all(client, token, &q).await?).map(|f| f.id))
}

/// [`folder_id`], scoped to one parent — the same oldest-wins rule, so two
/// devices racing to create the same subfolder still settle on one.
pub async fn folder_id_in(
    client: &Client,
    token: &str,
    name: &str,
    parent: &str,
) -> Result<Option<String>> {
    let q = format!(
        "name = '{}' and mimeType = '{FOLDER_MIME}' and trashed = false and '{}' in parents",
        escape(name),
        escape(parent)
    );
    Ok(oldest(find_all(client, token, &q).await?).map(|f| f.id))
}

/// Every non-trashed file this app can see that carries `key = value` in its
/// `appProperties`, wherever it sits. Addressing by marker rather than by
/// folder is what makes a listing complete when two devices raced to create
/// the same folder and each uploaded into its own.
pub async fn find_by_app_property(
    client: &Client,
    token: &str,
    key: &str,
    value: &str,
) -> Result<Vec<DriveFile>> {
    let q = format!(
        "appProperties has {{ key='{}' and value='{}' }} and trashed = false",
        escape(key),
        escape(value)
    );
    find_all(client, token, &q).await
}

/// Everything non-trashed that sits directly inside `parent`, folders included.
///
/// For a caller that has to see the whole folder rather than one known name:
/// the sync engine's vault-id resolution and first-run onboarding both learn
/// which vaults an account holds from the file names they find in `Vaults/`.
pub async fn list_folder(client: &Client, token: &str, parent: &str) -> Result<Vec<DriveFile>> {
    let q = format!("'{}' in parents and trashed = false", escape(parent));
    find_all(client, token, &q).await
}

pub async fn find_file(
    client: &Client,
    token: &str,
    name: &str,
    parent: &str,
) -> Result<Option<DriveFile>> {
    let q = format!(
        "name = '{}' and trashed = false and '{}' in parents",
        escape(name),
        escape(parent)
    );
    Ok(oldest(find_all(client, token, &q).await?))
}

/// Download a file's content, refusing anything over `max_bytes`.
///
/// The account is the user's own, but what sits in it is not under this app's
/// control: a pack swapped for something enormous — by another client, by a
/// bug in one — must fail with an error, not be buffered whole and take the
/// process down with it.
pub async fn read_file(
    client: &Client,
    token: &str,
    id: &str,
    max_bytes: usize,
) -> Result<Vec<u8>> {
    let mut resp = client
        .get(format!("{FILES}/{id}"))
        .bearer_auth(token)
        .query(&[("alt", "media")])
        .send()
        .await
        .map_err(other)?;
    let status = resp.status();
    if !status.is_success() {
        let body = read_capped(&mut resp, max_bytes).await;
        return Err(api_error(status, body));
    }
    if resp
        .content_length()
        .is_some_and(|len| len > max_bytes as u64)
    {
        return Err(Error::FileTooLarge);
    }
    read_capped(&mut resp, max_bytes).await
}

/// A folder in the account root.
pub async fn create_folder(client: &Client, token: &str, name: &str) -> Result<String> {
    create_folder_in(client, token, name, None).await
}

/// A folder, optionally inside `parent`.
pub async fn create_folder_in(
    client: &Client,
    token: &str,
    name: &str,
    parent: Option<&str>,
) -> Result<String> {
    let mut metadata = json!({ "name": name, "mimeType": FOLDER_MIME });
    if let Some(parent) = parent {
        metadata["parents"] = json!([parent]);
    }
    let resp = client
        .post(FILES)
        .bearer_auth(token)
        .json(&metadata)
        .send()
        .await
        .map_err(other)?;
    let data = check(resp).await?;
    data["id"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| Error::Other("Drive API returned no id".into()))
}

/// multipart/related upload of a vault pack: a JSON metadata part, then the raw
/// pack bytes.
pub async fn create_file(
    client: &Client,
    token: &str,
    name: &str,
    parent: &str,
    content: &[u8],
) -> Result<DriveFile> {
    upload_new(
        client,
        token,
        name,
        parent,
        content,
        &[],
        layout::VAULT_MIME,
    )
    .await
}

/// [`create_file`] plus `appProperties` — private metadata the listing carries
/// back, so a caller can tell its files apart without downloading them.
///
/// A share, not a vault pack, so it goes up as an opaque blob: nothing but the
/// recipient's app can make sense of the bytes, and Drive should not offer to.
pub async fn create_file_with_properties(
    client: &Client,
    token: &str,
    name: &str,
    parent: &str,
    content: &[u8],
    properties: &[(&str, &str)],
) -> Result<DriveFile> {
    upload_new(client, token, name, parent, content, properties, FILE_MIME).await
}

async fn upload_new(
    client: &Client,
    token: &str,
    name: &str,
    parent: &str,
    content: &[u8],
    properties: &[(&str, &str)],
    mime: &str,
) -> Result<DriveFile> {
    let mut metadata = json!({ "name": name, "mimeType": mime, "parents": [parent] });
    if !properties.is_empty() {
        metadata["appProperties"] = properties
            .iter()
            .map(|(k, v)| ((*k).to_string(), Value::from(*v)))
            .collect::<serde_json::Map<_, _>>()
            .into();
    }

    let resp = client
        .post(UPLOAD)
        .bearer_auth(token)
        .query(&[("uploadType", "multipart"), ("fields", FILE_FIELDS)])
        .header(
            reqwest::header::CONTENT_TYPE,
            format!("multipart/related; boundary={BOUNDARY}"),
        )
        .body(multipart_body(&metadata, content, mime))
        .send()
        .await
        .map_err(other)?;
    parse_file(&check(resp).await?).ok_or_else(|| Error::Other("Drive API returned no id".into()))
}

const BOUNDARY: &str = "rowel-boundary";

/// The envelope is assembled by hand because the body is binary — it is spliced
/// in between UTF-8 boundary lines rather than formatted into a `String`.
fn multipart_body(metadata: &Value, content: &[u8], mime: &str) -> Vec<u8> {
    let head = format!(
        "--{BOUNDARY}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{metadata}\r\n\
         --{BOUNDARY}\r\nContent-Type: {mime}\r\n\r\n"
    );
    let tail = format!("\r\n--{BOUNDARY}--");

    let mut body = Vec::with_capacity(head.len() + content.len() + tail.len());
    body.extend_from_slice(head.as_bytes());
    body.extend_from_slice(content);
    body.extend_from_slice(tail.as_bytes());
    body
}

/// Grant read access to anyone holding the file's id — what turns an uploaded
/// share into a link the recipient can fetch without a Google account.
///
/// The bytes are sealed before they are uploaded, so "anyone with the link" is
/// only as open as the key the sender hands over out of band.
pub async fn share_with_anyone(client: &Client, token: &str, id: &str) -> Result<()> {
    let resp = client
        .post(format!("{FILES}/{id}/permissions"))
        .bearer_auth(token)
        .json(&json!({ "type": "anyone", "role": "reader" }))
        .send()
        .await
        .map_err(other)?;
    check(resp).await.map(|_| ())
}

/// Delete a file. Already gone counts as deleted — that is the state the caller
/// asked for, and revoke/sweep both race other devices doing the same thing.
pub async fn delete_file(client: &Client, token: &str, id: &str) -> Result<()> {
    let resp = client
        .delete(format!("{FILES}/{id}"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(other)?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(());
    }
    check(resp).await.map(|_| ())
}

/// Fetch a link-shared file with no account, refusing anything over
/// `max_bytes`.
///
/// The id comes out of a pasted link, so it can name any public Drive file at
/// all — the cap is checked against the declared length first and then
/// enforced while streaming, so a body that lies about its size still cannot
/// be buffered whole before the ciphertext is even looked at.
pub async fn download_public(
    client: &Client,
    api_key: &str,
    id: &str,
    max_bytes: usize,
) -> Result<Vec<u8>> {
    let mut resp = client
        .get(format!("{FILES}/{id}"))
        .query(&[("alt", "media"), ("key", api_key)])
        .send()
        .await
        .map_err(other)?;
    let status = resp.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        // The only two ways a share the recipient was given disappears, and
        // Drive cannot tell them apart — nor could the recipient act on the
        // difference.
        return Err(Error::ShareExpired);
    }
    if !status.is_success() {
        let body = read_capped(&mut resp, max_bytes).await;
        return Err(api_error(status, body));
    }
    if resp
        .content_length()
        .is_some_and(|len| len > max_bytes as u64)
    {
        return Err(Error::ShareTooLarge);
    }
    read_capped(&mut resp, max_bytes)
        .await
        .map_err(|e| match e {
            Error::FileTooLarge => Error::ShareTooLarge,
            e => e,
        })
}

/// Read a response body, failing with [`Error::FileTooLarge`] the moment it
/// exceeds `max_bytes` — before the excess is ever held in memory. The check
/// is on what arrives, not on `Content-Length`, so a body that lies about its
/// size (or sends none) is bounded the same. Crate-visible: every download in
/// the app has a cap, and this is the one loop that enforces one.
pub(crate) async fn read_capped(resp: &mut reqwest::Response, max_bytes: usize) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(other)? {
        if body.len() + chunk.len() > max_bytes {
            return Err(Error::FileTooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// One file by id, or `None` when Drive says there is no such file.
///
/// This reads the object itself rather than searching for it, so unlike
/// [`find_by_app_property`] it sees a file the query index has not caught up
/// with yet — the difference between "no such share" and "a share uploaded
/// seconds ago". A 404 is the only absence there is: every other failure stays
/// a failure, because a caller must not read a network error as "already gone".
pub async fn get_file(client: &Client, token: &str, id: &str) -> Result<Option<DriveFile>> {
    let resp = client
        .get(format!("{FILES}/{id}"))
        .bearer_auth(token)
        .query(&[("fields", FILE_FIELDS)])
        .send()
        .await
        .map_err(other)?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    Ok(parse_file(&check(resp).await?))
}

/// Overwrite a vault pack's content, returning its new head revision.
pub async fn update_file(
    client: &Client,
    token: &str,
    id: &str,
    content: &[u8],
) -> Result<Option<String>> {
    let resp = client
        .patch(format!("{UPLOAD}/{id}"))
        .bearer_auth(token)
        .query(&[("uploadType", "media"), ("fields", FILE_FIELDS)])
        .header(reqwest::header::CONTENT_TYPE, layout::VAULT_MIME)
        .body(content.to_vec())
        .send()
        .await
        .map_err(other)?;
    Ok(check(resp)
        .await?
        .get("headRevisionId")
        .and_then(Value::as_str)
        .map(String::from))
}

#[cfg(test)]
mod tests {
    use super::{
        api_error, collect_pages, escape, layout, list_query, multipart_body, oldest, parse_file,
        parse_listing, parse_properties, parse_size, DriveFile, Error, FILE_FIELDS, LIST_FIELDS,
    };
    use serde_json::json;
    use std::cell::RefCell;

    #[test]
    fn escapes_quotes_and_backslashes() {
        assert_eq!(escape("Rowel"), "Rowel");
        assert_eq!(escape("a'b"), "a\\'b");
        assert_eq!(escape("a\\b"), "a\\\\b");
        // A name crafted to break out of the literal stays contained.
        assert_eq!(escape("x' or '1'='1"), "x\\' or \\'1\\'=\\'1");
    }

    fn file(id: &str, created: &str) -> DriveFile {
        DriveFile {
            id: id.into(),
            name: String::new(),
            created_time: created.into(),
            modified_time: String::new(),
            size: None,
            head_revision: None,
            app_properties: Default::default(),
        }
    }

    #[test]
    fn oldest_wins_whatever_order_drive_lists_in() {
        let old = file("old", "2023-01-01T00:00:00.000Z");
        let new = file("new", "2024-06-01T00:00:00.000Z");

        assert_eq!(oldest(vec![new.clone(), old.clone()]), Some(old.clone()));
        assert_eq!(oldest(vec![old.clone(), new]), Some(old));
    }

    #[test]
    fn an_exact_created_time_tie_is_broken_by_id() {
        let a = file("aaa", "2024-01-01T00:00:00.000Z");
        let b = file("bbb", "2024-01-01T00:00:00.000Z");
        assert_eq!(oldest(vec![b.clone(), a.clone()]), Some(a.clone()));
        assert_eq!(oldest(vec![a, b]).unwrap().id, "aaa");
    }

    #[test]
    fn a_listing_page_yields_its_files_and_the_next_token() {
        let (files, next) = parse_listing(&serde_json::json!({
            "nextPageToken": "page-2",
            "files": [{"id": "a", "createdTime": "2024-01-01T00:00:00.000Z"}]
        }));
        assert_eq!(files.len(), 1);
        assert_eq!(next.as_deref(), Some("page-2"));

        let (files, next) = parse_listing(&serde_json::json!({"files": []}));
        assert!(files.is_empty());
        assert_eq!(next, None);

        // An empty token is Drive's way of saying "last page" too.
        let (_, next) = parse_listing(&serde_json::json!({"nextPageToken": "", "files": []}));
        assert_eq!(next, None);
    }

    #[test]
    fn a_listing_asks_drive_for_the_next_page_token() {
        // The field mask filters the whole response. A mask that leaves the
        // token out gets no token back, and the loop below ends on page one
        // believing it saw everything.
        let first = list_query("q", None);
        let (_, fields) = first.iter().find(|(key, _)| *key == "fields").unwrap();
        assert!(fields
            .split(',')
            .map(str::trim)
            .any(|f| f == "nextPageToken"));
        assert!(!first.iter().any(|(key, _)| *key == "pageToken"));

        let later = list_query("q", Some("page-2"));
        assert!(later.contains(&("pageToken", "page-2")));
    }

    #[tokio::test]
    async fn a_listing_follows_page_tokens_to_the_last_page() {
        let asked = RefCell::new(Vec::new());
        let files = collect_pages(|token| {
            asked.borrow_mut().push(token.clone());
            async move {
                Ok(match token.as_deref() {
                    None => json!({
                        "nextPageToken": "page-2",
                        "files": [{"id": "a"}]
                    }),
                    Some("page-2") => json!({ "files": [{"id": "b"}] }),
                    Some(other) => panic!("asked for a page Drive never named: {other}"),
                })
            }
        })
        .await
        .unwrap();

        let ids: Vec<&str> = files.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(ids, ["a", "b"]);
        // Sent exactly what each page handed back, and stopped when nothing was.
        assert_eq!(*asked.borrow(), vec![None, Some("page-2".to_string())]);
    }

    #[test]
    fn no_matches_selects_nothing() {
        assert_eq!(oldest(vec![]), None);
    }

    #[test]
    fn a_listed_file_carries_its_name_and_properties() {
        let parsed = parse_file(&json!({
            "id": "f1",
            "name": "share-1.rowelshare",
            "createdTime": "2024-01-01T00:00:00.000Z",
            "headRevisionId": "r1",
            "appProperties": { "kind": "login", "expiresAt": "1700000000000" },
        }))
        .unwrap();

        assert_eq!(parsed.name, "share-1.rowelshare");
        assert_eq!(parsed.app_properties["kind"], "login");
        assert_eq!(parsed.app_properties["expiresAt"], "1700000000000");
        assert_eq!(parsed.head_revision.as_deref(), Some("r1"));
    }

    // The sync engine selects neither field, so every existing caller still
    // parses — it just reads them as empty.
    #[test]
    fn a_file_without_name_or_properties_still_parses() {
        let parsed = parse_file(&json!({ "id": "f1" })).unwrap();
        assert_eq!(parsed.name, "");
        assert!(parsed.app_properties.is_empty());
    }

    #[test]
    fn non_string_property_values_are_dropped() {
        let props = parse_properties(&json!({ "kind": "login", "count": 3, "on": true }));
        assert_eq!(props.len(), 1);
        assert_eq!(props["kind"], "login");
    }

    #[test]
    fn no_properties_object_is_no_properties() {
        assert!(parse_properties(&json!(null)).is_empty());
        assert!(parse_properties(&json!("nonsense")).is_empty());
    }

    // What onboarding shows about a remote vault before downloading it. Drive
    // sends the length as a string, which is the whole reason `parse_size`
    // exists — reading it as a JSON number would report every pack as unsized.
    #[test]
    fn a_listed_file_carries_its_size_and_modified_time() {
        let parsed = parse_file(&json!({
            "id": "f1",
            "name": "a1b2c3.rowel",
            "modifiedTime": "2024-05-04T10:11:12.000Z",
            "size": "2097152",
        }))
        .unwrap();

        assert_eq!(parsed.size, Some(2_097_152));
        assert_eq!(parsed.modified_time, "2024-05-04T10:11:12.000Z");
    }

    #[test]
    fn a_file_drive_reports_no_size_for_has_none() {
        // Folders and Google-native files simply have no `size`, and a garbled
        // one is no better than a missing one.
        assert_eq!(parse_size(&json!(null)), None);
        assert_eq!(parse_size(&json!("not a number")), None);
        // A plain number still reads, in case Drive ever sends one.
        assert_eq!(parse_size(&json!(17)), Some(17));
    }

    // Both masks filter the whole response, so a field left out of them is
    // absent from the parse no matter what the file actually has.
    #[test]
    fn both_field_masks_ask_for_size_and_modified_time() {
        for mask in [LIST_FIELDS, FILE_FIELDS] {
            assert!(mask.contains("size"), "{mask}");
            assert!(mask.contains("modifiedTime"), "{mask}");
        }
    }

    // Binary content must survive the envelope byte for byte — the share is
    // ciphertext, and a lossy round trip through `String` would corrupt it.
    #[test]
    fn the_upload_envelope_splices_raw_bytes_between_the_boundaries() {
        let content = [0x00u8, 0xff, 0x1a, b'\r', b'\n'];
        let body = multipart_body(&json!({ "name": "x" }), &content, layout::VAULT_MIME);

        let text = String::from_utf8_lossy(&body);
        assert!(text.starts_with("--rowel-boundary\r\n"));
        assert!(text.contains("{\"name\":\"x\"}"));
        assert!(text.contains(layout::VAULT_MIME));
        assert!(body.ends_with(b"\r\n--rowel-boundary--"));
        assert!(body.windows(content.len()).any(|w| w == content));
    }

    // A failed response whose body cannot be read is the case worth keeping: it
    // used to report `Drive API 500: ` — a status and nothing else to go on.
    #[test]
    fn an_unreadable_error_body_says_so_instead_of_reading_as_empty() {
        let status = reqwest::StatusCode::INTERNAL_SERVER_ERROR;

        let read = api_error(status, Ok(b"quota exceeded".to_vec()));
        assert_eq!(
            read.to_string(),
            "Drive API 500 Internal Server Error: quota exceeded"
        );

        let unread = api_error(status, Err(Error::FileTooLarge)).to_string();
        assert!(unread.starts_with("Drive API 500 Internal Server Error: <could not read body:"));
        assert!(unread.contains(&Error::FileTooLarge.to_string()));
    }
}
