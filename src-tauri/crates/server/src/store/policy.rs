//! Policy rules, per lockbox or tenant-wide, and the asks waiting on a
//! member's answer.

use rusqlite::{params, Row};
use serde::Serialize;

use rowel_broker::grant::CreatedBy;
use rowel_broker::policy::Rule;

use super::{collect, now_ms, text_column, Result, Store};

/// The word the server's rules are created by, as `rowel-serve` forces its
/// config rules to `CreatedBy::Admin("config")`: an audit row about a grant
/// under one names where the rule came from, whatever the stored JSON says.
pub const SERVER_ADMIN: &str = "server";

text_enum! {
    /// Where an ask stands: waiting, answered by a device (yes or no), or
    /// past its deadline unanswered.
    PendingState { Pending => "pending", Answered => "answered", Expired => "expired" }
}

/// An ask routed to members' apps, as recorded: the `Approval` the stream
/// carries (`docs/server-api.md`), kept as its JSON since the shape is the
/// app's, and what became of it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingApproval {
    pub id: String,
    pub lockbox_id: String,
    pub agent_id: Option<String>,
    pub client_key: String,
    pub approval: String,
    pub state: PendingState,
    pub created_at_ms: i64,
    pub expires_at_ms: i64,
    pub answered_at_ms: Option<i64>,
    pub answered_by_device: Option<String>,
}

const PENDING_COLUMNS: &str =
    "id, lockbox_id, agent_id, client_key, approval, state, created_at_ms,
                               expires_at_ms, answered_at_ms, answered_by_device";

fn read_pending(row: &Row<'_>) -> rusqlite::Result<PendingApproval> {
    Ok(PendingApproval {
        id: row.get("id")?,
        lockbox_id: row.get("lockbox_id")?,
        agent_id: row.get("agent_id")?,
        client_key: row.get("client_key")?,
        approval: row.get("approval")?,
        state: text_column(row, "state")?,
        created_at_ms: row.get("created_at_ms")?,
        expires_at_ms: row.get("expires_at_ms")?,
        answered_at_ms: row.get("answered_at_ms")?,
        answered_by_device: row.get("answered_by_device")?,
    })
}

fn read_rule(row: &Row<'_>) -> rusqlite::Result<Rule> {
    let json: String = row.get("rule")?;
    let mut rule: Rule = serde_json::from_str(&json).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })?;
    rule.created_by = CreatedBy::Admin(SERVER_ADMIN.into());
    Ok(rule)
}

impl Store {
    /// Keep `rule` for `lockbox_id`, or for every lockbox with `None`. The
    /// rule's own id is the row's, so `rowel admin rules remove` names it.
    pub fn add_rule(&self, lockbox_id: Option<&str>, rule: &Rule) -> Result<()> {
        self.conn().execute(
            "INSERT INTO rules (id, lockbox_id, rule, created_at_ms) VALUES (?1, ?2, ?3, ?4)",
            params![rule.id, lockbox_id, serde_json::to_string(rule)?, now_ms()],
        )?;
        Ok(())
    }

    pub fn remove_rule(&self, id: &str) -> Result<()> {
        self.one_row_changed(
            self.conn()
                .execute("DELETE FROM rules WHERE id = ?1", params![id])?,
            || format!("rule {id}"),
        )
    }

    /// The rules in force for `lockbox_id`: its own and the tenant-wide
    /// ones, oldest first, each the server's (`SERVER_ADMIN`).
    pub fn rules_for(&self, lockbox_id: &str) -> Result<Vec<Rule>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT rule FROM rules WHERE lockbox_id = ?1 OR lockbox_id IS NULL
             ORDER BY created_at_ms, rowid",
        )?;
        collect(&mut stmt, params![lockbox_id], read_rule)
    }

    /// Every rule with the lockbox it is for (`None` tenant-wide).
    pub fn list_rules(&self) -> Result<Vec<(Option<String>, Rule)>> {
        let conn = self.conn();
        let mut stmt =
            conn.prepare("SELECT lockbox_id, rule FROM rules ORDER BY created_at_ms, rowid")?;
        collect(&mut stmt, [], |row| {
            Ok((row.get("lockbox_id")?, read_rule(row)?))
        })
    }

    /// Record an ask as it goes out to the members' apps.
    pub fn record_pending(
        &self,
        id: &str,
        lockbox_id: &str,
        agent_id: Option<&str>,
        client_key: &str,
        approval_json: &str,
        expires_at_ms: i64,
    ) -> Result<PendingApproval> {
        let pending = PendingApproval {
            id: id.into(),
            lockbox_id: lockbox_id.into(),
            agent_id: agent_id.map(String::from),
            client_key: client_key.into(),
            approval: approval_json.into(),
            state: PendingState::Pending,
            created_at_ms: now_ms(),
            expires_at_ms,
            answered_at_ms: None,
            answered_by_device: None,
        };
        self.conn().execute(
            "INSERT INTO pending_approvals (id, lockbox_id, agent_id, client_key, approval, state,
                                            created_at_ms, expires_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                pending.id,
                pending.lockbox_id,
                pending.agent_id,
                pending.client_key,
                pending.approval,
                pending.state.as_str(),
                pending.created_at_ms,
                pending.expires_at_ms
            ],
        )?;
        Ok(pending)
    }

    /// Move a pending ask to `state`, by `by_device` when a device answered.
    /// `true` when this call settled it; `false` when it was no longer
    /// pending — the first answer wins, and a late one is told so.
    pub fn settle_pending(
        &self,
        id: &str,
        state: PendingState,
        by_device: Option<&str>,
    ) -> Result<bool> {
        let changed = self.conn().execute(
            "UPDATE pending_approvals SET state = ?1, answered_at_ms = ?2, answered_by_device = ?3
             WHERE id = ?4 AND state = ?5",
            params![
                state.as_str(),
                now_ms(),
                by_device,
                id,
                PendingState::Pending.as_str()
            ],
        )?;
        Ok(changed == 1)
    }

    /// The asks still waiting on any of `lockbox_ids`, oldest first: what a
    /// member's stream sends first when it opens.
    pub fn list_pending(&self, lockbox_ids: &[String]) -> Result<Vec<PendingApproval>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {PENDING_COLUMNS} FROM pending_approvals
             WHERE state = ?1 ORDER BY created_at_ms, rowid"
        ))?;
        let mut pending = collect(
            &mut stmt,
            params![PendingState::Pending.as_str()],
            read_pending,
        )?;
        pending.retain(|ask| lockbox_ids.contains(&ask.lockbox_id));
        Ok(pending)
    }
}
