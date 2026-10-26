//! Verifying a JWT against the issuer that signed it: OIDC discovery for the
//! issuer's endpoints, a JWKS cache per issuer, and one `verify` that does
//! the signature with `jsonwebtoken` and every claim itself against a clock
//! the caller supplies, so a test can stand at any time and the server never
//! trusts the token's own idea of when it is.
//!
//! Only RS256 and ES256 are accepted. `alg: none` does not even parse as an
//! algorithm, and an HMAC token names a key family no published JWKS can
//! carry, so both are refused before any key is looked up. Every error is
//! one enum whose text names what went wrong and never what the token said.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use jsonwebtoken::jwk::{
    AlgorithmParameters, EllipticCurve, Jwk, JwkSet, KeyAlgorithm, PublicKeyUse,
};
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use serde_json::{Map, Value};

/// How often, at most, one issuer's JWKS is fetched again because a token
/// named a key the cache lacks. Issuers rotate keys over hours; a flood of
/// tokens with made-up `kid`s must not turn the server into a JWKS fetcher.
pub const JWKS_REFRESH_INTERVAL_MS: i64 = 60_000;

/// Clock skew allowed on `exp` and `nbf`, in seconds: a runner's clock and
/// the server's are rarely the same second, and a minute is what the
/// libraries this replaces allowed.
pub const LEEWAY_SECS: i64 = 60;

/// The most an issuer's discovery document or JWKS may weigh. Both are a
/// few kilobytes; a megabyte is a server that is not an issuer.
const MAX_BODY_BYTES: usize = 1 << 20;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// The HTTP client this module reaches issuers with. The TLS provider is
/// installed first because `reqwest` is built without one (the same build
/// the app and the CLI use) and panics on a client without it.
pub fn http_client() -> reqwest::Client {
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(READ_TIMEOUT)
        .build()
        .expect("an HTTP client over the installed TLS provider builds")
}

/// Why a token, or the issuer it names, was not accepted. The text never
/// carries token material: a `Fetch` names the issuer's URL, which the
/// operator configured.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VerifyError {
    #[error("{0} could not be fetched or did not answer with what was expected")]
    Fetch(String),
    #[error("the discovery document names another issuer than the one configured")]
    IssuerMismatch,
    #[error("the token does not read as a JWT")]
    Malformed,
    #[error("the token's algorithm is not one this server accepts")]
    Algorithm,
    #[error("the token names a signing key the issuer does not publish")]
    UnknownKey,
    #[error("the token's signature does not verify")]
    Signature,
    #[error("the token is from another issuer")]
    Issuer,
    #[error("the token is for another audience")]
    Audience,
    #[error("the token has expired")]
    Expired,
    #[error("the token is not yet valid")]
    NotYetValid,
    #[error("the token lives longer than this use allows")]
    Lifetime,
    #[error("the token lacks its `{0}` claim")]
    MissingClaim(&'static str),
}

/// The endpoints an issuer publishes at `/.well-known/openid-configuration`.
#[derive(Clone, Debug, Deserialize)]
pub struct Discovery {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub jwks_uri: String,
}

/// Fetch `issuer`'s discovery document. The `issuer` inside it must be the
/// one asked for (RFC 8414 §3.3): a document that names another is a proxy
/// or a misconfiguration, and tokens checked against it would be checked
/// against the wrong `iss`.
pub async fn discover(client: &reqwest::Client, issuer: &str) -> Result<Discovery, VerifyError> {
    let url = format!(
        "{}/.well-known/openid-configuration",
        issuer.trim_end_matches('/')
    );
    let body = fetch(client, &url).await?;
    let discovery: Discovery =
        serde_json::from_slice(&body).map_err(|_| VerifyError::Fetch(url))?;
    if discovery.issuer != issuer {
        return Err(VerifyError::IssuerMismatch);
    }
    Ok(discovery)
}

/// One GET, the body read up to `MAX_BODY_BYTES` and no further, so an
/// issuer that streams forever costs a bounded buffer.
async fn fetch(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, VerifyError> {
    let failed = || VerifyError::Fetch(url.to_string());
    let mut response = client.get(url).send().await.map_err(|_| failed())?;
    if !response.status().is_success() {
        return Err(failed());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| failed())? {
        if body.len() + chunk.len() > MAX_BODY_BYTES {
            return Err(failed());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// A published key and the one algorithm it verifies: RSA keys RS256, P-256
/// keys ES256. Pinning the algorithm to the key, rather than reading it from
/// the token, is what keeps a token from choosing how it is checked.
pub struct Key {
    pub decoding: DecodingKey,
    pub algorithm: Algorithm,
}

impl Key {
    /// The key a JWK describes, or `None` for one this server does not use:
    /// an encryption key, another curve, or a key that declares an algorithm
    /// other than the one its type verifies here.
    fn from_jwk(jwk: &Jwk) -> Option<Self> {
        if matches!(jwk.common.public_key_use, Some(PublicKeyUse::Encryption)) {
            return None;
        }
        let (algorithm, declared_as) = match &jwk.algorithm {
            AlgorithmParameters::RSA(_) => (Algorithm::RS256, KeyAlgorithm::RS256),
            AlgorithmParameters::EllipticCurve(params) if params.curve == EllipticCurve::P256 => {
                (Algorithm::ES256, KeyAlgorithm::ES256)
            }
            _ => return None,
        };
        if jwk
            .common
            .key_algorithm
            .is_some_and(|declared| declared != declared_as)
        {
            return None;
        }
        let decoding = DecodingKey::from_jwk(jwk).ok()?;
        Some(Self {
            decoding,
            algorithm,
        })
    }
}

struct Keys {
    /// Every usable key, with the `kid` it was published under.
    published: Vec<(Option<String>, Arc<Key>)>,
    /// Whether a fetch has been attempted at all. The first token fetches
    /// whatever its `kid`, because an empty cache says nothing about any;
    /// from then on only a `kid` the set lacks does, and on the clock below.
    fetched: bool,
    /// When a `kid` the set lacked last caused a fetch, successfully or
    /// not. A failed one counts too: an issuer that is down is retried a
    /// minute later, not once per token that arrives meanwhile.
    refreshed_at_ms: Option<i64>,
}

/// One issuer's signing keys, fetched on first use and again when a token
/// names a `kid` the set lacks, at most once per `JWKS_REFRESH_INTERVAL_MS`.
pub struct Jwks {
    client: reqwest::Client,
    uri: String,
    keys: Mutex<Keys>,
}

impl Jwks {
    pub fn new(client: reqwest::Client, jwks_uri: String) -> Self {
        Self {
            client,
            uri: jwks_uri,
            keys: Mutex::new(Keys {
                published: Vec::new(),
                fetched: false,
                refreshed_at_ms: None,
            }),
        }
    }

    /// The key a token's header names. A token without a `kid` is matched
    /// only when the issuer publishes exactly one key, which is what such an
    /// issuer means by leaving it out.
    pub async fn key(&self, kid: Option<&str>, now_ms: i64) -> Result<Arc<Key>, VerifyError> {
        if !self.keys.lock().unwrap().fetched {
            self.fetch_keys().await?;
        }
        if let Some(key) = self.lookup(kid) {
            return Ok(key);
        }
        let due = {
            let mut keys = self.keys.lock().unwrap();
            let due = keys
                .refreshed_at_ms
                .is_none_or(|at| now_ms.saturating_sub(at) >= JWKS_REFRESH_INTERVAL_MS);
            if due {
                keys.refreshed_at_ms = Some(now_ms);
            }
            due
        };
        if due {
            self.fetch_keys().await?;
            if let Some(key) = self.lookup(kid) {
                return Ok(key);
            }
        }
        Err(VerifyError::UnknownKey)
    }

    fn lookup(&self, kid: Option<&str>) -> Option<Arc<Key>> {
        let keys = self.keys.lock().unwrap();
        match kid {
            Some(kid) => keys
                .published
                .iter()
                .find(|(published, _)| published.as_deref() == Some(kid))
                .map(|(_, key)| key.clone()),
            None => match keys.published.as_slice() {
                [(_, only)] => Some(only.clone()),
                _ => None,
            },
        }
    }

    async fn fetch_keys(&self) -> Result<(), VerifyError> {
        self.keys.lock().unwrap().fetched = true;
        let body = fetch(&self.client, &self.uri).await?;
        let set: JwkSet =
            serde_json::from_slice(&body).map_err(|_| VerifyError::Fetch(self.uri.clone()))?;
        let published = set
            .keys
            .iter()
            .filter_map(|jwk| {
                Key::from_jwk(jwk).map(|key| (jwk.common.key_id.clone(), Arc::new(key)))
            })
            .collect();
        self.keys.lock().unwrap().published = published;
        Ok(())
    }
}

/// Every issuer this server has discovered, each with its JWKS cache.
/// Discovery happens on an issuer's first use and is kept for the life of
/// the process. Callers pass only issuers an operator configured — a trust
/// policy's, the identity provider's — never one a token named on its own,
/// or any URL on the internet could be made to answer a discovery request.
pub struct Issuers {
    client: reqwest::Client,
    known: Mutex<HashMap<String, Arc<Jwks>>>,
}

impl Issuers {
    pub fn new(client: reqwest::Client) -> Self {
        Self {
            client,
            known: Mutex::new(HashMap::new()),
        }
    }

    /// The JWKS cache for `issuer`, discovering it first if this is its
    /// first use. Two first uses at once both discover; the second to finish
    /// keeps the first's cache rather than replacing it.
    pub async fn jwks(&self, issuer: &str) -> Result<Arc<Jwks>, VerifyError> {
        if let Some(jwks) = self.known.lock().unwrap().get(issuer) {
            return Ok(jwks.clone());
        }
        let discovery = discover(&self.client, issuer).await?;
        let fresh = Arc::new(Jwks::new(self.client.clone(), discovery.jwks_uri));
        Ok(self
            .known
            .lock()
            .unwrap()
            .entry(issuer.to_string())
            .or_insert(fresh)
            .clone())
    }
}

/// Which audience a token must name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AudienceRule {
    Exact(String),
}

/// What a token has to say to be accepted at `now_ms`.
#[derive(Clone, Debug)]
pub struct Expect {
    pub issuer: String,
    pub audience: AudienceRule,
    pub now_ms: i64,
    /// The most `exp - iat` may span, when the use cannot tolerate a long
    /// lived token (a fixed-audience workload proof).
    pub max_lifetime_ms: Option<i64>,
}

/// A verified token's claims: the whole set as the issuer wrote it, and the
/// standard ones read into their types. Times are seconds since the epoch,
/// as the JWT carries them.
#[derive(Clone, Debug)]
pub struct Claims {
    pub all: Map<String, Value>,
    pub iss: String,
    pub sub: Option<String>,
    /// `aud` as a list whether the token wrote one string or an array.
    pub aud: Vec<String>,
    pub exp: i64,
    pub iat: Option<i64>,
    pub nbf: Option<i64>,
    pub jti: Option<String>,
    pub nonce: Option<String>,
}

impl Claims {
    fn read(all: Map<String, Value>) -> Result<Self, VerifyError> {
        let text = |name: &str| all.get(name).and_then(Value::as_str).map(str::to_owned);
        let number = |name: &str| all.get(name).and_then(Value::as_i64);
        let iss = text("iss").ok_or(VerifyError::MissingClaim("iss"))?;
        let exp = number("exp").ok_or(VerifyError::MissingClaim("exp"))?;
        let aud = match all.get("aud") {
            Some(Value::String(one)) => vec![one.clone()],
            Some(Value::Array(many)) => many
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
            _ => Vec::new(),
        };
        Ok(Self {
            iss,
            sub: text("sub"),
            aud,
            exp,
            iat: number("iat"),
            nbf: number("nbf"),
            jti: text("jti"),
            nonce: text("nonce"),
            all,
        })
    }

    /// When the token expires, in the server's millisecond clock: the
    /// window a replay cache has to remember its `jti` for.
    pub fn exp_ms(&self) -> i64 {
        self.exp.saturating_mul(1000)
    }

    fn check(&self, expected: &Expect) -> Result<(), VerifyError> {
        if self.iss != expected.issuer {
            return Err(VerifyError::Issuer);
        }
        let AudienceRule::Exact(audience) = &expected.audience;
        if !self.aud.iter().any(|aud| aud == audience) {
            return Err(VerifyError::Audience);
        }
        let now = expected.now_ms.div_euclid(1000);
        if now >= self.exp.saturating_add(LEEWAY_SECS) {
            return Err(VerifyError::Expired);
        }
        if let Some(nbf) = self.nbf {
            if now < nbf.saturating_sub(LEEWAY_SECS) {
                return Err(VerifyError::NotYetValid);
            }
        }
        if let Some(max_lifetime_ms) = expected.max_lifetime_ms {
            let iat = self.iat.ok_or(VerifyError::MissingClaim("iat"))?;
            if self.exp.saturating_sub(iat).saturating_mul(1000) > max_lifetime_ms {
                return Err(VerifyError::Lifetime);
            }
        }
        Ok(())
    }
}

/// Verify `token` against the keys of the issuer `jwks` serves and the claims
/// `expected` asks for. The signature check is `jsonwebtoken`'s with every
/// one of its own claim checks turned off, because those read the system
/// clock; the claims are then checked here against `expected.now_ms`.
pub async fn verify(jwks: &Jwks, token: &str, expected: &Expect) -> Result<Claims, VerifyError> {
    let header = jsonwebtoken::decode_header(token).map_err(|_| VerifyError::Malformed)?;
    if !matches!(header.alg, Algorithm::RS256 | Algorithm::ES256) {
        return Err(VerifyError::Algorithm);
    }
    let key = jwks.key(header.kid.as_deref(), expected.now_ms).await?;
    if key.algorithm != header.alg {
        return Err(VerifyError::Algorithm);
    }
    let mut validation = Validation::new(key.algorithm);
    validation.required_spec_claims.clear();
    validation.validate_exp = false;
    validation.validate_nbf = false;
    validation.validate_aud = false;
    let data = jsonwebtoken::decode::<Map<String, Value>>(token, &key.decoding, &validation)
        .map_err(|error| match error.kind() {
            jsonwebtoken::errors::ErrorKind::InvalidSignature => VerifyError::Signature,
            jsonwebtoken::errors::ErrorKind::InvalidAlgorithm => VerifyError::Algorithm,
            _ => VerifyError::Malformed,
        })?;
    let claims = Claims::read(data.claims)?;
    claims.check(expected)?;
    Ok(claims)
}

/// [`verify`] for a caller on a std thread — the broker runs each connection
/// on one, and `Host::associate` is synchronous — driven on the runtime
/// `handle` belongs to.
pub fn verify_blocking(
    handle: &tokio::runtime::Handle,
    jwks: &Jwks,
    token: &str,
    expected: &Expect,
) -> Result<Claims, VerifyError> {
    handle.block_on(verify(jwks, token, expected))
}
