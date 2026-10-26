//! The lockbox rows: the server's record of each org lockbox — its name,
//! its `External` descriptor, the master wrapped under the tenant key, the
//! switches and the revision — who holds it, and the master sealed to each
//! device that may open it.

use rusqlite::{params, OptionalExtension, Row};
use serde::Serialize;

use super::{collect, now_ms, text_column, Result, Store};

/// A lockbox as the metadata knows it. The wrapped master is deliberately
/// not a field: it is read by the one function that unwraps it
/// (`Store::wrapped_master`), so a listing or a log line never carries it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Lockbox {
    pub id: String,
    pub name: String,
    /// The `KdfParams::External` descriptor as one JSON string, carried
    /// verbatim into the sidecar, every pack header and the listing.
    pub kdf: String,
    /// The key generation the descriptor is at; a device holding a lower
    /// one re-keys.
    pub gen: i64,
    /// Frozen by an admin: nothing is served from it, whatever policy says.
    pub frozen: bool,
    /// The agent-access switch, the lockbox attribute spec `01` names for
    /// this tier.
    pub agents: bool,
    /// Moved by every server-side write of the lockbox file; the pack ETag.
    pub revision: i64,
    pub created_at_ms: i64,
}

text_enum! {
    /// What a member may do with a lockbox: read it on their devices, also
    /// answer an agent's ask for it, also administer it.
    LockboxRole { Reader => "reader", Approver => "approver", Admin => "admin" }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LockboxMember {
    pub member_id: String,
    pub role: LockboxRole,
}

const LOCKBOX_COLUMNS: &str = "id, name, kdf, gen, frozen, agents, revision, created_at_ms";

fn read_lockbox(row: &Row<'_>) -> rusqlite::Result<Lockbox> {
    Ok(Lockbox {
        id: row.get("id")?,
        name: row.get("name")?,
        kdf: row.get("kdf")?,
        gen: row.get("gen")?,
        frozen: row.get("frozen")?,
        agents: row.get("agents")?,
        revision: row.get("revision")?,
        created_at_ms: row.get("created_at_ms")?,
    })
}

impl Store {
    /// The row for a lockbox `crate::lockboxes::create` has just made: its
    /// id is the file's and the vault's, the master arrives already wrapped,
    /// and the revision starts at zero.
    pub fn create_lockbox(
        &self,
        id: &str,
        name: &str,
        kdf: &str,
        wrapped_master: &[u8],
        gen: i64,
    ) -> Result<Lockbox> {
        let lockbox = Lockbox {
            id: id.into(),
            name: name.into(),
            kdf: kdf.into(),
            gen,
            frozen: false,
            agents: false,
            revision: 0,
            created_at_ms: now_ms(),
        };
        self.conn().execute(
            "INSERT INTO lockboxes (id, name, kdf, wrapped_master, gen, created_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                lockbox.id,
                lockbox.name,
                lockbox.kdf,
                wrapped_master,
                lockbox.gen,
                lockbox.created_at_ms
            ],
        )?;
        Ok(lockbox)
    }

    pub fn lockbox(&self, id: &str) -> Result<Option<Lockbox>> {
        Ok(self
            .conn()
            .query_row(
                &format!("SELECT {LOCKBOX_COLUMNS} FROM lockboxes WHERE id = ?1"),
                params![id],
                read_lockbox,
            )
            .optional()?)
    }

    /// The master of `id` as wrapped under the tenant key: ciphertext, for
    /// the root key to open and nothing else to look at.
    pub fn wrapped_master(&self, id: &str) -> Result<Option<Vec<u8>>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT wrapped_master FROM lockboxes WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn list_lockboxes(&self) -> Result<Vec<Lockbox>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {LOCKBOX_COLUMNS} FROM lockboxes ORDER BY created_at_ms, rowid"
        ))?;
        collect(&mut stmt, [], read_lockbox)
    }

    pub fn set_frozen(&self, id: &str, frozen: bool) -> Result<()> {
        self.one_row_changed(
            self.conn().execute(
                "UPDATE lockboxes SET frozen = ?1 WHERE id = ?2",
                params![frozen, id],
            )?,
            || format!("lockbox {id}"),
        )
    }

    pub fn set_agents(&self, id: &str, agents: bool) -> Result<()> {
        self.one_row_changed(
            self.conn().execute(
                "UPDATE lockboxes SET agents = ?1 WHERE id = ?2",
                params![agents, id],
            )?,
            || format!("lockbox {id}"),
        )
    }

    /// Move the revision once, for a write of the lockbox file that has
    /// landed; the new revision, which is the ETag the writer answers with.
    pub fn bump_revision(&self, id: &str) -> Result<i64> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let changed = tx.execute(
            "UPDATE lockboxes SET revision = revision + 1 WHERE id = ?1",
            params![id],
        )?;
        self.one_row_changed(changed, || format!("lockbox {id}"))?;
        let revision = tx.query_row(
            "SELECT revision FROM lockboxes WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )?;
        tx.commit()?;
        Ok(revision)
    }

    pub fn revision(&self, id: &str) -> Result<Option<i64>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT revision FROM lockboxes WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Give `member_id` `role` on `lockbox_id`, replacing the role they had.
    pub fn add_lockbox_member(
        &self,
        lockbox_id: &str,
        member_id: &str,
        role: LockboxRole,
    ) -> Result<()> {
        self.conn().execute(
            "INSERT INTO lockbox_members (lockbox_id, member_id, role) VALUES (?1, ?2, ?3)
             ON CONFLICT (lockbox_id, member_id) DO UPDATE SET role = excluded.role",
            params![lockbox_id, member_id, role.as_str()],
        )?;
        Ok(())
    }

    pub fn remove_lockbox_member(&self, lockbox_id: &str, member_id: &str) -> Result<()> {
        self.conn().execute(
            "DELETE FROM lockbox_members WHERE lockbox_id = ?1 AND member_id = ?2",
            params![lockbox_id, member_id],
        )?;
        Ok(())
    }

    pub fn lockbox_members(&self, lockbox_id: &str) -> Result<Vec<LockboxMember>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT member_id, role FROM lockbox_members WHERE lockbox_id = ?1 ORDER BY member_id",
        )?;
        collect(&mut stmt, params![lockbox_id], |row| {
            Ok(LockboxMember {
                member_id: row.get("member_id")?,
                role: text_column(row, "role")?,
            })
        })
    }

    /// Every lockbox `member_id` holds in any role.
    pub fn lockboxes_of_member(&self, member_id: &str) -> Result<Vec<Lockbox>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {LOCKBOX_COLUMNS} FROM lockboxes
             WHERE id IN (SELECT lockbox_id FROM lockbox_members WHERE member_id = ?1)
             ORDER BY created_at_ms, rowid"
        ))?;
        collect(&mut stmt, params![member_id], read_lockbox)
    }

    /// The members an ask for `lockbox_id` is routed to: approvers and
    /// admins of it.
    pub fn approvers_of(&self, lockbox_id: &str) -> Result<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT member_id FROM lockbox_members
             WHERE lockbox_id = ?1 AND role IN (?2, ?3) ORDER BY member_id",
        )?;
        collect(
            &mut stmt,
            params![
                lockbox_id,
                LockboxRole::Approver.as_str(),
                LockboxRole::Admin.as_str()
            ],
            |row| row.get(0),
        )
    }

    /// Keep the master of `lockbox_id` sealed to `device_id`, at the
    /// generation it was sealed at; a later seal replaces an earlier one.
    pub fn put_device_master(
        &self,
        lockbox_id: &str,
        device_id: &str,
        gen: i64,
        sealed: &str,
    ) -> Result<()> {
        self.conn().execute(
            "INSERT INTO device_masters (lockbox_id, device_id, gen, sealed) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (lockbox_id, device_id) DO UPDATE SET gen = excluded.gen, sealed = excluded.sealed",
            params![lockbox_id, device_id, gen, sealed],
        )?;
        Ok(())
    }

    /// The `(gen, sealed master)` held for the device, if any.
    pub fn device_master(
        &self,
        lockbox_id: &str,
        device_id: &str,
    ) -> Result<Option<(i64, String)>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT gen, sealed FROM device_masters WHERE lockbox_id = ?1 AND device_id = ?2",
                params![lockbox_id, device_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?)
    }

    pub fn delete_device_masters_of_device(&self, device_id: &str) -> Result<usize> {
        Ok(self.conn().execute(
            "DELETE FROM device_masters WHERE device_id = ?1",
            params![device_id],
        )?)
    }
}
