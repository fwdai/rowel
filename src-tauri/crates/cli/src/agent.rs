//! An agent at the app's door: who this process says it is, how it is let in
//! under the machine's agent key, and the five actions a client of kind
//! `agent` may say once it is. The MCP server speaks to the app only through
//! [`Broker`], so its tests can speak to the broker's scripted host instead.

use std::fmt;
use std::io;
use std::path::PathBuf;

use rowel_broker::actions::{ClientKind, Hello};
use rowel_broker::client::{ChannelError, Peer, PeerError, Refused, Transport};
use rowel_broker::grant::{ItemSelector, Mode, Target};
use rowel_broker::protocol::{str_of, Code};
use serde::Serialize;
use serde_json::{json, Map, Value};

use crate::keys::{self, AgentKey};
use crate::CLIENT_ID;

/// What this process is, for the app's dialog and its record of the client.
/// `$ROWEL_PLATFORM` names it outright; otherwise the variables the coding
/// agents set in the processes they launch say which one is running us.
pub fn detect_platform() -> String {
    if let Some(platform) = std::env::var("ROWEL_PLATFORM")
        .ok()
        .filter(|p| !p.trim().is_empty())
    {
        return platform;
    }
    let any_set = |names: &[&str]| names.iter().any(|name| std::env::var_os(name).is_some());
    if any_set(&["CLAUDECODE", "CLAUDE_CODE"]) {
        "claude-code"
    } else if any_set(&["CURSOR_TRACE_ID", "CURSOR"]) {
        "cursor"
    } else if any_set(&["CODEX_SANDBOX", "CODEX"]) {
        "codex"
    } else {
        "cli"
    }
    .to_string()
}

/// The platform's name as a person reads it, which is what a fresh agent
/// introduces itself as until the app's dialog gives it a name.
pub fn label(platform: &str) -> &'static str {
    match platform {
        "claude-code" => "Claude Code",
        "codex" => "Codex",
        "cursor" => "Cursor",
        _ => "Agent",
    }
}

/// What an agent asks for. The selector, targets and modes are the broker's
/// own types, so the request is the shape the host reads by construction;
/// the run id is the agent's and is added when the request is sent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrantAsk {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub items: ItemSelector,
    pub targets: Vec<Target>,
    pub modes: Vec<Mode>,
}

/// The five actions of an agent, each answering with the opened reply —
/// envelope fields included, as the channel hands it over.
pub trait Broker {
    fn request_grant(&mut self, ask: &GrantAsk) -> Result<Map<String, Value>, AgentError>;
    fn wait_grant(&mut self, grant_id: &str) -> Result<Map<String, Value>, AgentError>;
    fn end_grant(&mut self, grant_id: &str) -> Result<Map<String, Value>, AgentError>;
    fn list_visible(&mut self) -> Result<Map<String, Value>, AgentError>;
}

/// How long a `wait-grant` asks the host to hold the request for a pending
/// grant before answering with its state as it stands.
pub const WAIT_MS: u64 = 60_000;

/// The message that turns `AgentsDisabled` into the one thing to do about it.
pub const AGENTS_DISABLED: &str = "Turn on Agents in Rowel › Settings › Browser & agents";

/// Why an action got no answer: the app was not there, the key file could
/// not be read or written, the channel made no sense of a reply, or the host
/// said no.
#[derive(Debug)]
pub enum AgentError {
    Unreachable(io::Error),
    Keys(io::Error),
    Channel(ChannelError),
    Refused(Refused),
}

impl From<PeerError> for AgentError {
    fn from(error: PeerError) -> Self {
        match error {
            PeerError::Io(e) => AgentError::Unreachable(e),
            PeerError::Channel(e) => AgentError::Channel(e),
            PeerError::Refused(r) => AgentError::Refused(r),
        }
    }
}

impl fmt::Display for AgentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AgentError::Unreachable(e) => write!(f, "Rowel is not running or not reachable: {e}"),
            AgentError::Keys(e) => write!(f, "the agent key file could not be used: {e}"),
            AgentError::Channel(e) => write!(f, "the app answered out of protocol: {e}"),
            AgentError::Refused(r) if r.code == Some(Code::AgentsDisabled) => {
                f.write_str(AGENTS_DISABLED)
            }
            AgentError::Refused(r) => f.write_str(&r.message),
        }
    }
}

impl std::error::Error for AgentError {}

/// `test-associate` under `key`: the host knows this key under this name, or
/// refuses with `AssociationFailed`.
pub fn test_associate<T: Transport>(peer: &mut Peer<T>, key: &AgentKey) -> Result<(), PeerError> {
    peer.call("test-associate", json!({ "id": key.name, "key": key.key }))
        .map(|_| ())
}

/// `associate` under `key` as an agent from `platform`: the user is asked, and
/// the name they gave comes back as the reply's `id`.
pub fn associate<T: Transport>(
    peer: &mut Peer<T>,
    key: &AgentKey,
    platform: &str,
) -> Result<String, PeerError> {
    let hello = Hello {
        kind: ClientKind::Agent,
        platform: Some(platform.to_string()),
        version: Some(env!("CARGO_PKG_VERSION").to_string()),
        capabilities: vec!["grants".to_string()],
    };
    let reply = peer.call(
        "associate",
        json!({ "key": peer.public_key(), "idKey": key.key, "hello": hello }),
    )?;
    Ok(str_of(&reply, "id").to_string())
}

/// What connects an [`Agent`] to the app: the socket, or in a test the
/// broker's own `Connection`.
pub type Connector<T> = Box<dyn FnMut() -> io::Result<T>>;

/// An agent over one connection, made and let in at the first action — so the
/// MCP handshake works with the app closed — and made again after an I/O error,
/// which is the app having restarted since.
pub struct Agent<T: Transport> {
    connect: Connector<T>,
    key_path: PathBuf,
    platform: String,
    name: String,
    run_id: String,
    peer: Option<Peer<T>>,
}

impl<T: Transport> Agent<T> {
    /// An agent from `platform`, keeping its key at `key_path` and introducing
    /// itself as `name` until the app's dialog gives it one.
    pub fn new(connect: Connector<T>, key_path: PathBuf, platform: String, name: String) -> Self {
        Self {
            connect,
            key_path,
            platform,
            name,
            run_id: fresh_run_id(),
            peer: None,
        }
    }

    /// The run id every grant request of this process carries.
    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    /// One action on an associated connection. A connection the app has
    /// dropped is made again once; any other failure is the answer.
    fn call(&mut self, action: &str, message: Value) -> Result<Map<String, Value>, AgentError> {
        match self.associated()?.call(action, message.clone()) {
            Err(PeerError::Io(_)) => {
                self.peer = None;
                Ok(self.associated()?.call(action, message)?)
            }
            outcome => Ok(outcome?),
        }
    }

    /// The connection, made and let in if it is not yet. `test-associate` first
    /// — a key the app knows is let straight in — then `associate`, which asks
    /// the user and names the agent. A connection whose association failed is
    /// dropped, so the next action starts over rather than on a half-open door.
    fn associated(&mut self) -> Result<&mut Peer<T>, AgentError> {
        if self.peer.is_none() {
            let transport = (self.connect)().map_err(AgentError::Unreachable)?;
            let mut peer = Peer::connect(transport, CLIENT_ID)?;
            self.let_in(&mut peer)?;
            self.peer = Some(peer);
        }
        Ok(self.peer.as_mut().expect("connected above"))
    }

    fn let_in(&mut self, peer: &mut Peer<T>) -> Result<(), AgentError> {
        let mut key = keys::load_or_mint(&self.key_path, &self.name).map_err(AgentError::Keys)?;
        match test_associate(peer, &key) {
            Ok(()) => return Ok(()),
            Err(PeerError::Refused(refused)) if refused.code == Some(Code::AssociationFailed) => {}
            Err(other) => return Err(other.into()),
        }
        let name = associate(peer, &key, &self.platform)?;
        if !name.is_empty() && name != key.name {
            key.name = name;
            keys::save(&self.key_path, &key).map_err(AgentError::Keys)?;
        }
        Ok(())
    }
}

impl<T: Transport> Broker for Agent<T> {
    fn request_grant(&mut self, ask: &GrantAsk) -> Result<Map<String, Value>, AgentError> {
        let mut message =
            serde_json::to_value(ask).map_err(|_| AgentError::Channel(ChannelError::Sealing))?;
        message["runId"] = self.run_id.clone().into();
        self.call("request-grant", message)
    }

    fn wait_grant(&mut self, grant_id: &str) -> Result<Map<String, Value>, AgentError> {
        self.call(
            "wait-grant",
            json!({ "grantId": grant_id, "timeoutMs": WAIT_MS }),
        )
    }

    fn end_grant(&mut self, grant_id: &str) -> Result<Map<String, Value>, AgentError> {
        self.call("end-grant", json!({ "grantId": grant_id }))
    }

    fn list_visible(&mut self) -> Result<Map<String, Value>, AgentError> {
        self.call("list-visible", json!({}))
    }
}

/// Sixteen random bytes as hex: opaque, well under the host's 64 characters,
/// and new for every process so the audit log tells one run from the next.
fn fresh_run_id() -> String {
    rand::random::<[u8; 16]>()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_id_is_hex_and_fresh_per_process() {
        let (a, b) = (fresh_run_id(), fresh_run_id());
        assert_eq!(a.len(), 32);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn a_platform_has_a_label_a_person_reads() {
        assert_eq!(label("claude-code"), "Claude Code");
        assert_eq!(label("codex"), "Codex");
        assert_eq!(label("cursor"), "Cursor");
        assert_eq!(label("something-else"), "Agent");
    }

    #[test]
    fn a_grant_ask_is_the_shape_the_host_reads() {
        let ask = GrantAsk {
            reason: None,
            items: ItemSelector::Lockbox,
            targets: vec![Target::Origin("https://github.com".into())],
            modes: vec![Mode::Fill],
        };
        assert_eq!(
            serde_json::to_value(&ask).unwrap(),
            json!({ "items": "lockbox", "targets": [{ "origin": "https://github.com" }], "modes": ["fill"] })
        );
        let ask = GrantAsk {
            reason: Some("deploy".into()),
            items: ItemSelector::Ids(vec!["item-1".into()]),
            targets: vec![Target::Http {
                host: "api.stripe.com".into(),
                path: "/v1/*".into(),
                methods: vec![],
            }],
            modes: vec![Mode::Header],
        };
        assert_eq!(
            serde_json::to_value(&ask).unwrap(),
            json!({
                "reason": "deploy",
                "items": { "ids": ["item-1"] },
                "targets": [{ "http": { "host": "api.stripe.com", "path": "/v1/*", "methods": [] } }],
                "modes": ["header"],
            })
        );
    }
}
