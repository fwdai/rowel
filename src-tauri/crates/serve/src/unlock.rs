//! Opening the lockbox without the app: the master password typed at a
//! terminal, a key file holding the vault's key material, or that material
//! read once from a file descriptor. The derivation is the app's
//! (`session::derive_key`): Argon2id through the KDF sidecar beside the
//! database, the legacy deterministic key when there is no sidecar.
//!
//! A lockbox opened here is served from its columns as they are: the app's
//! best-effort backfill of derived metadata on unlock is app code, and a
//! column it has not stamped yet (a username, a card brand) only changes
//! what a list shows, never what a grant may cover.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use zeroize::Zeroizing;

use rowel_core::crypto::{self, KdfParams, PayloadCipher, VaultKey};
use rowel_core::layout::{DB_FILE, DB_REKEY_BACKUP_FILE, KDF_SIDECAR_FILE};
use rowel_core::store::{SqliteStore, StoreError};
use rowel_core::workspace;

/// How the lockbox is unlocked.
pub enum Unlock {
    /// The master password, as typed.
    Password(Zeroizing<String>),
    /// A file holding the vault's key material, as [`write_key_file`] writes
    /// it. Read again at every open, so an idle lock in this mode is lifted
    /// by the next request.
    KeyFile(PathBuf),
    /// The key material itself, read once by the caller — from a file
    /// descriptor a service manager hands the process.
    Material(Zeroizing<Vec<u8>>),
}

/// The lockbox, open: the key, the store and the payload cipher derived from
/// the key, and the KDF descriptor it was derived through (`None` for a
/// legacy vault), which a step-up ask derives a candidate through again.
pub struct Opened {
    pub key: VaultKey,
    pub store: SqliteStore,
    pub cipher: PayloadCipher,
    pub kdf: Option<KdfParams>,
}

/// Why the lockbox did not open.
#[derive(Debug)]
pub enum UnlockError {
    /// The key does not open the database: a wrong password, or a key file
    /// of another vault's.
    WrongKey,
    /// There is no database in the lockbox's directory.
    NoVault(PathBuf),
    /// A password change was interrupted: the pre-change snapshot is still
    /// beside the database, and the database and its payloads may be on
    /// different keys until the app's next unlock rolls it back.
    RekeyInterrupted(PathBuf),
    /// The key file could not be read.
    KeyFile(PathBuf, io::Error),
    Other(String),
}

impl fmt::Display for UnlockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongKey => f.write_str("wrong password, or a key file of another lockbox"),
            Self::NoVault(dir) => write!(f, "no vault in {}", dir.display()),
            Self::RekeyInterrupted(dir) => write!(
                f,
                "a password change of this lockbox was interrupted ({} in {}); unlock it in the Rowel app once, which rolls it back, then copy the data directory again",
                DB_REKEY_BACKUP_FILE,
                dir.display()
            ),
            Self::KeyFile(path, e) => write!(f, "cannot read the key file {}: {e}", path.display()),
            Self::Other(reason) => f.write_str(reason),
        }
    }
}

impl std::error::Error for UnlockError {}

impl From<rowel_core::error::Error> for UnlockError {
    fn from(e: rowel_core::error::Error) -> Self {
        Self::Other(e.to_string())
    }
}

/// The KDF descriptor of the lockbox at `dir`, or `None` for a legacy vault
/// without one.
pub fn kdf_params(dir: &Path) -> Result<Option<KdfParams>, UnlockError> {
    let path = dir.join(KDF_SIDECAR_FILE);
    if !path.exists() {
        return Ok(None);
    }
    let json = std::fs::read_to_string(&path)
        .map_err(|e| UnlockError::Other(format!("cannot read {}: {e}", path.display())))?;
    Ok(Some(KdfParams::from_json(&json)?))
}

/// The key `password` derives to under `kdf`: Argon2id through the sidecar's
/// parameters, or the legacy scheme when the vault has none. The derivation
/// the app's unlock runs, so the same password opens the same vault here.
pub fn derive(password: &str, kdf: Option<&KdfParams>) -> Result<VaultKey, UnlockError> {
    match kdf {
        Some(params) => Ok(VaultKey::Argon2 {
            master: crypto::derive(password.as_bytes(), params)?,
        }),
        None => Ok(VaultKey::legacy_from_password(password)),
    }
}

/// Open the lockbox `lockbox_id` under `root` with `unlock`.
pub fn open(root: &Path, lockbox_id: &str, unlock: &Unlock) -> Result<Opened, UnlockError> {
    let dir = workspace::dir_of(root, lockbox_id);
    let db = dir.join(DB_FILE);
    if !db.exists() {
        return Err(UnlockError::NoVault(dir));
    }
    if dir.join(DB_REKEY_BACKUP_FILE).exists() {
        return Err(UnlockError::RekeyInterrupted(dir));
    }
    let kdf = kdf_params(&dir)?;
    let key = match unlock {
        Unlock::Password(password) => derive(password, kdf.as_ref())?,
        Unlock::KeyFile(path) => VaultKey::from_material(read_key_file(path)?, kdf.is_some()),
        Unlock::Material(material) => VaultKey::from_material(material.clone(), kdf.is_some()),
    };
    // Only the store's own key check is a wrong key; an I/O failure keeps its
    // own name, as the app's `open_with_key` keeps them apart.
    let store = SqliteStore::open(&db, &*key.sqlcipher_key()).map_err(|e| match e {
        StoreError::WrongKey => UnlockError::WrongKey,
        StoreError::SchemaNewer => {
            UnlockError::Other("this lockbox was written by a newer Rowel; update rowel-cli".into())
        }
        e => UnlockError::Other(format!("could not open the vault: {e}")),
    })?;
    Ok(Opened {
        cipher: key.payload_cipher(),
        key,
        store,
        kdf,
    })
}

/// The key material in the file at `path`, wiped on drop.
pub fn read_key_file(path: &Path) -> Result<Zeroizing<Vec<u8>>, UnlockError> {
    std::fs::read(path)
        .map(Zeroizing::new)
        .map_err(|e| UnlockError::KeyFile(path.to_path_buf(), e))
}

/// Write `key`'s material to `path` as raw bytes, readable by its owner
/// alone from the first instant: what `rowel admin key-file` writes and
/// [`Unlock::KeyFile`] reads. The material is the vault's key — whoever
/// reads the file opens the lockbox — so the file belongs outside the data
/// directory, with the permissions a service manager gives a credential.
pub fn write_key_file(path: &Path, key: &VaultKey) -> io::Result<()> {
    use std::io::Write;
    let mut file = rowel_core::owner_only::create_new(path)?;
    file.write_all(key.biometric_material())?;
    file.sync_all()
}
