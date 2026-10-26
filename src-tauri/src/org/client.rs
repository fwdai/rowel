//! The Rowel Server's member API, as this app speaks it.
//!
//! The server is the OAuth2 authorization server the app signs in against —
//! authorization code with PKCE, the public client [`CLIENT_ID`], a loopback
//! redirect — and federates to the company's identity provider on its own
//! side; the app never sees that provider. Every other call is a bearer call
//! under the session that sign-in produced.
//!
//! The structs here are the wire contract the server implements, kept in one
//! place so the server's own tests can serialize what this parses
//! (`org::fake_server`). camelCase throughout, as the broker protocol is; the
//! one exception is the token endpoint, whose reply is RFC 6749's and keeps
//! that RFC's snake_case so an off-the-shelf OAuth client could read it too.
//!
//! ```text
//! GET  /v1/members/auth/authorize?response_type=code&client_id=rowel-app
//!        &redirect_uri&state&code_challenge&code_challenge_method=S256
//! POST /v1/members/auth/token      form: grant_type, code, code_verifier,
//!                                  redirect_uri | refresh_token; client_id
//!                                  → {access_token, refresh_token, expires_in, token_type}
//! GET  /v1/members/me              → {member:{id,email,name}, tenant:{name}}
//! POST /v1/members/enroll          {devicePubkey, deviceName} → {deviceId, state}
//! GET  /v1/members/me/devices      → [{id,name,state,publicKey,createdAtMs}]
//! POST /v1/members/devices/{id}/approve
//! GET  /v1/members/me/lockboxes    → [{id,name,kdf,wrappedMaster,etag}]
//! GET  /v1/members/me/approvals/stream   SSE: `event: approval` + `data: <Approval>`
//! POST /v1/approvals/{id}          {answer: GrantAnswer | null}
//! ```

use reqwest::{Response, StatusCode};
use serde::{Deserialize, Serialize};

use super::connections::normalize_url;
use crate::error::{Error, Result};
use crate::sync::{http_client, read_capped};

/// The public OAuth client every install of the app is: a public client has
/// no secret to keep, so PKCE is what binds the code to the app that asked.
pub(crate) const CLIENT_ID: &str = "rowel-app";

/// The most of a JSON reply (a listing, an error body) that is read. A member
/// has at most a handful of devices and lockboxes; anything past this is not
/// a Rowel Server answering.
const MAX_JSON_BYTES: usize = 1024 * 1024;

/// How much of the approvals stream may be buffered waiting for a frame to
/// end. An approval is a few hundred bytes; a server that never ends a frame
/// is not one worth staying connected to.
#[cfg(desktop)]
const MAX_SSE_BUFFER_BYTES: usize = 256 * 1024;

/// What a 401 is reported as: the one failure a member call has that the
/// caller acts on without the user — a refresh, then the call again
/// (`SessionResolver::as_member`) — so it has one spelling to be known by.
pub(crate) const SESSION_OVER: &str =
    "the Rowel Server no longer accepts this sign-in; sign in again";

/// Whether `error` is the session ending ([`SESSION_OVER`]).
pub(crate) fn session_over(error: &Error) -> bool {
    matches!(error, Error::Other(why) if why == SESSION_OVER)
}

fn other<E: std::fmt::Display>(e: E) -> Error {
    Error::Other(e.to_string())
}

/// A session as the token endpoint issues one (RFC 6749 §5.1).
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Session {
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// Seconds from issue until `access_token` stops working.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_in: Option<i64>,
    pub token_type: String,
}

// Presence, never the value, as `Tokens` prints: both are bearer credentials.
impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("access_token", &"<redacted>")
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "<redacted>"),
            )
            .field("expires_in", &self.expires_in)
            .field("token_type", &self.token_type)
            .finish()
    }
}

/// Who the session belongs to, and whose server it is. The sign-in that
/// asks is desktop's (`org::connect`), so a phone has no use for it yet.
#[cfg(any(desktop, test))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Me {
    pub member: Member,
    pub tenant: Tenant,
}

#[cfg(any(desktop, test))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Member {
    pub id: String,
    pub email: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[cfg(any(desktop, test))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Tenant {
    pub name: String,
}

/// Where an enrolled device stands with the server. Only an approved device
/// is handed wrapped masters; a revoked one never is again.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum DeviceState {
    Pending,
    Approved,
    Revoked,
}

/// What an enrollment comes back with: the device's id at the server, and
/// its standing — pending, until another device of the member or an admin
/// approves it.
#[cfg(any(desktop, test))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Enrolled {
    pub device_id: String,
    pub state: DeviceState,
}

/// One of the member's devices, as the server lists them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Device {
    pub id: String,
    pub name: String,
    pub state: DeviceState,
    /// The device's enrollment public key, base64: what the member compares
    /// fingerprints of before approving (`EnrollmentKey::fingerprint`).
    pub public_key: String,
    pub created_at_ms: i64,
}

/// One org lockbox the member may hold.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Lockbox {
    /// The lockbox's id at the server, which is also its vault id on every
    /// device: the pack's name in the account is made from it.
    pub id: String,
    pub name: String,
    /// The `KdfParams::External` descriptor, as the JSON string the sidecar
    /// and every pack header carry verbatim.
    pub kdf: String,
    /// The lockbox's master, sealed to this device's enrollment key
    /// (`crypto_box` sealed box, base64). `None` until the device is approved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wrapped_master: Option<String>,
    /// The current revision of the lockbox's pack, or `None` while no device
    /// has pushed one: the listing carries it so the sync adapter's listing
    /// needs no `HEAD` per lockbox.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,
}

/// A grant an agent asked for at the server, waiting on this member's
/// answer: what the consent dialog shows, in the shapes the app's own grant
/// asks already carry (`events::AgentGrantAsk`), so the one dialog draws
/// both. Desktop only, like the broker crate whose `VisibleItem` names the
/// candidates.
#[cfg(desktop)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Approval {
    pub id: String,
    pub lockbox_id: String,
    pub lockbox_name: String,
    pub agent_name: String,
    pub mode: rowel_broker::grant::Mode,
    /// Where the grant would let the item be used, as the dialog names a
    /// place: the site, the request pattern, the command line.
    pub targets: Vec<crate::events::AskTarget>,
    pub candidates: Vec<rowel_broker::grant::VisibleItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Whether the rule that admitted the ask wants the master password too.
    pub step_up: bool,
    pub expires_at_ms: i64,
}

/// One Rowel Server, reached as one member.
pub(crate) struct ServerClient {
    base_url: String,
    /// The bearer every member call carries; `None` before sign-in, when the
    /// only calls that make sense are the two that produce one.
    session: Option<String>,
    client: reqwest::Client,
}

impl ServerClient {
    /// A client for the server at `url`, signed out: for the sign-in itself
    /// and for a refresh, neither of which carries a bearer.
    pub(crate) fn new(url: &str) -> Result<Self> {
        Ok(Self {
            base_url: normalize_url(url)?,
            session: None,
            client: http_client(),
        })
    }

    /// A client for the server at `url` acting as the member whose
    /// `access_token` this is.
    pub(crate) fn with_session(url: &str, access_token: &str) -> Result<Self> {
        Ok(Self {
            session: Some(access_token.to_string()),
            ..Self::new(url)?
        })
    }

    /// The server, as [`normalize_url`] spells it. The app keeps that
    /// spelling in the connection record and never asks the client for it
    /// back; the tests build their records from a client and do.
    #[cfg(test)]
    pub(crate) fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Where to send the browser: the server's authorization page for this
    /// app, redirecting to `redirect_uri` with `state` and a code bound to
    /// the PKCE verifier behind `code_challenge` (`sync::challenge`).
    #[cfg(any(desktop, test))]
    pub(crate) fn authorize_url(
        &self,
        redirect_uri: &str,
        state: &str,
        code_challenge: &str,
    ) -> String {
        let mut url = url::Url::parse(&format!("{}/v1/members/auth/authorize", self.base_url))
            .expect("a normalized server URL with a fixed path parses");
        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", CLIENT_ID)
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("state", state)
            .append_pair("code_challenge", code_challenge)
            .append_pair("code_challenge_method", "S256");
        url.into()
    }

    /// Redeem the `code` the redirect carried, proving the `verifier` whose
    /// challenge the authorization request sent.
    #[cfg(any(desktop, test))]
    pub(crate) async fn token(
        &self,
        code: &str,
        verifier: &str,
        redirect_uri: &str,
    ) -> Result<Session> {
        self.post_token(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("code_verifier", verifier),
            ("redirect_uri", redirect_uri),
        ])
        .await
    }

    /// A fresh session from a refresh token. The server may rotate the
    /// refresh token; the caller keeps whichever the reply names.
    pub(crate) async fn refresh(&self, refresh_token: &str) -> Result<Session> {
        self.post_token(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
        ])
        .await
    }

    async fn post_token(&self, form: &[(&str, &str)]) -> Result<Session> {
        let mut form = form.to_vec();
        form.push(("client_id", CLIENT_ID));
        let resp = self
            .client
            .post(self.url("/v1/members/auth/token"))
            .form(&form)
            .send()
            .await
            .map_err(other)?;
        json(resp).await
    }

    #[cfg(any(desktop, test))]
    pub(crate) async fn me(&self) -> Result<Me> {
        json(self.get("/v1/members/me")?.send().await.map_err(other)?).await
    }

    /// Enroll this device under `device_name` with its enrollment public key
    /// (`EnrollmentKey::public_key_b64`). The device comes back pending.
    #[cfg(any(desktop, test))]
    pub(crate) async fn enroll(
        &self,
        device_pubkey_b64: &str,
        device_name: &str,
    ) -> Result<Enrolled> {
        let body = serde_json::json!({
            "devicePubkey": device_pubkey_b64,
            "deviceName": device_name,
        });
        json(
            self.post("/v1/members/enroll")?
                .json(&body)
                .send()
                .await
                .map_err(other)?,
        )
        .await
    }

    pub(crate) async fn devices(&self) -> Result<Vec<Device>> {
        json(
            self.get("/v1/members/me/devices")?
                .send()
                .await
                .map_err(other)?,
        )
        .await
    }

    /// Approve device `id` of the same member, from a device that is itself
    /// approved: the server refuses it from a pending one.
    pub(crate) async fn approve_device(&self, id: &str) -> Result<()> {
        let resp = self
            .post(&format!("/v1/members/devices/{id}/approve"))?
            .send()
            .await
            .map_err(other)?;
        checked(resp).await.map(|_| ())
    }

    pub(crate) async fn lockboxes(&self) -> Result<Vec<Lockbox>> {
        json(
            self.get("/v1/members/me/lockboxes")?
                .send()
                .await
                .map_err(other)?,
        )
        .await
    }

    /// Follow the member's pending approvals as the server announces them,
    /// handing every frame to `on_frame` until it answers `false` or the
    /// server ends the stream. A keepalive comment, or an event of a kind a
    /// later server may add, is handed over as `None`: it carries nothing,
    /// but it is the moment a caller that wants to stop between approvals
    /// gets to say so, which is why a server has to send one at least every
    /// 30 s — the client's read timeout is twice that, and a stream that
    /// goes quiet for longer is taken for dead and reconnected.
    #[cfg(desktop)]
    pub(crate) async fn approvals_stream(
        &self,
        mut on_frame: impl FnMut(Option<Approval>) -> bool,
    ) -> Result<()> {
        let resp = self
            .get("/v1/members/me/approvals/stream")?
            .header(reqwest::header::ACCEPT, "text/event-stream")
            .send()
            .await
            .map_err(other)?;
        let mut resp = checked(resp).await?;
        let mut framer = Framer::default();
        while let Some(chunk) = resp.chunk().await.map_err(other)? {
            for frame in framer.feed(&chunk)? {
                if !on_frame(approval_in(&frame)?) {
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    /// Answer approval `approval_id`: the item picked and for how long, or
    /// `None` to refuse.
    #[cfg(desktop)]
    pub(crate) async fn answer(
        &self,
        approval_id: &str,
        answer: Option<rowel_broker::agents::GrantAnswer>,
    ) -> Result<()> {
        let resp = self
            .post(&format!("/v1/approvals/{approval_id}"))?
            .json(&serde_json::json!({ "answer": answer }))
            .send()
            .await
            .map_err(other)?;
        checked(resp).await.map(|_| ())
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    fn bearer(&self) -> Result<&str> {
        self.session.as_deref().ok_or_else(|| {
            Error::Other(format!(
                "not signed in to the Rowel Server at {}",
                self.base_url
            ))
        })
    }

    fn get(&self, path: &str) -> Result<reqwest::RequestBuilder> {
        Ok(self.client.get(self.url(path)).bearer_auth(self.bearer()?))
    }

    fn post(&self, path: &str) -> Result<reqwest::RequestBuilder> {
        Ok(self.client.post(self.url(path)).bearer_auth(self.bearer()?))
    }
}

/// The response, or the error its status is. A 401 says the session is over,
/// which is the one failure the user acts on by signing in again; everything
/// else carries the server's own words, capped.
pub(crate) async fn checked(mut resp: Response) -> Result<Response> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    if status == StatusCode::UNAUTHORIZED {
        return Err(Error::Other(SESSION_OVER.into()));
    }
    let detail = match read_capped(&mut resp, MAX_JSON_BYTES).await {
        Ok(body) => String::from_utf8_lossy(&body).into_owned(),
        Err(e) => format!("<could not read body: {e}>"),
    };
    Err(Error::Other(format!("Rowel Server {status}: {detail}")))
}

async fn json<T: serde::de::DeserializeOwned>(resp: Response) -> Result<T> {
    let mut resp = checked(resp).await?;
    let body = read_capped(&mut resp, MAX_JSON_BYTES).await?;
    serde_json::from_slice(&body).map_err(other)
}

/// A server-sent event stream cut into frames as the bytes arrive, by the
/// specification's own rules: a line ends at `\r\n`, `\n` or a lone `\r`,
/// and a frame at an empty line. A `\r` ends its line at once, whatever
/// follows: waiting for the next byte to see whether it is a `\r\n` would
/// leave a CR-only server's last frame unfinished, since nothing comes after
/// the `\r` that ends it. A `\n` straight after a `\r` is then the second half
/// of that one line end, even when it opens the next chunk, and is skipped.
/// Comment lines are kept in the frame (the reader over a frame skips them),
/// so a keepalive is a frame of nothing but comments and reaches the caller
/// as one, which is what lets a caller stop between approvals.
#[cfg(desktop)]
#[derive(Default)]
struct Framer {
    /// Bytes past the last complete line.
    buffer: Vec<u8>,
    /// The lines of the frame under way.
    lines: Vec<String>,
    /// Whether the last line end read was a `\r`, so a `\n` opening the next
    /// chunk belongs to it.
    after_cr: bool,
}

#[cfg(desktop)]
impl Framer {
    /// The frames `chunk` completes, in order. What is held between calls —
    /// the frame under way and the line under way — is bounded by
    /// [`MAX_SSE_BUFFER_BYTES`].
    fn feed(&mut self, chunk: &[u8]) -> Result<Vec<Vec<String>>> {
        self.buffer.extend_from_slice(chunk);
        let held = self.buffer.len() + self.lines.iter().map(String::len).sum::<usize>();
        if held > MAX_SSE_BUFFER_BYTES {
            return Err(Error::Other(
                "the Rowel Server's approvals stream sent a frame too large to read".into(),
            ));
        }
        let mut frames = Vec::new();
        let mut start = 0;
        let mut at = 0;
        while at < self.buffer.len() {
            let byte = self.buffer[at];
            if byte == b'\n' && std::mem::take(&mut self.after_cr) && at == start {
                at += 1;
                start = at;
                continue;
            }
            self.after_cr = false;
            if byte != b'\n' && byte != b'\r' {
                at += 1;
                continue;
            }
            let line = String::from_utf8_lossy(&self.buffer[start..at]).into_owned();
            at += 1;
            self.after_cr = byte == b'\r';
            start = at;
            if line.is_empty() {
                if !self.lines.is_empty() {
                    frames.push(std::mem::take(&mut self.lines));
                }
            } else {
                self.lines.push(line);
            }
        }
        self.buffer.drain(..start);
        Ok(frames)
    }
}

/// The approval in one frame's lines, or `None` for a keepalive comment or
/// an event of another kind. `data:` lines are joined with newlines, as the
/// SSE specification has it.
#[cfg(desktop)]
fn approval_in(frame: &[String]) -> Result<Option<Approval>> {
    let mut event = None;
    let mut data = Vec::new();
    for line in frame {
        if line.starts_with(':') {
            continue;
        }
        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "event" => event = Some(value.to_string()),
            "data" => data.push(value.to_string()),
            _ => {}
        }
    }
    if event.as_deref() != Some("approval") || data.is_empty() {
        return Ok(None);
    }
    serde_json::from_str(&data.join("\n"))
        .map(Some)
        .map_err(|e| {
            Error::Other(format!(
                "the Rowel Server sent an approval this build cannot read: {e}"
            ))
        })
}

#[cfg(all(test, desktop))]
mod tests {
    use super::*;

    fn approval_json() -> String {
        serde_json::to_string(&Approval {
            id: "ap1".into(),
            lockbox_id: "lb1".into(),
            lockbox_name: "Deploy".into(),
            agent_name: "release-bot".into(),
            mode: rowel_broker::grant::Mode::Env,
            targets: vec![crate::events::AskTarget::Command {
                line: "stripe balance retrieve".into(),
            }],
            candidates: vec![rowel_broker::grant::VisibleItem {
                id: "i1".into(),
                title: "Stripe".into(),
                username: None,
            }],
            reason: Some("deploying 1.2".into()),
            step_up: false,
            expires_at_ms: 1_700_000_000_000,
        })
        .unwrap()
    }

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(String::from).collect()
    }

    #[test]
    fn an_approval_frame_is_read_and_everything_else_is_skipped() {
        let json = approval_json();
        let approval = approval_in(&lines(&format!("event: approval\ndata: {json}")))
            .unwrap()
            .unwrap();
        assert_eq!(approval.id, "ap1");
        assert_eq!(approval.candidates[0].title, "Stripe");
        // Data without the optional space, and a comment inside the frame.
        assert!(
            approval_in(&lines(&format!(": note\nevent:approval\ndata:{json}")))
                .unwrap()
                .is_some()
        );
        assert!(approval_in(&lines(": keepalive")).unwrap().is_none());
        assert!(
            approval_in(&lines(&format!("event: revoked\ndata: {json}")))
                .unwrap()
                .is_none()
        );
        assert!(approval_in(&lines("event: approval")).unwrap().is_none());
        assert!(approval_in(&lines("event: approval\ndata: {not json")).is_err());
    }

    // Every line ending the specification allows, arriving in chunks cut
    // anywhere — a `\r` on a chunk's edge included — frames the same.
    #[test]
    fn frames_are_found_whatever_the_line_ending_and_however_the_chunks_fall() {
        let expected = vec![
            vec![": keepalive".to_string()],
            vec!["event: approval".to_string(), "data: {\"a\":1}".to_string()],
            vec!["data: x".to_string(), "data: y".to_string()],
        ];
        for ending in ["\r\n", "\n", "\r"] {
            let text = format!(
                ": keepalive{e}{e}event: approval{e}data: {{\"a\":1}}{e}{e}data: x{e}data: y{e}{e}",
                e = ending
            );
            let bytes = text.as_bytes();
            // Whole, one byte at a time, and in a few odd pieces.
            let mut by_byte = Framer::default();
            let mut per_byte = Vec::new();
            for byte in bytes {
                per_byte.extend(by_byte.feed(std::slice::from_ref(byte)).unwrap());
            }
            assert_eq!(per_byte, expected, "{ending:?} byte by byte");
            assert_eq!(
                Framer::default().feed(bytes).unwrap(),
                expected,
                "{ending:?} whole"
            );
            let mut pieces = Framer::default();
            let mut framed = Vec::new();
            for piece in bytes.chunks(7) {
                framed.extend(pieces.feed(piece).unwrap());
            }
            assert_eq!(framed, expected, "{ending:?} in pieces");
        }

        // A `\r` ending a chunk ends its line at once; a `\n` opening the next
        // chunk is the rest of that `\r\n`, not a blank line, and a frame
        // ends at the `\r` that ends its blank line rather than waiting on a
        // byte that may never come.
        let mut framer = Framer::default();
        assert!(framer.feed(b"data: a\r").unwrap().is_empty());
        assert_eq!(
            framer.feed(b"\n\r").unwrap(),
            vec![vec!["data: a".to_string()]]
        );
        assert!(framer.feed(b"\n").unwrap().is_empty(), "the rest of \\r\\n");
        assert_eq!(
            framer.feed(b"data: c\n\n").unwrap(),
            vec![vec!["data: c".to_string()]]
        );
        let mut framer = Framer::default();
        assert!(framer.feed(b"data: a\r").unwrap().is_empty());
        assert_eq!(
            framer.feed(b"data: b\r\r").unwrap(),
            vec![vec!["data: a".to_string(), "data: b".to_string()]]
        );
        // Extra blank lines between frames are nothing, and a frame left
        // unfinished at the end stays unfinished.
        let mut framer = Framer::default();
        assert!(framer.feed(b"\n\n\ndata: tail").unwrap().is_empty());
        assert!(framer.feed(b"\n").unwrap().is_empty());
        // Past the cap, the stream is given up on rather than buffered on.
        let mut framer = Framer::default();
        assert!(framer.feed(&vec![b'x'; MAX_SSE_BUFFER_BYTES + 1]).is_err());
    }

    // The authorization request names everything RFC 7636 asks for, encoded,
    // so the server can bind the code to this app's verifier.
    #[test]
    fn the_authorize_url_carries_the_pkce_parameters() {
        let client = ServerClient::new("https://rowel.example.com/").unwrap();
        let url = client.authorize_url("http://127.0.0.1:4242/cb", "st&ate", "ch4ll");
        let parsed = url::Url::parse(&url).unwrap();
        assert_eq!(parsed.path(), "/v1/members/auth/authorize");
        let pairs: Vec<(String, String)> = parsed
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        assert!(pairs.contains(&("response_type".into(), "code".into())));
        assert!(pairs.contains(&("client_id".into(), CLIENT_ID.into())));
        assert!(pairs.contains(&("redirect_uri".into(), "http://127.0.0.1:4242/cb".into())));
        assert!(pairs.contains(&("state".into(), "st&ate".into())));
        assert!(pairs.contains(&("code_challenge".into(), "ch4ll".into())));
        assert!(pairs.contains(&("code_challenge_method".into(), "S256".into())));
    }
}
