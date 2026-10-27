//! The native messaging host a browser launches, relaying its stdio to the
//! broker's socket frame for frame: the app's own binary when the app's
//! manifest names it, or `rowel native-host` when `rowel serve
//! --install-host` wrote the manifest. One relay, whichever broker is at the
//! other end — both serve the same socket in their data directory.
//!
//! A native messaging manifest names an executable and nothing else — no
//! arguments — so there is no flag to launch under. What tells this launch
//! apart is what the browser puts on the command line: Chrome and every
//! Chromium passes the extension's origin (and, on Windows, its window),
//! Firefox the manifest's path and the extension's id. That is also how
//! KeePassXC's AppImage tells its proxy launches from its own.

use std::io::{self, Read, Write};
use std::path::PathBuf;

use interprocess::local_socket::{prelude::*, Stream};

use crate::{frame, manifest, socket};

/// Whether these arguments are a browser's, launching its native host.
pub fn launched_by_browser(mut args: impl Iterator<Item = String>) -> bool {
    args.any(|arg| arg.starts_with("chrome-extension://") || manifest::is_firefox_extension(&arg))
}

/// Whether `arg` is one a browser adds to the host it launches: Chrome's
/// caller origin and, on Windows, `--parent-window=<n>`; Firefox's manifest
/// path and the extension's id. The browser has checked the extension
/// against the manifest's list already, so the relay needs none of them; a
/// command line reading them can still tell them from a mistake.
pub fn is_browser_arg(arg: &str) -> bool {
    arg.starts_with("chrome-extension://")
        || arg.starts_with("--parent-window=")
        || manifest::is_firefox_extension(arg)
        || arg.ends_with(".json")
}

/// Relay stdio to the socket in `root` until either side closes. The exit
/// code is the process's: zero for a browser that closed the port or a
/// broker that went away, one for a broker that was not there to begin with
/// — the extension shows that as its "not connected" state and retries when
/// the user clicks it. `None` is a user with no data directory.
pub fn run(root: Option<PathBuf>) -> i32 {
    let Some(root) = root else {
        eprintln!("rowel: no data directory for this user");
        return 1;
    };
    let stream = socket::socket_name(&root).and_then(Stream::connect);
    let stream = match stream {
        Ok(stream) => stream,
        Err(e) => {
            eprintln!(
                "rowel: nothing is serving the extension at {} ({e})",
                root.display()
            );
            return 1;
        }
    };
    // The browser's port closing ends the process outright: the broker's
    // side would otherwise wait on a broker that has nothing to say.
    relay(stream, io::stdin(), io::stdout().lock(), || {
        std::process::exit(0)
    })
}

/// Relay frames from `browser` to `broker`, on a thread of their own, and
/// from `broker` to `to_browser` on this one; `browser_closed` runs when the
/// browser's side ends. The exit code as [`run`] gives it.
pub fn relay(
    broker: Stream,
    mut browser: impl Read + Send + 'static,
    mut to_browser: impl Write,
    browser_closed: impl FnOnce() + Send + 'static,
) -> i32 {
    let (mut from_broker, mut to_broker) = broker.split();
    std::thread::spawn(move || {
        while let Ok(Some(body)) = frame::read(&mut browser) {
            if frame::write(&mut to_broker, &body).is_err() {
                break;
            }
        }
        browser_closed();
    });
    loop {
        match frame::read(&mut from_broker) {
            Ok(Some(body)) => {
                if frame::write(&mut to_browser, &body).is_err() {
                    return 0;
                }
            }
            Ok(None) => return 0,
            Err(_) => return 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use interprocess::local_socket::ListenerOptions;

    #[test]
    fn a_browser_launch_is_told_by_what_it_puts_on_the_command_line() {
        fn launched(list: &[&str]) -> bool {
            launched_by_browser(list.iter().map(|s| s.to_string()))
        }
        assert!(launched(&[
            "chrome-extension://oboonakemofpalcgghocfoadofidjkkk/"
        ]));
        assert!(launched(&["chrome-extension://abc/", "--parent-window=0"]));
        assert!(launched(&[
            "/path/manifest.json",
            "keepassxc-browser@keepassxc.org"
        ]));
        assert!(launched(&["/path/manifest.json", "browser@rowel.app"]));
        assert!(!launched(&["/path/manifest.json", "other@example.org"]));
        assert!(!launched(&[]));
        assert!(!launched(&["/Users/me/backup.rowel"]));

        for arg in [
            "chrome-extension://dimghkhcdfaokfingegmgbnpnpcoeofj/",
            "--parent-window=4242",
            "/home/me/.mozilla/native-messaging-hosts/app.rowel.browser.json",
            "browser@rowel.app",
        ] {
            assert!(is_browser_arg(arg), "{arg}");
        }
        for arg in ["--data-dir", "/srv/rowel", "serve", "other@example.org"] {
            assert!(!is_browser_arg(arg), "{arg}");
        }
    }

    // Frames the browser writes reach the broker whole and in order, and the
    // broker's replies reach the browser the same way; the broker going away
    // ends the relay with zero, and the browser's end is said.
    #[test]
    fn the_relay_carries_frames_both_ways_until_the_broker_closes() {
        let dir = tempfile::tempdir().unwrap();
        let name = socket::socket_name(dir.path()).unwrap();
        let listener = ListenerOptions::new().name(name).create_sync().unwrap();
        let broker = std::thread::spawn(move || {
            let mut connection = listener.accept().unwrap();
            for _ in 0..2 {
                let body = frame::read(&mut connection).unwrap().unwrap();
                let mut reply = b"echo:".to_vec();
                reply.extend_from_slice(&body);
                frame::write(&mut connection, &reply).unwrap();
            }
        });

        let mut from_browser = Vec::new();
        frame::write(&mut from_browser, br#"{"action":"change-public-keys"}"#).unwrap();
        frame::write(&mut from_browser, br#"{"action":"get-databasehash"}"#).unwrap();
        let (closed, said) = std::sync::mpsc::channel();
        let stream = Stream::connect(socket::socket_name(dir.path()).unwrap()).unwrap();
        let mut to_browser = Vec::new();
        let code = relay(
            stream,
            io::Cursor::new(from_browser),
            &mut to_browser,
            move || closed.send(()).unwrap(),
        );
        broker.join().unwrap();

        assert_eq!(code, 0);
        let mut replies = io::Cursor::new(to_browser);
        assert_eq!(
            frame::read(&mut replies).unwrap().unwrap(),
            br#"echo:{"action":"change-public-keys"}"#
        );
        assert_eq!(
            frame::read(&mut replies).unwrap().unwrap(),
            br#"echo:{"action":"get-databasehash"}"#
        );
        assert_eq!(frame::read(&mut replies).unwrap(), None);
        said.recv_timeout(std::time::Duration::from_secs(5))
            .expect("the browser's end is said");
    }

    #[test]
    fn nothing_serving_the_data_directory_is_exit_one() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(run(Some(dir.path().to_path_buf())), 1);
        assert_eq!(run(None), 1);
    }
}
