//! The member and admin APIs on a loopback port, over a `TempServer` and a
//! `FakeIdp`, reached as the app and an admin reach them: with a client that
//! does not follow redirects, so each hop of a sign-in can be looked at.

use std::sync::Arc;

use axum::Router;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use crypto_box::SecretKey;
use reqwest::header::LOCATION;
use reqwest::{Method, Response};
use rowel_core::crypto::ENROLL_CHALLENGE_CONTEXT;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::config::{
    Config, Listen, Members, Oidc, Tenant, DEFAULT_ASK_TIMEOUT_MS, DEFAULT_KEY_IDLE_MS,
};
use crate::identity::testing::FakeIdp;
use crate::state::AppState;
use crate::testing::{TempServer, TENANT};
use crate::{admin, members, packs};

/// The app's loopback listener, as `sync::loopback_redirect_uri` spells it.
pub(super) const REDIRECT: &str = "http://127.0.0.1:4242/auth/callback";
/// The `state` the app sends and expects back.
pub(super) const APP_STATE: &str = "the-apps-own-state";
pub(super) const VERIFIER: &str = "a-pkce-verifier-of-at-least-forty-three-characters";
/// An hour, so `expires_in` reads as 3600.
const SESSION_TTL_MS: u64 = 3_600_000;

/// `BASE64URL(SHA-256(verifier))`, as the app computes its challenge.
pub(super) fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

pub(super) struct Tokens {
    pub access: String,
    pub refresh: String,
}

pub(super) struct Api {
    pub server: TempServer,
    pub state: Arc<AppState>,
    pub idp: FakeIdp,
    pub url: String,
    http: reqwest::Client,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Api {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Api {
    /// A server whose members come from `acme.example`.
    pub async fn start() -> Self {
        Self::with(Some(vec!["acme.example".into()])).await
    }

    /// A server admitting `domains` at its IdP; `None` has no IdP at all.
    pub async fn with(domains: Option<Vec<String>>) -> Self {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let idp = FakeIdp::start().await;
        let server = TempServer::new();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let config = Config {
            tenant: Tenant {
                name: TENANT.into(),
            },
            public_url: url.clone(),
            listen: Listen::default(),
            root_key_file: server.dir.path().join("root.key"),
            members: Members {
                oidc: domains.map(|allowed_domains| Oidc {
                    issuer: idp.issuer().into(),
                    client_id: "rowel-server".into(),
                    client_secret: Zeroizing::new("a-test-secret".into()),
                    allowed_domains,
                }),
            },
            ask_timeout_ms: DEFAULT_ASK_TIMEOUT_MS,
            key_idle_ms: DEFAULT_KEY_IDLE_MS,
            session_ttl_ms: SESSION_TTL_MS,
        };
        let state = AppState::with_config(config, &server.data_dir())
            .await
            .unwrap();
        let router = Router::new()
            .merge(members::router())
            .merge(admin::router())
            .merge(packs::router())
            .with_state(state.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        Self {
            server,
            state,
            idp,
            url,
            http,
            task,
        }
    }

    pub async fn get(&self, url: &str) -> Response {
        self.http.get(url).send().await.unwrap()
    }

    /// The `authorize` URL the app would open, with `overrides` replacing
    /// or adding query pairs.
    pub fn authorize_url(&self, challenge: &str, overrides: &[(&str, &str)]) -> String {
        let mut pairs = vec![
            ("response_type", "code"),
            ("client_id", members::CLIENT_ID),
            ("redirect_uri", REDIRECT),
            ("state", APP_STATE),
            ("code_challenge", challenge),
            ("code_challenge_method", "S256"),
        ];
        for &(name, value) in overrides {
            pairs.retain(|&(held, _)| held != name);
            pairs.push((name, value));
        }
        let mut url = url::Url::parse(&format!("{}/v1/members/auth/authorize", self.url)).unwrap();
        url.query_pairs_mut().extend_pairs(pairs);
        url.into()
    }

    /// The whole browser trip — `authorize`, the IdP, the callback — and
    /// where the server finally sends the browser: the app's redirect.
    pub async fn sign_in_redirect(&self, challenge: &str) -> url::Url {
        let started = self.get(&self.authorize_url(challenge, &[])).await;
        assert_eq!(started.status().as_u16(), 302);
        let at_idp = location(&started);
        assert!(at_idp.starts_with(self.idp.issuer()), "{at_idp}");
        let from_idp = self.get(&at_idp).await;
        assert!(from_idp.status().is_redirection());
        let callback = location(&from_idp);
        assert!(
            callback.starts_with(&format!("{}/v1/members/auth/callback?", self.url)),
            "{callback}"
        );
        let back = self.get(&callback).await;
        assert_eq!(back.status().as_u16(), 302);
        url::Url::parse(&location(&back)).unwrap()
    }

    pub async fn token(&self, form: &[(&str, &str)]) -> Response {
        self.http
            .post(format!("{}/v1/members/auth/token", self.url))
            .form(form)
            .send()
            .await
            .unwrap()
    }

    pub async fn redeem(&self, code: &str, verifier: &str) -> Response {
        self.token(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("code_verifier", verifier),
            ("redirect_uri", REDIRECT),
            ("client_id", members::CLIENT_ID),
        ])
        .await
    }

    pub async fn refresh(&self, refresh: &str) -> Response {
        self.token(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh),
            ("client_id", members::CLIENT_ID),
        ])
        .await
    }

    /// `email` signs in as the app does, end to end.
    pub async fn sign_in(&self, email: &str) -> Tokens {
        self.idp.set_identity(email, "Pat", true);
        let back = self.sign_in_redirect(&challenge(VERIFIER)).await;
        let reply = self.redeem(&code_of(&back), VERIFIER).await;
        assert_eq!(reply.status().as_u16(), 200);
        tokens_of(reply).await
    }

    /// A call under `access` (or none), with `body` as JSON when given.
    pub async fn call(
        &self,
        method: Method,
        path: &str,
        access: Option<&str>,
        body: Option<Value>,
    ) -> Response {
        let mut request = self.http.request(method, format!("{}{path}", self.url));
        if let Some(access) = access {
            request = request.bearer_auth(access);
        }
        if let Some(body) = body {
            request = request.json(&body);
        }
        request.send().await.unwrap()
    }

    /// A challenge issued to `access` for `public_key`, sealed as it comes.
    pub async fn enroll_challenge(&self, access: &str, public_key: &str) -> String {
        let reply = self
            .call(
                Method::POST,
                "/v1/members/enroll/challenge",
                Some(access),
                Some(json!({ "devicePubkey": public_key })),
            )
            .await;
        assert_eq!(reply.status().as_u16(), 200);
        json(reply).await["challenge"].as_str().unwrap().into()
    }

    /// The enrollment of `public_key` under `access`, with `proof` as sent.
    pub async fn enroll_with(
        &self,
        access: &str,
        public_key: &str,
        proof: Option<&str>,
    ) -> Response {
        let mut body = json!({ "devicePubkey": public_key, "deviceName": "Pat's laptop" });
        if let Some(proof) = proof {
            body["proof"] = proof.into();
        }
        self.call(Method::POST, "/v1/members/enroll", Some(access), Some(body))
            .await
    }

    /// `secret`'s device enrolled under `access` as the app enrolls one: a
    /// challenge for its public key, opened with the secret, and what it
    /// opened to sent as the proof.
    pub async fn enroll(&self, access: &str, secret: &SecretKey) -> Response {
        let public_key = public_key_of(secret);
        let sealed = self.enroll_challenge(access, &public_key).await;
        let proof = self.open_challenge(secret, &sealed);
        self.enroll_with(access, &public_key, Some(&proof)).await
    }

    /// The proof `secret` gives for `sealed`, as the app checks a challenge
    /// before answering it: the context, then this server's URL and a NUL,
    /// then the 32 bytes that are the answer, in base64.
    pub fn open_challenge(&self, secret: &SecretKey, sealed: &str) -> String {
        let opened = secret.unseal(&STANDARD.decode(sealed).unwrap()).unwrap();
        let expected = [ENROLL_CHALLENGE_CONTEXT, self.url.as_bytes(), &[0]].concat();
        let challenge = opened
            .strip_prefix(expected.as_slice())
            .expect("a challenge names this server");
        assert_eq!(challenge.len(), 32);
        STANDARD.encode(challenge)
    }
}

/// The base64 public key of `secret`, as the app sends it.
pub(super) fn public_key_of(secret: &SecretKey) -> String {
    STANDARD.encode(secret.public_key().as_bytes())
}

pub(super) fn location(response: &Response) -> String {
    response.headers()[LOCATION].to_str().unwrap().to_string()
}

/// The query pair `name` of `url`.
pub(super) fn param(url: &url::Url, name: &str) -> Option<String> {
    url.query_pairs()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
}

/// The code in the app's redirect, which must echo the app's `state`.
pub(super) fn code_of(back: &url::Url) -> String {
    assert_eq!(param(back, "state").as_deref(), Some(APP_STATE));
    param(back, "code").unwrap_or_else(|| panic!("no code in {back}"))
}

pub(super) async fn tokens_of(reply: Response) -> Tokens {
    let body: Value = reply.json().await.unwrap();
    Tokens {
        access: body["access_token"].as_str().unwrap().into(),
        refresh: body["refresh_token"].as_str().unwrap().into(),
    }
}

pub(super) async fn json(response: Response) -> Value {
    response.json().await.unwrap()
}
