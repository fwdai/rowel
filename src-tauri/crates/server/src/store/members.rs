//! The tenant, its members and their devices, and the challenges a session
//! answers to enroll one.

use rusqlite::{params, OptionalExtension, Row};
use serde::Serialize;
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

use rowel_core::crypto::random_hex_id;

use super::sessions::Session;
use super::{collect, now_ms, sha256_hex, text_column, Result, Store, StoreError};

/// How long an enrollment challenge waits for its proof. The app answers
/// within a second of asking; past this the challenge is refused and swept.
pub const ENROLL_CHALLENGE_TTL_MS: i64 = 5 * 60 * 1000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tenant {
    pub id: String,
    pub name: String,
}

text_enum! {
    /// What a member may do: an admin runs the admin API and approves any
    /// device; a member holds lockboxes and approves their own.
    Role { Member => "member", Admin => "admin" }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Member {
    pub id: String,
    pub email: String,
    pub name: Option<String>,
    pub role: Role,
    pub created_at_ms: i64,
    pub disabled_at_ms: Option<i64>,
}

text_enum! {
    /// Where a device stands (spec `02`): pending until an approved device of
    /// the same member, or an admin, lets it in; only an approved one is
    /// handed a wrapped master.
    DeviceState { Pending => "pending", Approved => "approved", Revoked => "revoked" }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub id: String,
    pub member_id: String,
    pub name: String,
    /// The X25519 enrollment public key, base64 of its 32 bytes, as the app
    /// sent it and as every device of the member fingerprints it.
    pub public_key: String,
    pub state: DeviceState,
    pub created_at_ms: i64,
    pub approved_at_ms: Option<i64>,
    pub approved_by: Option<String>,
}

fn read_member(row: &Row<'_>) -> rusqlite::Result<Member> {
    Ok(Member {
        id: row.get("id")?,
        email: row.get("email")?,
        name: row.get("name")?,
        role: text_column(row, "role")?,
        created_at_ms: row.get("created_at_ms")?,
        disabled_at_ms: row.get("disabled_at_ms")?,
    })
}

fn read_device(row: &Row<'_>) -> rusqlite::Result<Device> {
    Ok(Device {
        id: row.get("id")?,
        member_id: row.get("member_id")?,
        name: row.get("name")?,
        public_key: row.get("public_key")?,
        state: text_column(row, "state")?,
        created_at_ms: row.get("created_at_ms")?,
        approved_at_ms: row.get("approved_at_ms")?,
        approved_by: row.get("approved_by")?,
    })
}

const MEMBER_COLUMNS: &str = "id, email, name, role, created_at_ms, disabled_at_ms";
const DEVICE_COLUMNS: &str =
    "id, member_id, name, public_key, state, created_at_ms, approved_at_ms, approved_by";

impl Store {
    /// Mint the tenant row on the first open; on every later one, take the
    /// config's name and keep the id.
    pub(super) fn ensure_tenant(&self, name: &str) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let changed = tx.execute("UPDATE tenant SET name = ?1", params![name])?;
        if changed == 0 {
            tx.execute(
                "INSERT INTO tenant (id, name, created_at_ms) VALUES (?1, ?2, ?3)",
                params![random_hex_id(), name, now_ms()],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn tenant(&self) -> Result<Tenant> {
        Ok(self
            .conn()
            .query_row("SELECT id, name FROM tenant LIMIT 1", [], |row| {
                Ok(Tenant {
                    id: row.get(0)?,
                    name: row.get(1)?,
                })
            })?)
    }

    pub fn add_member(&self, email: &str, name: Option<&str>, role: Role) -> Result<Member> {
        let member = Member {
            id: random_hex_id(),
            email: email.into(),
            name: name.map(String::from),
            role,
            created_at_ms: now_ms(),
            disabled_at_ms: None,
        };
        self.conn().execute(
            "INSERT INTO members (id, email, name, role, created_at_ms) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                member.id,
                member.email,
                member.name,
                member.role.as_str(),
                member.created_at_ms
            ],
        )?;
        Ok(member)
    }

    pub fn member(&self, id: &str) -> Result<Option<Member>> {
        Ok(self
            .conn()
            .query_row(
                &format!("SELECT {MEMBER_COLUMNS} FROM members WHERE id = ?1"),
                params![id],
                read_member,
            )
            .optional()?)
    }

    /// The member with `email`, however it is cased.
    pub fn member_by_email(&self, email: &str) -> Result<Option<Member>> {
        Ok(self
            .conn()
            .query_row(
                &format!("SELECT {MEMBER_COLUMNS} FROM members WHERE email = ?1 COLLATE NOCASE"),
                params![email],
                read_member,
            )
            .optional()?)
    }

    pub fn list_members(&self) -> Result<Vec<Member>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {MEMBER_COLUMNS} FROM members ORDER BY created_at_ms, rowid"
        ))?;
        collect(&mut stmt, [], read_member)
    }

    pub fn set_member_role(&self, id: &str, role: Role) -> Result<()> {
        self.one_row_changed(
            self.conn().execute(
                "UPDATE members SET role = ?1 WHERE id = ?2",
                params![role.as_str(), id],
            )?,
            || format!("member {id}"),
        )
    }

    /// Mark the member disabled. Their sessions are ended separately
    /// (`revoke_sessions_of_member`): offboarding is the admin slice's
    /// sequence, and this is one step of it.
    pub fn disable_member(&self, id: &str) -> Result<()> {
        self.one_row_changed(
            self.conn().execute(
                "UPDATE members SET disabled_at_ms = ?1 WHERE id = ?2 AND disabled_at_ms IS NULL",
                params![now_ms(), id],
            )?,
            || format!("enabled member {id}"),
        )
    }

    /// Enroll a device, pending. The same public key enrolled again by the
    /// same member is the same device — an app that lost its record and
    /// signed in again — and gets its existing row back, approval and all,
    /// rather than a second pending one.
    ///
    /// That is only sound because the caller has proved possession of the
    /// key's secret half ([`Store::enroll_proven_device`]): the public half is
    /// on the member's device list, and handing back an approved row to
    /// whoever names it would hand them the approval with it.
    pub fn enroll_device(&self, member_id: &str, name: &str, public_key: &str) -> Result<Device> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let existing = tx
            .query_row(
                &format!(
                    "SELECT {DEVICE_COLUMNS} FROM devices WHERE member_id = ?1 AND public_key = ?2"
                ),
                params![member_id, public_key],
                read_device,
            )
            .optional()?;
        if let Some(device) = existing {
            return Ok(device);
        }
        let device = Device {
            id: random_hex_id(),
            member_id: member_id.into(),
            name: name.into(),
            public_key: public_key.into(),
            state: DeviceState::Pending,
            created_at_ms: now_ms(),
            approved_at_ms: None,
            approved_by: None,
        };
        tx.execute(
            "INSERT INTO devices (id, member_id, name, public_key, state, created_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                device.id,
                device.member_id,
                device.name,
                device.public_key,
                device.state.as_str(),
                device.created_at_ms
            ],
        )?;
        tx.commit()?;
        Ok(device)
    }

    /// A challenge for `session_id` to prove it holds the secret half of
    /// `public_key`: 32 random bytes for the caller to seal to the key, held
    /// here only as their SHA-256 and for [`ENROLL_CHALLENGE_TTL_MS`]. A new
    /// one for the same session and key replaces the last. Challenges past
    /// their time are swept on the way, as codes are.
    pub fn issue_enroll_challenge(
        &self,
        session_id: &str,
        public_key: &str,
    ) -> Result<Zeroizing<[u8; 32]>> {
        self.issue_enroll_challenge_at(session_id, public_key, now_ms())
    }

    pub fn issue_enroll_challenge_at(
        &self,
        session_id: &str,
        public_key: &str,
        now_ms: i64,
    ) -> Result<Zeroizing<[u8; 32]>> {
        let mut secret = Zeroizing::new([0u8; 32]);
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut *secret);
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM enroll_challenges WHERE expires_at_ms <= ?1",
            params![now_ms],
        )?;
        // One live challenge per session, the latest: a session enrolls one
        // device, and asking again replaces the last one, so the table is
        // bounded by the sessions rather than by the keys a member names.
        tx.execute(
            "INSERT OR REPLACE INTO enroll_challenges
               (session_id, public_key, secret_hash, expires_at_ms)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                session_id,
                public_key,
                sha256_hex(&*secret),
                now_ms + ENROLL_CHALLENGE_TTL_MS
            ],
        )?;
        tx.commit()?;
        Ok(secret)
    }

    /// Enroll `public_key` for the session's member and bind the session to
    /// the device, when `proof` is the secret of the live challenge issued to
    /// this session for this key; `None`, with nothing enrolled or bound,
    /// otherwise. This is the one way a session comes to stand for a device:
    /// a public key is no secret, so a session that only names one has
    /// proved nothing, and binding it would hand it that device's masters
    /// and its say over approvals.
    pub fn enroll_proven_device(
        &self,
        session: &Session,
        name: &str,
        public_key: &str,
        proof: &[u8],
    ) -> Result<Option<Device>> {
        if !self.spend_enroll_challenge(&session.id, public_key, proof)? {
            return Ok(None);
        }
        let device = self.enroll_device(&session.member_id, name, public_key)?;
        self.bind_session_device(&session.id, &device.id)?;
        Ok(Some(device))
    }

    /// A session of `device`'s member, bound to it the one way a session is
    /// bound ([`Store::enroll_proven_device`]), for a test that drives no
    /// HTTP. The challenge is answered with the secret the store issued
    /// rather than one opened under the device's key, so this proves nothing
    /// and exists only under `cfg(test)`; a test of enrollment itself goes
    /// through the member API (`tests::harness::Api::enroll`).
    #[cfg(test)]
    pub(crate) fn create_bound_session_at(
        &self,
        device: &Device,
        access_ttl_ms: i64,
        now_ms: i64,
    ) -> Result<super::sessions::Issued> {
        let (session, access, refresh) =
            self.create_session_at(&device.member_id, access_ttl_ms, now_ms)?;
        let secret = self.issue_enroll_challenge(&session.id, &device.public_key)?;
        let bound = self
            .enroll_proven_device(&session, &device.name, &device.public_key, &*secret)?
            .ok_or_else(|| StoreError::Other("the challenge just issued did not answer".into()))?;
        let session = Session {
            device_id: Some(bound.id),
            ..session
        };
        Ok((session, access, refresh))
    }

    /// Whether `proof` answers the challenge held for `session_id` and
    /// `public_key`: hashed to the stored SHA-256 and compared in constant
    /// time, within its time. The challenge is deleted whether or not it
    /// does, so a wrong answer costs a new challenge rather than leaving
    /// this one for another try.
    fn spend_enroll_challenge(
        &self,
        session_id: &str,
        public_key: &str,
        proof: &[u8],
    ) -> Result<bool> {
        let now = now_ms();
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let held = tx
            .query_row(
                "SELECT secret_hash, expires_at_ms FROM enroll_challenges
                 WHERE session_id = ?1 AND public_key = ?2",
                params![session_id, public_key],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?;
        // Any answer spends the session's challenge, right or wrong, for
        // whichever key it names.
        tx.execute(
            "DELETE FROM enroll_challenges WHERE session_id = ?1",
            params![session_id],
        )?;
        tx.commit()?;
        let Some((secret_hash, expires_at_ms)) = held else {
            return Ok(false);
        };
        let answered = bool::from(sha256_hex(proof).as_bytes().ct_eq(secret_hash.as_bytes()));
        Ok(answered && expires_at_ms > now)
    }

    pub fn device(&self, id: &str) -> Result<Option<Device>> {
        Ok(self
            .conn()
            .query_row(
                &format!("SELECT {DEVICE_COLUMNS} FROM devices WHERE id = ?1"),
                params![id],
                read_device,
            )
            .optional()?)
    }

    pub fn devices_of(&self, member_id: &str) -> Result<Vec<Device>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {DEVICE_COLUMNS} FROM devices WHERE member_id = ?1 ORDER BY created_at_ms, rowid"
        ))?;
        collect(&mut stmt, params![member_id], read_device)
    }

    /// Approve a pending device, recording who did (`by`: a device id, or
    /// an admin's member id). An approved device stays as it is; a revoked
    /// one is never let back in — the member enrolls afresh.
    pub fn approve_device(&self, id: &str, by: &str) -> Result<Device> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let device = tx
            .query_row(
                &format!("SELECT {DEVICE_COLUMNS} FROM devices WHERE id = ?1"),
                params![id],
                read_device,
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound(format!("device {id}")))?;
        let device = match device.state {
            DeviceState::Approved => device,
            DeviceState::Revoked => {
                return Err(StoreError::Other(
                    "a revoked device is not approved again".into(),
                ))
            }
            DeviceState::Pending => {
                let now = now_ms();
                tx.execute(
                    "UPDATE devices SET state = ?1, approved_at_ms = ?2, approved_by = ?3 WHERE id = ?4",
                    params![DeviceState::Approved.as_str(), now, by, id],
                )?;
                Device {
                    state: DeviceState::Approved,
                    approved_at_ms: Some(now),
                    approved_by: Some(by.into()),
                    ..device
                }
            }
        };
        tx.commit()?;
        Ok(device)
    }

    /// Revoke a device. The masters sealed to it go with it in the same
    /// transaction: they open to nothing the server should still be
    /// handing out, and a listing must never find one for this device.
    pub fn revoke_device(&self, id: &str) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let changed = tx.execute(
            "UPDATE devices SET state = ?1 WHERE id = ?2",
            params![DeviceState::Revoked.as_str(), id],
        )?;
        tx.execute(
            "DELETE FROM device_masters WHERE device_id = ?1",
            params![id],
        )?;
        // The sessions the device holds end with it: withholding the master
        // alone would leave its bearer answering approvals and pulling packs
        // it can no longer open, but still could read the names of.
        tx.execute(
            "UPDATE sessions SET revoked_at_ms = ?1
             WHERE device_id = ?2 AND revoked_at_ms IS NULL",
            params![now_ms(), id],
        )?;
        tx.commit()?;
        self.one_row_changed(changed, || format!("device {id}"))
    }

    /// `Ok` when `changed` rows were written, `NotFound` naming `what`
    /// otherwise: how a write that must land on a row says the row is not
    /// there.
    pub(super) fn one_row_changed(
        &self,
        changed: usize,
        what: impl FnOnce() -> String,
    ) -> Result<()> {
        if changed == 0 {
            return Err(StoreError::NotFound(what()));
        }
        Ok(())
    }
}
