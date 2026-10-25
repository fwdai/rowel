//! The in-process harness the end-to-end tests share: the broker's own
//! `Connection<Mock>` as the transport, a key file in a temporary directory,
//! and an [`Agent`] of either role over it, so `mcp`, `run` and `hook` are
//! driven exactly as the binary drives them, against the scripted host.

use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use rowel_broker::actions::Connection;
use rowel_broker::client::Transport;
use rowel_broker::testing::Mock;

use crate::agent::Agent;
use crate::keys::{ClientKey, Keys, Role};

/// The broker's own connection, shared with the test so the scripted host
/// can be read after the client has spoken to it. Behind a mutex rather than
/// a `RefCell` so the proxy's tests can drive it from the runtime's threads.
#[derive(Clone)]
pub struct Shared(pub Arc<Mutex<Connection<Mock>>>);

impl Transport for Shared {
    fn roundtrip(&mut self, request: &[u8]) -> io::Result<Vec<u8>> {
        self.0.lock().unwrap().roundtrip(request)
    }
}

/// The key the agent tests are known under, and the name.
pub const AGENT_KEY: &str = "agent-id-key";
pub const AGENT_NAME: &str = "Claude Code";
/// The key the sidecar tests are known under, and the name.
pub const SIDECAR_KEY: &str = "sidecar-id-key";
pub const SIDECAR_NAME: &str = "Rowel CLI";

pub struct Harness {
    pub _dir: tempfile::TempDir,
    pub key_path: PathBuf,
    pub role: Role,
    pub link: Shared,
    pub agent: Agent<Shared>,
}

impl Harness {
    /// The key file the client keeps, as a second handle on it. The file
    /// backend alone: a test never touches a developer's keychain.
    pub fn keys(&self) -> Keys {
        Keys::in_file(self.role, self.key_path.clone())
    }

    /// The host, to read what it saw.
    pub fn host(&self) -> MutexGuard<'_, Connection<Mock>> {
        self.link.0.lock().unwrap()
    }
}

/// A client of `role` over `mock`. `known` writes the key file the way a
/// previous run would have left it, with the name the host knows the key
/// under; pair it with `Mock::known_agent` / `known_sidecar` for that key.
pub fn harness(mock: Mock, role: Role, known: bool) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let key_path = dir.path().join(role.file_name());
    let (key, name, platform) = match role {
        Role::Agent => (AGENT_KEY, AGENT_NAME, "claude-code"),
        Role::Sidecar => (SIDECAR_KEY, SIDECAR_NAME, "cli"),
    };
    if known {
        Keys::in_file(role, key_path.clone())
            .save(&ClientKey {
                key: key.into(),
                name: name.into(),
            })
            .unwrap();
    }
    let link = Shared(Arc::new(Mutex::new(Connection::new(mock))));
    let connector = link.clone();
    let agent = Agent::new(
        Box::new(move || Ok(connector.clone())),
        Keys::in_file(role, key_path.clone()),
        platform.into(),
        name.into(),
    );
    Harness {
        _dir: dir,
        key_path,
        role,
        link,
        agent,
    }
}

/// A sidecar the host already knows, over `mock`.
pub fn known_sidecar(mock: Mock) -> Harness {
    harness(
        mock.known_sidecar(SIDECAR_NAME, SIDECAR_KEY),
        Role::Sidecar,
        true,
    )
}
