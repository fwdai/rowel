//! What the crate's own tests and the app's org tests share: a server's
//! data directory with a minted root key and an open metadata store, and
//! the few rows every test starts from — a member, a device with the secret
//! half of its enrollment key so the test can open what is sealed to it,
//! a lockbox.

use std::path::PathBuf;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use crypto_box::SecretKey;

use crate::lockboxes;
use crate::rootkey::FileRootKey;
use crate::store::{Device, Lockbox, Member, Role, Store};

/// The tenant every test server is.
pub const TENANT: &str = "Acme";

pub struct TempServer {
    pub dir: tempfile::TempDir,
    pub store: Store,
    pub root: FileRootKey,
}

impl TempServer {
    /// A fresh server: the root key beside the data directory, as an
    /// operator keeps it, and the store at `<data dir>/server.db`.
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = FileRootKey::mint(&dir.path().join("root.key")).unwrap();
        let store =
            Store::open(&dir.path().join("data").join(crate::store::DB_FILE), TENANT).unwrap();
        Self { dir, store, root }
    }

    pub fn data_dir(&self) -> PathBuf {
        self.dir.path().join("data")
    }

    pub fn member(&self, email: &str, role: Role) -> Member {
        self.store.add_member(email, None, role).unwrap()
    }

    /// A device of `member`, enrolled and approved by an admin, with the
    /// secret key that opens what the server seals to it.
    pub fn approved_device(&self, member: &Member) -> (Device, SecretKey) {
        let (device, secret) = self.pending_device(member);
        let device = self.store.approve_device(&device.id, "admin").unwrap();
        (device, secret)
    }

    /// A device of `member` as it stands right after enrollment.
    pub fn pending_device(&self, member: &Member) -> (Device, SecretKey) {
        let secret = SecretKey::generate(&mut rand::thread_rng());
        let public_key = STANDARD.encode(secret.public_key().as_bytes());
        let device = self
            .store
            .enroll_device(&member.id, "Pat's MacBook", &public_key)
            .unwrap();
        (device, secret)
    }

    /// A lockbox minted as `rowel admin lockboxes create` mints one.
    pub fn lockbox(&self, name: &str) -> Lockbox {
        lockboxes::create(&self.store, &self.root, &self.data_dir(), name).unwrap()
    }
}

impl Default for TempServer {
    fn default() -> Self {
        Self::new()
    }
}
