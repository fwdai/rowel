//! An agent's proof of who it is (spec `02`, workload identity; the shapes
//! in `docs/server-api.md`): a JWT its platform minted, bound to the sealed
//! channel it arrives on. Where the platform lets the agent choose the
//! audience, the audience carries the session key's digest and a token
//! lifted from one channel fails on every other. Where it cannot, the
//! audience is the tenant's fixed one and the binding is the challenge nonce
//! echoed inside the sealed `associate`, a ten-minute cap on the token's
//! life, and a replay cache on its `jti` for that window. The replay cache
//! runs for every token regardless.
//!
//! A proof that verifies is then matched to a trust policy, which is what
//! names the agent definition. Errors out of here are coarse on purpose: the
//! broker tells an agent `AssociationFailed` and nothing finer, and the
//! finer reason belongs in the server's own log.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

use base64::prelude::*;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

pub use rowel_broker::actions::Proof;

use super::jwt::{self, AudienceRule, Claims, Expect, Issuers};

/// The one proof kind so far: a JWT checked against its issuer's keys.
pub const OIDC: &str = "oidc";

/// How long a challenge nonce stays live. An agent fetches one, asks its
/// platform for a token and associates, which is seconds; five minutes
/// covers a slow runner without keeping nonces around for long.
pub const CHALLENGE_TTL_MS: i64 = 5 * 60 * 1000;

/// The longest a fixed-audience token may live (spec `02`): with no key in
/// the audience, the token's life is the window in which it could be
/// replayed, and the replay cache has to remember every `jti` for it.
pub const FIXED_AUDIENCE_MAX_LIFETIME_MS: i64 = 10 * 60 * 1000;

/// A pattern that starts with this is a glob; anything else is exact.
pub const GLOB_PREFIX: &str = "glob:";

/// The most live challenges kept. The challenge endpoint is unauthenticated,
/// so whoever can reach it can mint nonces as fast as it answers; past this
/// many, the oldest goes, which costs a slow legitimate agent one retry and
/// costs the server nothing.
const MAX_LIVE_CHALLENGES: usize = 100_000;

/// The length of an X25519 public key, which is what the session key is.
const SESSION_KEY_LEN: usize = 32;

/// What an admin wrote to recognise an agent: the issuer whose tokens count,
/// and the claims every one of them must carry. A claim's pattern is exact
/// unless it starts with `glob:`, where `*` matches any run of characters.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrustPolicy {
    pub issuer: String,
    #[serde(default)]
    pub claims: BTreeMap<String, String>,
    /// The platform cannot put the session key into the audience, so the
    /// token names the tenant's bare audience and is bound by nonce,
    /// lifetime and `jti` instead.
    #[serde(default)]
    pub fixed_audience: bool,
}

impl TrustPolicy {
    /// Whether `claims` carry every claim this policy names, each matching
    /// its pattern. A claim that is not a string is compared as its JSON
    /// text, so `"ref_protected": "true"` matches a boolean claim.
    pub fn matches(&self, claims: &Map<String, Value>) -> bool {
        self.claims.iter().all(|(name, pattern)| {
            claims
                .get(name)
                .is_some_and(|value| pattern_matches(pattern, &claim_text(value)))
        })
    }
}

fn claim_text(value: &Value) -> Cow<'_, str> {
    match value {
        Value::String(text) => Cow::Borrowed(text),
        other => Cow::Owned(other.to_string()),
    }
}

/// Whether `text` matches `pattern`: exactly, or by glob when the pattern
/// says so.
pub fn pattern_matches(pattern: &str, text: &str) -> bool {
    match pattern.strip_prefix(GLOB_PREFIX) {
        Some(glob) => glob_matches(glob, text),
        None => pattern == text,
    }
}

/// `*` matches any run of characters, including none; everything else
/// matches itself. Linear with one point of backtracking, the way a shell
/// matches a file name, so a pattern cannot be made expensive.
fn glob_matches(glob: &str, text: &str) -> bool {
    let glob: Vec<char> = glob.chars().collect();
    let text: Vec<char> = text.chars().collect();
    let (mut g, mut t) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while t < text.len() {
        if glob.get(g) == Some(&'*') {
            star = Some((g, t));
            g += 1;
        } else if glob.get(g) == Some(&text[t]) {
            g += 1;
            t += 1;
        } else if let Some((star_g, star_t)) = star {
            g = star_g + 1;
            t = star_t + 1;
            star = Some((star_g, star_t + 1));
        } else {
            return false;
        }
    }
    glob[g..].iter().all(|c| *c == '*')
}

/// The audience a key-bound token must name: the tenant's audience, a
/// colon, and the lowercase hex SHA-256 of the raw 32 bytes of the session
/// public key the channel was opened with. `None` when the key is not one,
/// which a channel the broker opened never sends.
pub fn expected_audience(tenant_audience: &str, session_public_key_b64: &str) -> Option<String> {
    let raw = BASE64_STANDARD
        .decode(session_public_key_b64)
        .ok()
        .filter(|raw| raw.len() == SESSION_KEY_LEN)?;
    Some(format!(
        "{tenant_audience}:{}",
        hex::encode(Sha256::digest(&raw))
    ))
}

/// What `GET /v1/agent/challenge` answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Challenge {
    pub nonce: String,
    pub audience: String,
}

/// The live challenge nonces: minted single-use, gone when consumed or
/// after `CHALLENGE_TTL_MS`, pruned on every mint.
#[derive(Default)]
pub struct Challenges {
    live: Mutex<HashMap<String, i64>>,
}

impl Challenges {
    /// A fresh nonce for `audience`, live until `now_ms + CHALLENGE_TTL_MS`.
    pub fn mint(&self, audience: &str, now_ms: i64) -> Challenge {
        let nonce = random_token();
        let mut live = self.live.lock().unwrap();
        live.retain(|_, expires_at_ms| *expires_at_ms > now_ms);
        if live.len() >= MAX_LIVE_CHALLENGES {
            if let Some(oldest) = live
                .iter()
                .min_by_key(|(_, expires_at_ms)| **expires_at_ms)
                .map(|(nonce, _)| nonce.clone())
            {
                live.remove(&oldest);
            }
        }
        live.insert(nonce.clone(), now_ms + CHALLENGE_TTL_MS);
        Challenge {
            nonce,
            audience: audience.to_string(),
        }
    }

    /// Take `nonce` if it is live: `true` once, `false` for one that was
    /// never minted, was used already, or has expired.
    pub fn consume(&self, nonce: &str, now_ms: i64) -> bool {
        self.live
            .lock()
            .unwrap()
            .remove(nonce)
            .is_some_and(|expires_at_ms| expires_at_ms > now_ms)
    }
}

/// 16 random bytes as base64url: a nonce, a state, anything that must be
/// unguessable and fit in a query string.
pub fn random_token() -> String {
    let mut bytes = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    BASE64_URL_SAFE_NO_PAD.encode(bytes)
}

/// Remembers every `jti` a verified token carried until the token expires.
/// `note` answers `true` for a `jti` seen for the first time and `false` for
/// one already noted; `now_ms` lets an implementation drop what has
/// expired. The in-memory one is here; the store-backed one shares a
/// server's memory across restarts.
pub trait ReplayCache: Send + Sync {
    fn note(&self, jti: &str, expires_at_ms: i64, now_ms: i64) -> bool;
}

/// The in-memory replay cache: enough for one process, forgotten with it.
#[derive(Default)]
pub struct MemoryReplay {
    seen: Mutex<HashMap<String, i64>>,
}

impl ReplayCache for MemoryReplay {
    fn note(&self, jti: &str, expires_at_ms: i64, now_ms: i64) -> bool {
        let mut seen = self.seen.lock().unwrap();
        seen.retain(|_, expires| *expires > now_ms);
        if seen.contains_key(jti) {
            return false;
        }
        seen.insert(jti.to_string(), expires_at_ms);
        true
    }
}

/// A proof that verified: the policy it matched, by index into the list it
/// was checked against, and the claims it carried.
#[derive(Clone, Debug)]
pub struct Verified {
    pub policy_index: usize,
    pub claims: Claims,
}

/// Why a proof was refused, coarse enough to map to one broker `Code`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ProofError {
    #[error("no trust policy names the token's issuer")]
    NoPolicy,
    #[error("the proof does not verify")]
    Invalid,
    #[error("the proof was presented before")]
    Replayed,
    #[error("the token matches no trust policy")]
    Unmatched,
}

/// The pieces every proof is checked against: the issuers' keys, the live
/// challenges and the replay cache, held by the server and borrowed for one
/// verification.
pub struct Verifier<'a> {
    pub issuers: &'a Issuers,
    pub challenges: &'a Challenges,
    pub replay: &'a dyn ReplayCache,
}

impl Verifier<'_> {
    /// Verify `proof` for the channel opened with `session_public_key_b64`
    /// against `policies`, in order; the first that matches wins. The token's
    /// `iss` is read before verification only to pick the candidate policies,
    /// and nothing else is believed until the signature holds.
    pub async fn verify_proof(
        &self,
        proof: &Proof,
        session_public_key_b64: &str,
        tenant_audience: &str,
        policies: &[TrustPolicy],
        now_ms: i64,
    ) -> Result<Verified, ProofError> {
        if proof.kind != OIDC {
            return Err(ProofError::Invalid);
        }
        let issuer = unverified_issuer(&proof.token).ok_or(ProofError::Invalid)?;
        let candidates: Vec<usize> = policies
            .iter()
            .enumerate()
            .filter(|(_, policy)| policy.issuer == issuer)
            .map(|(index, _)| index)
            .collect();
        if candidates.is_empty() {
            return Err(ProofError::NoPolicy);
        }
        let jwks = self
            .issuers
            .jwks(&issuer)
            .await
            .map_err(|_| ProofError::Invalid)?;

        // A token has one audience, so at most one of the two bindings can
        // hold; the key-bound one is tried first because it is the stronger.
        let mut verified = None;
        if candidates.iter().any(|&i| !policies[i].fixed_audience) {
            if let Some(audience) = expected_audience(tenant_audience, session_public_key_b64) {
                let expected = Expect {
                    issuer: issuer.clone(),
                    audience: AudienceRule::Exact(audience),
                    now_ms,
                    max_lifetime_ms: None,
                };
                if let Ok(claims) = jwt::verify(&jwks, &proof.token, &expected).await {
                    verified = Some((false, claims));
                }
            }
        }
        if verified.is_none() && candidates.iter().any(|&i| policies[i].fixed_audience) {
            let expected = Expect {
                issuer: issuer.clone(),
                audience: AudienceRule::Exact(tenant_audience.to_string()),
                now_ms,
                max_lifetime_ms: Some(FIXED_AUDIENCE_MAX_LIFETIME_MS),
            };
            if let Ok(claims) = jwt::verify(&jwks, &proof.token, &expected).await {
                verified = Some((true, claims));
            }
        }
        let (fixed, claims) = verified.ok_or(ProofError::Invalid)?;

        // Under the fixed audience the nonce is the channel binding and the
        // `jti` is the only handle on a replay, so both are required; under
        // the key-bound audience a token without a `jti` is still noted,
        // under its own digest, so that too is presented once.
        let jti = if fixed {
            let nonce = proof.nonce.as_deref().ok_or(ProofError::Invalid)?;
            if !self.challenges.consume(nonce, now_ms) {
                return Err(ProofError::Invalid);
            }
            claims.jti.clone().ok_or(ProofError::Invalid)?
        } else {
            claims.jti.clone().unwrap_or_else(|| {
                format!(
                    "sha256:{}",
                    hex::encode(Sha256::digest(proof.token.as_bytes()))
                )
            })
        };
        // Keyed by issuer too: a `jti` is unique within one issuer only, and
        // two platforms minting the same id must not refuse each other.
        if !self.replay.note(
            &format!("{issuer}|{jti}"),
            claims.accepted_until_ms(),
            now_ms,
        ) {
            return Err(ProofError::Replayed);
        }

        candidates
            .into_iter()
            .filter(|&i| policies[i].fixed_audience == fixed)
            .find(|&i| policies[i].matches(&claims.all))
            .map(|policy_index| Verified {
                policy_index,
                claims,
            })
            .ok_or(ProofError::Unmatched)
    }

    /// [`Self::verify_proof`] for the broker's connection thread.
    pub fn verify_proof_blocking(
        &self,
        handle: &tokio::runtime::Handle,
        proof: &Proof,
        session_public_key_b64: &str,
        tenant_audience: &str,
        policies: &[TrustPolicy],
        now_ms: i64,
    ) -> Result<Verified, ProofError> {
        handle.block_on(self.verify_proof(
            proof,
            session_public_key_b64,
            tenant_audience,
            policies,
            now_ms,
        ))
    }
}

/// The `iss` the token claims, read without checking anything. Good only
/// for choosing which issuer's keys — and which trust policies — to check it
/// against: a token lying here is checked against the keys of the issuer it
/// named and fails there.
pub fn unverified_issuer(token: &str) -> Option<String> {
    let payload = token.split('.').nth(1)?;
    let bytes = BASE64_URL_SAFE_NO_PAD.decode(payload).ok()?;
    let claims: Map<String, Value> = serde_json::from_slice(&bytes).ok()?;
    claims.get("iss")?.as_str().map(str::to_owned)
}
