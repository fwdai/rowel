//! The native messaging host end to end: the `rowel` binary, launched the way
//! a browser launches it, relays frames from its stdio to the app's socket
//! and back, and ends when the browser closes the port.
//!
//! Debug builds are what `cargo test` runs, so this exercises a console
//! subsystem binary on Windows; the release binary is a GUI subsystem one,
//! whose inherited stdio pipes are the same handles.

#![cfg(desktop)]

use std::process::{Command, Stdio};

use interprocess::local_socket::{prelude::*, ListenerOptions};
use rowel_lib::browser::{frame, socket_name};

fn proxy(root: &std::path::Path) -> std::process::Child {
    Command::new(env!("CARGO_BIN_EXE_rowel"))
        .arg("chrome-extension://oboonakemofpalcgghocfoadofidjkkk/")
        .env("ROWEL_DB_DIR", root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("the rowel binary launches")
}

#[test]
fn the_proxy_relays_frames_both_ways_and_ends_with_the_port() {
    let dir = tempfile::tempdir().unwrap();
    let listener = ListenerOptions::new()
        .name(socket_name(dir.path()).unwrap())
        .create_sync()
        .unwrap();

    let mut child = proxy(dir.path());
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();

    // Browser → proxy → app.
    frame::write(&mut stdin, br#"{"action":"change-public-keys"}"#).unwrap();
    let mut app = listener.accept().unwrap();
    assert_eq!(
        frame::read(&mut app).unwrap().as_deref(),
        Some(&br#"{"action":"change-public-keys"}"#[..])
    );

    // App → proxy → browser.
    frame::write(&mut app, br#"{"success":"true"}"#).unwrap();
    assert_eq!(
        frame::read(&mut stdout).unwrap().as_deref(),
        Some(&br#"{"success":"true"}"#[..])
    );

    // The browser closes the port: the proxy is done.
    drop(stdin);
    let status = child.wait().unwrap();
    assert!(status.success(), "proxy exited with {status}");
}

#[test]
fn without_the_app_the_proxy_gives_up() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = proxy(dir.path());
    let status = child.wait().unwrap();
    assert!(!status.success());
}
