//! The MCP server over stdio: JSON-RPC 2.0, one message per line, the way
//! Claude Code, Codex and Cursor launch a local tool server. It knows the
//! handshake, `ping`, the tool list and `tools/call`; everything a tool does
//! goes through [`Broker`], so `serve` runs the same against the app's socket
//! and against the broker's scripted host in a test. Nothing is written to
//! stdout but replies: a word of logging there would corrupt the stream.

pub mod rpc;
pub mod tools;

use std::io::{self, BufRead, Write};

use serde_json::{json, Value};

use self::rpc::{Request, METHOD_NOT_FOUND};
use crate::agent::{self, Agent, Broker};
use crate::{keys, transport};

/// The protocol revisions this server speaks; a client's is echoed when it is
/// one of them, else the newest is offered.
pub const PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// Serve stdin to stdout as an agent from `platform` (detected when not
/// given), named `name` until the app's dialog names it, until stdin closes.
pub fn run(platform: Option<String>, name: Option<String>) -> i32 {
    let platform = platform.unwrap_or_else(agent::detect_platform);
    let name = name.unwrap_or_else(|| agent::label(&platform).to_string());
    let Some(key_path) = keys::path() else {
        eprintln!("rowel-cli: no config directory for this user");
        return 1;
    };
    let mut agent = Agent::new(Box::new(transport::connect), key_path, platform, name);
    let (stdin, stdout) = (io::stdin(), io::stdout());
    match serve(stdin.lock(), stdout.lock(), &mut agent) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("rowel-cli: {error}");
            1
        }
    }
}

/// Answer every line of `reader` on `writer` until it ends. Only an I/O
/// failure on either side ends it early; a line that is not a request is
/// answered with the protocol's error and the next one read.
pub fn serve(
    reader: impl BufRead,
    mut writer: impl Write,
    broker: &mut dyn Broker,
) -> io::Result<()> {
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match rpc::parse(&line) {
            Ok(request) => handle(request, broker),
            Err(error) => Some(error),
        };
        if let Some(reply) = reply {
            serde_json::to_writer(&mut writer, &reply)?;
            writer.write_all(b"\n")?;
            writer.flush()?;
        }
    }
    Ok(())
}

/// One request's reply, or none for a notification — which is never acted
/// on either: a tool call without an id has nobody to answer to.
fn handle(request: Request, broker: &mut dyn Broker) -> Option<Value> {
    let Request { id, method, params } = request;
    let id = id?;
    let outcome = match method.as_str() {
        "initialize" => Ok(initialize(&params)),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools::list() })),
        "tools/call" => tools::call(&params, broker),
        _ => Err((METHOD_NOT_FOUND, format!("method not found: {method}"))),
    };
    Some(match outcome {
        Ok(result) => rpc::result(id, result),
        Err((code, message)) => rpc::error(id, code, &message),
    })
}

fn initialize(params: &Value) -> Value {
    let asked = params["protocolVersion"].as_str().unwrap_or_default();
    let version = PROTOCOL_VERSIONS
        .iter()
        .find(|known| **known == asked)
        .unwrap_or(&PROTOCOL_VERSIONS[0]);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": {} },
        "serverInfo": { "name": "rowel", "version": env!("CARGO_PKG_VERSION") },
    })
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::io::Cursor;
    use std::path::PathBuf;
    use std::rc::Rc;

    use rowel_broker::actions::{ClientKind, Connection};
    use rowel_broker::client::Transport;
    use rowel_broker::grant::{ItemSelector, Mode, Target};
    use rowel_broker::testing::{GrantAnswer, Mock, GRANT_ID};

    use super::*;
    use crate::agent::AGENTS_DISABLED;
    use crate::keys::AgentKey;

    /// The broker's own connection, shared with the test so the scripted host
    /// can be read after the server has spoken to it.
    #[derive(Clone)]
    struct Shared(Rc<RefCell<Connection<Mock>>>);

    impl Transport for Shared {
        fn roundtrip(&mut self, request: &[u8]) -> io::Result<Vec<u8>> {
            self.0.borrow_mut().roundtrip(request)
        }
    }

    const KEY: &str = "agent-id-key";
    const NAME: &str = "Claude Code";

    struct Harness {
        _dir: tempfile::TempDir,
        key_path: PathBuf,
        link: Shared,
        agent: Agent<Shared>,
    }

    /// A server over `mock`. `known` writes the key file the way a previous
    /// run would have left it, with the name the host knows the key under.
    fn harness(mock: Mock, known: bool) -> Harness {
        let dir = tempfile::tempdir().unwrap();
        let key_path = dir.path().join("agent.json");
        if known {
            keys::save(
                &key_path,
                &AgentKey {
                    key: KEY.into(),
                    name: NAME.into(),
                },
            )
            .unwrap();
        }
        let link = Shared(Rc::new(RefCell::new(Connection::new(mock))));
        let connector = link.clone();
        let agent = Agent::new(
            Box::new(move || Ok(connector.clone())),
            key_path.clone(),
            "claude-code".into(),
            NAME.into(),
        );
        Harness {
            _dir: dir,
            key_path,
            link,
            agent,
        }
    }

    fn known_agent(answer: GrantAnswer) -> Harness {
        harness(
            Mock {
                grant_answer: answer,
                ..Mock::unlocked().known_agent(NAME, KEY, "claude-code")
            },
            true,
        )
    }

    fn served(harness: &mut Harness, lines: &[Value]) -> Vec<Value> {
        let input: String = lines.iter().map(|line| format!("{line}\n")).collect();
        let mut output = Vec::new();
        serve(Cursor::new(input), &mut output, &mut harness.agent).unwrap();
        String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn request(id: u64, method: &str, params: Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
    }

    fn call(name: &str, arguments: Value) -> Value {
        request(
            1,
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
        )
    }

    /// The tool's text, as JSON, and whether it was an error.
    fn tool_result(reply: &Value) -> (Value, bool) {
        let result = &reply["result"];
        let text = result["content"][0]["text"].as_str().unwrap();
        assert_eq!(result["content"][0]["type"], "text");
        (
            serde_json::from_str(text).unwrap(),
            result["isError"].as_bool().unwrap(),
        )
    }

    #[test]
    fn the_handshake_echoes_a_known_version_and_lists_four_tools() {
        let mut harness = known_agent(GrantAnswer::Denied);
        let replies = served(
            &mut harness,
            &[
                request(
                    1,
                    "initialize",
                    json!({ "protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": { "name": "t", "version": "0" } }),
                ),
                json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
                request(2, "tools/list", json!({})),
                request(3, "initialize", json!({ "protocolVersion": "1999-01-01" })),
                request(4, "ping", Value::Null),
            ],
        );
        assert_eq!(replies.len(), 4, "{replies:?}");
        assert_eq!(replies[0]["id"], 1);
        assert_eq!(replies[0]["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(replies[0]["result"]["capabilities"], json!({ "tools": {} }));
        assert_eq!(replies[0]["result"]["serverInfo"]["name"], "rowel");

        let tools = replies[1]["result"]["tools"].as_array().unwrap();
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(
            names,
            [
                tools::REQUEST_LOGIN,
                tools::WAIT,
                tools::LIST_VISIBLE,
                tools::END
            ]
        );
        for tool in tools {
            assert_eq!(tool["inputSchema"]["type"], "object", "{tool}");
            assert!(tool["inputSchema"]["required"].is_array(), "{tool}");
            assert!(tool["description"].as_str().unwrap().len() > 20);
        }

        assert_eq!(replies[2]["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(replies[3]["result"], json!({}));
    }

    #[test]
    fn a_login_is_requested_end_to_end_and_the_answer_carries_no_secret() {
        let mut harness = known_agent(GrantAnswer::Active("gh".into()));
        let replies = served(
            &mut harness,
            &[call(
                tools::REQUEST_LOGIN,
                json!({ "site": "https://github.com/login?next=/settings", "reason": "open a PR" }),
            )],
        );
        let (result, is_error) = tool_result(&replies[0]);
        assert!(!is_error, "{result}");
        assert_eq!(result["grantId"], GRANT_ID);
        assert_eq!(result["state"], "active");
        assert_eq!(result["items"][0]["title"], "GitHub");
        assert_eq!(result["items"][0]["username"], "octocat");
        assert!(result.get("action").is_none() && result.get("nonce").is_none());

        let text = replies[0]["result"]["content"][0]["text"].as_str().unwrap();
        for secret in ["password", "totp", "hunter2", "123456", "next="] {
            assert!(!text.contains(secret), "{secret} in {text}");
        }

        let connection = harness.link.0.borrow();
        let requests = connection.host().grant_requests.borrow();
        let (client_key, request) = &requests[0];
        assert_eq!(client_key, KEY);
        assert_eq!(request.items, ItemSelector::Lockbox);
        assert_eq!(
            request.targets,
            vec![Target::Origin("https://github.com".into())]
        );
        assert_eq!(request.modes, vec![Mode::Fill]);
        assert_eq!(request.reason.as_deref(), Some("open a PR"));
        assert_eq!(request.run_id.as_deref(), Some(harness.agent.run_id()));
        assert_eq!(connection.host().seen.borrow().as_slice(), [KEY]);
    }

    // A `pending` whose wait is past the budget is the agent's to act on: it
    // comes back as it is, and a wait on it is the refusal the host gives
    // for a grant it never issued — the tool text sends the agent back to
    // requesting. (A wait that fits the budget is retried inside the tool;
    // `tools::tests` covers that.)
    #[test]
    fn a_pending_past_the_budget_comes_back_as_it_is_and_wait_cannot_settle_it() {
        let mut harness = known_agent(GrantAnswer::Pending(Some(tools::PENDING_BUDGET_MS + 1)));
        let replies = served(
            &mut harness,
            &[
                call(tools::REQUEST_LOGIN, json!({ "site": "github.com" })),
                call(tools::WAIT, json!({ "grant_id": GRANT_ID })),
            ],
        );
        let (pending, is_error) = tool_result(&replies[0]);
        assert!(!is_error);
        assert_eq!(pending["state"], "pending");
        assert_eq!(pending["retryAfterMs"], tools::PENDING_BUDGET_MS + 1);
        let (waited, is_error) = tool_result(&replies[1]);
        assert!(is_error);
        assert_eq!(waited["error"], "GrantDenied");
    }

    #[test]
    fn a_first_use_is_let_in_at_the_dialog_and_keeps_the_name_given() {
        let mut harness = harness(
            Mock {
                consent: Some("Claude at work".into()),
                ..Mock::unlocked()
            },
            false,
        );
        let replies = served(&mut harness, &[call(tools::LIST_VISIBLE, json!({}))]);
        let (result, is_error) = tool_result(&replies[0]);
        assert!(!is_error, "{result}");
        assert_eq!(result, json!({ "items": [] }));

        let stored = keys::load(&harness.key_path).unwrap().unwrap();
        assert_eq!(stored.name, "Claude at work");
        let connection = harness.link.0.borrow();
        let clients = connection.host().clients.borrow();
        assert_eq!(clients.len(), 1);
        assert_eq!(clients[0].key, stored.key);
        assert_eq!(clients[0].kind, ClientKind::Agent);
        assert_eq!(clients[0].platform.as_deref(), Some("claude-code"));
    }

    #[test]
    fn agents_off_is_a_tool_error_that_says_where_the_switch_is() {
        let mut harness = harness(Mock::unlocked(), false);
        harness.link.0.borrow().host().agents_enabled.set(false);
        let replies = served(
            &mut harness,
            &[
                call(tools::REQUEST_LOGIN, json!({ "site": "github.com" })),
                call(tools::LIST_VISIBLE, json!({})),
            ],
        );
        for reply in &replies {
            let (result, is_error) = tool_result(reply);
            assert!(is_error);
            assert_eq!(result["error"], "AgentsDisabled");
            assert_eq!(result["message"], AGENTS_DISABLED);
        }
    }

    #[test]
    fn ending_a_grant_is_recorded_by_the_host() {
        let mut harness = known_agent(GrantAnswer::Active("gh".into()));
        let replies = served(
            &mut harness,
            &[
                call(tools::REQUEST_LOGIN, json!({ "site": "github.com" })),
                call(tools::END, json!({ "grant_id": GRANT_ID })),
            ],
        );
        let (result, is_error) = tool_result(&replies[1]);
        assert!(!is_error, "{result}");
        assert_eq!(result, json!({ "ok": true }));
        let connection = harness.link.0.borrow();
        assert_eq!(connection.host().ended.borrow().as_slice(), [GRANT_ID]);
    }

    #[test]
    fn a_tool_that_does_not_exist_or_is_called_wrongly_is_a_tool_error() {
        let mut harness = known_agent(GrantAnswer::Denied);
        let replies = served(
            &mut harness,
            &[
                call("rowel_read_password", json!({})),
                call(tools::REQUEST_LOGIN, json!({})),
                call(tools::WAIT, json!({})),
            ],
        );
        let (unknown, is_error) = tool_result(&replies[0]);
        assert!(is_error);
        assert_eq!(unknown["error"], "unknown_tool");
        let (no_site, is_error) = tool_result(&replies[1]);
        assert!(is_error);
        assert_eq!(no_site["error"], "invalid_arguments");
        let (no_grant, is_error) = tool_result(&replies[2]);
        assert!(is_error);
        assert_eq!(no_grant["error"], "invalid_arguments");
        assert!(harness
            .link
            .0
            .borrow()
            .host()
            .grant_requests
            .borrow()
            .is_empty());
    }

    #[test]
    fn the_protocols_own_errors_and_silences() {
        let mut harness = known_agent(GrantAnswer::Denied);
        let replies = served(
            &mut harness,
            &[
                request(1, "resources/list", json!({})),
                json!({ "jsonrpc": "2.0", "method": "notifications/cancelled", "params": {} }),
                json!({ "jsonrpc": "2.0", "method": "tools/call", "params": { "name": tools::LIST_VISIBLE } }),
                request(2, "tools/call", json!({ "arguments": {} })),
                request(3, "tools/call", json!([])),
                Value::String("not a request".into()),
            ],
        );
        assert_eq!(replies.len(), 4, "{replies:?}");
        assert_eq!(replies[0]["id"], 1);
        assert_eq!(replies[0]["error"]["code"], rpc::METHOD_NOT_FOUND);
        assert_eq!(replies[1]["id"], 2);
        assert_eq!(replies[1]["error"]["code"], rpc::INVALID_PARAMS);
        assert_eq!(replies[2]["id"], 3);
        assert_eq!(replies[2]["error"]["code"], rpc::INVALID_PARAMS);
        assert_eq!(replies[3]["id"], Value::Null);
        assert_eq!(replies[3]["error"]["code"], rpc::INVALID_REQUEST);

        let garbled = serve(
            Cursor::new("{not json\n"),
            &mut Vec::new(),
            &mut harness.agent,
        );
        assert!(garbled.is_ok());
    }

    #[test]
    fn a_parse_error_is_answered_under_a_null_id() {
        let mut harness = known_agent(GrantAnswer::Denied);
        let mut output = Vec::new();
        serve(Cursor::new("{not json\n"), &mut output, &mut harness.agent).unwrap();
        let reply: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(reply["id"], Value::Null);
        assert_eq!(reply["error"]["code"], rpc::PARSE_ERROR);
    }
}
