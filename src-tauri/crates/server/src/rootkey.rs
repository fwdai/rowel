//! The root of the server's key hierarchy (spec `07`): a root key the
//! operator holds, a tenant key derived from it, and each lockbox master
//! wrapped under the tenant key with the lockbox's id in the tag. The
//! database holds only the wrapped form, so a copy of it opens nothing
//! without the root key, which is why the key file belongs outside the data
//! directory and the backups.
//!
//! `RootKey` is a trait so that a KMS (AWS, GCP, Vault transit) can stand
//! where the file does; this slice ships the file.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use zeroize::Zeroizing;

use rowel_core::crypto::{hkdf_subkey, seal_aead, unseal_aead, KEY_LEN};

/// Wrap a lockbox master under the tenant's key and open it again.
pub trait RootKey: Send + Sync {
    fn wrap(
        &self,
        tenant_id: &str,
        lockbox_id: &str,
        master: &[u8],
    ) -> Result<Vec<u8>, RootKeyError>;
    fn unwrap(
        &self,
        tenant_id: &str,
        lockbox_id: &str,
        wrapped: &[u8],
    ) -> Result<Zeroizing<Vec<u8>>, RootKeyError>;
}

#[derive(Debug)]
pub enum RootKeyError {
    /// The key file could not be read or written.
    Io(PathBuf, io::Error),
    /// The key file is readable by others. Refused, as a key anyone on the
    /// box can read protects nothing; the operator fixes the mode.
    NotOwnerOnly(PathBuf),
    /// The key file does not hold exactly 32 bytes: not one `mint` wrote.
    WrongLength(PathBuf, usize),
    /// `mint` was asked to write where a key already is. Never overwritten:
    /// every master in the database is wrapped under the key that is there.
    Exists(PathBuf),
    /// The wrapped master does not open under this key for this lockbox:
    /// another tenant's or lockbox's row, a tampered one, or another root.
    DoesNotOpen,
    Crypto(String),
}

impl fmt::Display for RootKeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(path, e) => write!(f, "root key file {}: {e}", path.display()),
            Self::NotOwnerOnly(path) => write!(
                f,
                "root key file {} is readable by others; make it mode 0600",
                path.display()
            ),
            Self::WrongLength(path, len) => write!(
                f,
                "root key file {} holds {len} bytes, not {KEY_LEN}",
                path.display()
            ),
            Self::Exists(path) => write!(
                f,
                "a root key is already at {}; it is never overwritten",
                path.display()
            ),
            Self::DoesNotOpen => f.write_str("the wrapped master does not open under the root key"),
            Self::Crypto(reason) => f.write_str(reason),
        }
    }
}

impl std::error::Error for RootKeyError {}

/// A root key held as 32 raw bytes in a file of the operator's.
pub struct FileRootKey(Zeroizing<[u8; KEY_LEN]>);

impl FileRootKey {
    /// The key in the file at `path`: exactly 32 bytes, readable by its
    /// owner alone.
    pub fn load(path: &Path) -> Result<Self, RootKeyError> {
        let io = |e| RootKeyError::Io(path.to_path_buf(), e);
        require_owner_only(path)?;
        let bytes = Zeroizing::new(std::fs::read(path).map_err(io)?);
        let key: [u8; KEY_LEN] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| RootKeyError::WrongLength(path.to_path_buf(), bytes.len()))?;
        Ok(Self(Zeroizing::new(key)))
    }

    /// Mint a fresh key into `path`, owner-only from the first instant and
    /// written whole or not at all; refused where a file already is.
    pub fn mint(path: &Path) -> Result<Self, RootKeyError> {
        if path.exists() {
            return Err(RootKeyError::Exists(path.to_path_buf()));
        }
        let mut key = Zeroizing::new([0u8; KEY_LEN]);
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut *key);
        rowel_core::atomic::atomic_write_private(path, &*key)
            .map_err(|e| RootKeyError::Io(path.to_path_buf(), io::Error::other(e.to_string())))?;
        Ok(Self(key))
    }

    /// The tenant key: a subkey of the root per tenant, so that two tenants
    /// on one root (the cloud's shape) never share wrapping material.
    fn tenant_key(&self, tenant_id: &str) -> Zeroizing<[u8; KEY_LEN]> {
        let info = [b"rowel-server tenant ".as_slice(), tenant_id.as_bytes()].concat();
        Zeroizing::new(hkdf_subkey(&*self.0, &info))
    }
}

impl RootKey for FileRootKey {
    /// AES-256-GCM under the tenant key with the lockbox id as the
    /// associated data, so a wrapped master moved to another lockbox's row
    /// fails its tag there.
    fn wrap(
        &self,
        tenant_id: &str,
        lockbox_id: &str,
        master: &[u8],
    ) -> Result<Vec<u8>, RootKeyError> {
        seal_aead(&*self.tenant_key(tenant_id), lockbox_id.as_bytes(), master)
            .map_err(|e| RootKeyError::Crypto(e.to_string()))
    }

    fn unwrap(
        &self,
        tenant_id: &str,
        lockbox_id: &str,
        wrapped: &[u8],
    ) -> Result<Zeroizing<Vec<u8>>, RootKeyError> {
        // One verdict for every failure inside: which of the key, the id and
        // the bytes was wrong is not something a caller should learn here.
        unseal_aead(&*self.tenant_key(tenant_id), lockbox_id.as_bytes(), wrapped)
            .map(Zeroizing::new)
            .map_err(|_| RootKeyError::DoesNotOpen)
    }
}

/// Refuse a key file anyone but its owner can read. Unix has the mode to
/// ask; elsewhere the file is taken as the operator left it.
#[cfg(unix)]
fn require_owner_only(path: &Path) -> Result<(), RootKeyError> {
    use std::os::unix::fs::PermissionsExt;
    let metadata = std::fs::metadata(path).map_err(|e| RootKeyError::Io(path.to_path_buf(), e))?;
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err(RootKeyError::NotOwnerOnly(path.to_path_buf()));
    }
    Ok(())
}

#[cfg(not(unix))]
fn require_owner_only(_path: &Path) -> Result<(), RootKeyError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MASTER: &[u8] = &[7u8; 32];

    #[test]
    fn a_minted_key_loads_and_wraps_a_master_that_unwraps() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("root.key");
        let minted = FileRootKey::mint(&path).unwrap();
        let loaded = FileRootKey::load(&path).unwrap();
        assert_eq!(*minted.0, *loaded.0);

        let wrapped = minted.wrap("t1", "lb1", MASTER).unwrap();
        assert_ne!(wrapped, MASTER);
        assert_eq!(&*loaded.unwrap("t1", "lb1", &wrapped).unwrap(), MASTER);
    }

    #[test]
    fn a_wrapped_master_opens_for_its_lockbox_and_tenant_alone() {
        let dir = tempfile::tempdir().unwrap();
        let root = FileRootKey::mint(&dir.path().join("root.key")).unwrap();
        let wrapped = root.wrap("t1", "lb1", MASTER).unwrap();

        assert!(matches!(
            root.unwrap("t1", "lb2", &wrapped),
            Err(RootKeyError::DoesNotOpen)
        ));
        assert!(matches!(
            root.unwrap("t2", "lb1", &wrapped),
            Err(RootKeyError::DoesNotOpen)
        ));
        let mut tampered = wrapped.clone();
        tampered[20] ^= 1;
        assert!(matches!(
            root.unwrap("t1", "lb1", &tampered),
            Err(RootKeyError::DoesNotOpen)
        ));
        let other = FileRootKey::mint(&dir.path().join("other.key")).unwrap();
        assert!(matches!(
            other.unwrap("t1", "lb1", &wrapped),
            Err(RootKeyError::DoesNotOpen)
        ));
    }

    #[test]
    fn a_key_is_never_minted_over_another() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("root.key");
        FileRootKey::mint(&path).unwrap();
        let before = std::fs::read(&path).unwrap();
        assert!(matches!(
            FileRootKey::mint(&path),
            Err(RootKeyError::Exists(_))
        ));
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[test]
    fn a_file_of_the_wrong_length_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("root.key");
        let mut file = rowel_core::owner_only::create_new(&path).unwrap();
        std::io::Write::write_all(&mut file, &[1u8; 16]).unwrap();
        drop(file);
        assert!(matches!(
            FileRootKey::load(&path),
            Err(RootKeyError::WrongLength(_, 16))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn a_key_file_others_can_read_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("root.key");
        FileRootKey::mint(&path).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
        assert!(matches!(
            FileRootKey::load(&path),
            Err(RootKeyError::NotOwnerOnly(_))
        ));
    }
}
