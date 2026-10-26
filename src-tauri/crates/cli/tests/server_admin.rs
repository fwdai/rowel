//! `rowel admin server` and `rowel serve --server` over the built binary and
//! a temporary data directory: a server set up from nothing to a lockbox
//! with a holder, an agent with a trust policy for itself and one for its
//! sidecar, a rule, and an empty audit mirror that exports nothing and
//! verifies; then the server itself started, answering on its port, alone
//! on its data directory, and stopped. Unlike `tests/serve.rs` this runs
//! everywhere: no client key of the CLI's is involved, so the OS secure
//! store is never touched.

#![cfg(not(any(target_os = "android", target_os = "ios")))]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use rowel_server::config::{self, Config};

const BIN: &str = env!("CARGO_BIN_EXE_rowel-cli");

/// `rowel admin server <args> --data-dir <data>`: exit code, stdout, stderr.
fn admin(data: &Path, args: &[&str]) -> (i32, String, String) {
    let output = Command::new(BIN)
        .arg("admin")
        .arg("server")
        .args(args)
        .arg("--data-dir")
        .arg(data)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", data.parent().unwrap())
        .output()
        .unwrap();
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// The command succeeded; its stdout.
fn ok(data: &Path, args: &[&str]) -> String {
    let (code, out, err) = admin(data, args);
    assert_eq!(code, 0, "{args:?}: {out}{err}");
    out
}

/// The command printed one id, as every `add` and `create` does.
fn id(data: &Path, args: &[&str]) -> String {
    let id = ok(data, args).trim().to_string();
    assert_eq!(id.len(), 32, "{args:?}: {id}");
    id
}

#[test]
fn a_server_is_set_up_from_nothing_through_the_admin_commands() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("data");
    let root_key = temp.path().join("keys").join("root.key");

    // 1. init: a config that loads, a root key apart from the data
    //    directory and readable by its owner alone; a second init is refused.
    let out = ok(
        &data,
        &[
            "init",
            "--tenant",
            "Acme",
            "--public-url",
            "https://Rowel.Acme.Example/",
            "--root-key",
            root_key.to_str().unwrap(),
        ],
    );
    assert!(
        out.contains("https://rowel.acme.example/v1/members/auth/callback"),
        "{out}"
    );
    assert!(out.contains("rowel serve --server --data-dir"), "{out}");
    let config = Config::load(&config::path_in(&data)).unwrap();
    assert_eq!(config.tenant.name, "Acme");
    assert_eq!(config.public_url, "https://rowel.acme.example");
    assert_eq!(config.root_key_file, root_key);
    assert_eq!(std::fs::read(&root_key).unwrap().len(), 32);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&root_key), 0o600);
        assert_eq!(mode(&config::path_in(&data)), 0o600);
    }
    let (code, _, err) = admin(&data, &["init"]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("never overwritten"), "{err}");

    // 2. A member, named afterwards by email.
    let pat = id(&data, &["members", "add", "pat@acme.example", "--admin"]);
    let listed = ok(&data, &["members", "list"]);
    assert!(
        listed.contains(&pat) && listed.contains("pat@acme.example") && listed.contains("admin"),
        "{listed}"
    );
    let (code, _, err) = admin(&data, &["members", "add", "PAT@acme.example"]);
    assert_eq!((code, err.lines().count()), (1, 1), "{err}");

    // 3. A lockbox, held by the member.
    let lockbox = id(&data, &["lockboxes", "create", "Deploy"]);
    let listed = ok(&data, &["lockboxes", "list"]);
    assert!(
        listed.contains(&lockbox) && listed.contains("Deploy"),
        "{listed}"
    );
    ok(
        &data,
        &[
            "lockboxes",
            "members",
            "add",
            &lockbox,
            "pat@acme.example",
            "--role",
            "approver",
        ],
    );
    let holders = ok(&data, &["lockboxes", "members", "list", &lockbox]);
    assert!(
        holders.contains(&pat) && holders.contains("approver"),
        "{holders}"
    );

    // 4. An agent and the trust policy that names it.
    let agent = id(&data, &["agents", "add", "Deployer", "--lockbox", &lockbox]);
    let policy = id(
        &data,
        &[
            "trust",
            "add",
            "--agent",
            &agent,
            "--issuer",
            "https://token.actions.githubusercontent.com",
            "--claim",
            "repository=acme/api",
            "--claim",
            "ref=glob:refs/heads/*",
        ],
    );
    let sidecar = id(
        &data,
        &[
            "trust",
            "add",
            "--agent",
            &agent,
            "--issuer",
            "https://token.actions.githubusercontent.com",
            "--claim",
            "job_workflow_ref=acme/ci/.github/workflows/rowel-proxy.yml@refs/heads/main",
            "--sidecar",
        ],
    );
    let agents = ok(&data, &["agents", "list"]);
    assert!(
        agents.contains(&agent)
            && agents.contains(&format!("trust {policy}  agent  "))
            && agents.contains(&format!("trust {sidecar}  sidecar  "))
            && agents.contains("repository=acme/api")
            && agents.contains("ref=glob:refs/heads/*"),
        "{agents}"
    );

    // 5. A rule for the lockbox, listed and removed.
    let rule = id(
        &data,
        &[
            "rules",
            "add",
            "--agent",
            &agent,
            "--items",
            "lockbox",
            "--mode",
            "header",
            "--target",
            "GET api.stripe.com/v1/*",
            "--lockbox",
            &lockbox,
        ],
    );
    let rules = ok(&data, &["rules", "list"]);
    assert!(
        rules.contains(&rule) && rules.contains(&format!("lockbox {lockbox}")),
        "{rules}"
    );
    ok(&data, &["rules", "remove", &rule]);
    assert_eq!(ok(&data, &["rules", "list"]).trim(), "no rules");

    // 6. The audit mirror is empty: nothing to export, and it verifies.
    assert_eq!(ok(&data, &["audit", "export"]), "");
    assert!(ok(&data, &["audit", "verify"]).starts_with("ok"));

    // 7. A usage mistake is 2, a name that is not there is 1, each one line.
    let (code, _, _) = admin(&data, &["lockboxes", "members", "add", &lockbox, &pat]);
    assert_eq!(code, 2);
    let (code, _, err) = admin(&data, &["devices", "list", "--member", "sam@acme.example"]);
    assert_eq!(code, 1);
    assert_eq!(err.trim(), "rowel admin: no member sam@acme.example");
}

#[test]
fn a_data_directory_without_a_server_is_not_made_into_one() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("data");
    std::fs::create_dir(&data).unwrap();
    std::fs::write(config::path_in(&data), r#"{ "rootKeyFile": "/k" }"#).unwrap();
    let (code, _, err) = admin(&data, &["members", "list"]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("run rowel admin server init first"), "{err}");
    assert!(!data.join("server.db").exists());
}

/// `rowel serve --server --data-dir <data>`, its stderr collected as it
/// comes so the pipe never fills; the collecting thread ends at the
/// process's exit.
fn start_server(data: &Path) -> (Child, Arc<Mutex<String>>, JoinHandle<()>) {
    let mut child = Command::new(BIN)
        .args(["serve", "--server", "--data-dir"])
        .arg(data)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", data.parent().unwrap())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let said = Arc::new(Mutex::new(String::new()));
    let mut stderr = child.stderr.take().unwrap();
    let heard = said.clone();
    let collecting = std::thread::spawn(move || {
        let mut chunk = [0u8; 1024];
        while let Ok(n) = stderr.read(&mut chunk) {
            if n == 0 {
                break;
            }
            heard
                .lock()
                .unwrap()
                .push_str(&String::from_utf8_lossy(&chunk[..n]));
        }
    });
    (child, said, collecting)
}

/// One `GET` over plain HTTP/1.1: the status and the body.
fn get(port: u16, path: &str) -> std::io::Result<(u16, String)> {
    let mut stream = TcpStream::connect(("127.0.0.1", port))?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    )?;
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    let status = response
        .split(' ')
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);
    let body = response
        .split_once("\r\n\r\n")
        .map(|(_, body)| body.to_string())
        .unwrap_or_default();
    Ok((status, body))
}

fn wait_for_exit(child: &mut Child, within: Duration) -> std::process::ExitStatus {
    let deadline = Instant::now() + within;
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("the server did not exit within {within:?}");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_server_starts_answers_keeps_its_data_directory_to_itself_and_stops_sealed() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("data");
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let public_url = format!("http://127.0.0.1:{port}");
    ok(
        &data,
        &["init", "--tenant", "Acme", "--public-url", &public_url],
    );
    // `init` writes the default listener; this one listens where the URL
    // says.
    let path = config::path_in(&data);
    let mut written = Config::load(&path).unwrap();
    written.listen.http = format!("127.0.0.1:{port}");
    std::fs::write(&path, serde_json::to_string_pretty(&written).unwrap()).unwrap();
    let lockbox = id(&data, &["lockboxes", "create", "Deploy"]);

    let (mut server, said, collecting) = start_server(&data);
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Ok((200, body)) = get(port, "/v1/health") {
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&body).unwrap(),
                serde_json::json!({ "ok": true })
            );
            break;
        }
        if let Some(status) = server.try_wait().unwrap() {
            panic!("the server exited with {status}: {}", said.lock().unwrap());
        }
        assert!(
            Instant::now() < deadline,
            "no health answer: {}",
            said.lock().unwrap()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(
        said.lock().unwrap().contains(&format!(
            "rowel serve --server: listening on http://127.0.0.1:{port}"
        )),
        "{}",
        said.lock().unwrap()
    );

    // The agent's challenge, unauthenticated.
    let (status, body) = get(port, "/v1/agent/challenge").unwrap();
    assert_eq!(status, 200, "{body}");
    let challenge: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        challenge["nonce"].as_str().is_some_and(|n| !n.is_empty()),
        "{body}"
    );
    assert!(
        challenge["audience"]
            .as_str()
            .unwrap()
            .starts_with("rowel:"),
        "{body}"
    );
    // A member call without a bearer.
    assert_eq!(get(port, "/v1/members/me").unwrap().0, 401);

    // A second server on the same data directory is refused while the first
    // runs.
    let second = Command::new(BIN)
        .args(["serve", "--server", "--data-dir"])
        .arg(&data)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", temp.path())
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let refused = String::from_utf8_lossy(&second.stderr);
    assert_eq!(second.status.code(), Some(1), "{refused}");
    assert!(refused.contains("another broker"), "{refused}");

    // A freeze from here would skip the running server's vault lock, so it
    // is refused and pointed at the admin API.
    let (code, _, said_freeze) = admin(&data, &["lockboxes", "freeze", &lockbox]);
    assert_eq!(code, 1, "{said_freeze}");
    assert!(said_freeze.contains("admin API"), "{said_freeze}");

    #[cfg(unix)]
    {
        let signalled = Command::new("kill")
            .args(["-TERM", &server.id().to_string()])
            .status()
            .unwrap();
        assert!(signalled.success());
        let status = wait_for_exit(&mut server, Duration::from_secs(20));
        collecting.join().unwrap();
        assert_eq!(status.code(), Some(0), "{}", said.lock().unwrap());
        assert!(
            said.lock()
                .unwrap()
                .contains("rowel serve --server: sealed, exiting"),
            "{}",
            said.lock().unwrap()
        );
    }
    #[cfg(not(unix))]
    {
        server.kill().unwrap();
        wait_for_exit(&mut server, Duration::from_secs(20));
        collecting.join().unwrap();
    }
    // With the server stopped, the offline freeze is the admin's to make.
    ok(&data, &["lockboxes", "freeze", &lockbox]);
}
