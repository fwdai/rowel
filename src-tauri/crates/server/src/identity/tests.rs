//! The identity slice against the in-process `FakeIdp`; nothing here leaves
//! the loopback interface.

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::prelude::*;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use rand::RngCore;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use super::jwt::{self, AudienceRule, Expect, Issuers, VerifyError};
use super::members::{allowed, IdpClient, LoginError};
use super::testing::{FakeIdp, KID};
use super::workload::{
    expected_audience, pattern_matches, Challenges, MemoryReplay, Proof, ProofError, TrustPolicy,
    Verifier, OIDC,
};

const TENANT: &str = "rowel:acme";

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

fn session_key() -> String {
    let mut key = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut key);
    BASE64_STANDARD.encode(key)
}

fn proof(token: &str, nonce: Option<&str>) -> Proof {
    Proof {
        kind: OIDC.to_string(),
        token: token.to_string(),
        nonce: nonce.map(str::to_owned),
    }
}

fn github_policy(idp: &FakeIdp, fixed_audience: bool) -> TrustPolicy {
    let claims: BTreeMap<String, String> = [
        ("repository", "acme/deploy"),
        ("ref", "refs/heads/main"),
        ("workflow", "release.yml"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    TrustPolicy {
        issuer: idp.issuer().to_string(),
        claims,
        fixed_audience,
    }
}

fn deploy_claims() -> Map<String, Value> {
    FakeIdp::github_claims("acme/deploy", "refs/heads/main", "release.yml")
}

/// A server's worth of verification state around one fake issuer.
struct Harness {
    idp: FakeIdp,
    issuers: Issuers,
    challenges: Challenges,
    replay: MemoryReplay,
}

impl Harness {
    async fn start() -> Self {
        Self {
            idp: FakeIdp::start().await,
            issuers: Issuers::new(jwt::http_client()),
            challenges: Challenges::default(),
            replay: MemoryReplay::default(),
        }
    }

    fn verifier(&self) -> Verifier<'_> {
        Verifier {
            issuers: &self.issuers,
            challenges: &self.challenges,
            replay: &self.replay,
        }
    }

    /// A ten-minute token for `aud` minted now with GitHub-shaped claims.
    fn token(&self, claims: Map<String, Value>, aud: &str) -> String {
        let now = now_ms() / 1000;
        self.idp.mint(claims, aud, now, now + 600)
    }
}

#[tokio::test]
async fn a_key_bound_token_verifies_and_matches_a_github_policy() {
    let h = Harness::start().await;
    let key = session_key();
    let audience = expected_audience(TENANT, &key).unwrap();
    assert!(audience.starts_with("rowel:acme:"));
    assert_eq!(audience.len(), TENANT.len() + 1 + 64);
    let token = h.token(deploy_claims(), &audience);
    let policies = [
        TrustPolicy {
            claims: [("repository".to_string(), "acme/other".to_string())].into(),
            ..github_policy(&h.idp, false)
        },
        github_policy(&h.idp, false),
    ];

    let verified = h
        .verifier()
        .verify_proof(&proof(&token, None), &key, TENANT, &policies, now_ms())
        .await
        .unwrap();
    assert_eq!(verified.policy_index, 1);
    assert_eq!(verified.claims.all["repository"], "acme/deploy");
    assert_eq!(verified.claims.aud, vec![audience]);
}

#[tokio::test]
async fn the_same_token_on_another_session_key_fails_the_audience() {
    let h = Harness::start().await;
    let token = h.token(
        deploy_claims(),
        &expected_audience(TENANT, &session_key()).unwrap(),
    );
    let policies = [github_policy(&h.idp, false)];
    let error = h
        .verifier()
        .verify_proof(
            &proof(&token, None),
            &session_key(),
            TENANT,
            &policies,
            now_ms(),
        )
        .await
        .unwrap_err();
    assert_eq!(error, ProofError::Invalid);
}

#[tokio::test]
async fn a_fixed_audience_policy_needs_a_live_nonce_and_a_short_token() {
    let h = Harness::start().await;
    let key = session_key();
    let policies = [github_policy(&h.idp, true)];
    let now = now_ms();
    let token = h.token(deploy_claims(), TENANT);
    let verifier = h.verifier();

    let no_nonce = verifier
        .verify_proof(&proof(&token, None), &key, TENANT, &policies, now)
        .await
        .unwrap_err();
    assert_eq!(no_nonce, ProofError::Invalid);

    let made_up = verifier
        .verify_proof(
            &proof(&token, Some("made-up")),
            &key,
            TENANT,
            &policies,
            now,
        )
        .await
        .unwrap_err();
    assert_eq!(made_up, ProofError::Invalid);

    let challenge = h.challenges.mint(TENANT, now);
    assert_eq!(challenge.audience, TENANT);
    let verified = verifier
        .verify_proof(
            &proof(&token, Some(&challenge.nonce)),
            &key,
            TENANT,
            &policies,
            now,
        )
        .await
        .unwrap();
    assert_eq!(verified.policy_index, 0);

    // The nonce went with the first use, so the same proof is refused
    // before the replay cache gets to say so.
    let again = verifier
        .verify_proof(
            &proof(&token, Some(&challenge.nonce)),
            &key,
            TENANT,
            &policies,
            now,
        )
        .await
        .unwrap_err();
    assert_eq!(again, ProofError::Invalid);

    let long = h
        .idp
        .mint(deploy_claims(), TENANT, now / 1000, now / 1000 + 20 * 60);
    let fresh = h.challenges.mint(TENANT, now);
    let too_long = verifier
        .verify_proof(
            &proof(&long, Some(&fresh.nonce)),
            &key,
            TENANT,
            &policies,
            now,
        )
        .await
        .unwrap_err();
    assert_eq!(too_long, ProofError::Invalid);

    let mut without_jti = deploy_claims();
    without_jti.remove("jti");
    let anonymous = h.token(without_jti, TENANT);
    let fresh = h.challenges.mint(TENANT, now);
    let no_jti = verifier
        .verify_proof(
            &proof(&anonymous, Some(&fresh.nonce)),
            &key,
            TENANT,
            &policies,
            now,
        )
        .await
        .unwrap_err();
    assert_eq!(no_jti, ProofError::Invalid);
}

#[tokio::test]
async fn a_replayed_jti_is_refused() {
    let h = Harness::start().await;
    let key = session_key();
    let audience = expected_audience(TENANT, &key).unwrap();
    let policies = [github_policy(&h.idp, false)];
    let verifier = h.verifier();

    let token = h.token(deploy_claims(), &audience);
    verifier
        .verify_proof(&proof(&token, None), &key, TENANT, &policies, now_ms())
        .await
        .unwrap();
    let replayed = verifier
        .verify_proof(&proof(&token, None), &key, TENANT, &policies, now_ms())
        .await
        .unwrap_err();
    assert_eq!(replayed, ProofError::Replayed);

    // Without a `jti` the token itself is the key, so it too is once-only.
    let mut without_jti = deploy_claims();
    without_jti.remove("jti");
    let anonymous = h.token(without_jti, &audience);
    verifier
        .verify_proof(&proof(&anonymous, None), &key, TENANT, &policies, now_ms())
        .await
        .unwrap();
    let replayed = verifier
        .verify_proof(&proof(&anonymous, None), &key, TENANT, &policies, now_ms())
        .await
        .unwrap_err();
    assert_eq!(replayed, ProofError::Replayed);
}

// A token is accepted until `exp` plus the leeway, so its `jti` must be
// remembered that long: past `exp` but inside the leeway, a second
// presentation is still a replay.
#[tokio::test]
async fn a_jti_is_remembered_for_the_whole_leeway() {
    let h = Harness::start().await;
    let key = session_key();
    let audience = expected_audience(TENANT, &key).unwrap();
    let policies = [github_policy(&h.idp, false)];
    let verifier = h.verifier();

    let now = now_ms() / 1000;
    let token = h.idp.mint(deploy_claims(), &audience, now - 120, now);
    let inside_leeway = (now + jwt::LEEWAY_SECS / 2) * 1000;
    verifier
        .verify_proof(&proof(&token, None), &key, TENANT, &policies, inside_leeway)
        .await
        .unwrap();
    let replayed = verifier
        .verify_proof(
            &proof(&token, None),
            &key,
            TENANT,
            &policies,
            inside_leeway + 1_000,
        )
        .await
        .unwrap_err();
    assert_eq!(replayed, ProofError::Replayed);
}

#[tokio::test]
async fn a_token_from_an_issuer_no_policy_names_or_matching_none_is_said_so() {
    let h = Harness::start().await;
    let key = session_key();
    let audience = expected_audience(TENANT, &key).unwrap();
    let token = h.token(deploy_claims(), &audience);

    let other = [TrustPolicy {
        issuer: "https://token.actions.githubusercontent.com".into(),
        ..github_policy(&h.idp, false)
    }];
    let error = h
        .verifier()
        .verify_proof(&proof(&token, None), &key, TENANT, &other, now_ms())
        .await
        .unwrap_err();
    assert_eq!(error, ProofError::NoPolicy);

    let strict = [TrustPolicy {
        claims: [("ref".to_string(), "refs/heads/release".to_string())].into(),
        ..github_policy(&h.idp, false)
    }];
    let error = h
        .verifier()
        .verify_proof(&proof(&token, None), &key, TENANT, &strict, now_ms())
        .await
        .unwrap_err();
    assert_eq!(error, ProofError::Unmatched);

    let wrong_kind = Proof {
        kind: "spiffe".into(),
        ..proof(&token, None)
    };
    let error = h
        .verifier()
        .verify_proof(&wrong_kind, &key, TENANT, &strict, now_ms())
        .await
        .unwrap_err();
    assert_eq!(error, ProofError::Invalid);
}

#[test]
fn glob_patterns_match_as_a_shell_would_and_plain_ones_exactly() {
    assert!(pattern_matches("refs/heads/main", "refs/heads/main"));
    assert!(!pattern_matches("refs/heads/main", "refs/heads/main2"));
    assert!(!pattern_matches("refs/heads/*", "refs/heads/main"));
    assert!(pattern_matches("glob:refs/heads/*", "refs/heads/main"));
    assert!(pattern_matches("glob:refs/heads/*", "refs/heads/"));
    assert!(pattern_matches("glob:refs/tags/v*.*", "refs/tags/v1.2.3"));
    assert!(!pattern_matches("glob:refs/tags/v*.*", "refs/tags/v1"));
    assert!(pattern_matches("glob:*acme*", "repo:acme/deploy"));
    assert!(pattern_matches("glob:*", ""));
    assert!(!pattern_matches("glob:a*b", "acb_"));
    assert!(pattern_matches("glob:a**b", "ab"));

    let policy = TrustPolicy {
        issuer: "x".into(),
        claims: [
            ("ref".to_string(), "glob:refs/tags/v*".to_string()),
            ("ref_protected".to_string(), "true".to_string()),
        ]
        .into(),
        fixed_audience: false,
    };
    let mut claims = Map::new();
    claims.insert("ref".into(), json!("refs/tags/v2.0"));
    claims.insert("ref_protected".into(), json!(true));
    assert!(policy.matches(&claims));
    claims.insert("ref_protected".into(), json!(false));
    assert!(!policy.matches(&claims));
    claims.remove("ref_protected");
    assert!(!policy.matches(&claims));
}

#[tokio::test]
async fn alg_none_and_hmac_tokens_are_refused() {
    let h = Harness::start().await;
    let key = session_key();
    let audience = expected_audience(TENANT, &key).unwrap();
    let now = now_ms() / 1000;
    let mut claims = deploy_claims();
    claims.insert("iss".into(), json!(h.idp.issuer()));
    claims.insert("aud".into(), json!(audience));
    claims.insert("iat".into(), json!(now));
    claims.insert("exp".into(), json!(now + 600));
    let jwks = h.issuers.jwks(h.idp.issuer()).await.unwrap();
    let expected = Expect {
        issuer: h.idp.issuer().to_string(),
        audience: AudienceRule::Exact(audience),
        now_ms: now_ms(),
        max_lifetime_ms: None,
    };

    let unsigned = format!(
        "{}.{}.",
        BASE64_URL_SAFE_NO_PAD.encode(r#"{"alg":"none","typ":"JWT"}"#),
        BASE64_URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap())
    );
    assert_eq!(
        jwt::verify(&jwks, &unsigned, &expected).await.unwrap_err(),
        VerifyError::Malformed
    );

    let mut header = Header::new(Algorithm::HS256);
    header.kid = Some(KID.to_string());
    let hmac =
        jsonwebtoken::encode(&header, &claims, &EncodingKey::from_secret(b"shared")).unwrap();
    assert_eq!(
        jwt::verify(&jwks, &hmac, &expected).await.unwrap_err(),
        VerifyError::Algorithm
    );

    let policies = [github_policy(&h.idp, false)];
    for token in [unsigned, hmac] {
        let error = h
            .verifier()
            .verify_proof(&proof(&token, None), &key, TENANT, &policies, now_ms())
            .await
            .unwrap_err();
        assert_eq!(error, ProofError::Invalid);
    }
}

#[tokio::test]
async fn an_unknown_kid_refreshes_the_jwks_once_a_minute() {
    let h = Harness::start().await;
    let jwks = h.issuers.jwks(h.idp.issuer()).await.unwrap();
    let t0 = now_ms();
    let now = t0 / 1000;
    let expect_at = |at_ms: i64| Expect {
        issuer: h.idp.issuer().to_string(),
        audience: AudienceRule::Exact(TENANT.to_string()),
        now_ms: at_ms,
        max_lifetime_ms: None,
    };
    assert_eq!(h.idp.jwks_fetches(), 0);

    let known = h.idp.mint(deploy_claims(), TENANT, now, now + 600);
    jwt::verify(&jwks, &known, &expect_at(t0)).await.unwrap();
    assert_eq!(h.idp.jwks_fetches(), 1);

    let rotated = h
        .idp
        .mint_with_kid("rotated", deploy_claims(), TENANT, now, now + 600);
    assert_eq!(
        jwt::verify(&jwks, &rotated, &expect_at(t0))
            .await
            .unwrap_err(),
        VerifyError::UnknownKey
    );
    assert_eq!(h.idp.jwks_fetches(), 2);

    let made_up = h
        .idp
        .mint_with_kid("made-up", deploy_claims(), TENANT, now, now + 600);
    assert_eq!(
        jwt::verify(&jwks, &made_up, &expect_at(t0 + 10_000))
            .await
            .unwrap_err(),
        VerifyError::UnknownKey
    );
    assert_eq!(h.idp.jwks_fetches(), 2);

    h.idp.publish_kid("rotated");
    assert_eq!(
        jwt::verify(&jwks, &rotated, &expect_at(t0 + 30_000))
            .await
            .unwrap_err(),
        VerifyError::UnknownKey
    );
    assert_eq!(h.idp.jwks_fetches(), 2);
    jwt::verify(&jwks, &rotated, &expect_at(t0 + 61_000))
        .await
        .unwrap();
    assert_eq!(h.idp.jwks_fetches(), 3);
}

#[tokio::test]
async fn expiry_and_lifetime_follow_the_callers_clock() {
    let h = Harness::start().await;
    let jwks = h.issuers.jwks(h.idp.issuer()).await.unwrap();
    let now = now_ms() / 1000;
    let token = h.idp.mint(deploy_claims(), TENANT, now, now + 600);
    let expect = |at_s: i64, max_lifetime_ms: Option<i64>| Expect {
        issuer: h.idp.issuer().to_string(),
        audience: AudienceRule::Exact(TENANT.to_string()),
        now_ms: at_s * 1000,
        max_lifetime_ms,
    };

    let claims = jwt::verify(&jwks, &token, &expect(now, None))
        .await
        .unwrap();
    assert_eq!(
        claims.accepted_until_ms(),
        (now + 600 + jwt::LEEWAY_SECS) * 1000
    );
    assert_eq!(claims.iat, Some(now));
    assert_eq!(
        claims.sub.as_deref(),
        Some("repo:acme/deploy:ref:refs/heads/main")
    );
    assert_eq!(
        jwt::verify(&jwks, &token, &expect(now + 700, None))
            .await
            .unwrap_err(),
        VerifyError::Expired
    );
    assert_eq!(
        jwt::verify(&jwks, &token, &expect(now, Some(300_000)))
            .await
            .unwrap_err(),
        VerifyError::Lifetime
    );
    let other_issuer = Expect {
        issuer: "https://elsewhere.example".into(),
        ..expect(now, None)
    };
    assert_eq!(
        jwt::verify(&jwks, &token, &other_issuer).await.unwrap_err(),
        VerifyError::Issuer
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verify_blocking_runs_on_a_std_thread() {
    let h = Harness::start().await;
    let jwks = h.issuers.jwks(h.idp.issuer()).await.unwrap();
    let token = h.token(deploy_claims(), TENANT);
    let expected = Expect {
        issuer: h.idp.issuer().to_string(),
        audience: AudienceRule::Exact(TENANT.to_string()),
        now_ms: now_ms(),
        max_lifetime_ms: None,
    };
    let handle = tokio::runtime::Handle::current();
    let claims =
        std::thread::spawn(move || jwt::verify_blocking(&handle, &jwks, &token, &expected))
            .join()
            .unwrap()
            .unwrap();
    assert_eq!(claims.all["workflow"], "release.yml");
}

#[test]
fn challenges_are_single_use_and_expire() {
    let challenges = Challenges::default();
    let now = 1_700_000_000_000;
    let a = challenges.mint(TENANT, now);
    let b = challenges.mint(TENANT, now);
    assert_ne!(a.nonce, b.nonce);
    assert!(challenges.consume(&a.nonce, now + 1000));
    assert!(!challenges.consume(&a.nonce, now + 1000));
    assert!(!challenges.consume(&b.nonce, now + 6 * 60 * 1000));
    assert!(!challenges.consume("never-minted", now));
}

#[test]
fn the_expected_audience_is_the_tenants_and_the_keys_digest() {
    let key = BASE64_STANDARD.encode([0u8; 32]);
    let audience = expected_audience(TENANT, &key).unwrap();
    assert_eq!(
        audience,
        "rowel:acme:66687aadf862bd776c8fc18b8e9f8e20089714856ee233b3902a591d0d5f2925"
    );
    assert!(expected_audience(TENANT, "not base64!").is_none());
    assert!(expected_audience(TENANT, &BASE64_STANDARD.encode([0u8; 16])).is_none());
}

/// A browser that stops at the IdP's redirect, so the test can read the
/// code the way the server's callback would receive it.
fn browser() -> reqwest::Client {
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
}

/// Follow `authorize_url` through the fake's sign-in and return the `code`
/// and `state` it sends back.
async fn sign_in_at(authorize_url: &str) -> (String, String) {
    let response = browser().get(authorize_url).send().await.unwrap();
    assert!(response.status().is_redirection(), "{}", response.status());
    let location = response.headers()[reqwest::header::LOCATION]
        .to_str()
        .unwrap()
        .to_string();
    let back = url::Url::parse(&location).unwrap();
    assert!(back.as_str().starts_with(CALLBACK));
    let param = |name: &str| {
        back.query_pairs()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.into_owned())
            .unwrap()
    };
    (param("code"), param("state"))
}

const CALLBACK: &str = "http://127.0.0.1:9/v1/members/auth/callback";

#[tokio::test]
async fn a_member_signs_in_through_the_idp_end_to_end() {
    let idp = FakeIdp::start().await;
    let server = IdpClient::new(
        jwt::http_client(),
        idp.issuer(),
        "rowel-server",
        None,
        CALLBACK,
    )
    .await
    .unwrap();
    let now = now_ms();

    let pending = server.start_login(now);
    let authorize = url::Url::parse(&pending.authorize_url).unwrap();
    assert!(pending
        .authorize_url
        .starts_with(&format!("{}/authorize?", idp.issuer())));
    let params: std::collections::HashMap<_, _> = authorize.query_pairs().into_owned().collect();
    assert_eq!(params["response_type"], "code");
    assert_eq!(params["client_id"], "rowel-server");
    assert_eq!(params["redirect_uri"], CALLBACK);
    assert_eq!(params["scope"], "openid email profile");
    assert_eq!(params["state"], pending.state);
    assert_eq!(params["nonce"], pending.nonce);
    assert_eq!(params["code_challenge_method"], "S256");
    assert_eq!(
        params["code_challenge"],
        BASE64_URL_SAFE_NO_PAD.encode(Sha256::digest(pending.code_verifier.as_bytes()))
    );

    let (code, state) = sign_in_at(&pending.authorize_url).await;
    assert_eq!(state, pending.state);
    let identity = server.finish_login(&state, &code, now).await.unwrap();
    assert_eq!(identity.email, "pat@acme.example");
    assert_eq!(identity.name.as_deref(), Some("Pat"));
    assert_eq!(identity.subject, "sub-pat@acme.example");

    // The state went with the login that finished.
    assert_eq!(
        server.finish_login(&state, &code, now).await.unwrap_err(),
        LoginError::UnknownState
    );
}

#[tokio::test]
async fn a_wrong_state_a_tampered_nonce_or_an_unverified_email_fails_the_login() {
    let idp = FakeIdp::start().await;
    let server = IdpClient::new(
        jwt::http_client(),
        idp.issuer(),
        "rowel-server",
        None,
        CALLBACK,
    )
    .await
    .unwrap();
    let now = now_ms();

    let pending = server.start_login(now);
    let (code, _) = sign_in_at(&pending.authorize_url).await;
    assert_eq!(
        server
            .finish_login("someone-elses", &code, now)
            .await
            .unwrap_err(),
        LoginError::UnknownState
    );
    assert_eq!(
        server
            .finish_login(&pending.state, &code, now + 11 * 60 * 1000)
            .await
            .unwrap_err(),
        LoginError::UnknownState
    );

    // The IdP is sent to with another nonce than the one the server holds,
    // as a request forged from a captured authorize URL would be.
    let pending = server.start_login(now);
    let mut tampered = url::Url::parse(&pending.authorize_url).unwrap();
    let pairs: Vec<(String, String)> = tampered
        .query_pairs()
        .map(|(k, v)| {
            let v = if k == "nonce" {
                "tampered".into()
            } else {
                v.into_owned()
            };
            (k.into_owned(), v)
        })
        .collect();
    tampered.query_pairs_mut().clear().extend_pairs(pairs);
    let (code, state) = sign_in_at(tampered.as_str()).await;
    assert_eq!(
        server.finish_login(&state, &code, now).await.unwrap_err(),
        LoginError::Nonce
    );

    idp.set_identity("guest@acme.example", "Guest", false);
    let pending = server.start_login(now);
    let (code, state) = sign_in_at(&pending.authorize_url).await;
    assert_eq!(
        server.finish_login(&state, &code, now).await.unwrap_err(),
        LoginError::Email
    );

    // A code the IdP never issued is refused at the exchange, and the
    // server says no more than the IdP's status.
    let pending = server.start_login(now);
    assert_eq!(
        server
            .finish_login(&pending.state, "never-issued", now)
            .await
            .unwrap_err(),
        LoginError::Exchange(400)
    );
}

#[tokio::test]
async fn discovery_insists_on_the_configured_issuer() {
    let idp = FakeIdp::start().await;
    let client = jwt::http_client();
    let discovery = jwt::discover(&client, idp.issuer()).await.unwrap();
    assert_eq!(discovery.jwks_uri, idp.jwks_url());
    assert_eq!(
        jwt::discover(&client, &format!("{}/", idp.issuer()))
            .await
            .unwrap_err(),
        VerifyError::IssuerMismatch
    );
    assert!(matches!(
        jwt::discover(&client, &format!("{}/tenant", idp.issuer())).await,
        Err(VerifyError::Fetch(_))
    ));
}

#[test]
fn allowed_domains_are_compared_without_case_and_an_empty_list_allows_any() {
    let domains = vec!["acme.example".to_string(), "Acme.co".to_string()];
    assert!(allowed("pat@acme.example", &domains));
    assert!(allowed("pat@ACME.CO", &domains));
    assert!(!allowed("pat@acme.example.evil", &domains));
    assert!(!allowed("pat@evil.example", &domains));
    assert!(!allowed("not-an-address", &domains));
    assert!(allowed("anyone@anywhere.example", &[]));
}
