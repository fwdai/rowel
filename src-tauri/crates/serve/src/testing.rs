//! What the crate's own tests and the CLI's integration test share: a data
//! directory with a vault made the way the app makes one (KDF sidecar, the
//! store, a vault id), items sealed into it, the registry flag, and a host
//! over it with its announcements collected rather than printed.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rowel_broker::actions::{Client, ClientKind};
use rowel_broker::lockbox::write_clients;
use rowel_broker::testing::seed;
use rowel_core::crypto::{self, KdfParams, VaultKey};
use rowel_core::layout::{DB_FILE, KDF_SIDECAR_FILE};
use rowel_core::models::Entry;
use rowel_core::store::{identity, migrate, SqliteStore, VaultStore};
use rowel_core::workspace::{self, Registry};

use crate::host::{HeadlessHost, Overrides};
use crate::unlock::{self, Opened, Unlock};

/// The master password every test vault is made with.
pub const PASSWORD: &str = "correct horse battery staple";

/// A data directory for the tests, with the primary lockbox's vault in it —
/// an Argon2id sidecar at the app's defaults (the test pays the derivation
/// once per vault), the store, a vault id — and nothing else yet.
pub struct DataDir {
    pub dir: tempfile::TempDir,
    pub root: PathBuf,
}

impl DataDir {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let opened = create_vault(&root, workspace::PRIMARY_ID, PASSWORD);
        drop(opened);
        Self { dir, root }
    }

    /// The vault, open with the password.
    pub fn open(&self) -> Opened {
        unlock::open(
            &self.root,
            workspace::PRIMARY_ID,
            &Unlock::Password(PASSWORD.to_string().into()),
        )
        .unwrap()
    }

    /// The registry's `agents` flag for the primary lockbox.
    pub fn set_agents(&self, on: bool) {
        set_agents(&self.root, workspace::PRIMARY_ID, on);
    }

    /// A host over the primary lockbox, unlocked with the password, its
    /// announcements collected into `notices`.
    pub fn host(&self, notices: &Notices) -> HeadlessHost {
        self.host_with(notices, Overrides::default(), None)
    }

    /// [`DataDir::host`] with the command line's overrides and, when given,
    /// the key file to reopen from after a seal.
    pub fn host_with(
        &self,
        notices: &Notices,
        overrides: Overrides,
        key_file: Option<PathBuf>,
    ) -> HeadlessHost {
        let host = HeadlessHost::new(
            &self.root,
            workspace::PRIMARY_ID,
            &crate::config::path_in(&self.root),
            overrides,
            key_file,
            Some(notices.notice()),
        )
        .unwrap();
        host.unlock(self.open());
        host
    }

    /// Every audit row of the vault, event and detail, through a connection
    /// of the test's own.
    pub fn rows(&self) -> Vec<(String, Option<String>)> {
        self.open()
            .store
            .audit_all()
            .unwrap()
            .into_iter()
            .map(|row| (row.event, row.detail))
            .collect()
    }
}

impl Default for DataDir {
    fn default() -> Self {
        Self::new()
    }
}

/// The lines a host would print to stderr, collected.
#[derive(Clone, Default)]
pub struct Notices(pub Arc<Mutex<Vec<String>>>);

impl Notices {
    pub fn notice(&self) -> crate::approvals::Notice {
        let lines = self.0.clone();
        Arc::new(move |line: &str| lines.lock().unwrap().push(line.to_string()))
    }

    pub fn lines(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

/// Make the vault of `lockbox_id` under `root` as `session::create_vault`
/// makes one: the sidecar first, then the database keyed with the derived
/// key, the descriptor mirrored into `meta`, and a vault id.
pub fn create_vault(root: &Path, lockbox_id: &str, password: &str) -> Opened {
    let dir = workspace::dir_of(root, lockbox_id);
    std::fs::create_dir_all(&dir).unwrap();
    let params = KdfParams::default_argon2id();
    rowel_core::atomic::atomic_write_file(&dir.join(KDF_SIDECAR_FILE), &params.to_json().unwrap())
        .unwrap();
    let key = VaultKey::Argon2 {
        master: crypto::derive(password.as_bytes(), &params).unwrap(),
    };
    let store = SqliteStore::open(&dir.join(DB_FILE), &*key.sqlcipher_key()).unwrap();
    store.meta_set("kdf", params.algo()).unwrap();
    store
        .meta_set("kdf_params", &params.to_json().unwrap())
        .unwrap();
    identity::assign_vault_id(&store).unwrap();
    Opened {
        cipher: key.payload_cipher(),
        key,
        store,
        kdf: Some(params),
    }
}

/// Flip the registry's `agents` flag for `lockbox_id`, writing the file as
/// `rowel admin agents` writes it.
pub fn set_agents(root: &Path, lockbox_id: &str, on: bool) {
    let mut registry = Registry::load(root);
    if let Some(workspace) = registry.workspaces.iter_mut().find(|w| w.id == lockbox_id) {
        workspace.agents = on;
    }
    registry.save(root).unwrap();
}

/// An API key item sealed into `opened`'s store.
pub fn seed_api_key(opened: &Opened, id: &str, title: &str, key: &str) {
    seed(
        &opened.store,
        &opened.cipher,
        &Entry {
            id: id.into(),
            kind: "apikey".into(),
            title: title.into(),
            api_key: Some(key.into()),
            ..Default::default()
        },
    );
}

/// A login sealed into `opened`'s store.
pub fn seed_login(
    opened: &Opened,
    id: &str,
    title: &str,
    website: &str,
    username: &str,
    password: &str,
) {
    seed(
        &opened.store,
        &opened.cipher,
        &Entry {
            id: id.into(),
            kind: "login".into(),
            title: title.into(),
            website: Some(website.into()),
            username: Some(username.into()),
            password: Some(password.into()),
            ..Default::default()
        },
    );
}

/// A card sealed into `opened`'s store: the kind the step-up floor is for.
pub fn seed_card(opened: &Opened, id: &str, title: &str, number: &str) {
    seed(
        &opened.store,
        &opened.cipher,
        &Entry {
            id: id.into(),
            kind: "card".into(),
            title: title.into(),
            number: Some(number.into()),
            cvc: Some("123".into()),
            ..Default::default()
        },
    );
}

/// A fresh entry id, as the store mints them.
pub fn new_id() -> String {
    migrate::new_entry_id()
}

/// Let `clients` into `opened`'s store, as `associate` writes them.
pub fn let_in(opened: &Opened, clients: &[Client]) {
    write_clients(&opened.store, clients).unwrap();
}

pub fn client(name: &str, key: &str, kind: ClientKind, platform: &str) -> Client {
    Client {
        name: name.into(),
        key: key.into(),
        kind,
        platform: Some(platform.into()),
        created_at_ms: 0,
        last_seen_at_ms: None,
        agent_id: None,
        driven_by_agent: false,
    }
}
