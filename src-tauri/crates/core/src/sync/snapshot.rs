//! The snapshot inside a pulled pack, opened in a throwaway database and read
//! out as what syncs: the receiving half of the exchange, which the app's
//! engine and a server merge in their own ways.

use std::fs;
use std::path::Path;

use super::Scratch;
use crate::store::{
    identity, Attachment, AuditRecord, Record, SqliteStore, StoreError, VaultStore,
};

/// What a snapshot holds that syncs: the entries, attachments and audit rows
/// in it, and the name it was pushed under.
pub struct Contents {
    pub records: Vec<Record>,
    pub attachments: Vec<Attachment>,
    /// Every writer's chain the pushing device held, in `(writer_id, seq)`
    /// order. Merged as a union, never last-writer-wins: a log row is a fact
    /// about what happened, and no device's copy outranks another's.
    pub audit: Vec<AuditRecord>,
    /// The name in the snapshot's `meta`, with the ms-epoch stamp it was set
    /// at. `(None, 0)` for one pushed before names travelled.
    pub name: Option<String>,
    pub name_ms: i64,
}

/// Why a snapshot could not be read. The first two are kept apart because the
/// caller tells the user something different for each.
#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    /// Stamped by a build newer than this one: "update the app", never "your
    /// remote vault is damaged" and never "wrong password" — for a password
    /// manager, a fake "wrong password" is the worst possible misdiagnosis.
    #[error("vault file was written by a newer version of the app")]
    SchemaNewer,
    /// Not a database `key` opens: truncated, replaced by an unrelated file,
    /// or sealed under another key. The open cannot tell these apart, and the
    /// pack's header has already caught what can be caught before this point.
    #[error("vault file is invalid")]
    Invalid,
    /// The scratch file could not be written.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// The database opened, but one of its tables could not be read out.
    #[error("{0}")]
    Store(#[from] StoreError),
}

/// Open `snapshot` under `key` in a throwaway database and export what syncs.
///
/// The snapshot must land on disk before SQLCipher can read it, so it goes to
/// a uniquely named scratch file under `scratch_dir` that a guard removes on
/// every exit path. No other database is opened, written, or replaced here: a
/// hostile or broken snapshot can only ever fail this function.
pub fn read(snapshot: &[u8], key: &[u8], scratch_dir: &Path) -> Result<Contents, SnapshotError> {
    let scratch = Scratch::new(scratch_dir, "remote")?;
    fs::write(scratch.path(), snapshot)?;

    let store = SqliteStore::open(scratch.path(), key).map_err(|e| match e {
        StoreError::SchemaNewer => SnapshotError::SchemaNewer,
        _ => SnapshotError::Invalid,
    })?;
    let (name, name_ms) = identity::vault_name(&store)?;
    Ok(Contents {
        records: store.export_for_sync()?,
        attachments: store.export_attachments_for_sync()?,
        audit: store.audit_all()?,
        name,
        name_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    const KEY: &[u8] = &[0x33; 32];

    fn tmp_dir() -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "rowel-snapshot-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn record(id: &str, payload: &[u8]) -> Record {
        Record {
            id: id.into(),
            kind: "login".into(),
            title: "Example".into(),
            tags: "[]".into(),
            url_host: "example.com".into(),
            created_at: 100,
            updated_at: 200,
            deleted_at: None,
            payload: payload.to_vec(),
            card_brand: None,
            favorite: false,
            has_passkey: false,
            file_name: None,
            var_count: None,
            username: None,
            doc_type: None,
            doc_country: None,
        }
    }

    // A named store with two entries, snapshotted the way a push does it.
    fn seeded_snapshot(dir: &Path) -> (SqliteStore, Vec<u8>) {
        let store = SqliteStore::open(&dir.join("vault.db"), KEY).unwrap();
        store
            .import(&[record("1", b"one"), record("2", b"two")])
            .unwrap();
        identity::set_vault_name(&store, "Home", 1234).unwrap();
        let snapshot = dir.join("snapshot.db");
        store.snapshot_to(&snapshot, KEY).unwrap();
        (store, fs::read(&snapshot).unwrap())
    }

    #[test]
    fn a_snapshot_reads_back_what_the_store_holds() {
        let dir = tmp_dir();
        let (store, snapshot) = seeded_snapshot(&dir);
        let scratch = dir.join("scratch");

        let contents = read(&snapshot, KEY, &scratch).unwrap();

        assert_eq!(contents.records, store.export_for_sync().unwrap());
        assert_eq!(contents.records[0].payload, b"one");
        assert!(contents.attachments.is_empty());
        assert!(contents.audit.is_empty());
        assert_eq!(contents.name.as_deref(), Some("Home"));
        assert_eq!(contents.name_ms, 1234);
        // The scratch database is cleaned up, siblings included.
        assert!(fs::read_dir(&scratch).unwrap().next().is_none());
    }

    #[test]
    fn a_wrong_key_is_invalid_not_a_panic() {
        let dir = tmp_dir();
        let (_store, snapshot) = seeded_snapshot(&dir);
        let scratch = dir.join("scratch");

        let result = read(&snapshot, &[0x44; 32], &scratch);

        assert!(matches!(result, Err(SnapshotError::Invalid)));
        assert!(fs::read_dir(&scratch).unwrap().next().is_none());
    }

    #[test]
    fn a_newer_schema_is_its_own_error() {
        let dir = tmp_dir();
        let store = SqliteStore::open(&dir.join("vault.db"), KEY).unwrap();
        store.set_user_version(99).unwrap();
        let snapshot = dir.join("snapshot.db");
        store.snapshot_to(&snapshot, KEY).unwrap();

        let result = read(&fs::read(&snapshot).unwrap(), KEY, &dir.join("scratch"));

        assert!(matches!(result, Err(SnapshotError::SchemaNewer)));
    }
}
