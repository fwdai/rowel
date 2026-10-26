//! Agent definitions, the trust policies that say which workload tokens
//! become which agent, and the clients those tokens became.

use std::collections::BTreeMap;

use rusqlite::{params, OptionalExtension, Row};
use serde::Serialize;

use rowel_broker::actions::{Client, ClientKind};
use rowel_core::crypto::random_hex_id;

use super::{collect, now_ms, Result, Store, StoreError};

/// An agent definition. In this iteration it is bound to exactly one org
/// lockbox: a connection that associated as the agent serves that lockbox.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Agent {
    pub id: String,
    pub name: String,
    pub lockbox_id: String,
    /// Off, the definition's clients are refused at their next request.
    pub enabled: bool,
    pub created_at_ms: i64,
}

/// Which workload tokens become `agent_id`: those from `issuer` whose
/// claims match `claims`, each a pattern an admin wrote (exact, or a glob
/// where the verifier allows one).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrustPolicy {
    pub id: String,
    pub agent_id: String,
    pub issuer: String,
    pub claims: BTreeMap<String, String>,
    /// The platform cannot set the token's audience, so the audience is the
    /// bare one and the challenge nonce binds the token to the channel
    /// instead (`docs/server-api.md`, "Verification").
    pub fixed_audience: bool,
    /// The one kind of client a token matching this policy may associate
    /// as: `agent` or `sidecar` (spec `05`, Tier 2: a sidecar proves an
    /// identity of its own). A token that matches only an agent's policy
    /// cannot become that agent's sidecar, and the other way round.
    #[serde(rename = "clientKind")]
    pub kind: ClientKind,
    pub created_at_ms: i64,
}

/// `kind` if a trust policy may admit it. A browser extension is never let
/// in by a workload token: it is a person's, and reaches a server through an
/// enrollment code (I11), so a policy naming it could only be a mistake.
pub fn workload_kind(kind: ClientKind) -> std::result::Result<ClientKind, &'static str> {
    match kind {
        ClientKind::Agent | ClientKind::Sidecar => Ok(kind),
        ClientKind::Extension => {
            Err("a trust policy admits an agent or a sidecar, never an extension")
        }
    }
}

const AGENT_COLUMNS: &str = "id, name, lockbox_id, enabled, created_at_ms";
const POLICY_COLUMNS: &str =
    "id, agent_id, issuer, claims, fixed_audience, client_kind, created_at_ms";
const CLIENT_COLUMNS: &str = "key, agent_id, name, kind, platform, created_at_ms, last_seen_at_ms";

fn read_agent(row: &Row<'_>) -> rusqlite::Result<Agent> {
    Ok(Agent {
        id: row.get("id")?,
        name: row.get("name")?,
        lockbox_id: row.get("lockbox_id")?,
        enabled: row.get("enabled")?,
        created_at_ms: row.get("created_at_ms")?,
    })
}

fn read_policy(row: &Row<'_>) -> rusqlite::Result<TrustPolicy> {
    let claims: String = row.get("claims")?;
    Ok(TrustPolicy {
        id: row.get("id")?,
        agent_id: row.get("agent_id")?,
        issuer: row.get("issuer")?,
        claims: serde_json::from_str(&claims).map_err(json_column)?,
        fixed_audience: row.get("fixed_audience")?,
        kind: read_kind(row, "client_kind")?,
        created_at_ms: row.get("created_at_ms")?,
    })
}

fn read_kind(row: &Row<'_>, column: &str) -> rusqlite::Result<ClientKind> {
    let kind: String = row.get(column)?;
    serde_json::from_value(serde_json::Value::String(kind)).map_err(json_column)
}

fn read_client(row: &Row<'_>) -> rusqlite::Result<Client> {
    Ok(Client {
        key: row.get("key")?,
        agent_id: Some(row.get("agent_id")?),
        name: row.get("name")?,
        kind: read_kind(row, "kind")?,
        platform: row.get("platform")?,
        created_at_ms: row.get("created_at_ms")?,
        last_seen_at_ms: row.get("last_seen_at_ms")?,
        // A server client is never a person's browser extension marked as
        // agent-driven; the switch is the app's.
        driven_by_agent: false,
    })
}

fn json_column(e: serde_json::Error) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
}

/// The word `kind` is stored as: its wire spelling, so the column reads
/// like the protocol does.
fn kind_text(kind: ClientKind) -> Result<String> {
    match serde_json::to_value(kind)? {
        serde_json::Value::String(text) => Ok(text),
        other => Err(StoreError::Other(format!(
            "a client kind is a word, not {other}"
        ))),
    }
}

impl Store {
    pub fn add_agent(&self, name: &str, lockbox_id: &str) -> Result<Agent> {
        let agent = Agent {
            id: random_hex_id(),
            name: name.into(),
            lockbox_id: lockbox_id.into(),
            enabled: true,
            created_at_ms: now_ms(),
        };
        self.conn().execute(
            "INSERT INTO agents (id, name, lockbox_id, enabled, created_at_ms) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                agent.id,
                agent.name,
                agent.lockbox_id,
                agent.enabled,
                agent.created_at_ms
            ],
        )?;
        Ok(agent)
    }

    pub fn agent(&self, id: &str) -> Result<Option<Agent>> {
        Ok(self
            .conn()
            .query_row(
                &format!("SELECT {AGENT_COLUMNS} FROM agents WHERE id = ?1"),
                params![id],
                read_agent,
            )
            .optional()?)
    }

    pub fn list_agents(&self) -> Result<Vec<Agent>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {AGENT_COLUMNS} FROM agents ORDER BY created_at_ms, rowid"
        ))?;
        collect(&mut stmt, [], read_agent)
    }

    pub fn rename_agent(&self, id: &str, name: &str) -> Result<()> {
        self.one_row_changed(
            self.conn().execute(
                "UPDATE agents SET name = ?1 WHERE id = ?2",
                params![name, id],
            )?,
            || format!("agent {id}"),
        )
    }

    pub fn set_agent_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        self.one_row_changed(
            self.conn().execute(
                "UPDATE agents SET enabled = ?1 WHERE id = ?2",
                params![enabled, id],
            )?,
            || format!("agent {id}"),
        )
    }

    /// Remove the definition; its trust policies and clients go with it.
    pub fn remove_agent(&self, id: &str) -> Result<()> {
        self.one_row_changed(
            self.conn()
                .execute("DELETE FROM agents WHERE id = ?1", params![id])?,
            || format!("agent {id}"),
        )
    }

    pub fn add_trust_policy(
        &self,
        agent_id: &str,
        issuer: &str,
        claims: BTreeMap<String, String>,
        fixed_audience: bool,
        kind: ClientKind,
    ) -> Result<TrustPolicy> {
        let policy = TrustPolicy {
            id: random_hex_id(),
            agent_id: agent_id.into(),
            issuer: issuer.into(),
            claims,
            fixed_audience,
            kind: workload_kind(kind).map_err(|e| StoreError::Other(e.into()))?,
            created_at_ms: now_ms(),
        };
        self.conn().execute(
            "INSERT INTO trust_policies (id, agent_id, issuer, claims, fixed_audience, client_kind, created_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                policy.id,
                policy.agent_id,
                policy.issuer,
                serde_json::to_string(&policy.claims)?,
                policy.fixed_audience,
                kind_text(policy.kind)?,
                policy.created_at_ms
            ],
        )?;
        Ok(policy)
    }

    pub fn remove_trust_policy(&self, id: &str) -> Result<()> {
        self.one_row_changed(
            self.conn()
                .execute("DELETE FROM trust_policies WHERE id = ?1", params![id])?,
            || format!("trust policy {id}"),
        )
    }

    pub fn trust_policies_of(&self, agent_id: &str) -> Result<Vec<TrustPolicy>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {POLICY_COLUMNS} FROM trust_policies WHERE agent_id = ?1 ORDER BY created_at_ms, rowid"
        ))?;
        collect(&mut stmt, params![agent_id], read_policy)
    }

    /// Every policy for tokens from `issuer`, across agents: what a
    /// verifier matches a token's claims against.
    pub fn trust_policies_for_issuer(&self, issuer: &str) -> Result<Vec<TrustPolicy>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {POLICY_COLUMNS} FROM trust_policies WHERE issuer = ?1 ORDER BY created_at_ms, rowid"
        ))?;
        collect(&mut stmt, params![issuer], read_policy)
    }

    /// Keep `client` as association wrote it. A server client is always a
    /// definition's: one without `agent_id` is refused, since nothing could
    /// ever look it up again.
    pub fn put_client(&self, client: &Client) -> Result<()> {
        let agent_id = client.agent_id.as_deref().ok_or_else(|| {
            StoreError::Other("a server client carries the agent definition it resolved to".into())
        })?;
        self.conn().execute(
            "INSERT INTO clients (key, agent_id, name, kind, platform, created_at_ms, last_seen_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT (key) DO UPDATE SET agent_id = excluded.agent_id, name = excluded.name,
                 kind = excluded.kind, platform = excluded.platform,
                 last_seen_at_ms = excluded.last_seen_at_ms",
            params![
                client.key,
                agent_id,
                client.name,
                kind_text(client.kind)?,
                client.platform,
                client.created_at_ms,
                client.last_seen_at_ms
            ],
        )?;
        Ok(())
    }

    pub fn client(&self, key: &str) -> Result<Option<Client>> {
        Ok(self
            .conn()
            .query_row(
                &format!("SELECT {CLIENT_COLUMNS} FROM clients WHERE key = ?1"),
                params![key],
                read_client,
            )
            .optional()?)
    }

    pub fn clients(&self) -> Result<Vec<Client>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {CLIENT_COLUMNS} FROM clients ORDER BY created_at_ms, rowid"
        ))?;
        collect(&mut stmt, [], read_client)
    }

    pub fn client_seen(&self, key: &str) -> Result<()> {
        self.one_row_changed(
            self.conn().execute(
                "UPDATE clients SET last_seen_at_ms = ?1 WHERE key = ?2",
                params![now_ms(), key],
            )?,
            || format!("client {key}"),
        )
    }

    pub fn delete_clients_of_agent(&self, agent_id: &str) -> Result<usize> {
        Ok(self
            .conn()
            .execute("DELETE FROM clients WHERE agent_id = ?1", params![agent_id])?)
    }

    /// Forget clients not seen since `unseen_before_ms` (one never seen
    /// counts from when it was let in). A CI run mints a key per run and
    /// never comes back under it, so without this the table only grows.
    pub fn prune_clients(&self, unseen_before_ms: i64) -> Result<usize> {
        Ok(self.conn().execute(
            "DELETE FROM clients WHERE COALESCE(last_seen_at_ms, created_at_ms) < ?1",
            params![unseen_before_ms],
        )?)
    }
}
