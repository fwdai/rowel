//! `rowel proxy`: `header` mode (`spec/05` §4). An agent calls an API through
//! a local address with a placeholder where the key would go and the grant's
//! id in `X-Rowel-Grant`; the proxy resolves the placeholder under that grant
//! for that host, path and method, swaps it into the outgoing request, sends
//! the request over TLS, and redacts the value from what comes back. The
//! agent's process, transcript and environment never contain the key.
//!
//! Addressing is a path prefix, `/https/<host>/<path>`, because every SDK
//! takes a base URL and that is all the agent needs; a `CONNECT` forward
//! proxy with a local CA is not offered (Q15). The proxy listens on the
//! loopback address only: it holds the sidecar key, and must not be
//! reachable from the network. Placeholders are swapped in headers and in a
//! JSON or form body, never in the URL, which lands in logs upstream. A
//! request with none is refused, never forwarded: the resolve is what checks
//! the grant covers the host, path and method, so the proxy carries only
//! what a live grant covers, and is no open forwarder for whatever names a
//! grant id.
//!
//! The proxy follows no redirect. The grant was checked for the host, path
//! and method the agent asked for; a hop the upstream chose was not, and a
//! secret header or body replayed to it could land anywhere. A 3xx is
//! relayed with its `Location` addressed through the proxy
//! (`/https/<host>/<path>`), so the agent's client, following it or not,
//! comes back here for the next hop as a request of its own with its own
//! grant check; a `Location` that is not an HTTP URL is dropped rather than
//! handed over as a hop that would leave the proxy. Where a value lands decides how it is written: as
//! is in a header, JSON-escaped in a JSON document, form-encoded in a form —
//! so a value with a quote or an ampersand cannot restructure what carries it.

use std::collections::HashMap;
use std::convert::Infallible;
use std::io;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use bytes::Bytes;
use http::header::{
    HeaderMap, HeaderName, HeaderValue, ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_LENGTH,
    CONTENT_TYPE, HOST, LOCATION,
};
use http::{Method, Request, Response, StatusCode};
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use percent_encoding::percent_decode_str;
use rowel_broker::grant::{ResolveContext, ResolveTarget};
use rowel_broker::protocol::Code;
use rowel_broker::reference::{Reference, SCHEME};
use serde_json::json;
use tokio::net::TcpListener;
use zeroize::Zeroizing;

use crate::agent::{AgentError, Broker};
use crate::authority::authority;
use crate::redact::{form_encoded, json_escaped, Redactor};

pub const DEFAULT_LISTEN: &str = "127.0.0.1:7400";
/// The header that names the grant a request runs under.
pub const GRANT_HEADER: &str = "x-rowel-grant";
/// The most a request body may be: it is buffered whole to be scanned.
pub const MAX_BODY: usize = 32 * 1024 * 1024;
/// Requests per minute per grant: a bucket of this many, refilled one a
/// second, so a looping agent cannot hammer an upstream with a real key.
pub const RATE_PER_MINUTE: u32 = 60;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProxyArgs {
    pub listen: SocketAddr,
    /// Whether `/http/<host>/…` is served: a dev server on localhost. Off,
    /// because a key sent in the clear is a key given away.
    pub allow_http: bool,
}

impl Default for ProxyArgs {
    fn default() -> Self {
        Self {
            listen: DEFAULT_LISTEN.parse().expect("a valid default"),
            allow_http: false,
        }
    }
}

/// `text` as the address to listen on: a socket address on a loopback
/// interface. Anything else is refused: the proxy holds the sidecar key.
pub fn listen_address(text: &str) -> Result<SocketAddr, String> {
    let address: SocketAddr = text
        .parse()
        .map_err(|_| format!("--listen takes an address like {DEFAULT_LISTEN}, not '{text}'"))?;
    if !address.ip().is_loopback() {
        return Err(format!(
            "--listen must be a loopback address (127.0.0.1 or ::1), not {}: the proxy holds a key and must not be reachable from the network",
            address.ip()
        ));
    }
    Ok(address)
}

/// Serve until the process ends.
pub fn run(args: &ProxyArgs, broker: Box<dyn Broker + Send>) -> i32 {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("rowel proxy: {error}");
            return 1;
        }
    };
    let outcome = runtime.block_on(async {
        let state = State::new(broker, args.allow_http)?;
        let listener = TcpListener::bind(args.listen).await?;
        eprintln!(
            "rowel proxy: listening on http://{} (call /https/<host>/<path> with X-Rowel-Grant)",
            listener.local_addr()?
        );
        serve(listener, Arc::new(state)).await
    });
    match outcome {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("rowel proxy: {error}");
            1
        }
    }
}

/// What every request handler shares: the sidecar (sealed calls are one at
/// a time anyway), the upstream client, and the buckets.
pub struct State {
    broker: Mutex<Box<dyn Broker + Send>>,
    upstream: reqwest::Client,
    allow_http: bool,
    buckets: Mutex<HashMap<String, Bucket>>,
}

impl State {
    /// The TLS provider is installed here, once, as the app's `sync` installs
    /// it: reqwest is built without one so the app and the CLI link the same
    /// ring backend, and a client cannot be built before it is there. The
    /// client follows no redirect (see the module doc).
    pub fn new(broker: Box<dyn Broker + Send>, allow_http: bool) -> io::Result<Self> {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let upstream = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| io::Error::other(e.without_url().to_string()))?;
        Ok(Self {
            broker: Mutex::new(broker),
            upstream,
            allow_http,
            buckets: Mutex::new(HashMap::new()),
        })
    }
}

/// Accept connections on `listener` and answer each request through
/// [`handle`], one task per connection.
pub async fn serve(listener: TcpListener, state: Arc<State>) -> io::Result<()> {
    loop {
        let (stream, _) = listener.accept().await?;
        let state = state.clone();
        tokio::spawn(async move {
            let service = service_fn(move |request| handle(request, state.clone()));
            let _ = http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service)
                .await;
        });
    }
}

/// One request: relayed, or refused with a JSON body that names what was
/// wrong and never a value. One line on stderr either way — method, host
/// and path (never the query, a header or a body), and the status.
async fn handle(
    request: Request<Incoming>,
    state: Arc<State>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let method = request.method().clone();
    let shown = request.uri().path().to_string();
    let response = match relay(request, &state).await {
        Ok(response) => response,
        Err(refusal) => refusal.into_response(),
    };
    eprintln!("rowel proxy: {method} {shown} -> {}", response.status());
    Ok(response)
}

/// Why a request was not relayed: the status to answer with, a short word
/// and a sentence. Never a value.
#[derive(Debug)]
struct Refusal {
    status: StatusCode,
    error: String,
    message: String,
    retry_after_secs: Option<u64>,
}

impl Refusal {
    fn new(status: StatusCode, error: &str, message: impl Into<String>) -> Self {
        Self {
            status,
            error: error.to_string(),
            message: message.into(),
            retry_after_secs: None,
        }
    }

    fn into_response(self) -> Response<Full<Bytes>> {
        let body = json!({ "error": self.error, "message": self.message }).to_string();
        let mut response = Response::builder()
            .status(self.status)
            .header(CONTENT_TYPE, "application/json");
        if let Some(secs) = self.retry_after_secs {
            response = response.header("retry-after", secs.to_string());
        }
        response
            .body(Full::new(Bytes::from(body)))
            .expect("a well-formed response")
    }
}

/// The upstream a request addresses: scheme, host (with port as written),
/// path and query as the URL to call, and the host (by the one rule,
/// `authority`) and path the grant is checked against.
#[derive(Debug)]
struct Upstream {
    url: String,
    host: String,
    path: String,
}

/// `/https/<host>/<path>?<query>` read into the upstream URL. A placeholder
/// anywhere in it, encoded or not, is refused: never in URLs.
fn upstream_of(uri: &http::Uri, allow_http: bool) -> Result<Upstream, Refusal> {
    let bad = |message: &str| Refusal::new(StatusCode::BAD_REQUEST, "bad_request", message);
    let path = uri.path();
    let query = uri.query();
    let whole = format!("{path}?{}", query.unwrap_or_default());
    if carries_placeholder(&whole) {
        return Err(bad(
            "a placeholder in the URL is never sent: put it in a header or the body",
        ));
    }
    let (scheme, rest) = if let Some(rest) = path.strip_prefix("/https/") {
        ("https", rest)
    } else if let Some(rest) = path.strip_prefix("/http/") {
        if !allow_http {
            return Err(bad("/http/<host>/… is served only with --allow-http"));
        }
        ("http", rest)
    } else {
        return Err(bad("the path must be /https/<host>/<path>"));
    };
    let (authority, tail) = match rest.split_once('/') {
        Some((authority, tail)) => (authority, format!("/{tail}")),
        None => (rest, "/".to_string()),
    };
    if authority.is_empty()
        || authority.contains(['@', '\\', '?', '#'])
        || authority.contains(char::is_whitespace)
    {
        return Err(bad("the path must be /https/<host>/<path>"));
    }
    let mut url = format!("{scheme}://{authority}{tail}");
    if let Some(query) = query {
        url.push('?');
        url.push_str(query);
    }
    // The URL as it will be sent: parsing collapses dot segments (`..`,
    // `%2e%2e`), and the grant is checked against that path, never the one
    // written — or `/v1/public/%2e%2e/charges` would pass a grant for
    // `/v1/public/*` and reach `/v1/charges`.
    let url =
        reqwest::Url::parse(&url).map_err(|_| bad("the path must be /https/<host>/<path>"))?;
    Ok(Upstream {
        host: self::authority(scheme, authority),
        path: url.path().to_string(),
        url: url.into(),
    })
}

/// Whether `text` holds a placeholder, as written or percent-encoded.
fn carries_placeholder(text: &str) -> bool {
    text.contains(SCHEME)
        || percent_decode_str(text)
            .decode_utf8_lossy()
            .contains(SCHEME)
}

/// Resolve what the request carries and send it on, then bring the answer
/// back with the released values redacted.
async fn relay(
    request: Request<Incoming>,
    state: &Arc<State>,
) -> Result<Response<Full<Bytes>>, Refusal> {
    let (parts, body) = request.into_parts();
    let upstream = upstream_of(&parts.uri, state.allow_http)?;
    let grant = parts
        .headers
        .get(GRANT_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| {
            Refusal::new(
                StatusCode::FORBIDDEN,
                "no_grant",
                "the X-Rowel-Grant header must name the grant this request runs under",
            )
        })?
        .to_string();
    let counted = take_token(state, &grant, false)?;

    let body = Limited::new(body, MAX_BODY)
        .collect()
        .await
        .map_err(|_| {
            Refusal::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                "body_too_large",
                format!("the body must be at most {MAX_BODY} bytes"),
            )
        })?
        .to_bytes();

    // What the request names: every distinct placeholder, in headers and —
    // for a JSON or form body — in the body, each resolved once.
    let body_placement = placement_of(&parts.headers);
    let body_text = body_placement.and_then(|_| std::str::from_utf8(&body).ok());
    let mut wanted: Vec<Found> = Vec::new();
    for (name, value) in &parts.headers {
        if name.as_str() == GRANT_HEADER {
            continue;
        }
        if let Ok(text) = value.to_str() {
            wanted.extend(placeholders_in(text));
        }
    }
    if let Some(text) = body_text {
        wanted.extend(placeholders_in(text));
    }
    let references: Vec<Reference> = wanted.iter().fold(Vec::new(), |mut all, found| {
        if !all.contains(&found.reference) {
            all.push(found.reference.clone());
        }
        all
    });
    // A request with nothing to put in is refused, not forwarded: what is
    // checked against the grant is a resolve, so a request that resolves
    // nothing would go anywhere under any id at all. The agent can call such
    // an API itself; the proxy is for putting a credential in.
    if references.is_empty() {
        return Err(Refusal::new(
            StatusCode::BAD_REQUEST,
            "no_placeholder",
            "the request carries no rowel:// placeholder: the proxy forwards only a request it puts a value in, so call this API directly",
        ));
    }
    let released = resolve_all(state, &grant, &upstream, &parts.method, references).await?;
    if !counted {
        take_token(state, &grant, true)?;
    }
    let values: Vec<(String, Zeroizing<String>)> = released
        .iter()
        .map(|(reference, value)| (reference.to_string(), value.clone()))
        .collect();

    // The outgoing request: the method, the headers but ours and the
    // hop-by-hop ones, the values in place; reqwest sets Host and the length.
    // The answer is asked for unencoded: the redactor reads the bytes as they
    // come, and a gzipped echo of the value would pass it and be inflated by
    // the agent's own client.
    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT_ENCODING, HeaderValue::from_static("identity"));
    for (name, value) in &parts.headers {
        if name.as_str() == GRANT_HEADER
            || is_hop_by_hop(name)
            || *name == HOST
            || *name == CONTENT_LENGTH
            || *name == ACCEPT_ENCODING
        {
            continue;
        }
        let value = match value.to_str() {
            Ok(text) if text.contains(SCHEME) => {
                HeaderValue::from_str(&replaced(text, &values, Placement::Header)).map_err(
                    |_| {
                        Refusal::new(
                            StatusCode::BAD_REQUEST,
                            "bad_header",
                            format!("the value for {name} cannot be sent as a header"),
                        )
                    },
                )?
            }
            _ => value.clone(),
        };
        headers.append(name.clone(), value);
    }
    let body = match (body_text, body_placement) {
        (Some(text), Some(placement)) if !values.is_empty() => {
            Bytes::from(replaced(text, &values, placement))
        }
        _ => body,
    };

    let response = state
        .upstream
        .request(parts.method.clone(), &upstream.url)
        .headers(headers)
        .body(body)
        .send()
        .await
        .map_err(bad_gateway)?;
    let status = response.status();
    let upstream_headers = response.headers().clone();
    if upstream_headers
        .get_all(CONTENT_ENCODING)
        .iter()
        .any(|coding| !coding.as_bytes().eq_ignore_ascii_case(b"identity"))
    {
        return Err(Refusal::new(
            StatusCode::BAD_GATEWAY,
            "encoded_response",
            "the upstream answered with an encoded body, which cannot be redacted",
        ));
    }
    let upstream_body = response.bytes().await.map_err(bad_gateway)?;

    // The answer, with every value this request released redacted from the
    // headers and the body; the length is the redacted body's.
    let mut redactor = Redactor::new(
        values
            .into_iter()
            .map(|(placeholder, value)| (value, placeholder))
            .collect(),
    );
    let mut reply = Response::builder().status(status);
    let reply_headers = reply.headers_mut().expect("a fresh builder");
    for (name, value) in &upstream_headers {
        if is_hop_by_hop(name) || *name == CONTENT_LENGTH {
            continue;
        }
        // Redacted as the upstream wrote it, before a `Location` is
        // re-addressed: rewriting re-encodes some characters, and a value in
        // that form is one the redactor would not know.
        let mut value = redacted(&mut redactor, value.as_bytes());
        if *name == LOCATION {
            match proxied_location(&upstream.url, &value) {
                Some(location) => value = location.into_bytes(),
                None => continue,
            }
        }
        if let Ok(value) = HeaderValue::from_bytes(&value) {
            reply_headers.append(name.clone(), value);
        }
    }
    let body = redacted(&mut redactor, &upstream_body);
    Ok(reply
        .body(Full::new(Bytes::from(body)))
        .expect("a well-formed response"))
}

/// A `Location` the upstream answered, addressed through this proxy:
/// resolved against `base` (the URL the request went to), an `http` or
/// `https` target becomes `/<scheme>/<host>[:<port>]<path>[?<query>]`, a path
/// on the proxy itself, so the client's next hop — followed by its own client
/// or rebuilt by hand — comes back here as a request of its own, with its own
/// grant check and the grant header still on the proxy's origin. Anything
/// else — another scheme, a value that is not a URL — is `None`, and the
/// header is dropped rather than handed over as a hop that would leave the
/// proxy. Credentials and a fragment in the URL are not carried.
fn proxied_location(base: &str, location: &[u8]) -> Option<String> {
    let target = reqwest::Url::parse(base)
        .ok()?
        .join(std::str::from_utf8(location).ok()?)
        .ok()?;
    let scheme = target.scheme();
    if !matches!(scheme, "http" | "https") {
        return None;
    }
    let mut proxied = format!("/{scheme}/{}", target.host_str()?);
    if let Some(port) = target.port() {
        proxied.push_str(&format!(":{port}"));
    }
    proxied.push_str(target.path());
    if let Some(query) = target.query() {
        proxied.push('?');
        proxied.push_str(query);
    }
    Some(proxied)
}

fn bad_gateway(error: reqwest::Error) -> Refusal {
    // reqwest's text would name the URL, which is the agent's own anyway;
    // the error without it is enough and never a value.
    Refusal::new(
        StatusCode::BAD_GATEWAY,
        "upstream",
        format!("the upstream could not be reached: {}", error.without_url()),
    )
}

/// One token from the grant's bucket, or 429 saying when one is back, and
/// whether one was taken. A bucket is made only for a grant id the host has
/// `accepted` — one a placeholder just resolved under — so an id the caller
/// made up never takes a place in the map. Before the resolve, a grant with
/// no bucket yet takes nothing, and its first token is taken once the
/// resolve has proved it real; a grant that has one is held to it before
/// anything is resolved under it again.
fn take_token(state: &State, grant: &str, accepted: bool) -> Result<bool, Refusal> {
    let mut buckets = state.buckets.lock().unwrap_or_else(|e| e.into_inner());
    let bucket = match buckets.get_mut(grant) {
        Some(bucket) => bucket,
        None if accepted => buckets
            .entry(grant.to_string())
            .or_insert_with(Bucket::full),
        None => return Ok(false),
    };
    match bucket.take(Instant::now()) {
        Ok(()) => Ok(true),
        Err(secs) => Err(Refusal {
            retry_after_secs: Some(secs),
            ..Refusal::new(
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                format!("at most {RATE_PER_MINUTE} requests a minute under one grant"),
            )
        }),
    }
}

/// A token bucket: [`RATE_PER_MINUTE`] deep, refilled one a second.
struct Bucket {
    tokens: f64,
    at: Instant,
}

impl Bucket {
    fn full() -> Self {
        Self {
            tokens: RATE_PER_MINUTE as f64,
            at: Instant::now(),
        }
    }

    /// Take one, or say in how many whole seconds one will be there.
    fn take(&mut self, now: Instant) -> Result<(), u64> {
        let refill = now.saturating_duration_since(self.at).as_secs_f64();
        self.tokens = (self.tokens + refill).min(RATE_PER_MINUTE as f64);
        self.at = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            Ok(())
        } else {
            Err((1.0 - self.tokens).ceil().max(1.0) as u64)
        }
    }
}

/// Resolve every reference under the grant at this request, on a blocking
/// thread: the sealed round-trip to the app must not sit on the runtime.
async fn resolve_all(
    state: &Arc<State>,
    grant: &str,
    upstream: &Upstream,
    method: &Method,
    references: Vec<Reference>,
) -> Result<Vec<(Reference, Zeroizing<String>)>, Refusal> {
    let ctx = ResolveContext {
        target: ResolveTarget::Http {
            host: upstream.host.clone(),
            path: upstream.path.clone(),
            method: method.as_str().to_string(),
        },
    };
    let state = state.clone();
    let grant = grant.to_string();
    tokio::task::spawn_blocking(move || {
        let mut broker = state.broker.lock().unwrap_or_else(|e| e.into_inner());
        let mut released = Vec::with_capacity(references.len());
        for reference in references {
            match broker.resolve(&grant, &reference, &ctx) {
                Ok(value) => released.push((reference, value)),
                Err(error) => {
                    // A log that would not take the row is the host's
                    // trouble, not a grant that does not cover.
                    let status = match &error {
                        AgentError::Refused(r) if r.code == Some(Code::AuditUnavailable) => {
                            StatusCode::SERVICE_UNAVAILABLE
                        }
                        _ => StatusCode::FORBIDDEN,
                    };
                    return Err(Refusal::new(
                        status,
                        &error.kind(),
                        format!("{reference} could not be resolved under this grant: {error}"),
                    ));
                }
            }
        }
        Ok(released)
    })
    .await
    .map_err(|_| {
        Refusal::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "the resolve task did not finish",
        )
    })?
}

/// Where a value lands, which decides how it is written: a header takes it
/// as is; a JSON document always holds a placeholder inside a string, so
/// the value goes in JSON-escaped and the string is the one the author
/// wrote with the value in it; a form field takes it form-encoded, whether
/// the placeholder was written plain or percent-encoded, so an `&` or `=`
/// in a value cannot add a field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Placement {
    Header,
    Json,
    Form,
}

/// The body's placement from its content type, decided once per request;
/// `None` for a body placeholders are not swapped in.
fn placement_of(headers: &HeaderMap) -> Option<Placement> {
    let kind = headers
        .get(CONTENT_TYPE)?
        .to_str()
        .ok()?
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    match kind.as_str() {
        "application/json" => Some(Placement::Json),
        "application/x-www-form-urlencoded" => Some(Placement::Form),
        _ => None,
    }
}

const HOP_BY_HOP: [&str; 8] = [
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

fn is_hop_by_hop(name: &HeaderName) -> bool {
    let name = name.as_str();
    HOP_BY_HOP.contains(&name) || name.starts_with("proxy-")
}

/// A placeholder found in text: where, what, and whether it was
/// percent-encoded (a form body), in which case its value goes in encoded.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Found {
    start: usize,
    end: usize,
    reference: Reference,
    encoded: bool,
}

/// Every placeholder in `text`, as written (`rowel://…` up to the first
/// character a placeholder cannot hold) or percent-encoded
/// (`rowel%3A%2F%2F…`, with `%XX` runs decoded before parsing). A span that
/// does not parse as a placeholder is left for what it is.
fn placeholders_in(text: &str) -> Vec<Found> {
    const ENCODED: &str = "rowel%3A%2F%2F";
    let mut found = Vec::new();
    let bytes = text.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        let plain = text[at..].find(SCHEME).map(|i| (at + i, false));
        let encoded = find_ignore_case(&text[at..], ENCODED).map(|i| (at + i, true));
        let Some((start, is_encoded)) = [plain, encoded].into_iter().flatten().min() else {
            break;
        };
        let prefix = if is_encoded {
            ENCODED.len()
        } else {
            SCHEME.len()
        };
        let end = start + prefix + span_len(&text[start + prefix..], is_encoded);
        let span = &text[start..end];
        let decoded = if is_encoded {
            percent_decode_str(span).decode_utf8_lossy().into_owned()
        } else {
            span.to_string()
        };
        if let Some(reference) = Reference::parse(&decoded) {
            found.push(Found {
                start,
                end,
                reference,
                encoded: is_encoded,
            });
        }
        at = end.max(start + 1);
    }
    found
}

fn find_ignore_case(text: &str, needle: &str) -> Option<usize> {
    let (text, needle) = (text.as_bytes(), needle.as_bytes());
    (0..text.len().saturating_sub(needle.len() - 1))
        .find(|&i| text[i..i + needle.len()].eq_ignore_ascii_case(needle))
}

/// How many bytes from the start of `text` — the part after the scheme —
/// could be one placeholder: the characters a slug, an id or a field name
/// holds, `/` between segments, and in the encoded form `%XX` triples in
/// their place.
fn span_len(text: &str, encoded: bool) -> usize {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_alphanumeric() || b"-._~".contains(&c) || (!encoded && c == b'/') {
            i += 1;
        } else if encoded
            && c == b'%'
            && i + 2 < bytes.len()
            && bytes[i + 1].is_ascii_hexdigit()
            && bytes[i + 2].is_ascii_hexdigit()
        {
            i += 3;
        } else {
            break;
        }
    }
    i
}

/// `text` with every placeholder among `values` replaced by its value,
/// written for `placement`. How the placeholder was found decides nothing:
/// a form field is form-encoded whether its author wrote it plain or
/// percent-encoded.
fn replaced(text: &str, values: &[(String, Zeroizing<String>)], placement: Placement) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for found in placeholders_in(text) {
        let Some((_, value)) = values
            .iter()
            .find(|(placeholder, _)| *placeholder == found.reference.to_string())
        else {
            continue;
        };
        out.push_str(&text[at..found.start]);
        match placement {
            Placement::Header => out.push_str(value),
            Placement::Json => out.push_str(&json_escaped(value)),
            Placement::Form => out.push_str(&form_encoded(value)),
        }
        at = found.end;
    }
    out.push_str(&text[at..]);
    out
}

fn redacted(redactor: &mut Redactor, bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let _ = redactor.feed(bytes, &mut out);
    let _ = redactor.finish(&mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_listen_address_must_be_loopback() {
        assert!(listen_address("127.0.0.1:7400").is_ok());
        assert!(listen_address("[::1]:7400").is_ok());
        assert!(listen_address("127.0.0.2:1").is_ok());
        assert!(listen_address("0.0.0.0:7400").is_err());
        assert!(listen_address("192.168.1.2:7400").is_err());
        assert!(listen_address("[::]:7400").is_err());
        assert!(listen_address("localhost:7400").is_err());
        assert!(listen_address("7400").is_err());
    }

    #[test]
    fn the_upstream_is_read_from_the_path_prefix() {
        let uri: http::Uri = "/https/api.stripe.com/v1/balance?limit=3".parse().unwrap();
        let up = upstream_of(&uri, false).unwrap();
        assert_eq!(up.url, "https://api.stripe.com/v1/balance?limit=3");
        assert_eq!(
            (up.host.as_str(), up.path.as_str()),
            ("api.stripe.com", "/v1/balance")
        );

        let uri: http::Uri = "/https/API.Example.com:8443".parse().unwrap();
        let up = upstream_of(&uri, false).unwrap();
        assert_eq!(up.url, "https://api.example.com:8443/");
        assert_eq!(
            (up.host.as_str(), up.path.as_str()),
            ("api.example.com:8443", "/")
        );

        let uri: http::Uri = "/http/127.0.0.1:3000/x".parse().unwrap();
        assert_eq!(
            upstream_of(&uri, false).unwrap_err().status,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(upstream_of(&uri, true).unwrap().host, "127.0.0.1:3000");
        let uri: http::Uri = "/https/API.stripe.com:443/v1".parse().unwrap();
        assert_eq!(upstream_of(&uri, false).unwrap().host, "api.stripe.com");
        // The path checked is the path sent: dot segments, written or
        // percent-encoded, are collapsed before the grant sees it.
        for dotted in [
            "/https/api.stripe.com/v1/public/../charges",
            "/https/api.stripe.com/v1/public/%2e%2e/charges",
            "/https/api.stripe.com/v1/public/.%2E/./charges",
        ] {
            let up = upstream_of(&dotted.parse().unwrap(), false).unwrap();
            assert_eq!(up.path, "/v1/charges", "{dotted}");
            assert_eq!(up.url, "https://api.stripe.com/v1/charges", "{dotted}");
        }
        for bad in [
            "/v1/balance",
            "/https/",
            "/https//x",
            "/ftp/host/x",
            "/https/api.stripe.com/v1?key=rowel://lb/stripe/apiKey",
            "/https/api.stripe.com/rowel://lb/stripe/apiKey",
            "/https/api.stripe.com/v1?key=rowel%3A%2F%2Flb%2Fstripe%2FapiKey",
            "/https/user@host/x",
        ] {
            let uri: http::Uri = bad.parse().unwrap();
            assert_eq!(
                upstream_of(&uri, true).unwrap_err().status,
                StatusCode::BAD_REQUEST,
                "{bad}"
            );
        }
    }

    #[test]
    fn placeholders_are_found_as_written_and_percent_encoded() {
        let text = "Bearer rowel://lb/stripe/apiKey, key=rowel%3a%2f%2Flb%2Fgh%2Fpassword&x=rowel://lb/bad plain";
        let found = placeholders_in(text);
        let refs: Vec<(String, bool)> = found
            .iter()
            .map(|f| (f.reference.to_string(), f.encoded))
            .collect();
        assert_eq!(
            refs,
            [
                ("rowel://lb/stripe/apiKey".to_string(), false),
                ("rowel://lb/gh/password".to_string(), true),
            ]
        );
        assert_eq!(
            &text[found[0].start..found[0].end],
            "rowel://lb/stripe/apiKey"
        );
        assert_eq!(
            &text[found[1].start..found[1].end],
            "rowel%3a%2f%2Flb%2Fgh%2Fpassword"
        );

        let values = vec![
            (
                "rowel://lb/stripe/apiKey".to_string(),
                Zeroizing::new("sk_live/1+2".to_string()),
            ),
            (
                "rowel://lb/gh/password".to_string(),
                Zeroizing::new("p w".to_string()),
            ),
        ];
        assert_eq!(
            replaced(text, &values, Placement::Header),
            "Bearer sk_live/1+2, key=p w&x=rowel://lb/bad plain"
        );
        assert_eq!(
            replaced(text, &values, Placement::Form),
            "Bearer sk_live%2F1%2B2, key=p+w&x=rowel://lb/bad plain"
        );
        assert_eq!(
            replaced("nothing here", &values, Placement::Json),
            "nothing here"
        );
    }

    #[test]
    fn a_json_value_is_escaped_so_the_document_still_parses() {
        // Quotes inside, at the start, at the end, and nothing but quotes:
        // only the one pair wrapping the JSON string is not the value's.
        for (case, secret) in [
            "sk\"live\\\nend\ttab",
            "ends\"",
            "\"starts",
            "\"both\"",
            "\"\"",
            "\\\"",
        ]
        .into_iter()
        .enumerate()
        {
            let values = vec![(
                "rowel://lb/stripe/apiKey".to_string(),
                Zeroizing::new(secret.to_string()),
            )];
            let document = replaced(
                r#"{"key": "rowel://lb/stripe/apiKey", "n": 1}"#,
                &values,
                Placement::Json,
            );
            let parsed: serde_json::Value = serde_json::from_str(&document).unwrap();
            assert_eq!(parsed["key"], secret, "case {case}");
            assert_eq!(parsed["n"], 1);
            assert_eq!(parsed.as_object().unwrap().len(), 2);
        }
    }

    #[test]
    fn a_form_value_is_encoded_so_no_field_is_added() {
        let secret = "a&b=c+d%e f";
        let values = vec![(
            "rowel://lb/stripe/apiKey".to_string(),
            Zeroizing::new(secret.to_string()),
        )];
        for body in [
            "key=rowel://lb/stripe/apiKey&n=1",
            "key=rowel%3A%2F%2Flb%2Fstripe%2FapiKey&n=1",
        ] {
            let sent = replaced(body, &values, Placement::Form);
            let pairs: Vec<(String, String)> = form_urlencoded::parse(sent.as_bytes())
                .into_owned()
                .collect();
            assert_eq!(
                pairs,
                [
                    ("key".to_string(), secret.to_string()),
                    ("n".to_string(), "1".to_string())
                ],
                "{body}"
            );
        }
    }

    #[test]
    fn a_bucket_holds_sixty_and_refills_one_a_second() {
        let start = Instant::now();
        let mut bucket = Bucket {
            tokens: RATE_PER_MINUTE as f64,
            at: start,
        };
        for _ in 0..RATE_PER_MINUTE {
            assert_eq!(bucket.take(start), Ok(()));
        }
        assert_eq!(bucket.take(start), Err(1));
        assert_eq!(
            bucket.take(start + std::time::Duration::from_millis(1500)),
            Ok(())
        );
        assert_eq!(
            bucket.take(start + std::time::Duration::from_millis(1500)),
            Err(1)
        );
        assert_eq!(
            bucket.take(start + std::time::Duration::from_secs(1000)),
            Ok(())
        );
        assert_eq!(bucket.tokens, (RATE_PER_MINUTE - 1) as f64);
    }

    #[test]
    fn the_placement_follows_the_content_type_and_hop_by_hop_is_known() {
        let with = |kind: &str| {
            let mut headers = HeaderMap::new();
            headers.insert(CONTENT_TYPE, HeaderValue::from_str(kind).unwrap());
            headers
        };
        assert_eq!(
            placement_of(&with("application/json; charset=utf-8")),
            Some(Placement::Json)
        );
        assert_eq!(
            placement_of(&with("Application/X-WWW-Form-Urlencoded")),
            Some(Placement::Form)
        );
        assert_eq!(placement_of(&with("text/plain")), None);
        assert_eq!(placement_of(&HeaderMap::new()), None);
        for name in [
            "connection",
            "Proxy-Authorization",
            "proxy-anything",
            "TE",
            "upgrade",
        ] {
            assert!(
                is_hop_by_hop(&HeaderName::from_bytes(name.as_bytes()).unwrap()),
                "{name}"
            );
        }
        assert!(!is_hop_by_hop(&http::header::AUTHORIZATION));
    }
}

#[cfg(test)]
mod end_to_end {
    use std::sync::Arc;

    use rowel_broker::grant::{Mode, Target};
    use rowel_broker::testing::Mock;
    use serde_json::Value;

    use super::*;
    use crate::testing::{known_sidecar, Harness};

    const GRANT: &str = "g1";
    const REF: &str = "rowel://lb/stripe/apiKey";
    const VALUE: &str = "sk_test_mock";

    /// What the echo upstream saw, request by request, read from its own
    /// side: the proxy redacts its reply, so only this shows what arrived.
    type Seen = Arc<Mutex<Vec<Value>>>;

    /// An upstream that answers every request with what it saw — method,
    /// path, query, the Authorization header, whether the grant header got
    /// through, and the body — and records the same. Under `/redirect/` it
    /// answers a redirect instead: to another path of its own, to another
    /// host, or a 307 (which a following client would replay the body to).
    async fn echo_upstream() -> (SocketAddr, Seen) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let seen: Seen = Arc::default();
        let recorded = seen.clone();
        tokio::spawn(async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let recorded = recorded.clone();
                tokio::spawn(async move {
                    let _ = http1::Builder::new()
                        .serve_connection(
                            TokioIo::new(stream),
                            service_fn(move |request: Request<Incoming>| {
                                let recorded = recorded.clone();
                                async move {
                                let (parts, body) = request.into_parts();
                                let body = body.collect().await.unwrap().to_bytes();
                                let seen = json!({
                                    "method": parts.method.as_str(),
                                    "path": parts.uri.path(),
                                    "query": parts.uri.query(),
                                    "authorization": parts.headers.get("authorization").and_then(|v| v.to_str().ok()),
                                    "grantHeader": parts.headers.contains_key(GRANT_HEADER),
                                    "connection": parts.headers.get("connection").and_then(|v| v.to_str().ok()),
                                    "acceptEncoding": parts.headers.get("accept-encoding").and_then(|v| v.to_str().ok()),
                                    "body": String::from_utf8_lossy(&body),
                                });
                                recorded.lock().unwrap().push(seen.clone());
                                let redirect = match parts.uri.path() {
                                    "/redirect/same" => Some((StatusCode::FOUND, "/other")),
                                    "/redirect/cross" => Some((StatusCode::FOUND, "https://elsewhere.example/x")),
                                    "/redirect/307" => Some((StatusCode::TEMPORARY_REDIRECT, "http://127.0.0.1:1/x")),
                                    _ => None,
                                };
                                let auth = parts.headers.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
                                let redirect = match parts.uri.path() {
                                    "/redirect/echo" => Some((StatusCode::FOUND, format!("/cb?k={auth}"))),
                                    _ => redirect.map(|(status, location)| (status, location.to_string())),
                                };
                                if parts.uri.path() == "/gzip" {
                                    return Ok::<_, Infallible>(
                                        Response::builder()
                                            .header("content-encoding", "gzip")
                                            .body(Full::new(Bytes::from_static(b"\x1f\x8b")))
                                            .unwrap(),
                                    );
                                }
                                if let Some((status, location)) = redirect {
                                    return Ok::<_, Infallible>(
                                        Response::builder()
                                            .status(status)
                                            .header("location", location)
                                            .body(Full::new(Bytes::from("moved")))
                                            .unwrap(),
                                    );
                                }
                                Ok::<_, Infallible>(
                                    Response::builder()
                                        .header(CONTENT_TYPE, "application/json")
                                        .header("x-echo", format!("auth={}", parts.headers.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or_default()))
                                        .body(Full::new(Bytes::from(seen.to_string())))
                                        .unwrap(),
                                )
                                }
                            }),
                        )
                        .await;
                });
            }
        });
        (address, seen)
    }

    struct Proxy {
        harness: Harness,
        state: Arc<State>,
        base: String,
        upstream: SocketAddr,
        seen: Seen,
        client: reqwest::Client,
    }

    /// The proxy over a known sidecar with the grant `mock` makes for the
    /// upstream's host (the ephemeral `127.0.0.1:<port>`, which is why the
    /// host is handed in), and an echo upstream, both on loopback. The test
    /// client follows no redirect either, so a 3xx the proxy relays is seen
    /// as such.
    async fn proxy(mock: impl FnOnce(&str) -> Mock) -> Proxy {
        let (upstream, seen) = echo_upstream().await;
        let harness = known_sidecar(mock(&upstream.to_string()));
        let state = Arc::new(State::new(sidecar_of(&harness), true).unwrap());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(serve(listener, state.clone()));
        Proxy {
            harness,
            state,
            base,
            upstream,
            seen,
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
        }
    }

    /// The harness's sidecar as the proxy holds it: boxed and `Send`, over
    /// the shared connection.
    fn sidecar_of(harness: &Harness) -> Box<dyn Broker + Send> {
        let link = harness.link.clone();
        Box::new(crate::agent::Agent::sidecar(
            Box::new(move || Ok(link.clone())),
            harness.keys(),
        ))
    }

    /// A header grant for the Stripe key at every path of `host`.
    fn granting_stripe(host: &str) -> Mock {
        Mock::unlocked().granting_at(
            GRANT,
            "stripe",
            Mode::Header,
            Target::Http {
                host: host.into(),
                path: "/*".into(),
                methods: vec![],
            },
        )
    }

    impl Proxy {
        fn url(&self, path_and_query: &str) -> String {
            format!("{}/http/{}{path_and_query}", self.base, self.upstream)
        }

        fn uses(&self) -> u32 {
            self.harness.host().host().grants.borrow()[GRANT].uses
        }

        /// The grant ids the proxy keeps a bucket for.
        fn buckets(&self) -> Vec<String> {
            let mut ids: Vec<String> = self.state.buckets.lock().unwrap().keys().cloned().collect();
            ids.sort();
            ids
        }

        /// The last request the upstream saw.
        fn last_seen(&self) -> Value {
            self.seen.lock().unwrap().last().cloned().unwrap()
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_header_placeholder_reaches_the_upstream_as_the_value_and_comes_back_as_itself() {
        let proxy = proxy(granting_stripe).await;
        let response = proxy
            .client
            .get(proxy.url("/v1/balance?limit=3"))
            .header("authorization", format!("Bearer {REF}"))
            .header(GRANT_HEADER, GRANT)
            .header("connection", "keep-alive")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get("x-echo").unwrap().to_str().unwrap(),
            format!("auth=Bearer {REF}"),
            "a response header is redacted too"
        );
        assert!(response.headers().get(CONTENT_LENGTH).is_some());
        let text = response.text().await.unwrap();
        assert!(!text.contains(VALUE), "{text}");
        let seen: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(seen["authorization"], format!("Bearer {REF}"));
        assert_eq!(seen["method"], "GET");
        assert_eq!(seen["path"], "/v1/balance");
        assert_eq!(seen["query"], "limit=3");
        assert_eq!(seen["grantHeader"], false);
        assert_eq!(proxy.uses(), 1);

        let arrived = proxy.last_seen();
        assert_eq!(arrived["authorization"], format!("Bearer {VALUE}"));
        assert_eq!(arrived["grantHeader"], false, "the grant header is ours");
        assert_eq!(arrived["connection"], Value::Null, "hop-by-hop is dropped");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_json_or_form_body_placeholder_is_swapped_and_a_text_body_is_not() {
        let proxy = proxy(granting_stripe).await;
        let through = proxy
            .client
            .post(proxy.url("/v1/charges"))
            .header(CONTENT_TYPE, "application/json")
            .header(GRANT_HEADER, GRANT)
            .body(format!("{{\"key\":\"{REF}\",\"n\":1}}"))
            .send()
            .await
            .unwrap();
        let text = through.text().await.unwrap();
        let seen: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            seen["body"],
            format!("{{\"key\":\"{REF}\",\"n\":1}}"),
            "redacted on the way back"
        );
        assert_eq!(
            proxy.last_seen()["body"],
            format!("{{\"key\":\"{VALUE}\",\"n\":1}}")
        );
        assert_eq!(proxy.uses(), 1);

        let form = proxy
            .client
            .post(proxy.url("/v1/charges"))
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header(GRANT_HEADER, GRANT)
            .body("key=rowel%3A%2F%2Flb%2Fstripe%2FapiKey&n=1")
            .send()
            .await
            .unwrap();
        let seen: Value = serde_json::from_str(&form.text().await.unwrap()).unwrap();
        assert_eq!(seen["body"], format!("key={REF}&n=1"));
        assert_eq!(proxy.last_seen()["body"], format!("key={VALUE}&n=1"));
        assert_eq!(proxy.uses(), 2);

        let plain = proxy
            .client
            .post(proxy.url("/v1/charges"))
            .header(CONTENT_TYPE, "text/plain")
            .header("authorization", format!("Bearer {REF}"))
            .header(GRANT_HEADER, GRANT)
            .body(REF)
            .send()
            .await
            .unwrap();
        let seen: Value = serde_json::from_str(&plain.text().await.unwrap()).unwrap();
        assert_eq!(seen["body"], REF, "a text body is not touched");
        assert_eq!(proxy.last_seen()["body"], REF);
        assert_eq!(proxy.uses(), 3, "the header's one use");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_placeholder_in_the_url_is_refused_before_anything_is_resolved() {
        let proxy = proxy(granting_stripe).await;
        for path in [
            format!("/v1/balance?key={REF}"),
            format!("/v1/{REF}"),
            "/v1/balance?key=rowel%3A%2F%2Flb%2Fstripe%2FapiKey".to_string(),
        ] {
            let response = proxy
                .client
                .get(proxy.url(&path))
                .header(GRANT_HEADER, GRANT)
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{path}");
            let body: Value = response.json().await.unwrap();
            assert_eq!(body["error"], "bad_request");
        }
        assert_eq!(proxy.uses(), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn no_grant_header_or_an_unknown_grant_is_forbidden_without_a_value() {
        let proxy = proxy(granting_stripe).await;
        let response = proxy
            .client
            .get(proxy.url("/v1/balance"))
            .header("authorization", format!("Bearer {REF}"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["error"], "no_grant");

        let response = proxy
            .client
            .get(proxy.url("/v1/balance"))
            .header("authorization", format!("Bearer {REF}"))
            .header(GRANT_HEADER, "not-a-grant")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let headers = format!("{:?}", response.headers());
        let text = response.text().await.unwrap();
        assert!(!text.contains(VALUE) && !headers.contains(VALUE), "{text}");
        let body: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(body["error"], "GrantDenied");
        assert!(body["message"].as_str().unwrap().contains(REF));
        assert_eq!(proxy.uses(), 0);
    }

    // A request with nothing to put in is not forwarded, whatever grant it
    // names: nothing is resolved, the upstream sees nothing, and no bucket
    // is made for the id.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_request_with_no_placeholder_is_refused_not_forwarded() {
        let proxy = proxy(granting_stripe).await;
        for grant in [GRANT, "made-up"] {
            let response = proxy
                .client
                .post(proxy.url("/v1/charges"))
                .header(CONTENT_TYPE, "application/json")
                .header("authorization", "Bearer sk_typed_by_hand")
                .header(GRANT_HEADER, grant)
                .body(r#"{"n":1}"#)
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{grant}");
            let body: Value = response.json().await.unwrap();
            assert_eq!(body["error"], "no_placeholder");
        }
        assert!(proxy.seen.lock().unwrap().is_empty(), "nothing went out");
        assert_eq!(proxy.uses(), 0);
        assert!(proxy.buckets().is_empty());
    }

    // A host whose log will not take the `used` row releases nothing, and
    // the proxy says it is the host's trouble, not a grant that does not
    // cover: 503, and nothing goes out.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_resolve_the_log_will_not_hold_is_unavailable_and_sends_nothing() {
        let proxy = proxy(granting_stripe).await;
        proxy.harness.host().host().audit_fails.set(true);
        let response = proxy
            .client
            .get(proxy.url("/v1/balance"))
            .header("authorization", format!("Bearer {REF}"))
            .header(GRANT_HEADER, GRANT)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["error"], "AuditUnavailable");
        assert!(proxy.seen.lock().unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_sixty_first_request_in_a_minute_is_rate_limited() {
        let proxy = proxy(|host| {
            granting_stripe(host).granting_at(
                "grant-2",
                "stripe",
                Mode::Header,
                Target::Http {
                    host: host.into(),
                    path: "/*".into(),
                    methods: vec![],
                },
            )
        })
        .await;
        let call = |grant: &str| {
            proxy
                .client
                .get(proxy.url("/v1/balance"))
                .header("authorization", format!("Bearer {REF}"))
                .header(GRANT_HEADER, grant.to_string())
                .send()
        };
        for n in 1..=RATE_PER_MINUTE {
            let response = call(GRANT).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK, "request {n}");
        }
        let response = call(GRANT).await.unwrap();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(response.headers().get("retry-after").unwrap(), "1");
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["error"], "rate_limited");
        assert_eq!(
            proxy.uses(),
            RATE_PER_MINUTE,
            "a limited request resolves nothing"
        );

        let other = call("grant-2").await.unwrap();
        assert_eq!(other.status(), StatusCode::OK, "buckets are per grant");

        // An id the host does not accept is refused and takes no place in
        // the map, however many are made up.
        for n in 0..5 {
            let made_up = call(&format!("made-up-{n}")).await.unwrap();
            assert_eq!(made_up.status(), StatusCode::FORBIDDEN);
        }
        assert_eq!(proxy.buckets(), [GRANT.to_string(), "grant-2".to_string()]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn http_is_refused_without_the_flag_and_a_dead_upstream_is_a_bad_gateway() {
        let proxy = proxy(|host| {
            granting_stripe(host).granting_at(
                "dead",
                "stripe",
                Mode::Header,
                Target::Http {
                    host: "127.0.0.1:1".into(),
                    path: "/*".into(),
                    methods: vec![],
                },
            )
        })
        .await;
        let strict_harness = known_sidecar(granting_stripe(&proxy.upstream.to_string()));
        let state_without_http = Arc::new(State::new(sidecar_of(&strict_harness), false).unwrap());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let strict = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(serve(listener, state_without_http));
        let response = proxy
            .client
            .get(format!("{strict}/http/{}/v1", proxy.upstream))
            .header(GRANT_HEADER, GRANT)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let response = proxy
            .client
            .get(format!("{}/http/127.0.0.1:1/v1", proxy.base))
            .header("authorization", format!("Bearer {REF}"))
            .header(GRANT_HEADER, "dead")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["error"], "upstream");
    }

    #[test]
    fn a_location_is_addressed_through_the_proxy_or_dropped() {
        let base = "https://api.stripe.com/v1/charges?limit=3";
        let proxied = |location: &str| proxied_location(base, location.as_bytes());
        for (location, expected) in [
            ("/v1/other?page=2", "/https/api.stripe.com/v1/other?page=2"),
            ("next", "/https/api.stripe.com/v1/next"),
            ("https://elsewhere.example/x", "/https/elsewhere.example/x"),
            ("https://API.example.com:443/x", "/https/api.example.com/x"),
            ("http://localhost:3000/cb#frag", "/http/localhost:3000/cb"),
            ("https://user:pw@example.com/x", "/https/example.com/x"),
            ("http://[::1]:8080/x", "/http/[::1]:8080/x"),
            ("//cdn.example/asset", "/https/cdn.example/asset"),
        ] {
            assert_eq!(proxied(location).as_deref(), Some(expected), "{location}");
        }
        for elsewhere in ["ftp://files.example/x", "javascript:alert(1)", "data:,x"] {
            assert_eq!(proxied(elsewhere), None, "{elsewhere}");
        }
        assert_eq!(proxied_location(base, b"/\xff"), None);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_redirect_is_relayed_through_the_proxy_and_never_followed() {
        let proxy = proxy(granting_stripe).await;
        let upstream = proxy.upstream;
        for (path, status, location) in [
            (
                "/redirect/same",
                StatusCode::FOUND,
                format!("/http/{upstream}/other"),
            ),
            (
                "/redirect/cross",
                StatusCode::FOUND,
                "/https/elsewhere.example/x".to_string(),
            ),
        ] {
            let response = proxy
                .client
                .get(proxy.url(path))
                .header("authorization", format!("Bearer {REF}"))
                .header(GRANT_HEADER, GRANT)
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), status, "{path}");
            assert_eq!(response.headers().get("location").unwrap(), &location);
            assert_eq!(response.text().await.unwrap(), "moved");
        }
        let response = proxy
            .client
            .post(proxy.url("/redirect/307"))
            .header(CONTENT_TYPE, "application/json")
            .header(GRANT_HEADER, GRANT)
            .body(format!("{{\"key\":\"{REF}\"}}"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(
            response.headers().get("location").unwrap(),
            "/http/127.0.0.1:1/x"
        );

        let seen = proxy.seen.lock().unwrap();
        assert_eq!(seen.len(), 3, "one upstream request per request, no hop");
        assert_eq!(seen[0]["authorization"], format!("Bearer {VALUE}"));
        assert_eq!(seen[1]["authorization"], format!("Bearer {VALUE}"));
        assert_eq!(seen[2]["body"], format!("{{\"key\":\"{VALUE}\"}}"));
        drop(seen);
        assert_eq!(proxy.uses(), 3);
    }

    // The upstream is asked for the answer unencoded, whatever the agent's
    // client accepts, and an encoded answer is refused rather than relayed
    // past the redactor. A `Location` echoing the value is redacted as the
    // upstream wrote it, before it is re-addressed (which re-encodes it).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_answer_is_redacted_as_sent_or_not_relayed() {
        let proxy = proxy(granting_stripe).await;
        let response = proxy
            .client
            .get(proxy.url("/v1/balance"))
            .header("accept-encoding", "gzip, br")
            .header("authorization", format!("Bearer {REF}"))
            .header(GRANT_HEADER, GRANT)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(proxy.last_seen()["acceptEncoding"], "identity");

        let response = proxy
            .client
            .get(proxy.url("/gzip"))
            .header("authorization", format!("Bearer {REF}"))
            .header(GRANT_HEADER, GRANT)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);

        let response = proxy
            .client
            .get(proxy.url("/redirect/echo"))
            .header("authorization", format!("Bearer {REF}"))
            .header(GRANT_HEADER, GRANT)
            .send()
            .await
            .unwrap();
        let location = response.headers()["location"].to_str().unwrap().to_string();
        assert!(!location.contains(VALUE), "{location}");
        assert!(
            location.starts_with(&format!("/http/{}/cb?k=", proxy.upstream)),
            "{location}"
        );
    }

    // A client that follows redirects follows them back through the proxy: a
    // hop on the granted host is served under the grant like any request, and
    // a hop to another host is refused there before any value is resolved —
    // the client never reaches it directly, with the grant header or without.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_following_client_comes_back_through_the_proxy_for_every_hop() {
        let proxy = proxy(granting_stripe).await;
        let following = reqwest::Client::new();

        let response = following
            .get(proxy.url("/redirect/same"))
            .header("authorization", format!("Bearer {REF}"))
            .header(GRANT_HEADER, GRANT)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["path"], "/other");
        assert_eq!(body["authorization"], format!("Bearer {REF}"), "redacted");
        {
            let seen = proxy.seen.lock().unwrap();
            assert_eq!(seen.len(), 2);
            assert_eq!(seen[1]["path"], "/other");
            assert_eq!(seen[1]["authorization"], format!("Bearer {VALUE}"));
        }
        assert_eq!(proxy.uses(), 2);

        let response = following
            .get(proxy.url("/redirect/cross"))
            .header("authorization", format!("Bearer {REF}"))
            .header(GRANT_HEADER, GRANT)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(proxy.seen.lock().unwrap().len(), 3, "no hop left the proxy");
        assert_eq!(proxy.uses(), 3, "the refused hop resolved nothing");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_grant_names_the_host_with_its_port_by_the_one_rule() {
        let without_port = proxy(|_| granting_stripe("127.0.0.1")).await;
        let response = without_port
            .client
            .get(without_port.url("/v1/balance"))
            .header("authorization", format!("Bearer {REF}"))
            .header(GRANT_HEADER, GRANT)
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::FORBIDDEN,
            "a grant for the host without its (non-default) port is for another host"
        );
        assert_eq!(without_port.uses(), 0);
        assert!(without_port.seen.lock().unwrap().is_empty());
    }
}
