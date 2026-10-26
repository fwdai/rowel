//! The tenant's own events: a member signing in, a device enrolled,
//! approved or revoked, and every change an admin makes. They are kept
//! apart from the audit mirror on purpose. A lockbox's audit chain is the
//! lockbox's — it lives inside the file, syncs to every device that holds
//! it and verifies as a chain — and a member's sign-in or an admin adding
//! an agent is not an event of any one lockbox, so writing it into one
//! would hand it to every device of that lockbox and leave the other
//! lockboxes without it.

use rusqlite::{params, Row};
use serde::Serialize;

use rowel_core::crypto::random_hex_id;

use super::{collect, now_ms, Result, Store};

/// One tenant event. `member_id` is who acted (the member signing in, the
/// admin making a change); `device_id` is the device the event is about,
/// for the events that are about one; `detail` is compact JSON naming what
/// else the action touched, never a token or a key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerEvent {
    pub id: String,
    pub ts_ms: i64,
    pub event: String,
    pub member_id: Option<String>,
    pub device_id: Option<String>,
    pub detail: Option<String>,
}

const EVENT_COLUMNS: &str = "id, ts_ms, event, member_id, device_id, detail";

fn read_event(row: &Row<'_>) -> rusqlite::Result<ServerEvent> {
    Ok(ServerEvent {
        id: row.get("id")?,
        ts_ms: row.get("ts_ms")?,
        event: row.get("event")?,
        member_id: row.get("member_id")?,
        device_id: row.get("device_id")?,
        detail: row.get("detail")?,
    })
}

impl Store {
    pub fn record_event(
        &self,
        event: &str,
        member_id: Option<&str>,
        device_id: Option<&str>,
        detail: Option<&str>,
    ) -> Result<ServerEvent> {
        let recorded = ServerEvent {
            id: random_hex_id(),
            ts_ms: now_ms(),
            event: event.into(),
            member_id: member_id.map(String::from),
            device_id: device_id.map(String::from),
            detail: detail.map(String::from),
        };
        self.conn().execute(
            "INSERT INTO server_events (id, ts_ms, event, member_id, device_id, detail)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                recorded.id,
                recorded.ts_ms,
                recorded.event,
                recorded.member_id,
                recorded.device_id,
                recorded.detail
            ],
        )?;
        Ok(recorded)
    }

    /// At most `limit` events from `since_ms` on, oldest first, so a reader
    /// pages forward from the last `tsMs` it saw.
    pub fn events_since(&self, since_ms: i64, limit: usize) -> Result<Vec<ServerEvent>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {EVENT_COLUMNS} FROM server_events WHERE ts_ms >= ?1
             ORDER BY ts_ms, rowid LIMIT ?2"
        ))?;
        collect(
            &mut stmt,
            params![since_ms, i64::try_from(limit).unwrap_or(i64::MAX)],
            read_event,
        )
    }
}
