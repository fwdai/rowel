//! A complete-enough OIDC provider in the test process, on a loopback port:
//! discovery, a JWKS, an `authorize` that redirects straight back with a
//! code for whatever identity the test configured, a `token` endpoint that
//! checks PKCE and signs an id token, and `mint` for workload tokens with
//! any claims a test wants. Everything is signed RS256 with the fixture
//! keypair beside this file, which exists for these tests and nothing else.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::extract::{Form, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::prelude::*;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::identity::workload::random_token;

/// The test-only RSA keypair: a PKCS#8 PEM `openssl genrsa 2048` wrote, and
/// the public half as the JWK the fake publishes. Never used outside tests.
const SIGNING_KEY_PEM: &str = include_str!("test-only-signing-key.pem");
const SIGNING_KEY_JWK: &str = include_str!("test-only-signing-key.jwk.json");

/// The `kid` the fake publishes its key under at start.
pub const KID: &str = "test-2026";

/// Who the fake signs in: what `set_identity` set.
#[derive(Clone)]
struct FakeIdentity {
    email: String,
    name: String,
    email_verified: bool,
}

/// A code the fake handed out, and what the token endpoint checks it
/// against.
struct Issued {
    client_id: String,
    redirect_uri: String,
    nonce: Option<String>,
    code_challenge: Option<String>,
}

struct Inner {
    issuer: String,
    identity: Mutex<FakeIdentity>,
    codes: Mutex<HashMap<String, Issued>>,
    /// The kids the one key is published under. Publishing it under a second
    /// kid is how a test stands in for a key rotation.
    kids: Mutex<Vec<String>>,
    jwks_fetches: AtomicUsize,
    signing: EncodingKey,
    public_jwk: Map<String, Value>,
}

pub struct FakeIdp {
    inner: Arc<Inner>,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for FakeIdp {
    fn drop(&mut self) {
        self.server.abort();
    }
}

impl FakeIdp {
    /// Bind a loopback port and serve the provider on the current runtime.
    pub async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a loopback port is free");
        let issuer = format!("http://{}", listener.local_addr().unwrap());
        let public_jwk: Map<String, Value> =
            serde_json::from_str(SIGNING_KEY_JWK).expect("the fixture JWK parses");
        let inner = Arc::new(Inner {
            issuer,
            identity: Mutex::new(FakeIdentity {
                email: "pat@acme.example".into(),
                name: "Pat".into(),
                email_verified: true,
            }),
            codes: Mutex::new(HashMap::new()),
            kids: Mutex::new(vec![KID.to_string()]),
            jwks_fetches: AtomicUsize::new(0),
            signing: EncodingKey::from_rsa_pem(SIGNING_KEY_PEM.as_bytes())
                .expect("the fixture key parses"),
            public_jwk,
        });
        let router = Router::new()
            .route("/.well-known/openid-configuration", get(discovery))
            .route("/jwks.json", get(jwks))
            .route("/authorize", get(authorize))
            .route("/token", post(token))
            .with_state(inner.clone());
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        Self { inner, server }
    }

    pub fn issuer(&self) -> &str {
        &self.inner.issuer
    }

    pub fn jwks_url(&self) -> String {
        format!("{}/jwks.json", self.inner.issuer)
    }

    /// How many times the JWKS has been fetched.
    pub fn jwks_fetches(&self) -> usize {
        self.inner.jwks_fetches.load(Ordering::SeqCst)
    }

    /// Who the next `authorize` signs in.
    pub fn set_identity(&self, email: &str, name: &str, email_verified: bool) {
        *self.inner.identity.lock().unwrap() = FakeIdentity {
            email: email.into(),
            name: name.into(),
            email_verified,
        };
    }

    /// Publish the key under `kid` as well, from the next JWKS fetch on.
    pub fn publish_kid(&self, kid: &str) {
        self.inner.kids.lock().unwrap().push(kid.to_string());
    }

    /// A workload token with `claims` plus `iss`, `aud`, `iat` and `exp`,
    /// under the published `kid`.
    pub fn mint(&self, claims: Map<String, Value>, aud: &str, iat: i64, exp: i64) -> String {
        self.mint_with_kid(KID, claims, aud, iat, exp)
    }

    /// [`Self::mint`] under any `kid`, published or not.
    pub fn mint_with_kid(
        &self,
        kid: &str,
        claims: Map<String, Value>,
        aud: &str,
        iat: i64,
        exp: i64,
    ) -> String {
        self.inner.sign(kid, claims, aud, iat, exp)
    }

    /// Claims shaped like a GitHub Actions token for `repository` at `ref`,
    /// run by `workflow`, with a fresh `jti`.
    pub fn github_claims(repository: &str, git_ref: &str, workflow: &str) -> Map<String, Value> {
        let mut claims = Map::new();
        claims.insert("repository".into(), json!(repository));
        claims.insert("ref".into(), json!(git_ref));
        claims.insert("workflow".into(), json!(workflow));
        claims.insert(
            "job_workflow_ref".into(),
            json!(format!(
                "{repository}/.github/workflows/{workflow}@{git_ref}"
            )),
        );
        claims.insert(
            "sub".into(),
            json!(format!("repo:{repository}:ref:{git_ref}")),
        );
        claims.insert("jti".into(), json!(random_token()));
        claims
    }
}

impl Inner {
    fn sign(
        &self,
        kid: &str,
        mut claims: Map<String, Value>,
        aud: &str,
        iat: i64,
        exp: i64,
    ) -> String {
        claims.insert("iss".into(), json!(self.issuer));
        claims.insert("aud".into(), json!(aud));
        claims.insert("iat".into(), json!(iat));
        claims.insert("exp".into(), json!(exp));
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(kid.to_string());
        jsonwebtoken::encode(&header, &claims, &self.signing).expect("the fixture key signs")
    }
}

async fn discovery(State(idp): State<Arc<Inner>>) -> Json<Value> {
    Json(json!({
        "issuer": idp.issuer,
        "authorization_endpoint": format!("{}/authorize", idp.issuer),
        "token_endpoint": format!("{}/token", idp.issuer),
        "jwks_uri": format!("{}/jwks.json", idp.issuer),
        "response_types_supported": ["code"],
        "id_token_signing_alg_values_supported": ["RS256"],
    }))
}

async fn jwks(State(idp): State<Arc<Inner>>) -> Json<Value> {
    idp.jwks_fetches.fetch_add(1, Ordering::SeqCst);
    let keys: Vec<Value> = idp
        .kids
        .lock()
        .unwrap()
        .iter()
        .map(|kid| {
            let mut jwk = idp.public_jwk.clone();
            jwk.insert("kid".into(), json!(kid));
            Value::Object(jwk)
        })
        .collect();
    Json(json!({ "keys": keys }))
}

/// No login page: the configured identity is signed in at once and the
/// browser is sent back with a code and the caller's `state`.
async fn authorize(
    State(idp): State<Arc<Inner>>,
    Query(query): Query<HashMap<String, String>>,
) -> axum::response::Response {
    let (Some(client_id), Some(redirect_uri)) = (query.get("client_id"), query.get("redirect_uri"))
    else {
        return (
            StatusCode::BAD_REQUEST,
            "client_id and redirect_uri are required",
        )
            .into_response();
    };
    if query.get("response_type").map(String::as_str) != Some("code") {
        return (StatusCode::BAD_REQUEST, "response_type must be code").into_response();
    }
    let code = random_token();
    idp.codes.lock().unwrap().insert(
        code.clone(),
        Issued {
            client_id: client_id.clone(),
            redirect_uri: redirect_uri.clone(),
            nonce: query.get("nonce").cloned(),
            code_challenge: query.get("code_challenge").cloned(),
        },
    );
    let mut back = url::Url::parse(redirect_uri).expect("a test redirect URI parses");
    back.query_pairs_mut().append_pair("code", &code);
    if let Some(state) = query.get("state") {
        back.query_pairs_mut().append_pair("state", state);
    }
    Redirect::to(back.as_str()).into_response()
}

async fn token(
    State(idp): State<Arc<Inner>>,
    Form(form): Form<HashMap<String, String>>,
) -> axum::response::Response {
    let refuse = |why: &str| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "invalid_grant", "error_description": why })),
        )
            .into_response()
    };
    if form.get("grant_type").map(String::as_str) != Some("authorization_code") {
        return refuse("grant_type must be authorization_code");
    }
    let Some(issued) = form
        .get("code")
        .and_then(|code| idp.codes.lock().unwrap().remove(code))
    else {
        return refuse("the code is not one this provider issued");
    };
    if form.get("client_id") != Some(&issued.client_id)
        || form.get("redirect_uri") != Some(&issued.redirect_uri)
    {
        return refuse("the code was issued to another client or redirect");
    }
    if let Some(challenge) = &issued.code_challenge {
        let verified = form.get("code_verifier").is_some_and(|verifier| {
            BASE64_URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())) == *challenge
        });
        if !verified {
            return refuse("the code_verifier does not match the challenge");
        }
    }
    let identity = idp.identity.lock().unwrap().clone();
    let now = chrono::Utc::now().timestamp();
    let mut claims = Map::new();
    claims.insert("sub".into(), json!(format!("sub-{}", identity.email)));
    claims.insert("email".into(), json!(identity.email));
    claims.insert("email_verified".into(), json!(identity.email_verified));
    claims.insert("name".into(), json!(identity.name));
    if let Some(nonce) = issued.nonce {
        claims.insert("nonce".into(), json!(nonce));
    }
    let id_token = idp.sign(KID, claims, &issued.client_id, now, now + 3600);
    Json(json!({
        "access_token": random_token(),
        "token_type": "Bearer",
        "expires_in": 3600,
        "id_token": id_token,
    }))
    .into_response()
}
