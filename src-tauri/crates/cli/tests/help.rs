//! The built binary's exit status and streams for the usage: asked for, it
//! is the answer, on stdout with success; given after a mistake, it is the
//! error, on stderr with the usage status.

#![cfg(not(any(target_os = "android", target_os = "ios")))]

use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_rowel-cli");

#[test]
fn help_prints_the_usage_to_stdout_and_succeeds() {
    for flag in ["--help", "-h", "help"] {
        let output = Command::new(BIN).arg(flag).output().unwrap();
        assert_eq!(output.status.code(), Some(0), "{flag}");
        assert!(
            String::from_utf8_lossy(&output.stdout).starts_with("usage: rowel-cli <command>"),
            "{flag}"
        );
        assert!(output.stderr.is_empty(), "{flag}");
    }
}

#[test]
fn a_mistake_prints_the_usage_to_stderr_and_exits_2() {
    let output = Command::new(BIN).arg("serv").output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.starts_with("rowel-cli: unknown command 'serv'"));
    assert!(stderr.contains("usage: rowel-cli <command>"));
}
