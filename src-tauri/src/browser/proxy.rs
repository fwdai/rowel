//! The native messaging host the browser launches: this same binary, relaying
//! its stdio to the running app's socket frame for frame.
//!
//! A native messaging manifest names an executable and nothing else — no
//! arguments — so there is no flag to launch under. What tells this launch
//! apart is what the browser puts on the command line: Chrome and every
//! Chromium passes the extension's origin, Firefox the extension's id. That
//! is also how KeePassXC's AppImage tells its proxy launches from its own.

use std::io;

use interprocess::local_socket::{prelude::*, Stream};

use super::{frame, root_dir, socket_name};

/// Whether these arguments are a browser's, launching its native host.
pub fn launched_by_browser(mut args: impl Iterator<Item = String>) -> bool {
    args.any(|arg| arg.starts_with("chrome-extension://") || arg.ends_with("@keepassxc.org"))
}

/// Relay until either side closes. The exit code is the process's: zero for
/// a browser that closed the port or an app that went away, one for an app
/// that was not there to begin with — the extension shows that as its
/// "not connected" state and retries when the user clicks it.
pub fn run() -> i32 {
    let Some(root) = root_dir() else {
        eprintln!("rowel: no data directory for this user");
        return 1;
    };
    let stream = socket_name(&root).and_then(Stream::connect);
    let stream = match stream {
        Ok(stream) => stream,
        Err(e) => {
            eprintln!("rowel: the app is not running ({e})");
            return 1;
        }
    };
    let (mut from_app, mut to_app) = stream.split();

    // The browser's side. Its port closing ends the process outright: the
    // read below would otherwise wait on an app that has nothing to say.
    std::thread::spawn(move || {
        let mut stdin = io::stdin().lock();
        while let Ok(Some(body)) = frame::read(&mut stdin) {
            if frame::write(&mut to_app, &body).is_err() {
                break;
            }
        }
        std::process::exit(0);
    });

    let mut stdout = io::stdout().lock();
    loop {
        match frame::read(&mut from_app) {
            Ok(Some(body)) => {
                if frame::write(&mut stdout, &body).is_err() {
                    return 0;
                }
            }
            Ok(None) => return 0,
            Err(_) => return 1,
        }
    }
}
