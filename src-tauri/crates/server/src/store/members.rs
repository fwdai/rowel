//! The tenant, its members and their devices.

use rusqlite::{params, OptionalExtension, Row};
use serde::Serialize;

use rowel_core::crypto::random_hex_id;

use super::{collect, now_ms, text_column, Result, Store, StoreError};

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
