//! The audit mirror: every lockbox's rows copied out of the SQLCipher file
//! as they are written or merged, so an export covers the tenant without
//! opening each lockbox, and a lockbox restored elsewhere still carries its
//! own history inside it (spec `09`).

use rusqlite::{params, Row};

use rowel_core::store::AuditRecord;

use super::{collect, Result, Store};

/// What to export: a time window, one lockbox, one actor; each `None` is
/// no filter.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AuditFilter {
    pub since_ms: Option<i64>,
    pub until_ms: Option<i64>,
    pub lockbox_id: Option<String>,
    /// The `actor_id` column: an agent's id, or a member's.
    pub actor_id: Option<String>,
}

const RECORD_COLUMNS: &str = "lockbox_id, writer_id, seq, id, ts_ms, prev_hash, hash, actor_kind,
                              actor_id, actor_name, run_id, acting_for, grant_id, item_id, target,
                              mode, approval, approver, event, detail";

fn hash_column(row: &Row<'_>, column: &str) -> rusqlite::Result<[u8; 32]> {
    let bytes: Vec<u8> = row.get(column)?;
    bytes.try_into().map_err(|bytes: Vec<u8>| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Blob,
            format!("a hash is 32 bytes, not {}", bytes.len()).into(),
        )
    })
}

fn read_record(row: &Row<'_>) -> rusqlite::Result<(String, AuditRecord)> {
    Ok((
        row.get("lockbox_id")?,
        AuditRecord {
            id: row.get("id")?,
            writer_id: row.get("writer_id")?,
            seq: row.get("seq")?,
            ts_ms: row.get("ts_ms")?,
            prev_hash: hash_column(row, "prev_hash")?,
            hash: hash_column(row, "hash")?,
            actor_kind: row.get("actor_kind")?,
            actor_id: row.get("actor_id")?,
            actor_name: row.get("actor_name")?,
            run_id: row.get("run_id")?,
            acting_for: row.get("acting_for")?,
            grant_id: row.get("grant_id")?,
            item_id: row.get("item_id")?,
            target: row.get("target")?,
            mode: row.get("mode")?,
            approval: row.get("approval")?,
            approver: row.get("approver")?,
            event: row.get("event")?,
            detail: row.get("detail")?,
        },
    ))
}

impl Store {
    /// Copy `rows` of `lockbox_id` in; how many were new. A row already
    /// held under its `(writer, seq)` is left as it is: the mirror is
    /// insert-only like the chain it copies.
    pub fn mirror_audit(&self, lockbox_id: &str, rows: &[AuditRecord]) -> Result<usize> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let mut inserted = 0;
        {
            let mut stmt = tx.prepare(&format!(
                "INSERT OR IGNORE INTO audit ({RECORD_COLUMNS})
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)"
            ))?;
            for row in rows {
                inserted += stmt.execute(params![
                    lockbox_id,
                    row.writer_id,
                    row.seq,
                    row.id,
                    row.ts_ms,
                    row.prev_hash.as_slice(),
                    row.hash.as_slice(),
                    row.actor_kind,
                    row.actor_id,
                    row.actor_name,
                    row.run_id,
                    row.acting_for,
                    row.grant_id,
                    row.item_id,
                    row.target,
                    row.mode,
                    row.approval,
                    row.approver,
                    row.event,
                    row.detail,
                ])?;
            }
        }
        tx.commit()?;
        Ok(inserted)
    }

    /// The rows `filter` selects, in `(lockbox, writer, seq)` order, so an
    /// export reads as the chains it came from and verifies as a lockbox
    /// does.
    pub fn audit_export(&self, filter: &AuditFilter) -> Result<Vec<(String, AuditRecord)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {RECORD_COLUMNS} FROM audit
             WHERE (?1 IS NULL OR ts_ms >= ?1)
               AND (?2 IS NULL OR ts_ms < ?2)
               AND (?3 IS NULL OR lockbox_id = ?3)
               AND (?4 IS NULL OR actor_id = ?4)
             ORDER BY lockbox_id, writer_id, seq"
        ))?;
        collect(
            &mut stmt,
            params![
                filter.since_ms,
                filter.until_ms,
                filter.lockbox_id,
                filter.actor_id
            ],
            read_record,
        )
    }

    /// The highest `seq` mirrored for `writer_id`'s chain in `lockbox_id`,
    /// 0 when none: where the next mirror pass starts reading the lockbox.
    pub fn audit_high_water(&self, lockbox_id: &str, writer_id: &str) -> Result<i64> {
        Ok(self.conn().query_row(
            "SELECT COALESCE(MAX(seq), 0) FROM audit WHERE lockbox_id = ?1 AND writer_id = ?2",
            params![lockbox_id, writer_id],
            |row| row.get(0),
        )?)
    }
}
