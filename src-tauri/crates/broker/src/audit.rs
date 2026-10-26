//! The audit log: what agents, people and policy did with the lockbox's
//! secrets, one row per decision and per use, written the same way by every
//! tier. The question it answers after the fact is "what did my agents do?",
//! and for a company's auditor "who approved it, and is this log complete?" —
//! so each writer chains its rows: a row's `hash` covers the row and the
//! previous row's hash, and [`verify`] walks a chain and names the first row
//! that is missing or was altered. Writers chain separately (a device, a
//! server), so syncing rows between them never breaks a chain.
//!
//! Nothing secret goes in a row, ever: not a value, not a code, not a URL's
//! query string. `detail` is a small JSON the caller composes and is capped.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::grant::{Approval, Mode};
use rowel_core::crypto::random_hex_id;
use rowel_core::error::{store_err, Error};
use rowel_core::store::{AuditRecord, SqliteStore};

/// The most a `detail` may hold. A row describes; it does not carry.
pub const DETAIL_MAX_BYTES: usize = 2048;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Event {
    Requested,
    Granted,
    Denied,
    Used,
    Exposed,
    Ended,
    Revoked,
    Associated,
    RevokedClient,
    RuleAdded,
    RuleRemoved,
    KeyUnwrapped,
    RateLimited,
    PrunedThrough,
}

impl Event {
    /// The column's word.
    pub fn as_str(self) -> &'static str {
        match self {
            Event::Requested => "requested",
            Event::Granted => "granted",
            Event::Denied => "denied",
            Event::Used => "used",
            Event::Exposed => "exposed",
            Event::Ended => "ended",
            Event::Revoked => "revoked",
            Event::Associated => "associated",
            Event::RevokedClient => "revoked_client",
            Event::RuleAdded => "rule_added",
            Event::RuleRemoved => "rule_removed",
            Event::KeyUnwrapped => "key_unwrapped",
            Event::RateLimited => "rate_limited",
            Event::PrunedThrough => "pruned_through",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ActorKind {
    Human,
    Extension,
    Agent,
    Sidecar,
    Policy,
    Server,
}

impl ActorKind {
    /// The column's word.
    pub fn as_str(self) -> &'static str {
        match self {
            ActorKind::Human => "human",
            ActorKind::Extension => "extension",
            ActorKind::Agent => "agent",
            ActorKind::Sidecar => "sidecar",
            ActorKind::Policy => "policy",
            ActorKind::Server => "server",
        }
    }
}

/// A row before it is chained: what the caller knows. The writer adds the id,
/// its own id, the sequence number and the hashes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Draft {
    pub actor_kind: ActorKind,
    pub actor_id: Option<String>,
    pub actor_name: Option<String>,
    pub run_id: Option<String>,
    pub acting_for: Option<String>,
    pub grant_id: Option<String>,
    pub item_id: Option<String>,
    /// Scheme, host and path at most; never a query string.
    pub target: Option<String>,
    pub mode: Option<Mode>,
    pub approval: Option<Approval>,
    pub approver: Option<String>,
    pub event: Event,
    /// Small JSON the caller composes, at most [`DETAIL_MAX_BYTES`]. No secrets.
    pub detail: Option<String>,
}

impl Draft {
    /// Who did what, and nothing else yet.
    pub fn new(actor_kind: ActorKind, event: Event) -> Self {
        Self {
            actor_kind,
            actor_id: None,
            actor_name: None,
            run_id: None,
            acting_for: None,
            grant_id: None,
            item_id: None,
            target: None,
            mode: None,
            approval: None,
            approver: None,
            event,
            detail: None,
        }
    }
}

/// Where rows go. The store in Tier 1; a server's database in Tier 2.
pub trait AuditSink {
    /// The newest row of `writer_id`'s chain: its `seq` and `hash`.
    fn last(&self, writer_id: &str) -> Result<Option<(i64, [u8; 32])>, Error>;
    fn insert(&self, record: &AuditRecord) -> Result<(), Error>;
}

impl AuditSink for SqliteStore {
    fn last(&self, writer_id: &str) -> Result<Option<(i64, [u8; 32])>, Error> {
        self.audit_last(writer_id).map_err(store_err)
    }
    fn insert(&self, record: &AuditRecord) -> Result<(), Error> {
        self.audit_insert(record).map_err(store_err)
    }
}

/// One writer's pen: rows it records chain from the last row it wrote. A
/// writer is not a lock — two of them with the same id racing on one sink
/// would compute the same `seq`, and the sink refuses the second
/// (`SqliteStore::audit_insert`) — so a host keeps one and records under its
/// own lock.
pub struct Writer<'a, S> {
    writer_id: String,
    sink: &'a S,
}

impl<'a, S: AuditSink> Writer<'a, S> {
    /// `writer_id` is the device's install id in Tier 1, the node's in Tier 2.
    pub fn new(writer_id: &str, sink: &'a S) -> Self {
        Self {
            writer_id: writer_id.to_string(),
            sink,
        }
    }

    pub fn writer_id(&self) -> &str {
        &self.writer_id
    }

    /// Chain `draft` onto this writer's log at `ts_ms` and write it: `seq` is
    /// the last row's plus one (the first is 1), `prev_hash` the last row's
    /// hash (zeros for the first), `hash` over both and the row.
    pub fn record(&self, draft: Draft, ts_ms: i64) -> Result<AuditRecord, Error> {
        let record = self.chain(draft, ts_ms)?;
        self.sink.insert(&record)?;
        Ok(record)
    }

    /// [`Writer::record`] without the write: the row as it would go on, for
    /// a caller that inserts it itself, in one transaction with the write it
    /// stands on (`SqliteStore::commit_with_audit`). The chain's head is read
    /// now and the row goes in later, so the caller holds the lock every
    /// writer of this chain records under across both — a host's session
    /// lock — or the insert finds its `seq` taken and refuses it.
    pub fn chain(&self, draft: Draft, ts_ms: i64) -> Result<AuditRecord, Error> {
        let head = self.sink.last(&self.writer_id)?;
        self.chained_after(draft, ts_ms, head)
    }

    /// [`Writer::record`] chained onto `head` — the `(seq, hash)` of the row
    /// before — rather than onto the sink's newest row. The two differ in one
    /// case only: a chain retention took whole, whose next row must carry on
    /// from the row that went (see [`prune`]) rather than start over at 1.
    fn record_after(
        &self,
        draft: Draft,
        ts_ms: i64,
        head: Option<(i64, [u8; 32])>,
    ) -> Result<AuditRecord, Error> {
        let record = self.chained_after(draft, ts_ms, head)?;
        self.sink.insert(&record)?;
        Ok(record)
    }

    fn chained_after(
        &self,
        draft: Draft,
        ts_ms: i64,
        head: Option<(i64, [u8; 32])>,
    ) -> Result<AuditRecord, Error> {
        if draft
            .detail
            .as_ref()
            .is_some_and(|d| d.len() > DETAIL_MAX_BYTES)
        {
            return Err(Error::Other(format!(
                "audit: detail over {DETAIL_MAX_BYTES} bytes"
            )));
        }
        let (seq, prev_hash) = match head {
            Some((seq, hash)) => (seq + 1, hash),
            None => (1, [0u8; 32]),
        };
        let mut record = AuditRecord {
            id: random_hex_id(),
            writer_id: self.writer_id.clone(),
            seq,
            ts_ms,
            prev_hash,
            hash: [0u8; 32],
            actor_kind: draft.actor_kind.as_str().to_string(),
            actor_id: draft.actor_id,
            actor_name: draft.actor_name,
            run_id: draft.run_id,
            acting_for: draft.acting_for,
            grant_id: draft.grant_id,
            item_id: draft.item_id,
            target: draft.target,
            mode: draft.mode.map(|m| m.as_str().to_string()),
            approval: draft.approval.map(|a| a.as_str().to_string()),
            approver: draft.approver,
            event: draft.event.as_str().to_string(),
            detail: draft.detail,
        };
        record.hash = hash_of(&record)?;
        Ok(record)
    }
}

/// Retention for the log in `store`, by the device whose writer is
/// `writer_id`: reclaim every chain's rows stamped before `cutoff_ms`
/// ([`SqliteStore::audit_prune`]), and when this writer's own rows were among
/// them, chain a `pruned_through` row at `now_ms` saying how far —
/// `{"seq":…,"beforeMs":…}` — so its chain stays verifiable from that row on.
/// The actor is `policy`: this is the device's own housekeeping, not something
/// a person or an agent did. Returns the row written, or `None` when nothing
/// of this writer's was old enough (whatever went of the other chains).
///
/// Only its own chain gets the note: nobody else can extend another writer's
/// chain, and [`verify`] takes a chain that starts past `seq` 1 as pruned, so
/// another writer's chain losing its prefix here stays verifiable without one.
///
/// The note chains from the chain's newest remaining row — or, when the whole
/// chain went, from the row the cut names, as if it had stayed: a peer still
/// holding the old rows then reads them and the note as one unbroken chain,
/// and the note never claims a `(writer_id, seq)` those rows already hold.
pub fn prune(
    store: &SqliteStore,
    writer_id: &str,
    cutoff_ms: i64,
    now_ms: i64,
) -> Result<Option<AuditRecord>, Error> {
    let cuts = store.audit_prune(cutoff_ms).map_err(store_err)?;
    let Some(cut) = cuts.iter().find(|cut| cut.writer_id == writer_id) else {
        return Ok(None);
    };
    let head = store.last(writer_id)?.or(Some((cut.seq, cut.hash)));
    // Spelled out rather than built as a `Value`: the field order is part of
    // what the row hashes to, and a map's order is a crate feature's.
    let detail = format!("{{\"seq\":{},\"beforeMs\":{cutoff_ms}}}", cut.seq);
    Writer::new(writer_id, store)
        .record_after(
            Draft {
                detail: Some(detail),
                ..Draft::new(ActorKind::Policy, Event::PrunedThrough)
            },
            now_ms,
            head,
        )
        .map(Some)
}

/// The row as it is hashed: every column but `hash`, **in this order**. The
/// declaration order is the canonical form — serde writes fields as declared
/// — so reordering, renaming or adding a field here changes every hash ever
/// written. Append-only, and only with a version bump of the whole chain.
/// `prev_hash` is lowercase hex; absent columns are `null`.
#[derive(Serialize)]
struct Canonical<'a> {
    id: &'a str,
    writer_id: &'a str,
    seq: i64,
    ts_ms: i64,
    prev_hash: String,
    actor_kind: &'a str,
    actor_id: Option<&'a str>,
    actor_name: Option<&'a str>,
    run_id: Option<&'a str>,
    acting_for: Option<&'a str>,
    grant_id: Option<&'a str>,
    item_id: Option<&'a str>,
    target: Option<&'a str>,
    mode: Option<&'a str>,
    approval: Option<&'a str>,
    approver: Option<&'a str>,
    event: &'a str,
    detail: Option<&'a str>,
}

/// The JSON a row's hash is computed over (see [`Canonical`]).
pub fn canonical_json(record: &AuditRecord) -> Result<String, Error> {
    serde_json::to_string(&Canonical {
        id: &record.id,
        writer_id: &record.writer_id,
        seq: record.seq,
        ts_ms: record.ts_ms,
        prev_hash: hex(&record.prev_hash),
        actor_kind: &record.actor_kind,
        actor_id: record.actor_id.as_deref(),
        actor_name: record.actor_name.as_deref(),
        run_id: record.run_id.as_deref(),
        acting_for: record.acting_for.as_deref(),
        grant_id: record.grant_id.as_deref(),
        item_id: record.item_id.as_deref(),
        target: record.target.as_deref(),
        mode: record.mode.as_deref(),
        approval: record.approval.as_deref(),
        approver: record.approver.as_deref(),
        event: &record.event,
        detail: record.detail.as_deref(),
    })
    .map_err(|e| Error::Other(format!("audit: {e}")))
}

/// `sha256(prev_hash || canonical_json(row))`: what `hash` must equal.
pub fn hash_of(record: &AuditRecord) -> Result<[u8; 32], Error> {
    let mut hasher = Sha256::new();
    hasher.update(record.prev_hash);
    hasher.update(canonical_json(record)?.as_bytes());
    Ok(hasher.finalize().into())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::with_capacity(64), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

/// Where a chain fails verification: the `seq` at fault (for a gap, the
/// sequence number that is missing) and how.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Break {
    pub seq: i64,
    pub reason: Reason,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    /// A row of another writer is in this chain.
    Writer,
    /// Two rows, each sound on its own, claim this `seq`: the chain forked.
    /// Sync keeps both sides on every device, so every device names the same
    /// `seq`.
    Fork,
    /// A sequence number is missing.
    Gap,
    /// The row does not chain from the one before it.
    PrevHash,
    /// The row's hash is not the hash of its contents.
    Hash,
}

/// Walk one writer's rows in `seq` order: each must follow the one before it
/// by one (never share its `seq`), chain from its hash, and hash to what it
/// says. A chain that starts
/// at `seq` 1 must start from zeros; one that starts later is taken as
/// pruned, and is verified from where it begins.
pub fn verify(rows: &[AuditRecord]) -> Result<(), Break> {
    let mut previous: Option<&AuditRecord> = None;
    for row in rows {
        // The row's own contents first: an altered copy kept beside its
        // original (sync keeps both) is named as altered, not as a fork,
        // whichever of the two the walk meets first.
        let altered = Break {
            seq: row.seq,
            reason: Reason::Hash,
        };
        if hash_of(row).map_err(|_| altered)? != row.hash {
            return Err(altered);
        }
        match previous {
            Some(prev) => {
                if row.writer_id != prev.writer_id {
                    return Err(Break {
                        seq: row.seq,
                        reason: Reason::Writer,
                    });
                }
                if row.seq == prev.seq {
                    return Err(Break {
                        seq: row.seq,
                        reason: Reason::Fork,
                    });
                }
                if row.seq != prev.seq + 1 {
                    return Err(Break {
                        seq: prev.seq + 1,
                        reason: Reason::Gap,
                    });
                }
                if row.prev_hash != prev.hash {
                    return Err(Break {
                        seq: row.seq,
                        reason: Reason::PrevHash,
                    });
                }
            }
            None if row.seq == 1 && row.prev_hash != [0u8; 32] => {
                return Err(Break {
                    seq: row.seq,
                    reason: Reason::PrevHash,
                });
            }
            None => {}
        }
        previous = Some(row);
    }
    Ok(())
}

/// A sink that keeps its rows in memory, for the tests and for a `Mock` host
/// to collect what a protocol exchange logged.
#[cfg(any(test, feature = "test-support"))]
#[derive(Default)]
pub struct MemorySink {
    pub rows: std::cell::RefCell<Vec<AuditRecord>>,
}

#[cfg(any(test, feature = "test-support"))]
impl AuditSink for MemorySink {
    fn last(&self, writer_id: &str) -> Result<Option<(i64, [u8; 32])>, Error> {
        Ok(self
            .rows
            .borrow()
            .iter()
            .filter(|r| r.writer_id == writer_id)
            .max_by_key(|r| r.seq)
            .map(|r| (r.seq, r.hash)))
    }
    fn insert(&self, record: &AuditRecord) -> Result<(), Error> {
        let mut rows = self.rows.borrow_mut();
        if rows
            .iter()
            .any(|r| r.writer_id == record.writer_id && r.seq == record.seq)
        {
            return Err(Error::Other("audit: duplicate (writer_id, seq)".into()));
        }
        rows.push(record.clone());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn used(grant_id: &str) -> Draft {
        Draft {
            actor_id: Some("id-key".into()),
            actor_name: Some("Chrome".into()),
            grant_id: Some(grant_id.into()),
            item_id: Some("gh".into()),
            target: Some("https://github.com/login".into()),
            mode: Some(Mode::Fill),
            approval: Some(Approval::Ask),
            approver: Some("user".into()),
            detail: Some("{\"submitted\":true}".into()),
            ..Draft::new(ActorKind::Extension, Event::Used)
        }
    }

    fn chain(n: usize) -> (MemorySink, Vec<AuditRecord>) {
        let sink = MemorySink::default();
        let writer = Writer::new("device-a", &sink);
        let rows = (0..n)
            .map(|i| {
                writer
                    .record(used(&format!("g{i}")), 1_000 + i as i64)
                    .unwrap()
            })
            .collect();
        (sink, rows)
    }

    #[test]
    fn the_canonical_form_is_the_columns_in_order_without_the_hash() {
        let record = AuditRecord {
            id: "0011".into(),
            writer_id: "device-a".into(),
            seq: 1,
            ts_ms: 1_700_000_000_000,
            prev_hash: [0u8; 32],
            hash: [9u8; 32],
            actor_kind: "extension".into(),
            actor_id: Some("id-key".into()),
            actor_name: Some("Chrome".into()),
            run_id: None,
            acting_for: None,
            grant_id: Some("g1".into()),
            item_id: Some("gh".into()),
            target: Some("https://github.com/login".into()),
            mode: Some("fill".into()),
            approval: Some("ask".into()),
            approver: Some("user".into()),
            event: "used".into(),
            detail: Some("{\"submitted\":true}".into()),
        };
        assert_eq!(
            canonical_json(&record).unwrap(),
            concat!(
                "{\"id\":\"0011\",\"writer_id\":\"device-a\",\"seq\":1,\"ts_ms\":1700000000000,",
                "\"prev_hash\":\"0000000000000000000000000000000000000000000000000000000000000000\",",
                "\"actor_kind\":\"extension\",\"actor_id\":\"id-key\",\"actor_name\":\"Chrome\",",
                "\"run_id\":null,\"acting_for\":null,\"grant_id\":\"g1\",\"item_id\":\"gh\",",
                "\"target\":\"https://github.com/login\",\"mode\":\"fill\",\"approval\":\"ask\",",
                "\"approver\":\"user\",\"event\":\"used\",\"detail\":\"{\\\"submitted\\\":true}\"}"
            )
        );
        // The hash is over prev_hash || that JSON, and nothing in `hash`
        // itself takes part: the same row under another `hash` hashes the same.
        let mut hasher = Sha256::new();
        hasher.update([0u8; 32]);
        hasher.update(canonical_json(&record).unwrap().as_bytes());
        let expected: [u8; 32] = hasher.finalize().into();
        assert_eq!(hash_of(&record).unwrap(), expected);
        assert_eq!(
            hash_of(&AuditRecord {
                hash: [1u8; 32],
                ..record
            })
            .unwrap(),
            expected
        );
    }

    #[test]
    fn the_same_draft_under_the_same_predecessor_hashes_the_same() {
        let (_, a) = chain(2);
        let (_, b) = chain(2);
        // Ids and hashes differ only through the random id: pin it.
        let pin = |mut row: AuditRecord| {
            row.id = "fixed".into();
            row.hash = hash_of(&row).unwrap();
            row
        };
        let (a1, b1) = (pin(a[0].clone()), pin(b[0].clone()));
        assert_eq!(a1.hash, b1.hash);
        assert_ne!(a1.hash, [0u8; 32]);
        // And a change to any column changes it.
        let altered = pin(AuditRecord {
            detail: Some("{\"submitted\":false}".into()),
            ..a[0].clone()
        });
        assert_ne!(altered.hash, a1.hash);
    }

    #[test]
    fn a_writer_chains_its_rows_and_a_chain_verifies() {
        let (sink, rows) = chain(3);
        assert_eq!(rows.iter().map(|r| r.seq).collect::<Vec<_>>(), [1, 2, 3]);
        assert_eq!(rows[0].prev_hash, [0u8; 32]);
        assert_eq!(rows[1].prev_hash, rows[0].hash);
        assert_eq!(rows[2].prev_hash, rows[1].hash);
        assert_eq!(rows[0].writer_id, "device-a");
        assert_eq!(rows[0].actor_kind, "extension");
        assert_eq!(rows[0].mode.as_deref(), Some("fill"));
        assert_eq!(rows[0].approval.as_deref(), Some("ask"));
        assert_eq!(rows[0].event, "used");
        assert!(rows.iter().all(|r| r.id.len() == 32));
        assert_eq!(verify(&rows), Ok(()));
        assert_eq!(verify(&sink.rows.borrow()), Ok(()));
        assert_eq!(verify(&[]), Ok(()));

        // A second writer on the same sink starts its own chain.
        let other = Writer::new("device-b", &sink);
        let first = other.record(used("g9"), 2_000).unwrap();
        assert_eq!((first.seq, first.prev_hash), (1, [0u8; 32]));
        let next = Writer::new("device-a", &sink)
            .record(used("g10"), 2_001)
            .unwrap();
        assert_eq!((next.seq, next.prev_hash), (4, rows[2].hash));
    }

    #[test]
    fn verify_names_the_first_altered_row() {
        let (_, mut rows) = chain(3);
        rows[1].detail = Some("{\"submitted\":false}".into());
        assert_eq!(
            verify(&rows),
            Err(Break {
                seq: 2,
                reason: Reason::Hash
            })
        );
        // Re-hashing the altered row does not help: the next row no longer
        // chains from it.
        rows[1].hash = hash_of(&rows[1]).unwrap();
        assert_eq!(
            verify(&rows),
            Err(Break {
                seq: 3,
                reason: Reason::PrevHash
            })
        );
    }

    #[test]
    fn verify_names_the_missing_row() {
        let (_, mut rows) = chain(4);
        rows.remove(1);
        assert_eq!(
            verify(&rows),
            Err(Break {
                seq: 2,
                reason: Reason::Gap
            })
        );
        let (_, mut rows) = chain(2);
        rows.pop();
        assert_eq!(verify(&rows), Ok(()), "a shorter chain is still a chain");
        let (_, rows) = chain(2);
        assert_eq!(
            verify(&rows[1..]),
            Ok(()),
            "a pruned chain verifies from where it begins"
        );
    }

    #[test]
    fn a_first_row_that_does_not_start_from_zeros_is_a_break() {
        let (_, mut rows) = chain(1);
        rows[0].prev_hash = [7u8; 32];
        rows[0].hash = hash_of(&rows[0]).unwrap();
        assert_eq!(
            verify(&rows),
            Err(Break {
                seq: 1,
                reason: Reason::PrevHash
            })
        );
    }

    #[test]
    fn another_writers_row_in_the_chain_is_a_break() {
        let (_, mut rows) = chain(2);
        rows[1].writer_id = "device-b".into();
        rows[1].hash = hash_of(&rows[1]).unwrap();
        assert_eq!(
            verify(&rows),
            Err(Break {
                seq: 2,
                reason: Reason::Writer
            })
        );
    }

    #[test]
    fn two_rows_at_one_seq_are_a_fork() {
        let (_, mut rows) = chain(2);
        // Each side hashes and chains correctly on its own: only the two of
        // them together say the chain forked.
        let mut other = rows[1].clone();
        other.id = "other-side".into();
        other.hash = hash_of(&other).unwrap();
        rows.push(other);
        assert_eq!(
            verify(&rows),
            Err(Break {
                seq: 2,
                reason: Reason::Fork
            })
        );
        assert_eq!(verify(&rows[..2]), Ok(()));
    }

    #[test]
    fn an_altered_copy_beside_its_original_is_named_altered_in_either_order() {
        let (_, rows) = chain(2);
        let altered = AuditRecord {
            target: Some("https://evil.example".into()),
            ..rows[1].clone()
        };
        let broken = Err(Break {
            seq: 2,
            reason: Reason::Hash,
        });
        let original_first = [rows[0].clone(), rows[1].clone(), altered.clone()];
        let altered_first = [rows[0].clone(), altered, rows[1].clone()];
        assert_eq!(verify(&original_first), broken);
        assert_eq!(verify(&altered_first), broken);
    }

    #[test]
    fn a_detail_over_the_cap_is_refused_before_anything_is_written() {
        let sink = MemorySink::default();
        let writer = Writer::new("device-a", &sink);
        let long = Draft {
            detail: Some(format!("{{\"x\":\"{}\"}}", "y".repeat(DETAIL_MAX_BYTES))),
            ..Draft::new(ActorKind::Agent, Event::Requested)
        };
        assert!(writer.record(long, 1).is_err());
        assert!(sink.rows.borrow().is_empty());
        let fits = Draft {
            detail: Some("y".repeat(DETAIL_MAX_BYTES)),
            ..Draft::new(ActorKind::Agent, Event::Requested)
        };
        assert!(writer.record(fits, 1).is_ok());
    }

    #[test]
    fn the_words_match_their_serde_forms() {
        for event in [
            Event::Requested,
            Event::Granted,
            Event::Denied,
            Event::Used,
            Event::Exposed,
            Event::Ended,
            Event::Revoked,
            Event::Associated,
            Event::RevokedClient,
            Event::RuleAdded,
            Event::RuleRemoved,
            Event::KeyUnwrapped,
            Event::RateLimited,
            Event::PrunedThrough,
        ] {
            assert_eq!(
                serde_json::to_value(event).unwrap(),
                serde_json::json!(event.as_str())
            );
        }
        for kind in [
            ActorKind::Human,
            ActorKind::Extension,
            ActorKind::Agent,
            ActorKind::Sidecar,
            ActorKind::Policy,
            ActorKind::Server,
        ] {
            assert_eq!(
                serde_json::to_value(kind).unwrap(),
                serde_json::json!(kind.as_str())
            );
        }
    }

    // The store as a sink: the chain lands in the table and reads back whole.
    #[test]
    fn the_store_keeps_the_chain() {
        let (_dir, store, _cipher) = crate::testing::vault();
        let writer = Writer::new("device-a", &store);
        let first = writer.record(used("g1"), 1_000).unwrap();
        let second = writer.record(used("g2"), 1_001).unwrap();
        assert_eq!(
            store.audit_last("device-a").unwrap(),
            Some((2, second.hash))
        );
        let rows = store.audit_rows("device-a").unwrap();
        assert_eq!(rows, vec![first, second]);
        assert_eq!(verify(&rows), Ok(()));
    }

    // Retention takes the old rows of every chain; the device's own chain gets
    // a row that says so and verifies from it, and another writer's chain, cut
    // without a note, verifies from where it now begins.
    #[test]
    fn a_pruned_chain_starts_at_its_pruned_through_row_and_verifies() {
        let (_dir, store, _cipher) = crate::testing::vault();
        let writer = Writer::new("device-a", &store);
        for ts in [1_000, 2_000, 3_000] {
            writer.record(used("g"), ts).unwrap();
        }
        let kept = writer.record(used("g4"), 9_000).unwrap();
        let other = Writer::new("device-b", &store);
        other.record(used("g"), 1_000).unwrap();
        let other_kept = other.record(used("g"), 8_000).unwrap();

        let row = prune(&store, "device-a", 5_000, 10_000).unwrap().unwrap();
        assert_eq!(row.seq, 5);
        assert_eq!(row.prev_hash, kept.hash);
        assert_eq!(row.actor_kind, "policy");
        assert_eq!(row.event, "pruned_through");
        assert_eq!(row.detail.as_deref(), Some("{\"seq\":3,\"beforeMs\":5000}"));
        assert_eq!(row.ts_ms, 10_000);

        let rows = store.audit_rows("device-a").unwrap();
        assert_eq!(rows, vec![kept, row]);
        assert_eq!(verify(&rows), Ok(()));
        let others = store.audit_rows("device-b").unwrap();
        assert_eq!(others, vec![other_kept], "cut, with no note of its own");
        assert_eq!(verify(&others), Ok(()));

        // Nothing of this writer's old enough: no row is written and its chain
        // is as it was — even when another chain still had rows to lose.
        assert_eq!(prune(&store, "device-a", 5_000, 10_001).unwrap(), None);
        assert_eq!(store.audit_rows("device-a").unwrap(), rows);
        assert_eq!(prune(&store, "device-c", 8_001, 10_001).unwrap(), None);
        assert!(store.audit_rows("device-b").unwrap().is_empty());
        assert_eq!(store.audit_rows("device-a").unwrap(), rows);
    }

    // A chain retention takes whole does not start over: the note carries on
    // from the row that went, so a peer still holding the old rows reads one
    // chain, and the two copies cannot collide on a `(writer_id, seq)`.
    #[test]
    fn a_chain_pruned_whole_carries_on_from_the_row_that_went() {
        let (_dir, store, _cipher) = crate::testing::vault();
        let writer = Writer::new("device-a", &store);
        let old: Vec<AuditRecord> = (0..3)
            .map(|i| writer.record(used("g"), 1_000 + i).unwrap())
            .collect();

        let note = prune(&store, "device-a", 5_000, 10_000).unwrap().unwrap();
        assert_eq!((note.seq, note.prev_hash), (4, old[2].hash));
        assert_eq!(
            note.detail.as_deref(),
            Some("{\"seq\":3,\"beforeMs\":5000}")
        );
        assert_eq!(store.audit_rows("device-a").unwrap(), vec![note.clone()]);
        assert_eq!(verify(std::slice::from_ref(&note)), Ok(()));

        let whole: Vec<AuditRecord> = old.into_iter().chain([note]).collect();
        assert_eq!(
            verify(&whole),
            Ok(()),
            "as a peer holding the old rows sees it"
        );
        assert_eq!(
            writer.record(used("g"), 10_001).unwrap().seq,
            5,
            "and the chain goes on from the note"
        );
    }
}
