//! `rowel admin server` over the built binary and a temporary data
//! directory: a server set up from nothing to a lockbox with a holder, an
//! agent with a trust policy, a rule, and an empty audit mirror that exports
//! nothing and verifies. Unlike `tests/serve.rs` this runs everywhere: no
//! client key of the CLI's is involved, so the OS secure store is never
//! touched.

#![cfg(not(any(target_os = "android", target_os = "ios")))]

use std::path::Path;
use std::process::Command;

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
    let agents = ok(&data, &["agents", "list"]);
    assert!(
        agents.contains(&agent)
            && agents.contains(&policy)
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
