//! Dropbox v2 REST surface, App folder access. All calls are authorized with a
//! bearer access token; every path is relative to the app's own folder
//! (`/Apps/<app name>/` in the user's Dropbox), so the vaults live at
//! `/Vaults` and the app can see nothing outside it.
//!
//! Two hosts, two calling conventions:
//! - RPC endpoints (`api.dropboxapi.com`) take their argument as a JSON body
//!   and answer with JSON.
//! - Content endpoints (`content.dropboxapi.com`) take their argument as JSON
//!   in the `Dropbox-API-Arg` header, the file bytes as the body, and (for a
//!   download) hand the metadata back in the `Dropbox-API-Result` header.
//!
//! Bodies are **bytes**, never strings: a `.rowel` pack is SQLCipher ciphertext
//! with a binary header.
//!
//! Errors: a route-specific failure is a 409 whose JSON body carries an
//! `error_summary` such as `path/not_found/..`. That one summary is the only
//! "absence" there is; every other failure stays a failure, because a caller
//! must not read a network error as "no such file".

use std::future::Future;

use reqwest::header::{HeaderMap, CONTENT_TYPE, RETRY_AFTER};
use reqwest::{Client, StatusCode};
use serde_json::{json, Value};

use crate::error::{Error, Result};
use crate::sync::layout;

const API: &str = "https://api.dropboxapi.com/2";
const CONTENT: &str = "https://content.dropboxapi.com/2";
const API_ARG: &str = "Dropbox-API-Arg";
const API_RESULT: &str = "Dropbox-API-Result";

/// The most of a Dropbox JSON response (metadata, a listing page, an error) we
/// will hold. A full `list_folder` page is tens of KiB; this is only here so
/// no response body is ever buffered unbounded (see [`read_capped`]).
const MAX_JSON_BYTES: usize = 4 * 1024 * 1024;

/// The largest body a single `files/upload` call accepts: 150 MiB ("Do not
/// use this to upload a file larger than 150 MiB", files/upload docs; the
/// stone spec says "150 MB", so the binary reading is the safe one). Anything bigger
/// needs an upload session (`upload_session/start`, `append_v2`, `finish`),
/// which is a follow-up; until then such a pack is refused before a byte of it
/// is sent, rather than failing half way through on Dropbox's side.
pub const MAX_UPLOAD_BYTES: usize = 150 * 1024 * 1024;

/// One Dropbox file or folder, with the fields listing, probing and race
/// detection need.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropboxEntry {
    /// `id:...`, stable across renames and moves, case-sensitive. Accepted in
    /// `path` by every call in this module (per `files.stone`).
    pub id: String,
    /// The name as the user (or this app) spelled it.
    pub name: String,
    /// The lowercased full path inside the app folder. Dropbox paths are
    /// case-insensitive, so this is the one to compare.
    pub path_lower: String,
    pub is_folder: bool,
    /// Byte length. Files only.
    pub size: Option<u64>,
    /// The file's revision; changes on every write. Files only. `None` is
    /// "unknown", never a made-up value.
    pub rev: Option<String>,
    /// Last content change on the server, RFC 3339 UTC (`2024-05-04T10:11:12Z`).
    /// Empty for folders, which have none.
    pub server_modified: String,
}

/// The vaults folder: `/Vaults`, relative to the app folder.
pub fn vaults_path() -> String {
    format!("/{}", layout::VAULTS_FOLDER)
}

/// `name` inside the vaults folder: `/Vaults/<name>`.
pub fn vault_entry_path(name: &str) -> String {
    format!("{}/{name}", vaults_path())
}

fn other<E: std::fmt::Display>(e: E) -> Error {
    Error::Other(e.to_string())
}

// --- errors ---

/// Whether an `error_summary` says the path does not exist.
///
/// The summary is the error union's tags joined by `/`, then a `/` and a
/// random number of `.` characters (`path/not_found/..`,
/// `path/not_found/....`; see the error-handling docs), so this matches the
/// tag prefix, never the whole string. `list_folder`, `get_metadata` and
/// `download` report the lookup under `path`; `delete_v2` under
/// `path_lookup`.
fn is_not_found(summary: &str) -> bool {
    ["path/not_found", "path_lookup/not_found"]
        .iter()
        .any(|tag| tag_prefix(summary, tag))
}

// `summary` starts with `tag` as whole `/`-separated segments, so
// `path/not_found_ish` is not `path/not_found`.
fn tag_prefix(summary: &str, tag: &str) -> bool {
    summary
        .strip_prefix(tag)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

/// The `error_summary` of an error body, if it is Dropbox's JSON error shape.
fn error_summary(body: &[u8]) -> Option<String> {
    serde_json::from_slice::<Value>(body)
        .ok()?
        .get("error_summary")?
        .as_str()
        .map(String::from)
}

/// The error for a non-success response. The detail is the `error_summary`
/// when there is one (it is the useful, stable part), else whatever of the body
/// could be read, else why it could not: collapsing an unreadable body into an
/// empty string leaves `Dropbox API 500: ` and nothing to diagnose from. A 429
/// names its `Retry-After`, so a failed run says when trying again is useful.
fn api_error(status: StatusCode, retry_after: Option<&str>, body: &Result<Vec<u8>>) -> Error {
    let detail = match body {
        Ok(body) => {
            error_summary(body).unwrap_or_else(|| String::from_utf8_lossy(body).into_owned())
        }
        Err(e) => format!("<could not read body: {e}>"),
    };
    let mut message = format!("Dropbox API {status}: {detail}");
    if status == StatusCode::TOO_MANY_REQUESTS {
        match retry_after {
            Some(secs) => message.push_str(&format!(" (rate limited; retry after {secs}s)")),
            None => message.push_str(" (rate limited)"),
        }
    }
    Error::Other(message)
}

/// A non-success response, read (capped) so it can be classified.
struct Failure {
    status: StatusCode,
    retry_after: Option<String>,
    body: Result<Vec<u8>>,
}

impl Failure {
    async fn read(mut resp: reqwest::Response) -> Self {
        let status = resp.status();
        let retry_after = header_str(resp.headers(), RETRY_AFTER.as_str());
        let body = read_capped(&mut resp, MAX_JSON_BYTES).await;
        Self {
            status,
            retry_after,
            body,
        }
    }

    fn is_not_found(&self) -> bool {
        self.status == StatusCode::CONFLICT
            && self
                .body
                .as_ref()
                .ok()
                .and_then(|b| error_summary(b))
                .is_some_and(|s| is_not_found(&s))
    }

    fn into_error(self) -> Error {
        api_error(self.status, self.retry_after.as_deref(), &self.body)
    }
}

fn header_str(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(String::from)
}

async fn json_body(mut resp: reqwest::Response) -> Result<Value> {
    let body = read_capped(&mut resp, MAX_JSON_BYTES).await?;
    if body.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_slice(&body).map_err(other)
}

/// One RPC call. `Ok(None)` is a 409 `not_found`, the one failure every caller
/// here reads as absence; anything else non-2xx is an error.
async fn rpc(client: &Client, token: &str, endpoint: &str, arg: &Value) -> Result<Option<Value>> {
    let resp = client
        .post(format!("{API}/{endpoint}"))
        .bearer_auth(token)
        .json(arg)
        .send()
        .await
        .map_err(other)?;
    if resp.status().is_success() {
        return json_body(resp).await.map(Some);
    }
    let failure = Failure::read(resp).await;
    if failure.is_not_found() {
        return Ok(None);
    }
    Err(failure.into_error())
}

// --- parsing ---

/// One entry of a listing or a metadata response. `None` for anything that is
/// not a live file or folder: a `deleted` tombstone (only sent when asked
/// for), or an object with no id.
fn parse_entry(value: &Value) -> Option<DropboxEntry> {
    let is_folder = match value[".tag"].as_str()? {
        "file" => false,
        "folder" => true,
        _ => return None,
    };
    Some(DropboxEntry {
        id: value["id"].as_str()?.to_string(),
        name: value["name"].as_str().unwrap_or("").to_string(),
        path_lower: value["path_lower"].as_str().unwrap_or("").to_string(),
        is_folder,
        size: value["size"].as_u64(),
        rev: value["rev"].as_str().map(String::from),
        server_modified: value["server_modified"].as_str().unwrap_or("").to_string(),
    })
}

/// One page of `list_folder` / `list_folder/continue`: its entries, and the
/// cursor to continue from when Dropbox says there is more.
fn parse_listing(data: &Value) -> (Vec<DropboxEntry>, Option<String>) {
    let entries = data["entries"]
        .as_array()
        .map(|entries| entries.iter().filter_map(parse_entry).collect())
        .unwrap_or_default();
    let next = data["has_more"]
        .as_bool()
        .unwrap_or(false)
        .then(|| data["cursor"].as_str().filter(|c| !c.is_empty()))
        .flatten()
        .map(String::from);
    (entries, next)
}

/// The `rev` out of a download's `Dropbox-API-Result` header (the file's
/// metadata as JSON). `None` when the header is missing, garbled or carries
/// no rev: unknown, not fabricated.
fn rev_from_result_header(header: &str) -> Option<String> {
    serde_json::from_str::<Value>(header)
        .ok()?
        .get("rev")?
        .as_str()
        .filter(|r| !r.is_empty())
        .map(String::from)
}

/// `arg` serialized for the `Dropbox-API-Arg` header. Header values must be
/// ASCII, so Dropbox requires every character from U+007F up to be written as
/// a JSON `\uXXXX` escape (UTF-16, so a character outside the BMP becomes a
/// surrogate pair). serde_json leaves them raw; this escapes them. Outside of
/// strings JSON is pure ASCII, so escaping char by char is always valid.
fn header_json(arg: &Value) -> String {
    let mut out = String::new();
    for c in arg.to_string().chars() {
        if (c as u32) < 0x7f {
            out.push(c);
        } else {
            let mut units = [0u16; 2];
            for unit in c.encode_utf16(&mut units) {
                out.push_str(&format!("\\u{unit:04x}"));
            }
        }
    }
    out
}

// --- listing ---

// Follow a listing to its last page. Dropbox pages whenever it likes, so one
// response is never the whole answer. `fetch` is given `None` for the first
// request and the previous page's cursor after that; it returns `None` when
// the folder does not exist, which is an empty listing. Keeping the transport
// out here is what lets the loop be tested end to end.
async fn collect_pages<F, Fut>(mut fetch: F) -> Result<Vec<DropboxEntry>>
where
    F: FnMut(Option<String>) -> Fut,
    Fut: Future<Output = Result<Option<Value>>>,
{
    let mut entries = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        // Gone between pages counts as gone: a half listing of a folder that
        // no longer exists is not more true than an empty one.
        let Some(page) = fetch(cursor).await? else {
            return Ok(Vec::new());
        };
        let (page, next) = parse_listing(&page);
        entries.extend(page);
        match next {
            Some(next) => cursor = Some(next),
            None => return Ok(entries),
        }
    }
}

/// Everything directly inside `path` (non-recursive), folders included,
/// following `list_folder/continue` to the last page. A folder that does not
/// exist lists as empty. The app folder's own root is `""`, not `"/"`.
pub async fn list_folder(client: &Client, token: &str, path: &str) -> Result<Vec<DropboxEntry>> {
    collect_pages(|cursor| async move {
        match cursor {
            None => rpc(client, token, "files/list_folder", &json!({ "path": path })).await,
            Some(cursor) => {
                rpc(
                    client,
                    token,
                    "files/list_folder/continue",
                    &json!({ "cursor": cursor }),
                )
                .await
            }
        }
    })
    .await
}

/// One entry by path or `id:...`, or `None` when there is no such thing.
/// Always asks the server; never a cache.
pub async fn get_metadata(
    client: &Client,
    token: &str,
    path: &str,
) -> Result<Option<DropboxEntry>> {
    Ok(rpc(
        client,
        token,
        "files/get_metadata",
        &json!({ "path": path }),
    )
    .await?
    .as_ref()
    .and_then(parse_entry))
}

// --- content ---

/// Download a file by path or `id:...`, refusing anything over `max_bytes`,
/// and return its bytes with the `rev` they are.
///
/// What sits in the account is not under this app's control: a pack swapped
/// for something enormous must fail with [`Error::FileTooLarge`], not be
/// buffered whole. The declared length is checked first and the cap is then
/// enforced while streaming, so a body that lies about its size is bounded the
/// same.
pub async fn download(
    client: &Client,
    token: &str,
    path_or_id: &str,
    max_bytes: usize,
) -> Result<(Vec<u8>, Option<String>)> {
    let mut resp = client
        .post(format!("{CONTENT}/files/download"))
        .bearer_auth(token)
        .header(API_ARG, header_json(&json!({ "path": path_or_id })))
        .send()
        .await
        .map_err(other)?;
    if !resp.status().is_success() {
        return Err(Failure::read(resp).await.into_error());
    }
    if resp
        .content_length()
        .is_some_and(|len| len > max_bytes as u64)
    {
        return Err(Error::FileTooLarge);
    }
    let rev = header_str(resp.headers(), API_RESULT)
        .as_deref()
        .and_then(rev_from_result_header);
    let bytes = read_capped(&mut resp, max_bytes).await?;
    Ok((bytes, rev))
}

/// Refuse an upload a single `files/upload` call cannot carry.
fn check_upload_size(len: usize) -> Result<()> {
    if len > MAX_UPLOAD_BYTES {
        return Err(Error::Other(format!(
            "Dropbox upload of {len} bytes is over the {} MiB single-upload limit",
            MAX_UPLOAD_BYTES / (1024 * 1024)
        )));
    }
    Ok(())
}

/// Create or overwrite the file at `path`, returning its new `rev`. Parent
/// folders are created by Dropbox as needed. `mute` keeps the desktop client
/// from notifying the user about every sync push.
///
/// Single-call upload only, so bodies over [`MAX_UPLOAD_BYTES`] are refused
/// before sending. TODO(follow-up): upload sessions for larger packs.
pub async fn upload(
    client: &Client,
    token: &str,
    path: &str,
    bytes: &[u8],
) -> Result<Option<String>> {
    check_upload_size(bytes.len())?;
    let arg = json!({ "path": path, "mode": "overwrite", "mute": true });
    let resp = client
        .post(format!("{CONTENT}/files/upload"))
        .bearer_auth(token)
        .header(API_ARG, header_json(&arg))
        .header(CONTENT_TYPE, "application/octet-stream")
        .body(bytes.to_vec())
        .send()
        .await
        .map_err(other)?;
    if !resp.status().is_success() {
        return Err(Failure::read(resp).await.into_error());
    }
    Ok(json_body(resp)
        .await?
        .get("rev")
        .and_then(Value::as_str)
        .map(String::from))
}

/// Delete a file or folder by path or `id:...`. Already gone counts as
/// deleted: that is the state the caller asked for, and another device may be
/// doing the same thing.
pub async fn delete(client: &Client, token: &str, path: &str) -> Result<()> {
    rpc(client, token, "files/delete_v2", &json!({ "path": path }))
        .await
        .map(|_| ())
}

// --- the capped read ---

/// Append `chunk`, failing with [`Error::FileTooLarge`] instead if that would
/// take `body` past `max_bytes`, so the excess is never held in memory.
fn append_capped(body: &mut Vec<u8>, chunk: &[u8], max_bytes: usize) -> Result<()> {
    if body.len() + chunk.len() > max_bytes {
        return Err(Error::FileTooLarge);
    }
    body.extend_from_slice(chunk);
    Ok(())
}

/// Read a response body, failing the moment it exceeds `max_bytes`. The check
/// is on what arrives, not on `Content-Length`, so a body that lies about its
/// size (or sends none) is bounded the same. Same loop as the Drive client's.
async fn read_capped(resp: &mut reqwest::Response, max_bytes: usize) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(other)? {
        append_capped(&mut body, &chunk, max_bytes)?;
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn the_vaults_folder_sits_at_the_app_folder_root() {
        assert_eq!(vaults_path(), "/Vaults");
        assert_eq!(vault_entry_path("a1b2.rowel"), "/Vaults/a1b2.rowel");
    }

    #[test]
    fn a_file_entry_carries_size_rev_and_modified_time() {
        let entry = parse_entry(&json!({
            ".tag": "file",
            "id": "id:a4ayc_80_OEAAAAAAAAAXw",
            "name": "A1B2.rowel",
            "path_lower": "/vaults/a1b2.rowel",
            "path_display": "/Vaults/A1B2.rowel",
            "rev": "a1c10ce0dd78",
            "size": 2_097_152u64,
            "server_modified": "2024-05-04T10:11:12Z",
            "client_modified": "2024-05-04T10:11:10Z",
            "content_hash": "e3b0c442",
        }))
        .unwrap();

        assert_eq!(
            entry,
            DropboxEntry {
                id: "id:a4ayc_80_OEAAAAAAAAAXw".into(),
                name: "A1B2.rowel".into(),
                path_lower: "/vaults/a1b2.rowel".into(),
                is_folder: false,
                size: Some(2_097_152),
                rev: Some("a1c10ce0dd78".into()),
                server_modified: "2024-05-04T10:11:12Z".into(),
            }
        );
    }

    #[test]
    fn a_folder_entry_has_no_size_rev_or_modified_time() {
        let entry = parse_entry(&json!({
            ".tag": "folder",
            "id": "id:folder1",
            "name": "Vaults",
            "path_lower": "/vaults",
        }))
        .unwrap();

        assert!(entry.is_folder);
        assert_eq!(entry.size, None);
        assert_eq!(entry.rev, None);
        assert_eq!(entry.server_modified, "");
    }

    #[test]
    fn deleted_and_malformed_entries_are_skipped() {
        assert_eq!(
            parse_entry(&json!({ ".tag": "deleted", "name": "x", "path_lower": "/x" })),
            None
        );
        assert_eq!(
            parse_entry(&json!({ ".tag": "file", "name": "no id" })),
            None
        );
        assert_eq!(parse_entry(&json!({ "id": "id:untagged" })), None);
    }

    #[test]
    fn a_listing_page_yields_its_entries_and_the_cursor_only_while_has_more() {
        let (entries, next) = parse_listing(&json!({
            "entries": [
                { ".tag": "file", "id": "id:a", "name": "a.rowel" },
                { ".tag": "deleted", "name": "gone.rowel" },
            ],
            "cursor": "c1",
            "has_more": true,
        }));
        assert_eq!(entries.len(), 1);
        assert_eq!(next.as_deref(), Some("c1"));

        // Every page carries a cursor (it is how `longpoll` works), so the
        // cursor alone is not "there is more".
        let (_, next) = parse_listing(&json!({ "entries": [], "cursor": "c2", "has_more": false }));
        assert_eq!(next, None);

        // `has_more` with no usable cursor cannot be followed.
        let (_, next) = parse_listing(&json!({ "entries": [], "cursor": "", "has_more": true }));
        assert_eq!(next, None);
    }

    #[tokio::test]
    async fn a_listing_follows_the_cursor_to_the_last_page() {
        let asked = RefCell::new(Vec::new());
        let entries = collect_pages(|cursor| {
            asked.borrow_mut().push(cursor.clone());
            async move {
                Ok(Some(match cursor.as_deref() {
                    None => json!({
                        "entries": [{ ".tag": "file", "id": "id:a" }],
                        "cursor": "c1",
                        "has_more": true,
                    }),
                    Some("c1") => json!({
                        "entries": [{ ".tag": "file", "id": "id:b" }],
                        "cursor": "c2",
                        "has_more": false,
                    }),
                    Some(other) => panic!("asked for a cursor Dropbox never handed out: {other}"),
                }))
            }
        })
        .await
        .unwrap();

        let ids: Vec<&str> = entries.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["id:a", "id:b"]);
        assert_eq!(*asked.borrow(), vec![None, Some("c1".to_string())]);
    }

    #[tokio::test]
    async fn a_missing_folder_lists_as_empty() {
        let entries = collect_pages(|_| async { Ok(None) }).await.unwrap();
        assert!(entries.is_empty());
    }

    #[tokio::test]
    async fn a_failed_page_fails_the_listing() {
        let result = collect_pages(|cursor| async move {
            match cursor {
                None => Ok(Some(json!({
                    "entries": [{ ".tag": "file", "id": "id:a" }],
                    "cursor": "c1",
                    "has_more": true,
                }))),
                Some(_) => Err(Error::Other("Dropbox API 500".into())),
            }
        })
        .await;
        assert!(
            result.is_err(),
            "a partial listing must not pass as complete"
        );
    }

    #[test]
    fn not_found_is_read_from_the_error_summary_prefix() {
        // list_folder, get_metadata, download.
        assert!(is_not_found("path/not_found/.."));
        // The trailing dots are random in number.
        assert!(is_not_found("path/not_found/."));
        assert!(is_not_found("path/not_found/......"));
        // delete_v2 reports its lookup under `path_lookup`.
        assert!(is_not_found("path_lookup/not_found/.."));
        assert!(is_not_found("path_lookup/not_found/...."));
        assert!(is_not_found("path/not_found"));

        assert!(!is_not_found("path/not_folder/.."));
        assert!(!is_not_found("path/malformed_path/.."));
        assert!(!is_not_found("path_write/conflict/file/.."));
        assert!(!is_not_found("path/not_found_extra/.."));
        assert!(!is_not_found("other/path/not_found/.."));
        assert!(!is_not_found(""));
    }

    #[test]
    fn the_error_summary_comes_out_of_dropboxs_error_body() {
        let body = br#"{"error_summary": "path/not_found/..", "error": {".tag": "path", "path": {".tag": "not_found"}}}"#;
        assert_eq!(error_summary(body).as_deref(), Some("path/not_found/.."));
        assert_eq!(error_summary(b"Error in call to API function"), None);
        assert_eq!(error_summary(b"{}"), None);
    }

    #[test]
    fn an_api_error_names_the_status_and_the_summary_or_body() {
        let conflict = api_error(
            StatusCode::CONFLICT,
            None,
            &Ok(br#"{"error_summary": "path/not_folder/.."}"#.to_vec()),
        );
        assert_eq!(
            conflict.to_string(),
            "Dropbox API 409 Conflict: path/not_folder/.."
        );

        let bad = api_error(
            StatusCode::BAD_REQUEST,
            None,
            &Ok(b"Error in call to API function \"files/upload\"".to_vec()),
        );
        assert_eq!(
            bad.to_string(),
            "Dropbox API 400 Bad Request: Error in call to API function \"files/upload\""
        );

        let unread = api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            None,
            &Err(Error::FileTooLarge),
        )
        .to_string();
        assert!(unread.starts_with("Dropbox API 500 Internal Server Error: <could not read body:"));
    }

    #[test]
    fn a_rate_limit_names_its_retry_after() {
        let body = Ok(br#"{"error_summary": "too_many_requests/.."}"#.to_vec());
        let with = api_error(StatusCode::TOO_MANY_REQUESTS, Some("300"), &body).to_string();
        assert_eq!(
            with,
            "Dropbox API 429 Too Many Requests: too_many_requests/.. (rate limited; retry after 300s)"
        );

        let without = api_error(StatusCode::TOO_MANY_REQUESTS, None, &body).to_string();
        assert!(without.ends_with("(rate limited)"), "{without}");
    }

    #[test]
    fn the_rev_comes_out_of_the_api_result_header() {
        let header =
            r#"{"name": "a1b2.rowel", "id": "id:x", "rev": "015f9d2a3c1e000000001", "size": 12}"#;
        assert_eq!(
            rev_from_result_header(header).as_deref(),
            Some("015f9d2a3c1e000000001")
        );
        assert_eq!(rev_from_result_header(r#"{"name": "a"}"#), None);
        assert_eq!(rev_from_result_header(r#"{"rev": ""}"#), None);
        assert_eq!(rev_from_result_header("not json"), None);
    }

    #[test]
    fn the_api_arg_header_escapes_everything_from_del_up() {
        let arg = json!({ "path": "/Vaults/caf\u{e9} \u{1f512}\u{7f}.rowel" });
        let header = header_json(&arg);
        assert!(header.is_ascii(), "{header}");
        // U+00E9 is one unit; U+1F512 is the surrogate pair D83D DD12.
        let expected = [
            "{\"path\":\"/Vaults/caf",
            "\\u00e9",
            " ",
            "\\ud83d\\udd12",
            "\\u007f",
            ".rowel\"}",
        ]
        .concat();
        assert_eq!(header, expected);
        // Still the same JSON once parsed back.
        assert_eq!(serde_json::from_str::<Value>(&header).unwrap(), arg);
    }

    #[test]
    fn uploads_over_150_mib_are_refused_before_sending() {
        assert_eq!(MAX_UPLOAD_BYTES, 150 * 1024 * 1024);
        assert!(check_upload_size(0).is_ok());
        assert!(check_upload_size(MAX_UPLOAD_BYTES).is_ok());

        let err = check_upload_size(MAX_UPLOAD_BYTES + 1).unwrap_err();
        assert!(matches!(err, Error::Other(_)));
        assert!(err.to_string().contains("150 MiB"), "{err}");
    }

    #[test]
    fn the_capped_read_stops_before_holding_the_excess() {
        let mut body = Vec::new();
        append_capped(&mut body, b"abcd", 6).unwrap();
        append_capped(&mut body, b"ef", 6).unwrap();
        assert_eq!(body, b"abcdef");

        // One byte over: refused, and nothing of the chunk is kept.
        assert!(matches!(
            append_capped(&mut body, b"g", 6),
            Err(Error::FileTooLarge)
        ));
        assert_eq!(body, b"abcdef");

        // A single oversized chunk into an empty body is refused too.
        let mut empty = Vec::new();
        assert!(matches!(
            append_capped(&mut empty, &[0u8; 7], 6),
            Err(Error::FileTooLarge)
        ));
        assert!(empty.is_empty());
    }
}
