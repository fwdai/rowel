//! Self-contained vault storage engine. Depends only on `std`, `rusqlite`,
//! `serde`, and its own error type — never on the app's commands/state/models
//! or Tauri — so the whole layer can be swapped behind [`VaultStore`].
//!
//! The store persists an **opaque** `payload: Vec<u8>`: it never inspects or
//! encrypts it (the caller applies its own AEAD). The only crypto here is
//! SQLCipher at-rest, keyed by a byte key injected at [`SqliteStore::open`].

mod hash;
mod sqlite;

// Migration glue lives in-module but deliberately references the app's crypto —
// it is the boundary adapter, not part of the pure trait.
pub mod migrate;
// The vault id in `meta`: same boundary status as `migrate` (it mints ids with
// the app's crypto).
pub mod identity;

#[cfg(test)]
mod tests;

pub use hash::{record_hash, state_digest};
pub use sqlite::{create_private_dir, SqliteStore, SYNC_META_PREFIX};

use std::time::{SystemTime, UNIX_EPOCH};

/// Storage error. Small and self-owned so the module stays decoupled; the app
/// boundary maps it into the application error type.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),

    // The DB was stamped by a build with more migrations than this one knows.
    // Kept distinct so the app boundary can say "update the app" instead of
    // the catch-all "wrong password".
    #[error("vault schema is newer than this app version")]
    SchemaNewer,

    // The key does not decrypt this database. Kept distinct from the I/O and
    // corruption failures an open can also hit, so only this one is allowed to
    // reach the user as "wrong password" (and to count against the lockout).
    #[error("cannot open database: wrong key")]
    WrongKey,

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, StoreError>;

/// A full stored row: queryable metadata plus the opaque encrypted payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub tags: String,
    pub url_host: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub deleted_at: Option<i64>,
    pub payload: Vec<u8>,
    /// Card network slug derived from the number at save time ("visa", …);
    /// None for non-cards and for rows saved before the column existed.
    pub card_brand: Option<String>,
    /// Starred by the user. A column rather than a payload field so listings can
    /// filter and pin on it without decrypting anything; [`VaultStore::set_favorite`]
    /// is its only writer outside a sync merge.
    pub favorite: bool,
    /// Whether the entry carries at least one passkey, derived from the payload
    /// at save time (see `migrate::derived_has_passkey`) so a listing can mark
    /// the row without unsealing it. Unlike `favorite` this is not user state:
    /// it is a projection of the payload, so every write recomputes it.
    pub has_passkey: bool,
    /// What an `env` entry's file was called (`.env.production`) and how many
    /// variables it holds, both derived from the payload at save time so the
    /// list can subtitle the row without unsealing it. `None` on every other
    /// kind, on env rows saved before the columns existed, and — for the name
    /// alone — on a pasted file that never had one.
    pub file_name: Option<String>,
    pub var_count: Option<i64>,
    /// A login's username, derived from the payload at save time. `Some` for
    /// every stamped login (`""` when it has none), `None` otherwise.
    pub username: Option<String>,
}

/// A row's metadata without its payload (what listings need).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryMeta {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub tags: String,
    pub url_host: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub deleted_at: Option<i64>,
    pub card_brand: Option<String>,
    pub favorite: bool,
    pub has_passkey: bool,
    pub file_name: Option<String>,
    pub var_count: Option<i64>,
    pub username: Option<String>,
}

/// The swappable storage contract. Any backend behind this interface is a drop-in.
pub trait VaultStore: Send {
    /// Read a `meta` value (schema_version, kdf, salt, …). Values are caller-owned.
    fn meta_get(&self, key: &str) -> Result<Option<String>>;
    /// Write a `meta` value.
    fn meta_set(&self, key: &str, value: &str) -> Result<()>;
    /// Write several `meta` values as one step: either every pair lands or none
    /// of them does. Values that only mean anything together — a value and the
    /// stamp it was set at — must not be split across two writes, or a failure
    /// between them leaves the pair disagreeing (see [`identity::set_vault_name`]).
    fn meta_set_many(&self, pairs: &[(&str, &str)]) -> Result<()>;
    /// List live entries' metadata (excludes tombstones).
    fn list(&self) -> Result<Vec<EntryMeta>>;
    /// List tombstoned entries' metadata — what the Trash shows. Excludes rows
    /// already purged (see [`VaultStore::purge`]), newest deletion first.
    fn list_deleted(&self) -> Result<Vec<EntryMeta>>;
    /// Fetch one live record with its payload (`None` if missing or tombstoned).
    fn get(&self, id: &str) -> Result<Option<Record>>;
    /// Insert or update a record, stamping `updated_at` to now.
    fn upsert(&self, rec: &Record) -> Result<()>;
    /// Tombstone a record (`deleted_at = now`); the row is kept for sync.
    fn delete(&self, id: &str) -> Result<()>;
    /// Un-tombstone a record (`deleted_at = NULL`), stamping `updated_at` so the
    /// restore wins the sync merge against the tombstone peers still hold.
    fn restore(&self, id: &str) -> Result<()>;
    /// Discard a tombstoned record's contents for good.
    fn purge(&self, id: &str) -> Result<()>;
    /// Star or unstar a **live** record, stamping `updated_at`; a tombstone has
    /// no star to set, so it is left alone.
    fn set_favorite(&self, id: &str, favorite: bool) -> Result<()>;
    /// Every record including tombstones, with timestamps intact (for sync).
    fn export_for_sync(&self) -> Result<Vec<Record>>;
    /// Bulk-write records in one transaction, preserving their timestamps (sync in).
    fn import(&self, recs: &[Record]) -> Result<()>;
}

/// Milliseconds since the Unix epoch.
fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
