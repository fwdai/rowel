//! Who this process is, said with a token its platform minted (`spec/02`,
//! workload identity). A client in CI has no person at the dialog, so when
//! `ROWEL_BROKER` names a Rowel Server and the platform offers a token, the
//! `associate` carries a [`Proof`] the server verifies against its trust
//! policy instead of asking. Two sources, tried in order: GitHub Actions,
//! whose runner mints a JWT for an audience of the requester's choosing, and
//! a file, for a platform that mints one for a fixed audience (a Kubernetes
//! projected token). The token is the credential: it is held wiped-on-drop
//! for as long as this process has it, and it is never printed or logged.

use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::{engine::general_purpose::STANDARD, Engine};
use reqwest::header::{HeaderValue, AUTHORIZATION};
use rowel_broker::actions::Proof;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::transport;

/// Where GitHub Actions says to ask for an id token, and what the ask is
/// authorised with. The runner sets both when the job has
/// `permissions: id-token: write`.
pub const GITHUB_URL_VAR: &str = "ACTIONS_ID_TOKEN_REQUEST_URL";
pub const GITHUB_TOKEN_VAR: &str = "ACTIONS_ID_TOKEN_REQUEST_TOKEN";
/// A file holding a token the platform minted for a fixed audience.
pub const TOKEN_FILE_VAR: &str = "ROWEL_WORKLOAD_TOKEN_FILE";
/// The challenge, under the server's origin, answered to anyone.
pub const CHALLENGE_PATH: &str = "/v1/agent/challenge";
/// What a proof from either source is called on the wire: a JWT a verifier
/// checks against its issuer's keys.
pub const OIDC: &str = "oidc";

/// The length of an X25519 public key, which is what the session key is.
const SESSION_KEY_LEN: usize = 32;
/// How long either HTTP ask may take: the runner and the server are both a
/// request away, and a proof that is not there in this time is not coming.
const TIMEOUT: Duration = Duration::from_secs(15);

/// What the server answers a challenge with: a nonce to echo in the proof,
/// and the audience a token for this server is minted for.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Challenge {
    pub nonce: String,
    pub audience: String,
}

/// Where a token comes from. No `Debug`: the request token is a credential
/// of its own.
pub enum Source {
    /// GitHub Actions: the runner's token endpoint, and the request token
    /// that authorises the ask.
    GitHub {
        url: String,
        request_token: Zeroizing<String>,
    },
    /// A token already minted, at this path.
    File(PathBuf),
}

impl Source {
    /// The source the environment offers, GitHub Actions before a file, or
    /// `None` for a shell that offers neither.
    pub fn from_env() -> Option<Self> {
        if let (Some(url), Some(token)) = (var(GITHUB_URL_VAR), var(GITHUB_TOKEN_VAR)) {
            return Some(Source::GitHub {
                url,
                request_token: Zeroizing::new(token),
            });
        }
        var(TOKEN_FILE_VAR).map(|path| Source::File(PathBuf::from(path)))
    }
}

/// The proof for the session whose public key is `session_key` (base64, as
/// the `associate` repeats it) against the broker `ROWEL_BROKER` names.
/// `None` for the app, and for a remote broker when the environment offers
/// no token: the association then asks a person, as it always has. A source
/// that is offered but fails is an error, not a quiet fall back to the ask —
/// in CI nobody is there to answer it, and the reason should be in the log.
pub fn proof_for(session_key: &str) -> io::Result<Option<Proof>> {
    let Some(broker) = transport::broker()? else {
        return Ok(None);
    };
    let Some(source) = Source::from_env() else {
        return Ok(None);
    };
    prove(&broker.origin(), &source, session_key).map(Some)
}

/// A proof from `source` for `session_key`, bound to a challenge from the
/// server at `origin`: the token is minted for the challenge's audience with
/// the session key's digest appended where the source lets the requester
/// choose it, and the nonce is echoed either way.
pub fn prove(origin: &str, source: &Source, session_key: &str) -> io::Result<Proof> {
    let challenge = challenge(origin)?;
    let token = match source {
        Source::GitHub { url, request_token } => github_token(
            url,
            request_token,
            &audience(&challenge.audience, session_key)?,
        )?,
        Source::File(path) => file_token(path)?,
    };
    Ok(Proof {
        kind: OIDC.to_string(),
        token: token.to_string(),
        nonce: Some(challenge.nonce),
    })
}

/// The audience a token for this session is minted for: the server's, then
/// the hex SHA-256 of the raw session key, so a token taken from one channel
/// fails the audience check on every other.
pub fn audience(server_audience: &str, session_key: &str) -> io::Result<String> {
    let raw = STANDARD
        .decode(session_key)
        .ok()
        .filter(|raw| raw.len() == SESSION_KEY_LEN)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "the session key is not a 32-byte key in base64",
            )
        })?;
    let digest = Sha256::digest(&raw);
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(format!("{server_audience}:{hex}"))
}

/// The server's challenge.
pub fn challenge(origin: &str) -> io::Result<Challenge> {
    let body = get(&format!("{origin}{CHALLENGE_PATH}"), &[], None)
        .map_err(|error| io::Error::other(format!("the server's challenge: {error}")))?;
    serde_json::from_slice(&body)
        .map_err(|error| io::Error::other(format!("the server's challenge did not read: {error}")))
}

/// A token from GitHub Actions for `audience`: the runner's endpoint with
/// the audience as a query parameter, authorised by the request token, as
/// the runner's own `getIDToken` asks.
pub fn github_token(
    url: &str,
    request_token: &str,
    audience: &str,
) -> io::Result<Zeroizing<String>> {
    #[derive(Deserialize)]
    struct Reply {
        value: String,
    }
    let body = get(url, &[("audience", audience)], Some(request_token))
        .map_err(|error| io::Error::other(format!("{GITHUB_URL_VAR}: {error}")))?;
    let reply: Reply = serde_json::from_slice(&body)
        .map_err(|_| io::Error::other(format!("{GITHUB_URL_VAR} answered with no token")))?;
    Ok(Zeroizing::new(reply.value))
}

/// The token in the file at `path`, trimmed: a projected token ends in a
/// newline, and the newline is not part of it.
fn file_token(path: &Path) -> io::Result<Zeroizing<String>> {
    let contents = Zeroizing::new(fs::read_to_string(path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("{TOKEN_FILE_VAR} {}: {error}", path.display()),
        )
    })?);
    let token = Zeroizing::new(contents.trim().to_string());
    if token.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{TOKEN_FILE_VAR} {}: the file is empty", path.display()),
        ));
    }
    Ok(token)
}

/// One GET, awaited on a runtime of its own: the CLI's commands are
/// synchronous and a proof is fetched once per connection, so a runtime
/// that lives for the request is the whole cost. The TLS provider is
/// installed first, as the proxy and the WebSocket transport install it:
/// reqwest is built without one so the CLI links a single ring backend.
/// Errors never repeat the URL: the token endpoint's carries nothing
/// secret today, and the rule is cheaper than checking that it stays so.
fn get(url: &str, query: &[(&str, &str)], bearer: Option<&str>) -> io::Result<Vec<u8>> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let client = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| io::Error::other(error.without_url().to_string()))?;
        let mut request = client.get(url).query(query);
        if let Some(bearer) = bearer {
            let mut value = HeaderValue::from_str(&format!("bearer {bearer}"))
                .map_err(|_| io::Error::other("the request token is not a header value"))?;
            value.set_sensitive(true);
            request = request.header(AUTHORIZATION, value);
        }
        let response = request
            .send()
            .await
            .map_err(|error| io::Error::other(error.without_url().to_string()))?;
        let status = response.status();
        if !status.is_success() {
            return Err(io::Error::other(format!("HTTP {status}")));
        }
        response
            .bytes()
            .await
            .map(|bytes| bytes.to_vec())
            .map_err(|error| io::Error::other(error.without_url().to_string()))
    })
}

/// An environment variable with something in it.
fn var(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread::{self, JoinHandle};

    use super::*;

    /// A server on the loopback address that answers one request with `body`
    /// as JSON, and hands back the request line and headers it was sent.
    fn one_request(body: &'static str) -> (String, JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).unwrap();
                head.push(byte[0]);
            }
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
            stream.flush().unwrap();
            String::from_utf8(head).unwrap()
        });
        (origin, handle)
    }

    const ZERO_KEY_SHA256: &str =
        "66687aadf862bd776c8fc18b8e9f8e20089714856ee233b3902a591d0d5f2925";

    #[test]
    fn the_audience_is_the_servers_then_the_session_keys_digest() {
        let key = STANDARD.encode([0u8; 32]);
        assert_eq!(
            audience("rowel:acme", &key).unwrap(),
            format!("rowel:acme:{ZERO_KEY_SHA256}")
        );
        let counting: Vec<u8> = (0..32).collect();
        assert_eq!(
            audience("rowel:acme", &STANDARD.encode(&counting)).unwrap(),
            "rowel:acme:630dcd2966c4336691125448bbb25b4ff412a49c732db2c8abc1b8581bd710dd"
        );
        // Anything but a 32-byte key is not a session key.
        assert!(audience("rowel:acme", &STANDARD.encode([0u8; 31])).is_err());
        assert!(audience("rowel:acme", "not base64!").is_err());
    }

    #[test]
    fn a_challenge_reply_reads_and_a_short_one_does_not() {
        let challenge: Challenge =
            serde_json::from_str(r#"{ "nonce": "n-1", "audience": "rowel:acme", "ttl": 60 }"#)
                .unwrap();
        assert_eq!(
            challenge,
            Challenge {
                nonce: "n-1".into(),
                audience: "rowel:acme".into(),
            }
        );
        assert!(serde_json::from_str::<Challenge>(r#"{ "nonce": "n-1" }"#).is_err());
    }

    #[test]
    fn the_challenge_is_fetched_at_the_servers_origin() {
        let (origin, server) = one_request(r#"{ "nonce": "n-1", "audience": "rowel:acme" }"#);
        let challenge = challenge(&origin).unwrap();
        assert_eq!(challenge.nonce, "n-1");
        assert_eq!(challenge.audience, "rowel:acme");
        let head = server.join().unwrap();
        assert!(
            head.starts_with("GET /v1/agent/challenge HTTP/1.1\r\n"),
            "{head}"
        );
    }

    #[test]
    fn githubs_token_is_asked_for_the_audience_under_the_request_token() {
        let (origin, server) = one_request(r#"{ "value": "header.claims.signature" }"#);
        let token = github_token(
            &format!("{origin}/token?api-version=2.0"),
            "request-token",
            "rowel:acme:abc",
        )
        .unwrap();
        assert_eq!(token.as_str(), "header.claims.signature");
        let head = server.join().unwrap().to_ascii_lowercase();
        assert!(
            head.starts_with("get /token?api-version=2.0&audience=rowel%3aacme%3aabc http/1.1\r\n"),
            "{head}"
        );
        assert!(
            head.contains("\r\nauthorization: bearer request-token\r\n"),
            "{head}"
        );
    }

    #[test]
    fn a_reply_without_a_token_is_an_error_naming_the_variable() {
        let (origin, server) = one_request(r#"{ "message": "no" }"#);
        let error = github_token(&origin, "request-token", "rowel:acme:abc").unwrap_err();
        assert!(error.to_string().contains(GITHUB_URL_VAR));
        server.join().unwrap();
    }

    #[test]
    fn a_file_token_is_sent_trimmed_with_the_challenges_nonce() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("token");
        fs::write(&path, "  eyJ.projected.token\n").unwrap();
        let (origin, server) = one_request(r#"{ "nonce": "n-2", "audience": "rowel:acme" }"#);
        let proof = prove(
            &origin,
            &Source::File(path.clone()),
            &STANDARD.encode([0u8; 32]),
        )
        .unwrap();
        assert_eq!(proof.kind, "oidc");
        assert_eq!(proof.token, "eyJ.projected.token");
        assert_eq!(proof.nonce.as_deref(), Some("n-2"));
        server.join().unwrap();

        fs::write(&path, "\n").unwrap();
        let error = file_token(&path).unwrap_err();
        assert!(error.to_string().contains(TOKEN_FILE_VAR));
        assert!(file_token(&dir.path().join("missing")).is_err());
    }
}
