//! Member sessions, the authorization codes that start them, and the replay
//! cache for workload tokens. Nothing a bearer presents is stored as it is
//! presented: the database holds SHA-256 hashes, so reading it yields no
//! token anyone could use.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rusqlite::{params, OptionalExtension, Row};
use sha2::Digest;
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

use rowel_core::crypto::random_hex_id;

use super::{now_ms, sha256_hex, Result, Store};

/// How long an authorization code may wait to be redeemed: RFC 6749 §4.1.2
/// recommends at most ten minutes; the app redeems within a second of the
/// browser coming back.
pub const AUTH_CODE_TTL_MS: i64 = 5 * 60 * 1000;

/// A member session: what a bearer token stands for. The tokens themselves
/// are not here; they were handed out once, at creation or refresh.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    pub id: String,
    pub member_id: String,
    /// The device enrolled under this session, once one was; a session is
    /// bound to its device across refreshes (`docs/server-api.md`).
    pub device_id: Option<String>,
    pub access_expires_at_ms: i64,
    pub created_at_ms: i64,
}

/// A fresh session, with the two tokens that reach the member once.
pub type Issued = (Session, Zeroizing<String>, Zeroizing<String>);

const SESSION_COLUMNS: &str = "id, member_id, device_id, access_expires_at_ms, created_at_ms";

fn read_session(row: &Row<'_>) -> rusqlite::Result<Session> {
    Ok(Session {
        id: row.get("id")?,
        member_id: row.get("member_id")?,
        device_id: row.get("device_id")?,
        access_expires_at_ms: row.get("access_expires_at_ms")?,
        created_at_ms: row.get("created_at_ms")?,
    })
}

/// Thirty-two random bytes as base64url: an access token, a refresh token,
/// an authorization code.
fn random_token() -> Zeroizing<String> {
    let mut bytes = Zeroizing::new([0u8; 32]);
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut *bytes);
    Zeroizing::new(URL_SAFE_NO_PAD.encode(*bytes))
}

/// The S256 challenge of `verifier`: `BASE64URL(SHA-256(verifier))`, RFC
/// 7636 §4.2.
pub fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(verifier.as_bytes()))
}

impl Store {
    /// A new session for `member_id`, the access token good for
    /// `access_ttl_ms`. It stands for no device: a session comes to be a
    /// device's only by proving it holds the device's key
    /// ([`Store::enroll_proven_device`]), never by being created as one.
    pub fn create_session(&self, member_id: &str, access_ttl_ms: i64) -> Result<Issued> {
        self.create_session_at(member_id, access_ttl_ms, now_ms())
    }

    pub fn create_session_at(
        &self,
        member_id: &str,
        access_ttl_ms: i64,
        now_ms: i64,
    ) -> Result<Issued> {
        let access = random_token();
        let refresh = random_token();
        let session = Session {
            id: random_hex_id(),
            member_id: member_id.into(),
            device_id: None,
            access_expires_at_ms: now_ms + access_ttl_ms,
            created_at_ms: now_ms,
        };
        self.conn().execute(
            "INSERT INTO sessions (id, member_id, access_hash, refresh_hash,
                                   access_expires_at_ms, created_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                session.id,
                session.member_id,
                sha256_hex(access.as_bytes()),
                sha256_hex(refresh.as_bytes()),
                session.access_expires_at_ms,
                session.created_at_ms
            ],
        )?;
        Ok((session, access, refresh))
    }

    /// The live session `token` is the access token of: unexpired and not
    /// revoked. Anything else is `None`, which the HTTP layer answers 401.
    pub fn session_by_access(&self, token: &str) -> Result<Option<Session>> {
        self.session_by_access_at(token, now_ms())
    }

    pub fn session_by_access_at(&self, token: &str, now_ms: i64) -> Result<Option<Session>> {
        Ok(self
            .conn()
            .query_row(
                &format!(
                    "SELECT {SESSION_COLUMNS} FROM sessions
                     WHERE access_hash = ?1 AND revoked_at_ms IS NULL AND access_expires_at_ms > ?2"
                ),
                params![sha256_hex(token.as_bytes()), now_ms],
                read_session,
            )
            .optional()?)
    }

    /// Spend `refresh_token` for a new pair. Single use: the hash moves to
    /// the spent column in the same transaction, so the next presentation
    /// of the same token finds no live session — and finds the spent one,
    /// which is revoked whole (RFC 6819 §5.2.2.3: a replayed refresh token
    /// means either the member or a thief holds a copy, and neither should
    /// keep the session). `None` for a token the server does not hold or
    /// has just revoked.
    pub fn refresh_session(
        &self,
        refresh_token: &str,
        access_ttl_ms: i64,
    ) -> Result<Option<Issued>> {
        self.refresh_session_at(refresh_token, access_ttl_ms, now_ms())
    }

    pub fn refresh_session_at(
        &self,
        refresh_token: &str,
        access_ttl_ms: i64,
        now_ms: i64,
    ) -> Result<Option<Issued>> {
        let hash = sha256_hex(refresh_token.as_bytes());
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let live = tx
            .query_row(
                &format!(
                    "SELECT {SESSION_COLUMNS} FROM sessions
                     WHERE refresh_hash = ?1 AND revoked_at_ms IS NULL"
                ),
                params![hash],
                read_session,
            )
            .optional()?;
        let Some(session) = live else {
            tx.execute(
                "UPDATE sessions SET revoked_at_ms = ?1
                 WHERE spent_refresh_hash = ?2 AND revoked_at_ms IS NULL",
                params![now_ms, hash],
            )?;
            tx.commit()?;
            return Ok(None);
        };
        let access = random_token();
        let refresh = random_token();
        let session = Session {
            access_expires_at_ms: now_ms + access_ttl_ms,
            ..session
        };
        tx.execute(
            "UPDATE sessions
             SET access_hash = ?1, refresh_hash = ?2, spent_refresh_hash = ?3, access_expires_at_ms = ?4
             WHERE id = ?5",
            params![
                sha256_hex(access.as_bytes()),
                sha256_hex(refresh.as_bytes()),
                hash,
                session.access_expires_at_ms,
                session.id
            ],
        )?;
        tx.commit()?;
        Ok(Some((session, access, refresh)))
    }

    /// From now on the session is `device_id`'s: what an enrollment under it
    /// does, so the listing knows which device to seal masters to. Reached
    /// only through [`Store::enroll_proven_device`], once the session has
    /// proved it holds the device's key.
    pub(super) fn bind_session_device(&self, session_id: &str, device_id: &str) -> Result<()> {
        self.one_row_changed(
            self.conn().execute(
                "UPDATE sessions SET device_id = ?1 WHERE id = ?2",
                params![device_id, session_id],
            )?,
            || format!("session {session_id}"),
        )
    }

    /// End every live session of `member_id`; how many there were.
    pub fn revoke_sessions_of_member(&self, member_id: &str) -> Result<usize> {
        Ok(self.conn().execute(
            "UPDATE sessions SET revoked_at_ms = ?1 WHERE member_id = ?2 AND revoked_at_ms IS NULL",
            params![now_ms(), member_id],
        )?)
    }

    /// An authorization code for `member_id`, bound to the PKCE challenge
    /// and the redirect the app sent, good for [`AUTH_CODE_TTL_MS`]. Codes
    /// past their time are swept on the way: they are refused anyway, and
    /// the table should not grow with abandoned sign-ins.
    pub fn issue_code(
        &self,
        member_id: &str,
        code_challenge: &str,
        redirect_uri: &str,
    ) -> Result<Zeroizing<String>> {
        self.issue_code_at(member_id, code_challenge, redirect_uri, now_ms())
    }

    pub fn issue_code_at(
        &self,
        member_id: &str,
        code_challenge: &str,
        redirect_uri: &str,
        now_ms: i64,
    ) -> Result<Zeroizing<String>> {
        let code = random_token();
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM auth_codes WHERE expires_at_ms <= ?1",
            params![now_ms],
        )?;
        tx.execute(
            "INSERT INTO auth_codes (code_hash, member_id, code_challenge, redirect_uri, expires_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                sha256_hex(code.as_bytes()),
                member_id,
                code_challenge,
                redirect_uri,
                now_ms + AUTH_CODE_TTL_MS
            ],
        )?;
        tx.commit()?;
        Ok(code)
    }

    /// Spend `code`: the member it was issued to, when `code_verifier`
    /// hashes to the challenge it was bound to (S256, compared in constant
    /// time) and `redirect_uri` is the one it was issued for, within its
    /// time. The code is deleted whether or not the checks pass — RFC 6749
    /// §4.1.2: a code presented with the wrong verifier is burnt, not left
    /// for another try.
    pub fn redeem_code(
        &self,
        code: &str,
        code_verifier: &str,
        redirect_uri: &str,
    ) -> Result<Option<String>> {
        self.redeem_code_at(code, code_verifier, redirect_uri, now_ms())
    }

    pub fn redeem_code_at(
        &self,
        code: &str,
        code_verifier: &str,
        redirect_uri: &str,
        now_ms: i64,
    ) -> Result<Option<String>> {
        let hash = sha256_hex(code.as_bytes());
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let found = tx
            .query_row(
                "SELECT member_id, code_challenge, redirect_uri, expires_at_ms
                 FROM auth_codes WHERE code_hash = ?1",
                params![hash],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                },
            )
            .optional()?;
        tx.execute("DELETE FROM auth_codes WHERE code_hash = ?1", params![hash])?;
        tx.commit()?;
        let Some((member_id, challenge, issued_for, expires_at_ms)) = found else {
            return Ok(None);
        };
        let verified = bool::from(
            pkce_challenge(code_verifier)
                .as_bytes()
                .ct_eq(challenge.as_bytes()),
        );
        if !verified || issued_for != redirect_uri || expires_at_ms <= now_ms {
            return Ok(None);
        }
        Ok(Some(member_id))
    }

    /// Remember a workload token's `jti` until `expires_at_ms`: `true` the
    /// first time, `false` for one already held and not yet expired, which
    /// is a replay. `expires_at_ms` must be the last instant the token is
    /// still accepted (`Claims::accepted_until_ms`, its `exp` plus the
    /// leeway), never the bare `exp`: a row dropped while the token still
    /// verifies lets it in again.
    pub fn note_jti(&self, jti: &str, expires_at_ms: i64) -> Result<bool> {
        self.note_jti_at(jti, expires_at_ms, now_ms())
    }

    pub fn note_jti_at(&self, jti: &str, expires_at_ms: i64, now_ms: i64) -> Result<bool> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM replay WHERE jti = ?1 AND expires_at_ms <= ?2",
            params![jti, now_ms],
        )?;
        let inserted = tx.execute(
            "INSERT OR IGNORE INTO replay (jti, expires_at_ms) VALUES (?1, ?2)",
            params![jti, expires_at_ms],
        )?;
        tx.commit()?;
        Ok(inserted == 1)
    }

    /// Drop every `jti` whose token is no longer accepted at `now_ms`; how
    /// many went.
    pub fn prune_replay(&self, now_ms: i64) -> Result<usize> {
        Ok(self.conn().execute(
            "DELETE FROM replay WHERE expires_at_ms <= ?1",
            params![now_ms],
        )?)
    }
}
