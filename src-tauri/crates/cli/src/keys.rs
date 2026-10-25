//! The client keys: this machine's identification as an agent and as a
//! sidecar, which are the whole credential in Tier 1 (`spec/02`). Only the
//! public half is ever used — `associate` and `test-associate` carry it, the
//! app stores it against a name — so only the public half is kept. The name
//! beside it is the one the app's dialog gave the client, which
//! `test-associate` has to repeat.
//!
//! There are two keys because there are two kinds of client in one binary
//! (`spec/04`): `rowel mcp` is an agent and can never resolve a value; `rowel
//! run` and `rowel proxy` are sidecars and can. Each keeps its own record,
//! so the model-facing surface holds nothing that would let it resolve.
//!
//! Where a record lives (Q9): in the OS secure store where the platform has
//! one — a generic password in the login keychain on macOS, a Credential
//! Manager entry on Windows — else a file of the user's own in the config
//! directory, readable by them alone. The store is not gated by biometrics;
//! what it adds over a file is the store's own access control, under which
//! another program reading the item prompts the user. A record written as a
//! file by an earlier release is moved into the store the first time it is
//! read, and a store that refuses (anything but "not there") is stood in for
//! by the file for that run, so a broken keychain never strands an
//! association. Everything goes through one [`Keys`] per role, so the tests
//! can keep to the file backend and a developer's keychain is never touched.

use std::cell::Cell;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use rowel_broker::actions::ClientKind;
use rowel_broker::client::Channel;
use serde::{Deserialize, Serialize};

/// Which of the two clients a key identifies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Role {
    /// `rowel mcp`: asks for grants, never sees a value.
    Agent,
    /// `rowel run`, `rowel proxy`, `rowel redact`, `rowel hook post`:
    /// resolves placeholders under a grant.
    Sidecar,
}

impl Role {
    /// The kind the client introduces itself as, which is what the host
    /// stores and gates the vocabulary on.
    pub fn kind(self) -> ClientKind {
        match self {
            Role::Agent => ClientKind::Agent,
            Role::Sidecar => ClientKind::Sidecar,
        }
    }

    pub fn file_name(self) -> &'static str {
        match self {
            Role::Agent => "agent.json",
            Role::Sidecar => "sidecar.json",
        }
    }

    /// The word `status` and error messages call the key by, and the account
    /// the secure store's item is filed under.
    pub fn label(self) -> &'static str {
        match self {
            Role::Agent => "agent",
            Role::Sidecar => "sidecar",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientKey {
    /// The X25519 public key, base64, as the protocol carries it.
    pub key: String,
    /// The name the app knows this key under, once it does.
    pub name: String,
}

/// `<config dir>/rowel/<agent|sidecar>.json`: under `~/Library/Application
/// Support/rowel/` on macOS, `$XDG_CONFIG_HOME/rowel/` on Linux,
/// `%APPDATA%\rowel\` on Windows. The file backend's place, and where a
/// record from before the secure store is looked for.
pub fn path_for(role: Role) -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("rowel").join(role.file_name()))
}

/// The OS secure store as the keys use it: one record per role, read and
/// written whole. `Ok(None)` is a store with no record; an `Err` is a store
/// that could not answer, which the file then stands in for.
pub trait SecureStore {
    fn read(&self, role: Role) -> io::Result<Option<Vec<u8>>>;
    fn write(&self, role: Role, raw: &[u8]) -> io::Result<()>;
    /// Where the record is, in words, for `status`.
    fn name(&self) -> &'static str;
}

/// The place one role's key record is kept: the secure store when the
/// platform has one, with the file as the record's old home and the
/// fallback; the file alone otherwise.
pub struct Keys {
    role: Role,
    file: PathBuf,
    store: Option<Box<dyn SecureStore + Send>>,
    /// Set the first time the store refuses: from then on, for this process,
    /// the file is read and written instead, and the refusal was said once.
    store_failed: Cell<bool>,
}

impl Keys {
    /// The record of `role` at `file`, moved into and kept in `store` when
    /// one is given.
    pub fn new(role: Role, file: PathBuf, store: Option<Box<dyn SecureStore + Send>>) -> Self {
        Self {
            role,
            file,
            store,
            store_failed: Cell::new(false),
        }
    }

    /// The record of `role` in the file at `file` and nowhere else: Linux,
    /// and every test.
    pub fn in_file(role: Role, file: PathBuf) -> Self {
        Self::new(role, file, None)
    }

    /// Where this platform keeps the record of `role`, or `None` when there
    /// is no config directory for this user.
    pub fn for_platform(role: Role) -> Option<Self> {
        let file = path_for(role)?;
        Some(Self::new(role, file, platform_store()))
    }

    pub fn role(&self) -> Role {
        self.role
    }

    /// Where the record is kept, in words: the store's name, or the file's
    /// path.
    pub fn location(&self) -> String {
        match self.store() {
            Some(store) => format!("in {}", store.name()),
            None => self.file.display().to_string(),
        }
    }

    /// The record, or `None` when there is none yet. A record that is there
    /// but does not read is an error, not a missing one: minting over it
    /// would silently orphan an association the user already approved. A
    /// file left by an earlier release is moved into the store here.
    pub fn load(&self) -> io::Result<Option<ClientKey>> {
        let Some(store) = self.store() else {
            return load_file(&self.file);
        };
        match store.read(self.role) {
            Ok(Some(raw)) => parse(
                &raw,
                &format!("the {} key in {}", self.role.label(), store.name()),
            )
            .map(Some),
            Ok(None) => {
                let Some(key) = load_file(&self.file)? else {
                    return Ok(None);
                };
                match store.write(self.role, &serde_json::to_vec_pretty(&key)?) {
                    Ok(()) => {
                        if let Err(error) = fs::remove_file(&self.file) {
                            eprintln!(
                                "rowel-cli: the {} key was moved into {} but {} could not be removed: {error}",
                                self.role.label(),
                                store.name(),
                                self.file.display()
                            );
                        }
                    }
                    Err(error) => self.fall_back(error),
                }
                Ok(Some(key))
            }
            Err(error) => {
                self.fall_back(error);
                load_file(&self.file)
            }
        }
    }

    /// The record, minted under `name` when there is none yet.
    pub fn load_or_mint(&self, name: &str) -> io::Result<ClientKey> {
        if let Some(key) = self.load()? {
            return Ok(key);
        }
        let key = ClientKey {
            key: Channel::new().public_key(),
            name: name.to_string(),
        };
        self.save(&key)?;
        Ok(key)
    }

    /// Write the record where it lives. The file is readable by its owner
    /// alone: the key is only a public one, but it is the credential, and
    /// whoever holds it is this client.
    pub fn save(&self, key: &ClientKey) -> io::Result<()> {
        let raw = serde_json::to_vec_pretty(key)?;
        if let Some(store) = self.store() {
            match store.write(self.role, &raw) {
                Ok(()) => return Ok(()),
                Err(error) => self.fall_back(error),
            }
        }
        save_file(&self.file, &raw)
    }

    /// The store, unless it has refused once this run.
    fn store(&self) -> Option<&(dyn SecureStore + Send)> {
        if self.store_failed.get() {
            return None;
        }
        self.store.as_deref()
    }

    /// The store refused: say so once, and use the file from here on.
    fn fall_back(&self, error: io::Error) {
        if self.store_failed.replace(true) {
            return;
        }
        let name = self.store.as_ref().map_or("the secure store", |s| s.name());
        eprintln!(
            "rowel-cli: {name} could not be used for the {} key ({error}); using {} for now",
            self.role.label(),
            self.file.display()
        );
    }
}

fn parse(raw: &[u8], what: &str) -> io::Result<ClientKey> {
    serde_json::from_slice(raw).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{what} is not a client key record: {e}"),
        )
    })
}

/// The key file at `path`, or `None` when there is none.
fn load_file(path: &Path) -> io::Result<Option<ClientKey>> {
    let raw = match fs::read(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    parse(&raw, &path.display().to_string()).map(Some)
}

fn save_file(path: &Path, raw: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        private_dir(dir)?;
    }
    private_file(path)?.write_all(raw)
}

#[cfg(unix)]
fn private_dir(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true).mode(0o700);
    builder.create(dir)
}

#[cfg(unix)]
fn private_file(path: &Path) -> io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
}

// Windows has no mode bits; a file under `%APPDATA%` is the user's by the
// profile's own ACL, which is what the owner-only mode means on Unix.
#[cfg(not(unix))]
fn private_dir(dir: &Path) -> io::Result<()> {
    fs::create_dir_all(dir)
}

#[cfg(not(unix))]
fn private_file(path: &Path) -> io::Result<fs::File> {
    fs::File::create(path)
}

/// The service both platforms' items are filed under; the account is the
/// role's label.
#[cfg(any(target_os = "macos", target_os = "windows"))]
const SERVICE: &str = "app.rowel.cli";

/// This platform's secure store, when it has one.
#[cfg(target_os = "macos")]
fn platform_store() -> Option<Box<dyn SecureStore + Send>> {
    Some(Box::new(keychain::Keychain))
}

#[cfg(target_os = "windows")]
fn platform_store() -> Option<Box<dyn SecureStore + Send>> {
    Some(Box::new(credential_manager::CredentialManager))
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn platform_store() -> Option<Box<dyn SecureStore + Send>> {
    None
}

/// A generic password in the login keychain: no access control of ours, no
/// entitlement, no biometric gate. Reading it from another program is what
/// the keychain itself prompts for.
#[cfg(target_os = "macos")]
mod keychain {
    use std::io;

    use security_framework::passwords::{generic_password, set_generic_password, PasswordOptions};

    use super::{Role, SecureStore, SERVICE};

    // errSecItemNotFound — no such keychain item.
    const ERR_ITEM_NOT_FOUND: i32 = -25300;

    pub struct Keychain;

    impl SecureStore for Keychain {
        fn read(&self, role: Role) -> io::Result<Option<Vec<u8>>> {
            match generic_password(PasswordOptions::new_generic_password(SERVICE, role.label())) {
                Ok(raw) => Ok(Some(raw)),
                Err(e) if e.code() == ERR_ITEM_NOT_FOUND => Ok(None),
                Err(e) => Err(io::Error::other(e.to_string())),
            }
        }

        fn write(&self, role: Role, raw: &[u8]) -> io::Result<()> {
            set_generic_password(SERVICE, role.label(), raw)
                .map_err(|e| io::Error::other(e.to_string()))
        }

        fn name(&self) -> &'static str {
            "the login keychain"
        }
    }
}

/// A Credential Manager entry, through `keyring`'s native backend, as the
/// app keeps its own.
#[cfg(target_os = "windows")]
mod credential_manager {
    use std::io;

    use keyring::{Entry, Error};

    use super::{Role, SecureStore, SERVICE};

    pub struct CredentialManager;

    fn entry(role: Role) -> io::Result<Entry> {
        Entry::new(SERVICE, role.label()).map_err(|e| io::Error::other(e.to_string()))
    }

    impl SecureStore for CredentialManager {
        fn read(&self, role: Role) -> io::Result<Option<Vec<u8>>> {
            match entry(role)?.get_secret() {
                Ok(raw) => Ok(Some(raw)),
                Err(Error::NoEntry) => Ok(None),
                Err(e) => Err(io::Error::other(e.to_string())),
            }
        }

        fn write(&self, role: Role, raw: &[u8]) -> io::Result<()> {
            entry(role)?
                .set_secret(raw)
                .map_err(|e| io::Error::other(e.to_string()))
        }

        fn name(&self) -> &'static str {
            "Credential Manager"
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use super::*;

    fn file_keys(dir: &Path) -> Keys {
        Keys::in_file(Role::Agent, dir.join("rowel").join("agent.json"))
    }

    #[test]
    fn a_key_is_minted_once_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let keys = file_keys(dir.path());
        assert_eq!(keys.load().unwrap(), None);

        let minted = keys.load_or_mint("Claude Code").unwrap();
        assert_eq!(minted.name, "Claude Code");
        assert!(!minted.key.is_empty());
        assert_eq!(keys.load().unwrap(), Some(minted.clone()));
        assert_eq!(keys.load_or_mint("Other").unwrap(), minted);

        let renamed = ClientKey {
            name: "Claude".into(),
            ..minted
        };
        keys.save(&renamed).unwrap();
        assert_eq!(keys.load().unwrap(), Some(renamed));
        assert_eq!(keys.location(), keys.file.display().to_string());
    }

    #[test]
    fn the_two_roles_have_their_own_files_and_kinds() {
        assert_eq!(Role::Agent.file_name(), "agent.json");
        assert_eq!(Role::Sidecar.file_name(), "sidecar.json");
        assert_eq!(Role::Agent.kind(), ClientKind::Agent);
        assert_eq!(Role::Sidecar.kind(), ClientKind::Sidecar);
        if let (Some(agent), Some(sidecar)) = (path_for(Role::Agent), path_for(Role::Sidecar)) {
            assert_ne!(agent, sidecar);
            assert_eq!(agent.parent(), sidecar.parent());
        }
    }

    #[cfg(unix)]
    #[test]
    fn the_file_and_its_directory_are_the_owners_alone() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let keys = Keys::in_file(Role::Sidecar, dir.path().join("rowel").join("sidecar.json"));
        keys.load_or_mint("Rowel CLI").unwrap();
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&keys.file), 0o600);
        assert_eq!(mode(keys.file.parent().unwrap()), 0o700);
    }

    #[test]
    fn a_file_that_is_not_a_key_is_an_error_not_a_missing_key() {
        let dir = tempfile::tempdir().unwrap();
        let keys = Keys::in_file(Role::Agent, dir.path().join("agent.json"));
        fs::write(&keys.file, b"{}").unwrap();
        let error = keys.load().unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(keys.load_or_mint("Agent").is_err());
    }

    /// A secure store in memory, shared with the test, that can be told to
    /// refuse.
    #[derive(Clone, Default)]
    struct Fake {
        items: Arc<Mutex<HashMap<Role, Vec<u8>>>>,
        broken: Arc<Mutex<bool>>,
    }

    impl Fake {
        fn item(&self, role: Role) -> Option<ClientKey> {
            self.items
                .lock()
                .unwrap()
                .get(&role)
                .map(|raw| serde_json::from_slice(raw).unwrap())
        }
    }

    impl SecureStore for Fake {
        fn read(&self, role: Role) -> io::Result<Option<Vec<u8>>> {
            if *self.broken.lock().unwrap() {
                return Err(io::Error::other("the keychain is locked"));
            }
            Ok(self.items.lock().unwrap().get(&role).cloned())
        }
        fn write(&self, role: Role, raw: &[u8]) -> io::Result<()> {
            if *self.broken.lock().unwrap() {
                return Err(io::Error::other("the keychain is locked"));
            }
            self.items.lock().unwrap().insert(role, raw.to_vec());
            Ok(())
        }
        fn name(&self) -> &'static str {
            "the fake store"
        }
    }

    fn with_store(dir: &Path, store: &Fake) -> Keys {
        Keys::new(
            Role::Sidecar,
            dir.join("sidecar.json"),
            Some(Box::new(store.clone())),
        )
    }

    fn key(name: &str) -> ClientKey {
        ClientKey {
            key: format!("{name}-key"),
            name: name.into(),
        }
    }

    #[test]
    fn a_file_is_moved_into_an_empty_store_and_the_store_then_wins() {
        let dir = tempfile::tempdir().unwrap();
        let store = Fake::default();
        let keys = with_store(dir.path(), &store);
        assert_eq!(keys.location(), "in the fake store");
        assert_eq!(keys.load().unwrap(), None, "nothing anywhere yet");

        save_file(&keys.file, &serde_json::to_vec(&key("from-file")).unwrap()).unwrap();
        assert_eq!(keys.load().unwrap(), Some(key("from-file")));
        assert_eq!(
            store.item(Role::Sidecar),
            Some(key("from-file")),
            "moved in"
        );
        assert!(!keys.file.exists(), "and the file is gone");

        save_file(&keys.file, &serde_json::to_vec(&key("stale-file")).unwrap()).unwrap();
        assert_eq!(
            keys.load().unwrap(),
            Some(key("from-file")),
            "the store wins over a file"
        );
        assert!(keys.file.exists(), "which is left alone");

        keys.save(&key("renamed")).unwrap();
        assert_eq!(store.item(Role::Sidecar), Some(key("renamed")));
        assert_eq!(
            load_file(&keys.file).unwrap(),
            Some(key("stale-file")),
            "a save goes to the store"
        );
    }

    #[test]
    fn a_store_that_refuses_is_stood_in_for_by_the_file_for_the_run() {
        let dir = tempfile::tempdir().unwrap();
        let store = Fake::default();
        *store.broken.lock().unwrap() = true;
        let keys = with_store(dir.path(), &store);
        save_file(&keys.file, &serde_json::to_vec(&key("from-file")).unwrap()).unwrap();

        assert_eq!(keys.load().unwrap(), Some(key("from-file")));
        assert!(keys.file.exists(), "the file is kept");
        assert_eq!(keys.location(), keys.file.display().to_string());
        keys.save(&key("renamed")).unwrap();
        assert_eq!(load_file(&keys.file).unwrap(), Some(key("renamed")));
        assert_eq!(store.item(Role::Sidecar), None);

        *store.broken.lock().unwrap() = false;
        assert_eq!(
            keys.load().unwrap(),
            Some(key("renamed")),
            "still the file this run"
        );
        assert_eq!(store.item(Role::Sidecar), None);

        let next_run = with_store(dir.path(), &store);
        assert_eq!(next_run.load().unwrap(), Some(key("renamed")));
        assert_eq!(
            store.item(Role::Sidecar),
            Some(key("renamed")),
            "moved on the next run"
        );
        assert!(!next_run.file.exists());
    }

    #[test]
    fn a_fresh_key_is_minted_straight_into_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let store = Fake::default();
        let keys = with_store(dir.path(), &store);
        let minted = keys.load_or_mint("Rowel CLI").unwrap();
        assert_eq!(store.item(Role::Sidecar), Some(minted));
        assert!(!keys.file.exists());
    }
}
