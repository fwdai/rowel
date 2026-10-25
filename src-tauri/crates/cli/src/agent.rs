//! A client at the app's door: who this process says it is, how it is let in
//! under one of the machine's two client keys, and what it may say once it
//! is. The MCP server speaks as an agent — grants, never a value — and
//! `rowel run`, `rowel redact` and the post-tool hook speak as the sidecar,
//! the one client that resolves a placeholder under a grant (`spec/04`).
//! Both speak to the app only through [`Broker`], so their tests can speak to
//! the broker's scripted host instead.

use std::fmt;
use std::io;

use rowel_broker::actions::Hello;
use rowel_broker::client::{ChannelError, Peer, PeerError, Refused, Transport};
use rowel_broker::grant::{ItemSelector, Mode, ResolveContext, Target};
use rowel_broker::protocol::{str_of, Code};
use rowel_broker::reference::Reference;
use serde::Serialize;
use serde_json::{json, Map, Value};
use zeroize::Zeroizing;

use crate::keys::{ClientKey, Keys, Role};
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

/// What the sidecar says it runs on, and the name it introduces itself as:
/// it is the CLI itself, whatever agent launched the shell it runs in.
pub const SIDECAR_PLATFORM: &str = "cli";
pub const SIDECAR_NAME: &str = "Rowel CLI";

/// What a client asks for. The selector, targets and modes are the broker's
/// own types, so the request is the shape the host reads by construction;
/// the run id is the client's and is added when the request is sent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrantAsk {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub items: ItemSelector,
    pub targets: Vec<Target>,
    pub modes: Vec<Mode>,
}

/// The actions of a client, each answering with the opened reply — envelope
/// fields included, as the channel hands it over — except `resolve`, whose
/// reply is the one value and nothing else is kept of it. The host refuses
/// `resolve` to an agent key; the trait carries it for both roles because
/// the sidecar is the same code under another key.
pub trait Broker {
    fn request_grant(&mut self, ask: &GrantAsk) -> Result<Map<String, Value>, AgentError>;
    fn wait_grant(&mut self, grant_id: &str) -> Result<Map<String, Value>, AgentError>;
    fn end_grant(&mut self, grant_id: &str) -> Result<Map<String, Value>, AgentError>;
    fn list_visible(&mut self) -> Result<Map<String, Value>, AgentError>;
    fn resolve(
        &mut self,
        grant_id: &str,
        reference: &Reference,
        ctx: &ResolveContext,
    ) -> Result<Zeroizing<String>, AgentError>;
}

/// How long a `wait-grant` asks the host to hold the request for a pending
/// grant before answering with its state as it stands.
pub const WAIT_MS: u64 = 60_000;

/// How long a request keeps asking while the app's consent slot is busy
/// before the caller is handed the `pending` as it stands.
pub const PENDING_BUDGET_MS: u64 = 60_000;

/// A grant request, asked again while the answer is `pending` with a
/// `retryAfterMs`. In Tier 1 that reply means the app's one consent slot was
/// taken by another dialog and the request was never put to the user — the
/// grant it names was not issued, so nothing waits on it and `wait-grant`
/// cannot settle it; asking again once that dialog has closed is what
/// settles it. The host is asked again as long as the waits fit the budget;
/// a `pending` that will not, or one without a `retryAfterMs` (a routed
/// approval, Tier 2), is the caller's to wait on. The reply comes back as
/// the host sent it; the MCP tools pick their fields from it.
pub fn decided(broker: &mut dyn Broker, ask: &GrantAsk) -> Result<Map<String, Value>, AgentError> {
    let mut spent_ms = 0;
    loop {
        let reply = broker.request_grant(ask)?;
        let retry_after_ms = reply.get("retryAfterMs").and_then(Value::as_u64);
        match (str_of(&reply, "state"), retry_after_ms) {
            ("pending", Some(ms)) if spent_ms + ms <= PENDING_BUDGET_MS => {
                std::thread::sleep(std::time::Duration::from_millis(ms));
                spent_ms += ms;
            }
            _ => return Ok(reply),
        }
    }
}

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

impl AgentError {
    /// The short machine-readable word for the error: the refusal's code
    /// name (or its raw number for one this client does not know), or what
    /// went wrong before the host was asked. What a tool result's `error`
    /// and the proxy's error body carry.
    pub fn kind(&self) -> String {
        match self {
            AgentError::Unreachable(_) => "unreachable".to_string(),
            AgentError::Keys(_) => "key_file".to_string(),
            AgentError::Channel(_) => "protocol".to_string(),
            AgentError::Refused(refused) => refused
                .code
                .map_or_else(|| refused.raw.to_string(), |code| format!("{code:?}")),
        }
    }
}

impl fmt::Display for AgentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AgentError::Unreachable(e) => write!(f, "Rowel is not running or not reachable: {e}"),
            AgentError::Keys(e) => write!(f, "the client key could not be used: {e}"),
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
pub fn test_associate<T: Transport>(peer: &mut Peer<T>, key: &ClientKey) -> Result<(), PeerError> {
    peer.call("test-associate", json!({ "id": key.name, "key": key.key }))
        .map(|_| ())
}

/// `associate` under `key` as a client of `role` from `platform`: the user is
/// asked, and the name they gave comes back as the reply's `id`. The
/// capabilities name what this binary can do with a grant, so a host can
/// tell a sidecar that resolves from an agent that only asks.
pub fn associate<T: Transport>(
    peer: &mut Peer<T>,
    key: &ClientKey,
    role: Role,
    platform: &str,
) -> Result<String, PeerError> {
    let capabilities = match role {
        Role::Agent => vec!["grants".to_string()],
        Role::Sidecar => vec!["grants".to_string(), "resolve".to_string()],
    };
    let hello = Hello {
        kind: role.kind(),
        platform: Some(platform.to_string()),
        version: Some(env!("CARGO_PKG_VERSION").to_string()),
        capabilities,
    };
    let reply = peer.call(
        "associate",
        json!({ "key": peer.public_key(), "idKey": key.key, "hello": hello }),
    )?;
    Ok(str_of(&reply, "id").to_string())
}

/// What connects an [`Agent`] to the app: the socket, or in a test the
/// broker's own `Connection`. `Send`, so the proxy can hold its sidecar
/// behind a mutex and reach it from any of the runtime's threads.
pub type Connector<T> = Box<dyn FnMut() -> io::Result<T> + Send>;

/// A client of either role over one connection, made and let in at the first
/// action — so the MCP handshake works with the app closed — and made again
/// after an I/O error, which is the app having restarted since.
pub struct Agent<T: Transport> {
    connect: Connector<T>,
    keys: Keys,
    platform: String,
    name: String,
    run_id: String,
    peer: Option<Peer<T>>,
}

impl<T: Transport> Agent<T> {
    /// A client from `platform`, of the role `keys` holds the key for, and
    /// introducing itself as `name` until the app's dialog gives it one.
    pub fn new(connect: Connector<T>, keys: Keys, platform: String, name: String) -> Self {
        Self {
            connect,
            keys,
            platform,
            name,
            run_id: fresh_run_id(),
            peer: None,
        }
    }

    /// The sidecar: the CLI's own identity, whatever launched it.
    pub fn sidecar(connect: Connector<T>, keys: Keys) -> Self {
        Self::new(
            connect,
            keys,
            SIDECAR_PLATFORM.to_string(),
            SIDECAR_NAME.to_string(),
        )
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
    /// the user and names the client. A connection whose association failed is
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
        let mut key = self
            .keys
            .load_or_mint(&self.name)
            .map_err(AgentError::Keys)?;
        match test_associate(peer, &key) {
            Ok(()) => return Ok(()),
            Err(PeerError::Refused(refused)) if refused.code == Some(Code::AssociationFailed) => {}
            Err(other) => return Err(other.into()),
        }
        let name = associate(peer, &key, self.keys.role(), &self.platform)?;
        if !name.is_empty() && name != key.name {
            key.name = name;
            self.keys.save(&key).map_err(AgentError::Keys)?;
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

    // The value is moved out of the opened reply, not copied, so the one
    // String that held it is the one wiped on drop; the rest of the reply is
    // the envelope.
    fn resolve(
        &mut self,
        grant_id: &str,
        reference: &Reference,
        ctx: &ResolveContext,
    ) -> Result<Zeroizing<String>, AgentError> {
        let message = json!({
            "grantId": grant_id,
            "ref": reference.to_string(),
            "context": ctx,
        });
        let mut reply = self.call("resolve", message)?;
        match reply.remove("value") {
            Some(Value::String(value)) => Ok(Zeroizing::new(value)),
            _ => Err(AgentError::Channel(ChannelError::BadReply)),
        }
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

    /// A host whose answers to `request-grant` are scripted, in order; the
    /// last one repeats.
    struct Scripted {
        answers: Vec<Map<String, Value>>,
        asked: usize,
    }

    impl Broker for Scripted {
        fn request_grant(&mut self, _ask: &GrantAsk) -> Result<Map<String, Value>, AgentError> {
            let answer = self.answers[self.asked.min(self.answers.len() - 1)].clone();
            self.asked += 1;
            Ok(answer)
        }
        fn wait_grant(&mut self, _grant_id: &str) -> Result<Map<String, Value>, AgentError> {
            Ok(Map::new())
        }
        fn end_grant(&mut self, _grant_id: &str) -> Result<Map<String, Value>, AgentError> {
            Ok(Map::new())
        }
        fn list_visible(&mut self) -> Result<Map<String, Value>, AgentError> {
            Ok(Map::new())
        }
        fn resolve(
            &mut self,
            _grant_id: &str,
            _reference: &Reference,
            _ctx: &ResolveContext,
        ) -> Result<Zeroizing<String>, AgentError> {
            Ok(Zeroizing::new(String::new()))
        }
    }

    fn reply(value: Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    fn login_ask() -> GrantAsk {
        GrantAsk {
            reason: None,
            items: ItemSelector::Lockbox,
            targets: vec![Target::Origin("https://github.com".into())],
            modes: vec![Mode::Fill],
        }
    }

    // A busy consent slot answers `pending` with a short `retryAfterMs` and
    // issues nothing: the request is asked again after the wait, as many
    // times as the budget allows, and the caller gets the decided state.
    #[test]
    fn a_pending_with_a_short_retry_is_asked_again_until_decided() {
        let busy = reply(json!({ "grantId": "g", "state": "pending", "retryAfterMs": 1 }));
        let active = reply(json!({ "grantId": "g", "state": "active", "items": [] }));
        let mut broker = Scripted {
            answers: vec![busy.clone(), busy, active],
            asked: 0,
        };
        let result = decided(&mut broker, &login_ask()).unwrap();
        assert_eq!(result["state"], "active");
        assert_eq!(broker.asked, 3);
    }

    // A `pending` the caller must act on itself comes back as it is: one
    // whose wait would not fit the budget, and one with no `retryAfterMs`
    // at all, which is a routed approval to be waited on.
    #[test]
    fn a_pending_past_the_budget_or_without_a_retry_comes_back_as_it_is() {
        let long = reply(
            json!({ "grantId": "g", "state": "pending", "retryAfterMs": PENDING_BUDGET_MS + 1 }),
        );
        let mut broker = Scripted {
            answers: vec![long],
            asked: 0,
        };
        let result = decided(&mut broker, &login_ask()).unwrap();
        assert_eq!(result["state"], "pending");
        assert_eq!(result["retryAfterMs"], PENDING_BUDGET_MS + 1);
        assert_eq!(broker.asked, 1);

        let routed = reply(json!({ "grantId": "g", "state": "pending" }));
        let mut broker = Scripted {
            answers: vec![routed],
            asked: 0,
        };
        let result = decided(&mut broker, &login_ask()).unwrap();
        assert_eq!(result["state"], "pending");
        assert_eq!(broker.asked, 1);
    }
}
