//! The crate's tests: the host driven as the broker's connection drives it,
//! over a vault in a temporary data directory, with the asks answered
//! through the approvals API, the control socket, and the WebSocket.

use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use zeroize::Zeroizing;

use rowel_broker::actions::{ClientKind, Connection};
use rowel_broker::client::{Channel, Peer, PeerError};
use rowel_broker::grant::{Approval, ItemSelector, Mode, Target};
use rowel_broker::policy::{AgentSelector, Rule};
use rowel_broker::protocol::Code;
use rowel_broker::testing::Extension;
use rowel_core::workspace;

use crate::approvals::{Answer, AskKind};
use crate::config::{self, Config};
use crate::host::{AnswerRequest, HeadlessHost, Overrides};
use crate::testing::{self, DataDir, Notices, PASSWORD};
use crate::unlock::{self, Unlock};
use crate::{control, listen};

const AGENT_KEY: &str = "agent-key";
const SIDECAR_KEY: &str = "sidecar-key";
const STRIPE: &str = "stripe-item";
const CARD: &str = "card-item";

/// A data directory with an API key, a card and a login in the vault, the
/// agent and the sidecar let in, and the lockbox's agent access on.
fn seeded() -> DataDir {
    let data = DataDir::new();
    let opened = data.open();
    testing::seed_api_key(&opened, STRIPE, "Stripe (live)", "sk_live_123456789");
    testing::seed_card(&opened, CARD, "Visa", "4111111111111111");
    testing::seed_login(
        &opened,
        "gh",
        "GitHub",
        "https://github.com",
        "octocat",
        "hunter2",
    );
    testing::let_in(
        &opened,
        &[
            testing::client("Claude Code", AGENT_KEY, ClientKind::Agent, "claude-code"),
            testing::client("Rowel CLI", SIDECAR_KEY, ClientKind::Sidecar, "cli"),
        ],
    );
    data.set_agents(true);
    data
}

fn config_with(data: &DataDir, rules: Vec<Rule>) {
    Config {
        rules,
        ..Config::default()
    }
    .save(&config::path_in(&data.root))
    .unwrap();
}

fn stripe_rule(agent: &str, approval: Approval) -> Rule {
    Rule {
        id: "cfg-stripe".into(),
        agents: AgentSelector::Ids(vec![agent.into()]),
        items: ItemSelector::Ids(vec![STRIPE.into()]),
        targets: vec![Target::Http {
            host: "api.stripe.com".into(),
            path: "/v1/*".into(),
            methods: vec![],
        }],
        modes: vec![Mode::Header],
        approval,
        ttl_ms: 15 * 60 * 1000,
        max_uses: None,
        reason_required: false,
        approvers: None,
        sidecar_for: None,
        created_at_ms: 0,
        created_by: rowel_broker::grant::CreatedBy::User,
        expires_at_ms: None,
    }
}

/// A connection let in under `key`, as the CLI lets itself in: the exchange,
/// then `test-associate`.
fn peer(host: &HeadlessHost, key: &str) -> Peer<Connection<HeadlessHost>> {
    let name = if key == AGENT_KEY {
        "Claude Code"
    } else {
        "Rowel CLI"
    };
    let mut peer = Peer::connect(Connection::new(host.clone()), "rowel-mcp").unwrap();
    peer.call("test-associate", json!({ "id": name, "key": key }))
        .unwrap();
    peer
}

fn header_request(reason: &str) -> Value {
    json!({
        "runId": "run-1",
        "reason": reason,
        "items": { "ids": [STRIPE] },
        "targets": [{ "http": { "host": "api.stripe.com", "path": "/v1/balance", "methods": ["GET"] } }],
        "modes": ["header"],
    })
}

fn env_request(items: &[&str], line: &str) -> Value {
    json!({
        "runId": "run-2",
        "items": { "ids": items },
        "targets": [{ "command": line }],
        "modes": ["env"],
    })
}

/// Wait for one ask to be queued, and hand its id back.
fn pending_id(host: &HeadlessHost) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(ask) = host.pending().first() {
            return ask.id.clone();
        }
        assert!(Instant::now() < deadline, "no ask was queued");
        thread::sleep(Duration::from_millis(10));
    }
}

fn refused(error: PeerError) -> Code {
    match error {
        PeerError::Refused(refused) => refused.code.expect("a known code"),
        other => panic!("not a refusal: {other:?}"),
    }
}

fn row(event: &str, detail: &str) -> (String, Option<String>) {
    (event.into(), Some(detail.into()))
}

fn events(rows: &[(String, Option<String>)]) -> Vec<&str> {
    rows.iter().map(|(event, _)| event.as_str()).collect()
}

// --- unlock -------------------------------------------------------------------------

#[test]
fn the_lockbox_opens_with_the_password_and_refuses_the_wrong_one() {
    let data = DataDir::new();
    let opened = data.open();
    assert!(opened.kdf.is_some(), "an Argon2id vault");
    let wrong = unlock::open(
        &data.root,
        workspace::PRIMARY_ID,
        &Unlock::Password("nope".to_string().into()),
    );
    assert!(matches!(wrong, Err(unlock::UnlockError::WrongKey)));
    let missing = unlock::open(
        &data.root,
        "other",
        &Unlock::Password(PASSWORD.to_string().into()),
    );
    assert!(matches!(missing, Err(unlock::UnlockError::NoVault(_))));
}

#[test]
fn a_key_file_round_trips_and_is_owner_only() {
    let data = DataDir::new();
    let opened = data.open();
    let path = data.root.join("vault.key");
    unlock::write_key_file(&path, &opened.key).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    let again = unlock::open(
        &data.root,
        workspace::PRIMARY_ID,
        &Unlock::KeyFile(path.clone()),
    )
    .unwrap();
    assert_eq!(
        &*again.key.sqlcipher_key(),
        &*opened.key.sqlcipher_key(),
        "the same key"
    );
    let material = unlock::read_key_file(&path).unwrap();
    let from_fd = unlock::open(
        &data.root,
        workspace::PRIMARY_ID,
        &Unlock::Material(material),
    )
    .unwrap();
    assert_eq!(&*from_fd.key.sqlcipher_key(), &*opened.key.sqlcipher_key());
}

#[test]
fn an_interrupted_password_change_refuses_to_open() {
    let data = DataDir::new();
    std::fs::write(
        data.root.join(rowel_core::layout::DB_REKEY_BACKUP_FILE),
        b"snapshot",
    )
    .unwrap();
    let outcome = unlock::open(
        &data.root,
        workspace::PRIMARY_ID,
        &Unlock::Password(PASSWORD.to_string().into()),
    );
    assert!(matches!(
        outcome,
        Err(unlock::UnlockError::RekeyInterrupted(_))
    ));
}

#[test]
fn the_install_id_is_minted_once_and_read_back() {
    let data = DataDir::new();
    let id = crate::install_id_in(&data.root);
    assert_eq!(id.len(), 32);
    assert_eq!(crate::install_id_in(&data.root), id);
    assert_eq!(
        std::fs::read_to_string(data.root.join(crate::INSTALL_ID_FILE)).unwrap(),
        id
    );
}

// --- association ------------------------------------------------------------------

#[test]
fn an_association_waits_for_the_answer_and_is_then_stored_and_on_the_log() {
    let data = seeded();
    let notices = Notices::default();
    let host = data.host(&notices);
    let asked = {
        let host = host.clone();
        thread::spawn(move || {
            let mut peer = Peer::connect(Connection::new(host), "rowel-mcp").unwrap();
            let hello = json!({ "kind": "agent", "platform": "codex", "version": "1", "capabilities": ["grants"] });
            peer.call(
                "associate",
                json!({ "key": peer.public_key(), "idKey": "new-agent-key", "hello": hello }),
            )
        })
    };
    let id = pending_id(&host);
    assert!(matches!(host.pending()[0].kind, AskKind::Associate { .. }));
    let line = notices.lines().last().unwrap().clone();
    assert!(
        line.contains(&id) && line.contains("agent") && line.contains("codex"),
        "{line}"
    );
    assert!(
        !line.contains("new-agent-key"),
        "the key is shown by its ends: {line}"
    );
    host.answer(AnswerRequest {
        id,
        ..AnswerRequest::default()
    })
    .unwrap();
    let reply = asked.join().unwrap().unwrap();
    assert_eq!(
        reply["id"], "Codex",
        "the platform's label when no name is given"
    );
    assert_eq!(reply["clientId"], "new-agent-key");

    let clients = data.open().store;
    let clients = rowel_broker::lockbox::clients_in(&clients);
    let stored = clients.iter().find(|c| c.key == "new-agent-key").unwrap();
    assert_eq!(stored.name, "Codex");
    assert_eq!(stored.kind, ClientKind::Agent);
    assert_eq!(data.rows(), vec![("associated".to_string(), None)]);
}

#[test]
fn a_denied_association_is_refused() {
    let data = seeded();
    let host = data.host(&Notices::default());
    let asked = {
        let host = host.clone();
        thread::spawn(move || {
            let mut peer = Peer::connect(Connection::new(host), "rowel-mcp").unwrap();
            peer.call(
                "associate",
                json!({ "key": peer.public_key(), "idKey": "k2", "hello": { "kind": "agent" } }),
            )
        })
    };
    let id = pending_id(&host);
    host.answer(AnswerRequest {
        id,
        deny: true,
        ..AnswerRequest::default()
    })
    .unwrap();
    assert_eq!(
        refused(asked.join().unwrap().unwrap_err()),
        Code::ActionCancelledOrDenied
    );
    assert!(data.rows().is_empty());
}

// --- grants -----------------------------------------------------------------------

#[test]
fn a_config_auto_rule_issues_a_header_grant_without_an_ask() {
    let data = seeded();
    config_with(&data, vec![stripe_rule(AGENT_KEY, Approval::Auto)]);
    let host = data.host(&Notices::default());
    let mut agent = peer(&host, AGENT_KEY);
    let reply = agent
        .call("request-grant", header_request("check the balance"))
        .unwrap();
    assert_eq!(reply["state"], "active");
    assert_eq!(reply["items"][0]["id"], STRIPE);
    assert!(host.pending().is_empty());
    assert_eq!(events(&data.rows()), ["requested", "granted"]);

    // The sidecar resolves under it, at the place the rule names, and that
    // is the use.
    let mut sidecar = peer(&host, SIDECAR_KEY);
    let value = sidecar
        .call(
            "resolve",
            json!({
                "grantId": reply["grantId"],
                "ref": format!("rowel://default/{STRIPE}/apiKey"),
                "context": { "target": { "http": { "host": "api.stripe.com", "path": "/v1/balance", "method": "GET" } } },
            }),
        )
        .unwrap();
    assert_eq!(value["value"], "sk_live_123456789");
    let rows = data.rows();
    assert_eq!(events(&rows), ["requested", "granted", "used"]);
    assert_eq!(
        rows[2].1.as_deref(),
        Some(r#"{"via":"resolve","field":"apiKey"}"#)
    );
}

#[test]
fn a_request_no_rule_covers_asks_and_approve_activates_it() {
    let data = seeded();
    let notices = Notices::default();
    let host = data.host(&notices);
    let asked = {
        let host = host.clone();
        thread::spawn(move || {
            let mut agent = peer(&host, AGENT_KEY);
            agent.call("request-grant", header_request("deploy"))
        })
    };
    let id = pending_id(&host);
    let ask = host.pending().remove(0);
    let AskKind::Grant {
        client_name,
        mode,
        targets,
        reason,
        candidates,
        step_up,
        ..
    } = ask.kind
    else {
        panic!("a grant ask")
    };
    assert_eq!(client_name, "Claude Code");
    assert_eq!(mode, Mode::Header);
    assert_eq!(targets, ["GET api.stripe.com/v1/balance"]);
    assert_eq!(reason.as_deref(), Some("deploy"));
    assert_eq!(candidates[0].title, "Stripe (live)");
    assert!(!step_up);
    let line = notices.lines().last().unwrap().clone();
    assert!(
        line.contains("Claude Code (agent) asks to use \"Stripe (live)\" in header mode at GET api.stripe.com/v1/balance — reason: deploy"),
        "{line}"
    );
    assert!(
        line.contains(&format!("rowel approve {id} --deny")),
        "{line}"
    );
    assert!(!line.contains("sk_live"), "{line}");

    host.answer(AnswerRequest {
        id,
        ..AnswerRequest::default()
    })
    .unwrap();
    let reply = asked.join().unwrap().unwrap();
    assert_eq!(reply["state"], "active");
    assert_eq!(events(&data.rows()), ["requested", "granted"]);

    let mut sidecar = peer(&host, SIDECAR_KEY);
    let value = sidecar
        .call(
            "resolve",
            json!({
                "grantId": reply["grantId"],
                "ref": format!("rowel://default/{STRIPE}/apiKey"),
                "context": { "target": { "http": { "host": "api.stripe.com", "path": "/v1/balance", "method": "GET" } } },
            }),
        )
        .unwrap();
    assert_eq!(value["value"], "sk_live_123456789");
    assert_eq!(events(&data.rows()), ["requested", "granted", "used"]);
}

#[test]
fn a_deny_and_a_timed_out_ask_are_refused_with_their_reasons() {
    let data = seeded();
    let host = data.host_with(
        &Notices::default(),
        Overrides {
            ask_timeout_ms: Some(300),
            idle_lock_ms: None,
        },
        None,
    );
    let asked = {
        let host = host.clone();
        thread::spawn(move || {
            let mut agent = peer(&host, AGENT_KEY);
            agent.call("request-grant", header_request("one"))
        })
    };
    let id = pending_id(&host);
    host.answer(AnswerRequest {
        id,
        deny: true,
        ..AnswerRequest::default()
    })
    .unwrap();
    assert_eq!(
        refused(asked.join().unwrap().unwrap_err()),
        Code::GrantDenied
    );
    assert_eq!(
        data.rows(),
        vec![
            ("requested".to_string(), None),
            row("denied", r#"{"reason":"user"}"#)
        ]
    );

    let mut agent = peer(&host, AGENT_KEY);
    let started = Instant::now();
    let outcome = agent.call("request-grant", header_request("two"));
    assert!(started.elapsed() >= Duration::from_millis(300));
    assert_eq!(refused(outcome.unwrap_err()), Code::GrantDenied);
    assert_eq!(data.rows()[3], row("denied", r#"{"reason":"timeout"}"#));
    assert!(host.pending().is_empty(), "the slot is taken back");
}

#[test]
fn the_lockbox_flag_off_refuses_on_record_and_cuts_a_running_broker_off() {
    let data = seeded();
    data.set_agents(false);
    config_with(&data, vec![stripe_rule(AGENT_KEY, Approval::Auto)]);
    let host = data.host(&Notices::default());
    let mut agent = peer(&host, AGENT_KEY);
    let outcome = agent.call("request-grant", header_request("x"));
    assert_eq!(refused(outcome.unwrap_err()), Code::GrantDenied);
    assert_eq!(data.rows(), vec![row("denied", r#"{"reason":"lockbox"}"#)]);
    assert_eq!(
        refused(agent.call("list-visible", json!({})).unwrap_err()),
        Code::GrantDenied
    );

    // The registry's watcher looks at most once a second.
    thread::sleep(Duration::from_millis(1100));
    data.set_agents(true);
    host.refresh();
    let reply = agent.call("request-grant", header_request("y")).unwrap();
    assert_eq!(reply["state"], "active");

    thread::sleep(Duration::from_millis(1100));
    data.set_agents(false);
    host.refresh();
    let rows = data.rows();
    assert_eq!(
        rows.last().unwrap(),
        &row("revoked", r#"{"reason":"lockbox"}"#)
    );
    let status = agent
        .call("grant-status", json!({ "grantId": reply["grantId"] }))
        .unwrap();
    assert_eq!(status["state"], "revoked");
}

#[test]
fn an_env_request_for_a_card_is_a_step_up_and_a_wrong_password_is_a_no() {
    let data = seeded();
    let host = data.host(&Notices::default());
    let line = "stripe charge --card 'rowel://default/card-item/number'";
    let ask_for = |host: &HeadlessHost| {
        let host = host.clone();
        thread::spawn(move || {
            let mut sidecar = peer(&host, SIDECAR_KEY);
            sidecar.call("request-grant", env_request(&[CARD], line))
        })
    };
    let asked = ask_for(&host);
    let id = pending_id(&host);
    let AskKind::Grant { step_up, .. } = host.pending()[0].kind.clone() else {
        panic!("a grant ask")
    };
    assert!(step_up, "a card is stepped up whatever the rule says");

    // No password at all leaves the ask waiting.
    let error = host
        .answer(AnswerRequest {
            id: id.clone(),
            ..AnswerRequest::default()
        })
        .unwrap_err();
    assert!(error.contains("master password"), "{error}");
    assert_eq!(host.pending().len(), 1);

    let error = host
        .answer(AnswerRequest {
            id,
            password: Some(Zeroizing::new("not it".into())),
            ..AnswerRequest::default()
        })
        .unwrap_err();
    assert!(error.contains("did not match"), "{error}");
    assert_eq!(
        refused(asked.join().unwrap().unwrap_err()),
        Code::GrantDenied
    );
    assert_eq!(
        data.rows(),
        vec![
            ("requested".to_string(), None),
            row("denied", r#"{"reason":"user"}"#)
        ]
    );

    let asked = ask_for(&host);
    let id = pending_id(&host);
    host.answer(AnswerRequest {
        id,
        password: Some(Zeroizing::new(PASSWORD.into())),
        ..AnswerRequest::default()
    })
    .unwrap();
    let reply = asked.join().unwrap().unwrap();
    assert_eq!(reply["state"], "active");
    assert_eq!(reply["items"][0]["id"], CARD);
}

#[test]
fn a_seal_clears_the_grants_and_refuses_the_ask_up() {
    let data = seeded();
    config_with(&data, vec![stripe_rule(AGENT_KEY, Approval::Auto)]);
    let host = data.host(&Notices::default());
    let mut agent = peer(&host, AGENT_KEY);
    let granted = agent.call("request-grant", header_request("a")).unwrap();
    assert_eq!(granted["state"], "active");

    let asked = {
        let host = host.clone();
        thread::spawn(move || {
            let mut sidecar = peer(&host, SIDECAR_KEY);
            sidecar.call("request-grant", env_request(&[STRIPE], "stripe balance"))
        })
    };
    pending_id(&host);
    assert!(host.seal());
    assert_eq!(
        refused(asked.join().unwrap().unwrap_err()),
        Code::GrantDenied
    );
    assert!(host.pending().is_empty());
    assert!(!host.is_unlocked());
    assert!(!host.seal(), "nothing left to seal");

    // Sealed, the broker serves nothing — and after the next unlock the
    // grant from before is gone.
    assert_eq!(
        refused(
            agent
                .call("grant-status", json!({ "grantId": granted["grantId"] }))
                .unwrap_err()
        ),
        Code::DatabaseNotOpened
    );
    host.unlock(data.open());
    assert_eq!(
        refused(
            agent
                .call("grant-status", json!({ "grantId": granted["grantId"] }))
                .unwrap_err()
        ),
        Code::GrantDenied
    );
}

#[test]
fn an_idle_lock_in_key_file_mode_is_lifted_by_the_next_request() {
    let data = seeded();
    config_with(&data, vec![stripe_rule(AGENT_KEY, Approval::Auto)]);
    let key_file = data.root.join("vault.key");
    unlock::write_key_file(&key_file, &data.open().key).unwrap();
    let host = data.host_with(
        &Notices::default(),
        Overrides {
            ask_timeout_ms: None,
            idle_lock_ms: Some(1),
        },
        Some(key_file),
    );
    thread::sleep(Duration::from_millis(5));
    assert!(host.tick(), "idle: sealed");
    assert!(!host.is_unlocked());
    let mut agent = peer(&host, AGENT_KEY);
    let reply = agent.call("request-grant", header_request("a")).unwrap();
    assert_eq!(reply["state"], "active");
    assert!(host.is_unlocked());
}

// --- the extension's side ------------------------------------------------------------

#[test]
fn the_extensions_fill_and_the_refusals_it_handles() {
    let data = seeded();
    let opened = data.open();
    testing::let_in(
        &opened,
        &[testing::client(
            "Chrome",
            "ext-key",
            ClientKind::Extension,
            "browser",
        )],
    );
    drop(opened);
    let host = data.host(&Notices::default());
    let mut extension = Extension::new();
    let mut connection = Connection::new(host.clone());
    extension.exchange(&mut connection);
    extension.send(
        &mut connection,
        "test-associate",
        json!({ "id": "Chrome", "key": "ext-key" }),
    );
    let logins = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": "https://github.com/login", "keys": [{ "id": "Chrome", "key": "ext-key" }] }),
    );
    assert_eq!(logins["entries"][0]["login"], "octocat");
    assert_eq!(logins["entries"][0]["password"], "hunter2");
    let generated = extension.send(&mut connection, "generate-password", json!({}));
    assert_eq!(
        rowel_broker::testing::error_code(&generated),
        Code::ActionCancelledOrDenied as u8
    );
    let hash = extension.send(&mut connection, "get-databasehash", json!({}));
    assert_eq!(hash["hash"].as_str().unwrap().len(), 64);
}

// --- the control socket -----------------------------------------------------------------

#[test]
fn the_control_socket_lists_and_answers_asks_and_seals() {
    let data = seeded();
    let host = data.host(&Notices::default());
    let listener = control::bind(&data.root).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(data.root.join(control::SOCKET_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    let served = host.clone();
    thread::spawn(move || control::serve(listener, served));

    let mut client = control::Client::connect(&data.root).unwrap();
    assert!(client.pending().unwrap().is_empty());
    let status = client.status().unwrap();
    assert!(status.unlocked);
    assert_eq!(status.lockbox, "default");

    let asked = {
        let host = host.clone();
        thread::spawn(move || {
            let mut agent = peer(&host, AGENT_KEY);
            agent.call("request-grant", header_request("via control"))
        })
    };
    let id = pending_id(&host);
    let pending = client.pending().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, id);
    client
        .answer(AnswerRequest {
            id: id.clone(),
            days: Some(7),
            ..AnswerRequest::default()
        })
        .unwrap();
    let reply = asked.join().unwrap().unwrap();
    assert_eq!(reply["state"], "active");
    assert_eq!(
        events(&data.rows()),
        ["requested", "rule_added", "granted"],
        "for 7 days wrote a standing rule"
    );
    let error = client
        .answer(AnswerRequest {
            id,
            ..AnswerRequest::default()
        })
        .unwrap_err();
    assert!(error.to_string().contains("no ask"), "{error}");

    client.lock().unwrap();
    assert!(!host.is_unlocked());
    assert!(!client.status().unwrap().unlocked);
}

// --- the WebSocket ------------------------------------------------------------------

/// One frame out over the WebSocket, and the next binary message in.
fn roundtrip<S: std::io::Read + std::io::Write>(
    socket: &mut tungstenite::WebSocket<S>,
    request: &Value,
) -> Value {
    socket
        .send(tungstenite::Message::binary(
            serde_json::to_vec(request).unwrap(),
        ))
        .unwrap();
    loop {
        if let tungstenite::Message::Binary(body) = socket.read().unwrap() {
            return serde_json::from_slice(&body).unwrap();
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_websocket_carries_frames_and_pushed_signals() {
    let data = seeded();
    let host = data.host(&Notices::default());
    let listener = listen::bind_ws("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(listen::serve_ws(listener, None, host.clone()));

    let sealed = host.clone();
    tokio::task::spawn_blocking(move || {
        let (mut socket, _) = tungstenite::connect(format!("ws://{address}/")).unwrap();
        let mut channel = Channel::new();
        let (request, nonce) = channel.exchange_request("rowel-mcp");
        let reply = roundtrip(&mut socket, &request);
        channel.accept_exchange(&nonce, &reply).unwrap();
        let (request, nonce) = channel
            .seal("rowel-mcp", "get-databasehash", json!({}))
            .unwrap();
        let reply = roundtrip(&mut socket, &request);
        let opened = channel.open("get-databasehash", &nonce, &reply).unwrap();
        assert_eq!(opened["hash"].as_str().unwrap().len(), 64);
        // A text message is not a frame of ours and is ignored, not answered.
        socket.send(tungstenite::Message::text("hello")).unwrap();
        // A clear, unsolicited signal reaches the connection as a binary
        // message of its own.
        sealed.seal();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            assert!(Instant::now() < deadline, "no signal arrived");
            if let tungstenite::Message::Binary(body) = socket.read().unwrap() {
                assert_eq!(&body[..], rowel_broker::server::LOCKED);
                break;
            }
        }
        let _ = socket.close(None);
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_websocket_turns_connections_past_the_ceiling_away() {
    let data = seeded();
    let host = data.host(&Notices::default());
    let listener = listen::bind_ws("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(listen::serve_ws_limited(listener, None, host.clone(), 1));

    tokio::task::spawn_blocking(move || {
        let url = format!("ws://{address}/");
        let (mut first, _) = tungstenite::connect(&url).unwrap();
        // The one slot is taken: the next handshake is closed before it
        // completes, and the broker is otherwise unaffected.
        assert!(
            tungstenite::connect(&url).is_err(),
            "a connection past the ceiling was accepted"
        );
        let mut channel = Channel::new();
        let (request, nonce) = channel.exchange_request("rowel-mcp");
        let reply = roundtrip(&mut first, &request);
        channel.accept_exchange(&nonce, &reply).unwrap();
        // Closing the first frees its slot; the release follows the close by
        // the time the connection task notices it.
        let _ = first.close(None);
        drop(first);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match tungstenite::connect(&url) {
                Ok((mut third, _)) => {
                    let _ = third.close(None);
                    break;
                }
                Err(_) if Instant::now() < deadline => thread::sleep(Duration::from_millis(50)),
                Err(e) => panic!("the slot was not released: {e}"),
            }
        }
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_hung_up_connection_keeps_its_slot_until_its_ask_is_answered() {
    let data = seeded();
    let host = data.host(&Notices::default());
    let listener = listen::bind_ws("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(listen::serve_ws_limited(listener, None, host.clone(), 1));

    let asked = host.clone();
    tokio::task::spawn_blocking(move || {
        let url = format!("ws://{address}/");
        // The agent is let in, asks for a grant no rule covers, and hangs up
        // while the ask waits.
        let (mut socket, _) = tungstenite::connect(&url).unwrap();
        let mut channel = Channel::new();
        let (request, nonce) = channel.exchange_request("rowel-mcp");
        let reply = roundtrip(&mut socket, &request);
        channel.accept_exchange(&nonce, &reply).unwrap();
        let (request, nonce) = channel
            .seal(
                "rowel-mcp",
                "test-associate",
                json!({ "id": "Claude Code", "key": AGENT_KEY }),
            )
            .unwrap();
        let reply = roundtrip(&mut socket, &request);
        channel.open("test-associate", &nonce, &reply).unwrap();
        let (request, _) = channel
            .seal("rowel-mcp", "request-grant", header_request("then hang up"))
            .unwrap();
        socket
            .send(tungstenite::Message::binary(
                serde_json::to_vec(&request).unwrap(),
            ))
            .unwrap();
        let id = pending_id(&asked);
        drop(socket);

        // The socket is gone, the thread is not: the one slot stays taken
        // for as long as the ask does, so hanging up buys nothing.
        thread::sleep(Duration::from_millis(200));
        assert!(
            tungstenite::connect(&url).is_err(),
            "a hung-up connection released its slot while its ask waited"
        );
        asked
            .answer(AnswerRequest {
                id,
                deny: true,
                ..AnswerRequest::default()
            })
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match tungstenite::connect(&url) {
                Ok((mut next, _)) => {
                    let _ = next.close(None);
                    break;
                }
                Err(_) if Instant::now() < deadline => thread::sleep(Duration::from_millis(50)),
                Err(e) => panic!("the slot was not released once the ask was answered: {e}"),
            }
        }
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_websocket_message_over_the_frame_cap_ends_the_connection() {
    let data = seeded();
    let host = data.host(&Notices::default());
    let listener = listen::bind_ws("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(listen::serve_ws(listener, None, host.clone()));

    tokio::task::spawn_blocking(move || {
        let (mut socket, _) = tungstenite::connect(format!("ws://{address}/")).unwrap();
        let too_big = vec![b'x'; rowel_broker::frame::MAX_FRAME + 1];
        // The broker refuses the message on its header, never buffering it
        // whole, and closes; the client's send breaks on the closed socket,
        // or its next read sees the close. Neither way is there a reply.
        if socket.send(tungstenite::Message::binary(too_big)).is_err() {
            return;
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            assert!(Instant::now() < deadline, "the connection stayed open");
            match socket.read() {
                Ok(tungstenite::Message::Close(_)) | Err(_) => break,
                Ok(tungstenite::Message::Binary(_)) => panic!("an oversized message was answered"),
                Ok(_) => {}
            }
        }
    })
    .await
    .unwrap();
}

#[test]
fn a_config_file_is_read_strictly_and_reloaded_on_change() {
    let data = DataDir::new();
    let path = config::path_in(&data.root);
    assert_eq!(Config::load(&path).unwrap(), Config::default());
    std::fs::write(
        &path,
        r#"{ "rules": [], "askTimeoutMs": 5000, "bogus": 1 }"#,
    )
    .unwrap();
    assert!(Config::load(&path).is_err(), "an unknown key is a mistake");
    std::fs::write(
        &path,
        r#"{ "rules": [], "askTimeoutMs": 5000, "idleLockMs": 60000, "listen": { "socket": false, "ws": "127.0.0.1:7401", "tls": null } }"#,
    )
    .unwrap();
    let loaded = Config::load(&path).unwrap();
    assert_eq!(loaded.ask_timeout_ms, 5000);
    assert_eq!(loaded.idle_lock_ms, Some(60_000));
    assert!(!loaded.listen.socket);
    assert_eq!(loaded.listen.ws.as_deref(), Some("127.0.0.1:7401"));

    let config = Config {
        rules: vec![stripe_rule("someone", Approval::Ask)],
        ..Config::default()
    };
    config.save(&path).unwrap();
    let rules = Config::load(&path).unwrap().rules;
    assert_eq!(
        rules[0].created_by,
        rowel_broker::grant::CreatedBy::Admin("config".into()),
        "the operator's, whatever the file said"
    );
}

#[test]
fn an_answer_of_the_wrong_kind_does_not_land() {
    let approvals = crate::approvals::Approvals::new(std::sync::Arc::new(|_| {}));
    let ticket = approvals.enqueue(
        AskKind::Passkey {
            origin: "https://example.test".into(),
            client_name: "Chrome".into(),
            ceremony: "sign in".into(),
            accounts: vec!["alice".into()],
        },
        Duration::from_secs(5),
    );
    let id = ticket.id().to_string();
    assert_eq!(
        approvals.answer(&id, Answer::Grant(None)),
        Err(crate::approvals::AnswerError::WrongKind)
    );
    approvals.answer(&id, Answer::Account(Some(0))).unwrap();
    assert_eq!(ticket.wait(), Some(Answer::Account(Some(0))));
    assert_eq!(
        approvals.answer(&id, Answer::Account(None)),
        Err(crate::approvals::AnswerError::NotFound)
    );
}
