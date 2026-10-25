//! The I5 integration test: the built binary as `rowel serve` over a vault
//! in a temporary data directory, unlocked from a key file, with `rowel
//! mcp` and `rowel proxy` reaching it over `ROWEL_BROKER=ws://…` and `rowel
//! approve` answering the one ask a standing rule does not cover. The
//! clients' keys live in a config directory under the temporary one, so no
//! key of the developer's is touched — which is why the test runs only
//! where the keys are files: on macOS and Windows the CLI keeps them in the
//! OS secure store whatever the config directory says, and there is no seam
//! to make it use the file instead (`keys::platform_store`).

#![cfg(not(any(target_os = "android", target_os = "ios")))]

use std::io::{BufRead, BufReader, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use serde_json::{json, Value};
use tokio::net::TcpListener;

use rowel_broker::client::Channel;
use rowel_broker::grant::{Approval, CreatedBy, ItemSelector, Mode, Target};
use rowel_broker::policy::{AgentSelector, Rule};
use rowel_cli::keys::{ClientKey, Keys, Role};
use rowel_serve::config::{self, Config};
use rowel_serve::testing::{self, DataDir};
use rowel_serve::unlock;

const BIN: &str = env!("CARGO_BIN_EXE_rowel-cli");
const STRIPE: &str = "stripe-item";
const KEY_VALUE: &str = "sk_live_integration_123456";

/// A child of the binary, killed when the test is done with it.
struct Process(Child);

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// The directories and the environment every CLI process runs under.
struct Sandbox {
    data: DataDir,
    config_home: PathBuf,
    broker: Mutex<Option<String>>,
}

impl Sandbox {
    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(BIN);
        command
            .args(args)
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", self.data.root.as_os_str())
            .env("XDG_CONFIG_HOME", self.config_home.as_os_str())
            .env("ROWEL_PLATFORM", "claude-code")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(broker) = self.broker.lock().unwrap().as_ref() {
            command.env("ROWEL_BROKER", broker);
        }
        command
    }

    /// Run one short command to its end: its exit code, stdout and stderr.
    fn run(&self, args: &[&str]) -> (i32, String, String) {
        let output = self.command(args).output().unwrap();
        (
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    }
}

/// The key the agent and the sidecar introduce themselves with, minted into
/// the config directory as a first run would, and let into the vault as an
/// answered association would — so the processes find themselves known.
fn minted(config_home: &Path, role: Role, name: &str) -> String {
    let key = Channel::new().public_key();
    Keys::in_file(role, config_home.join("rowel").join(role.file_name()))
        .save(&ClientKey {
            key: key.clone(),
            name: name.into(),
        })
        .unwrap();
    key
}

fn rule(id: &str, agent: &str, mode: Mode, target: Target) -> Rule {
    Rule {
        id: id.into(),
        agents: AgentSelector::Ids(vec![agent.into()]),
        items: ItemSelector::Ids(vec![STRIPE.into()]),
        targets: vec![target],
        modes: vec![mode],
        approval: Approval::Auto,
        ttl_ms: 15 * 60 * 1000,
        max_uses: None,
        reason_required: false,
        approvers: None,
        sidecar_for: None,
        created_at_ms: 0,
        created_by: CreatedBy::Admin("config".into()),
        expires_at_ms: None,
    }
}

/// A line of `reader` containing `needle`, within `timeout`; the lines
/// before it are discarded.
fn wait_for_line(lines: &mpsc::Receiver<String>, needle: &str, timeout: Duration) -> String {
    let deadline = Instant::now() + timeout;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match lines.recv_timeout(left) {
            Ok(line) if line.contains(needle) => return line,
            Ok(_) => continue,
            Err(_) => panic!("no line with {needle:?} within {timeout:?}"),
        }
    }
}

/// Every line of `reader`, as it comes, on a channel.
fn lines_of(reader: impl std::io::Read + Send + 'static) -> mpsc::Receiver<String> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(reader).lines().map_while(Result::ok) {
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    receiver
}

/// `rowel mcp` over stdio: one JSON-RPC line out, one in.
struct Mcp {
    _process: Process,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl Mcp {
    fn start(sandbox: &Sandbox) -> Self {
        let mut child = sandbox.command(&["mcp"]).spawn().unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Self {
            _process: Process(child),
            stdin,
            stdout,
            next_id: 1,
        }
    }

    fn call(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let request = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        writeln!(self.stdin, "{request}").unwrap();
        self.stdin.flush().unwrap();
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        let reply: Value =
            serde_json::from_str(&line).unwrap_or_else(|_| panic!("not a reply: {line}"));
        assert_eq!(reply["id"], id, "{reply}");
        reply
    }

    /// A tool call's result text as JSON, and whether it was an error.
    fn tool(&mut self, name: &str, arguments: Value) -> (Value, bool) {
        let reply = self.call(
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
        );
        let result = &reply["result"];
        let text = result["content"][0]["text"]
            .as_str()
            .unwrap_or_else(|| panic!("no tool text: {reply}"));
        (
            serde_json::from_str(text).unwrap(),
            result["isError"].as_bool().unwrap_or(false),
        )
    }
}

/// An upstream on loopback that records the `Authorization` header it is
/// sent and answers with it, so the test sees what the proxy put in.
async fn echo_upstream(seen: Arc<Mutex<Vec<String>>>) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let seen = seen.clone();
            tokio::spawn(async move {
                let _ = http1::Builder::new()
                    .serve_connection(
                        TokioIo::new(stream),
                        service_fn(move |request: Request<Incoming>| {
                            let seen = seen.clone();
                            async move {
                                let (parts, body) = request.into_parts();
                                let _ = body.collect().await;
                                let auth = parts
                                    .headers
                                    .get("authorization")
                                    .and_then(|v| v.to_str().ok())
                                    .unwrap_or_default()
                                    .to_string();
                                seen.lock().unwrap().push(auth.clone());
                                Ok::<_, std::convert::Infallible>(
                                    Response::builder()
                                        .header("content-type", "application/json")
                                        .body(Full::new(Bytes::from(
                                            json!({ "authorization": auth }).to_string(),
                                        )))
                                        .unwrap(),
                                )
                            }
                        }),
                    )
                    .await;
            });
        }
    });
    address
}

#[test]
#[cfg_attr(
    any(target_os = "macos", target_os = "windows"),
    ignore = "the CLI keeps its keys in the OS secure store here, which the test must not touch; runs on Linux"
)]
fn serve_mcp_approve_and_proxy_end_to_end() {
    let data = DataDir::new();
    let config_home = data.root.join("config");
    let agent_key = minted(&config_home, Role::Agent, "Claude Code");
    let sidecar_key = minted(&config_home, Role::Sidecar, "Rowel CLI");
    {
        let opened = data.open();
        testing::seed_api_key(&opened, STRIPE, "Stripe (live)", KEY_VALUE);
        testing::let_in(
            &opened,
            &[
                testing::client(
                    "Claude Code",
                    &agent_key,
                    rowel_broker::actions::ClientKind::Agent,
                    "claude-code",
                ),
                testing::client(
                    "Rowel CLI",
                    &sidecar_key,
                    rowel_broker::actions::ClientKind::Sidecar,
                    "cli",
                ),
            ],
        );
    }
    data.set_agents(true);
    let key_file = data.root.join("vault.key");
    unlock::write_key_file(&key_file, &data.open().key).unwrap();

    let runtime = tokio::runtime::Runtime::new().unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let upstream = runtime.block_on(echo_upstream(seen.clone()));
    let upstream_host = format!("127.0.0.1:{}", upstream.port());

    // Two standing rules for the agent: the env command outright, and the
    // header target at the upstream; the Stripe API target is left to the
    // baseline, which asks.
    let env_line = "printenv STRIPE_KEY";
    Config {
        rules: vec![
            rule(
                "env-rule",
                &agent_key,
                Mode::Env,
                Target::Command(env_line.into()),
            ),
            rule(
                "upstream-rule",
                &agent_key,
                Mode::Header,
                Target::Http {
                    host: upstream_host.clone(),
                    path: "/*".into(),
                    methods: vec![],
                },
            ),
        ],
        ..Config::default()
    }
    .save(&config::path_in(&data.root))
    .unwrap();

    let sandbox = Sandbox {
        config_home,
        broker: Mutex::new(None),
        data,
    };
    let root = sandbox.data.root.to_str().unwrap().to_string();

    // The broker, from the key file, on an ephemeral WebSocket port.
    let mut serve = sandbox
        .command(&[
            "serve",
            "--data-dir",
            &root,
            "--key-file",
            key_file.to_str().unwrap(),
            "--ws",
            "127.0.0.1:0",
            "--ask-timeout",
            "30",
        ])
        .spawn()
        .unwrap();
    let serve_err = lines_of(serve.stderr.take().unwrap());
    let serve = Process(serve);
    let line = wait_for_line(&serve_err, "listening on ws://", Duration::from_secs(30));
    let address = line.rsplit("ws://").next().unwrap().trim().to_string();
    *sandbox.broker.lock().unwrap() = Some(format!("ws://{address}"));

    // 1. `rowel mcp`: a handshake, then an env secret under the standing
    //    rule, active with no ask.
    let mut mcp = Mcp::start(&sandbox);
    let init = mcp.call("initialize", json!({ "protocolVersion": "2025-03-26" }));
    assert_eq!(init["result"]["serverInfo"]["name"], "rowel");
    let reference = format!("rowel://default/{STRIPE}/apiKey");
    let (env_grant, is_error) = mcp.tool(
        "rowel_request_secret",
        json!({ "ref": reference, "mode": "env", "target": env_line, "reason": "ci" }),
    );
    assert!(!is_error, "{env_grant}");
    assert_eq!(env_grant["state"], "active", "{env_grant}");
    let (code, listed, err) = sandbox.run(&["approve", "--data-dir", &root]);
    assert_eq!(code, 0, "{err}");
    assert!(listed.starts_with("nothing is waiting"), "{listed}");

    // 2. A header request the baseline asks for: answered with
    //    `rowel approve` while the tool call waits.
    let asked = thread::spawn({
        let mut mcp = mcp;
        let reference = reference.clone();
        move || {
            let outcome = mcp.tool(
                "rowel_request_secret",
                json!({ "ref": reference, "mode": "header", "target": "GET https://api.stripe.com/v1/*", "reason": "balance" }),
            );
            (mcp, outcome)
        }
    });
    let announced = wait_for_line(&serve_err, "asks to use", Duration::from_secs(30));
    assert!(
        announced.contains("Claude Code (agent)") && announced.contains("api.stripe.com/v1/*"),
        "{announced}"
    );
    let (code, listed, err) = sandbox.run(&["approve", "--data-dir", &root]);
    assert_eq!(code, 0, "{err}");
    let id = listed
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().next())
        .unwrap_or_else(|| panic!("nothing listed: {listed}"))
        .to_string();
    assert_eq!(id.len(), 32, "{listed}");
    let (code, out, err) = sandbox.run(&["approve", &id, "--data-dir", &root]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(out.starts_with("answered:"), "{out}");
    let (mut mcp, (header_grant, is_error)) = asked.join().unwrap();
    assert!(!is_error, "{header_grant}");
    assert_eq!(header_grant["state"], "active", "{header_grant}");
    assert!(header_grant.get("value").is_none());

    // 3. A header grant at the upstream under the other standing rule, used
    //    through `rowel proxy`: the upstream sees the key, the agent's side
    //    the placeholder.
    let (upstream_grant, is_error) = mcp.tool(
        "rowel_request_secret",
        json!({ "ref": reference, "mode": "header", "target": format!("http://{upstream_host}/*") }),
    );
    assert!(!is_error, "{upstream_grant}");
    assert_eq!(upstream_grant["state"], "active", "{upstream_grant}");
    let grant_id = upstream_grant["grantId"].as_str().unwrap().to_string();

    let mut proxy = sandbox
        .command(&["proxy", "--listen", "127.0.0.1:0", "--allow-http"])
        .spawn()
        .unwrap();
    let proxy_err = lines_of(proxy.stderr.take().unwrap());
    let _proxy = Process(proxy);
    let line = wait_for_line(&proxy_err, "listening on http://", Duration::from_secs(30));
    let proxy_base = line
        .split("listening on ")
        .nth(1)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_string();
    let body: Value = runtime.block_on(async {
        let client = reqwest::Client::new();
        let response = client
            .get(format!("{proxy_base}/http/{upstream_host}/echo"))
            .header("authorization", format!("Bearer {reference}"))
            .header("x-rowel-grant", &grant_id)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        response.json().await.unwrap()
    });
    assert_eq!(
        seen.lock().unwrap().as_slice(),
        [format!("Bearer {KEY_VALUE}")],
        "the upstream was sent the value"
    );
    assert_eq!(
        body["authorization"],
        format!("Bearer {reference}"),
        "and the agent's side gets the placeholder back"
    );

    // 4. The log says what happened, and a SIGTERM seals and exits 0.
    let rows = sandbox.data.rows();
    let events: Vec<&str> = rows.iter().map(|(event, _)| event.as_str()).collect();
    assert!(
        events.contains(&"granted") && events.contains(&"used"),
        "{events:?}"
    );
    drop(mcp);
    let pid = serve.0.id();
    #[cfg(unix)]
    {
        let killed = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status()
            .unwrap();
        assert!(killed.success());
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut serve = serve;
        loop {
            if let Some(status) = serve.0.try_wait().unwrap() {
                assert_eq!(status.code(), Some(0), "{status}");
                break;
            }
            assert!(Instant::now() < deadline, "serve did not stop on SIGTERM");
            thread::sleep(Duration::from_millis(50));
        }
        wait_for_line(&serve_err, "sealed, exiting", Duration::from_secs(5));
    }
    #[cfg(not(unix))]
    drop(serve);
}
