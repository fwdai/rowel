//! The metadata database, `server.db` on the data directory: the tenant,
//! its members and their devices, sessions, the lockbox rows with their
//! wrapped masters, agents and the trust policies that name them, the
//! clients they became, policy rules, the approvals waiting on a member, the
//! audit mirror and the replay cache. Plain SQLite — nothing in here is a
//! secret in the clear: masters are wrapped under the root key, tokens are
//! stored as hashes — in WAL mode, the file readable by its owner alone, and
//! migrated the way the core's lockbox store is.
//!
//! The lockboxes themselves are not here. They are SQLCipher files beside
//! this database (`crate::lockboxes`), the same files the app writes.
//!
//! Every method is short and takes the one connection for exactly as long
//! as its statement or transaction runs, so the HTTP layer can call from any
//! task without holding a guard across an await.

use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use rusqlite::{Connection, Row, Statement};

/// A closed vocabulary stored as text: the word in the column, and the
/// variant it reads back as. Unknown text is an error at the row, not a
/// silent default, since every one of these decides what someone may do.
macro_rules! text_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident => $text:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
        #[serde(rename_all = "lowercase")]
        pub enum $name {
            $($variant),+
        }

        impl $name {
            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $text),+
                }
            }
        }

        impl std::str::FromStr for $name {
            type Err = String;

            fn from_str(text: &str) -> std::result::Result<Self, String> {
                match text {
                    $($text => Ok(Self::$variant),)+
                    other => Err(format!("not a {}: {other:?}", stringify!($name))),
                }
            }
        }
    };
}

mod agents;
mod audit;
pub mod events;
mod lockboxes;
mod members;
mod policy;
mod schema;
mod sessions;

#[cfg(test)]
mod tests;

pub use agents::{Agent, TrustPolicy};
pub use audit::AuditFilter;
pub use lockboxes::{Lockbox, LockboxMember, LockboxRole};
pub use members::{Device, DeviceState, Member, Role, Tenant};
pub use policy::{PendingApproval, PendingState, SERVER_ADMIN};
pub use sessions::{Session, AUTH_CODE_TTL_MS};

/// The database's file name on the data directory.
pub const DB_FILE: &str = "server.db";

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("a stored row does not parse: {0}")]
    Json(#[from] serde_json::Error),
    /// The file was written by a build with more migrations than this one
    /// knows; refused rather than run against a schema it cannot read.
    #[error("the server database was written by a newer Rowel; update rowel-cli")]
    SchemaNewer,
    #[error("no {0}")]
    NotFound(String),
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, StoreError>;

pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    /// Open or create the database at `path`, migrate it, and make sure the
    /// tenant row is there, named `tenant_name`. The name follows the config
    /// on every open; the id never changes, since the tenant key every
    /// master is wrapped under is derived from it.
    pub fn open(path: &Path, tenant_name: &str) -> Result<Self> {
        if let Some(parent) = path.parent() {
            rowel_core::store::create_private_dir(parent)?;
        }
        // Before SQLite touches the path, so the file — and the `-wal` and
        // `-shm` it creates beside it — is never on disk under the umask's
        // mode (the core's store does the same for a lockbox).
        restrict(path)?;
        let mut conn = Connection::open(path)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version > schema::schema_version() {
            return Err(StoreError::SchemaNewer);
        }
        schema::migrations()
            .to_latest(&mut conn)
            .map_err(|e| StoreError::Other(format!("migration failed: {e}")))?;
        let store = Self {
            conn: Mutex::new(conn),
        };
        store.ensure_tenant(tenant_name)?;
        Ok(store)
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        // A panic while the lock was held leaves the connection as SQLite
        // left it — every write here is a statement or a committed
        // transaction — so the next caller can go on with it.
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Milliseconds since the epoch, the stamp every row carries.
pub(crate) fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Lowercase hex SHA-256 of `bytes`: how a token is stored, so the database
/// never holds what a bearer would present.
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    hex::encode(sha2::Sha256::digest(bytes))
}

/// Every row `stmt` yields under `params`, through `read`.
fn collect<T>(
    stmt: &mut Statement<'_>,
    params: impl rusqlite::Params,
    read: impl FnMut(&Row<'_>) -> rusqlite::Result<T>,
) -> Result<Vec<T>> {
    Ok(stmt
        .query_map(params, read)?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

/// A text column read as one of a [`text_enum!`], so an unknown word fails
/// the row with the column named rather than being taken for a default.
fn text_column<T: std::str::FromStr<Err = String>>(
    row: &Row<'_>,
    column: &str,
) -> rusqlite::Result<T> {
    let text: String = row.get(column)?;
    text.parse().map_err(|e: String| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, e.into())
    })
}

// Create `path` owner-only if it is not there yet; otherwise bring an
// existing file to the same mode, for a database made by a build before this
// crate did so. The handle is dropped at once: SQLite opens its own.
#[cfg(unix)]
fn restrict(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    match rowel_core::owner_only::create_new(path) {
        Ok(_created) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
            Ok(())
        }
        Err(e) => Err(e.into()),
    }
}

#[cfg(not(unix))]
fn restrict(_path: &Path) -> Result<()> {
    Ok(())
}
