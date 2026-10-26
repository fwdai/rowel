//! The company's identity provider, behind the server. The app signs in
//! against the server as an OAuth2 public client (`rowel-app`, PKCE, a
//! loopback redirect; `docs/server-api.md`), and the server is in turn an
//! OIDC relying party to the IdP: it sends the person there, takes the code
//! back at `<publicUrl>/v1/members/auth/callback`, exchanges it, verifies
//! the id token and learns who signed in. One flow in the app covers every
//! company, and the IdP's tokens never leave the server.
//!
//! Each login in flight is kept by its `state` in memory for ten minutes:
//! a person who walks away from the IdP's page leaves nothing behind.

use std::collections::HashMap;
use std::sync::Mutex;

use base64::prelude::*;
use rand::RngCore;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

use super::jwt::{self, discover, AudienceRule, Discovery, Expect, Jwks, VerifyError};
use super::workload::random_token;

/// What the server asks the IdP for: the subject, an email and a name.
pub const SCOPE: &str = "openid email profile";

/// How long a login may take between the redirect to the IdP and the code
/// coming back.
pub const LOGIN_TTL_MS: i64 = 10 * 60 * 1000;

/// The most logins kept in flight. Starting one is unauthenticated, so the
/// map is bounded; past this the oldest is dropped and that person signs in
/// again.
const MAX_PENDING_LOGINS: usize = 10_000;

/// Why a login did not finish. `Exchange` carries the IdP's status and
/// nothing of its body, which could echo the code.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LoginError {
    /// Also an expired one: a state that is no longer held is as unknown as
    /// one that never was, and telling the two apart tells a guesser which
    /// states were real.
    #[error("the login is not one in progress")]
    UnknownState,
    #[error("the identity provider refused the code exchange ({0})")]
    Exchange(u16),
    #[error("the id token was refused: {0}")]
    IdToken(#[from] VerifyError),
    #[error("the id token answers another login")]
    Nonce,
    #[error("the identity provider did not vouch for an email address")]
    Email,
}

/// A login started: what the browser is sent to, and what the server holds
/// to recognise the answer.
#[derive(Clone, Debug)]
pub struct Pending {
    pub state: String,
    pub nonce: String,
    pub code_verifier: String,
    pub authorize_url: String,
}

/// Who signed in, as the id token says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub subject: String,
    pub email: String,
    pub name: Option<String>,
}

struct InFlight {
    nonce: String,
    code_verifier: Zeroizing<String>,
    started_ms: i64,
    expires_at_ms: i64,
}

/// The server's side of the company's IdP: discovered once, keys cached.
pub struct IdpClient {
    client: reqwest::Client,
    issuer: String,
    client_id: String,
    client_secret: Option<Zeroizing<String>>,
    redirect_uri: String,
    authorization_endpoint: url::Url,
    token_endpoint: String,
    jwks: Jwks,
    pending: Mutex<HashMap<String, InFlight>>,
}

#[derive(Deserialize)]
struct TokenReply {
    id_token: Zeroizing<String>,
}

impl IdpClient {
    /// Discover `issuer` and get ready to send people there. `redirect_uri`
    /// is the server's own callback, registered at the IdP; `client_secret`
    /// is `None` for an IdP that registered the server as a public client.
    pub async fn new(
        client: reqwest::Client,
        issuer: &str,
        client_id: &str,
        client_secret: Option<&str>,
        redirect_uri: &str,
    ) -> Result<Self, VerifyError> {
        let Discovery {
            authorization_endpoint,
            token_endpoint,
            jwks_uri,
            ..
        } = discover(&client, issuer).await?;
        let authorization_endpoint = url::Url::parse(&authorization_endpoint)
            .map_err(|_| VerifyError::Fetch(authorization_endpoint))?;
        Ok(Self {
            jwks: Jwks::new(client.clone(), jwks_uri),
            client,
            issuer: issuer.to_string(),
            client_id: client_id.to_string(),
            client_secret: client_secret.map(|secret| Zeroizing::new(secret.to_string())),
            redirect_uri: redirect_uri.to_string(),
            authorization_endpoint,
            token_endpoint,
            pending: Mutex::new(HashMap::new()),
        })
    }

    pub fn issuer(&self) -> &str {
        &self.issuer
    }

    /// Start a login: a fresh `state`, `nonce` and PKCE verifier, kept until
    /// `now_ms + LOGIN_TTL_MS`, and the IdP's authorize URL carrying them.
    pub fn start_login(&self, now_ms: i64) -> Pending {
        let state = random_token();
        let nonce = random_token();
        let mut verifier_bytes = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut verifier_bytes);
        let code_verifier = Zeroizing::new(BASE64_URL_SAFE_NO_PAD.encode(verifier_bytes));
        let code_challenge =
            BASE64_URL_SAFE_NO_PAD.encode(Sha256::digest(code_verifier.as_bytes()));

        let mut authorize_url = self.authorization_endpoint.clone();
        authorize_url
            .query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", &self.client_id)
            .append_pair("redirect_uri", &self.redirect_uri)
            .append_pair("scope", SCOPE)
            .append_pair("state", &state)
            .append_pair("nonce", &nonce)
            .append_pair("code_challenge", &code_challenge)
            .append_pair("code_challenge_method", "S256");

        let mut pending = self.pending.lock().unwrap();
        pending.retain(|_, login| login.expires_at_ms > now_ms);
        if pending.len() >= MAX_PENDING_LOGINS {
            if let Some(oldest) = pending
                .iter()
                .min_by_key(|(_, login)| login.started_ms)
                .map(|(state, _)| state.clone())
            {
                pending.remove(&oldest);
            }
        }
        pending.insert(
            state.clone(),
            InFlight {
                nonce: nonce.clone(),
                code_verifier: code_verifier.clone(),
                started_ms: now_ms,
                expires_at_ms: now_ms + LOGIN_TTL_MS,
            },
        );

        Pending {
            state,
            nonce,
            code_verifier: code_verifier.to_string(),
            authorize_url: authorize_url.into(),
        }
    }

    /// Finish the login the IdP redirected back for: redeem `state`,
    /// exchange `code` and read who it was from the id token.
    pub async fn finish_login(
        &self,
        state: &str,
        code: &str,
        now_ms: i64,
    ) -> Result<Identity, LoginError> {
        let login = self
            .pending
            .lock()
            .unwrap()
            .remove(state)
            .filter(|login| login.expires_at_ms > now_ms)
            .ok_or(LoginError::UnknownState)?;

        let mut form = vec![
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", &self.redirect_uri),
            ("client_id", &self.client_id),
            ("code_verifier", &login.code_verifier),
        ];
        if let Some(secret) = &self.client_secret {
            form.push(("client_secret", secret));
        }
        let response = self
            .client
            .post(&self.token_endpoint)
            .form(&form)
            .send()
            .await
            .map_err(|_| LoginError::Exchange(0))?;
        let status = response.status();
        if !status.is_success() {
            return Err(LoginError::Exchange(status.as_u16()));
        }
        let reply: TokenReply = response
            .json()
            .await
            .map_err(|_| LoginError::Exchange(status.as_u16()))?;

        let claims = jwt::verify(
            &self.jwks,
            &reply.id_token,
            &Expect {
                issuer: self.issuer.clone(),
                audience: AudienceRule::Exact(self.client_id.clone()),
                now_ms,
                max_lifetime_ms: None,
            },
        )
        .await?;
        let nonce = claims.nonce.as_deref().ok_or(LoginError::Nonce)?;
        if nonce.as_bytes().ct_eq(login.nonce.as_bytes()).unwrap_u8() == 0 {
            return Err(LoginError::Nonce);
        }

        // An address the IdP has not verified is one anyone could have
        // typed at sign-up; letting it through would let a stranger with an
        // account at the same IdP claim an address in an allowed domain and
        // become a member. An IdP that sends no `email_verified` at all is
        // taken at its word, as the spec allows, because many enterprise
        // IdPs only issue addresses they provisioned.
        let email = claims
            .all
            .get("email")
            .and_then(serde_json::Value::as_str)
            .filter(|email| !email.is_empty())
            .ok_or(LoginError::Email)?;
        if let Some(verified) = claims.all.get("email_verified") {
            if verified.as_bool() != Some(true) {
                return Err(LoginError::Email);
            }
        }
        Ok(Identity {
            subject: claims.sub.clone().ok_or(VerifyError::MissingClaim("sub"))?,
            email: email.to_string(),
            name: claims
                .all
                .get("name")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
        })
    }
}

/// Whether `email` is in one of `allowed_domains`, compared without case;
/// an empty list allows any address the IdP vouched for.
pub fn allowed(email: &str, allowed_domains: &[String]) -> bool {
    if allowed_domains.is_empty() {
        return true;
    }
    let Some((_, domain)) = email.rsplit_once('@') else {
        return false;
    };
    allowed_domains
        .iter()
        .any(|allowed| allowed.eq_ignore_ascii_case(domain))
}
