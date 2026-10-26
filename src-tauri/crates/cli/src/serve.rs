//! `rowel serve`: the headless broker, run from this binary. The command line
//! says where the data directory and the lockbox are and how to unlock —
//! a key file, a file descriptor a service manager hands in, or the master
//! password at the terminal, which is the one thing this module does that
//! `rowel-serve` will not: read a TTY. With `--server` it is a Rowel Server
//! instead (`rowel_server::run`), which unlocks nothing at the terminal: its
//! lockboxes open under the root key `server.json` names.

use std::io::{self, BufRead, IsTerminal};
use std::path::PathBuf;

use rowel_serve::{PasswordPrompt, ServeOptions, Unlock};
use rowel_server::run::RunOptions;
use zeroize::Zeroizing;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ServeArgs {
    pub data_dir: Option<PathBuf>,
    pub lockbox: Option<String>,
    pub config: Option<PathBuf>,
    pub key_file: Option<PathBuf>,
    pub key_fd: Option<u32>,
    pub ws: Option<String>,
    pub no_socket: bool,
    pub idle_lock_minutes: Option<u64>,
    pub ask_timeout_seconds: Option<u64>,
    /// Server mode: only `data_dir` and `config` go with it (`args`).
    pub server: bool,
    /// Register the extension's native messaging host for this data
    /// directory, or take it back (`native_host`), and serve nothing.
    pub install_host: bool,
    pub uninstall_host: bool,
}

pub fn run(args: ServeArgs) -> i32 {
    if args.install_host {
        return crate::native_host::install(args.data_dir);
    }
    if args.uninstall_host {
        return crate::native_host::uninstall(args.data_dir);
    }
    if args.server {
        let Some(data_dir) = args.data_dir else {
            eprintln!("rowel serve --server: --data-dir is required");
            return 2;
        };
        return rowel_server::run::run(RunOptions {
            data_dir,
            config: args.config,
        });
    }
    let unlock = match (args.key_file, args.key_fd) {
        (Some(path), _) => Some(Unlock::KeyFile(path)),
        (None, Some(fd)) => match read_fd(fd) {
            Ok(material) => Some(Unlock::Material(material)),
            Err(e) => {
                eprintln!("rowel serve: cannot read --key-fd {fd}: {e}");
                return 2;
            }
        },
        (None, None) => None,
    };
    rowel_serve::run(ServeOptions {
        data_dir: args.data_dir,
        lockbox: args.lockbox,
        config: args.config,
        unlock,
        prompt: Some(prompt()),
        ws: args.ws,
        no_socket: args.no_socket,
        idle_lock_ms: args.idle_lock_minutes.map(|m| m * 60 * 1000),
        ask_timeout_ms: args.ask_timeout_seconds.map(|s| s * 1000),
    })
}

/// Ask for the master password: without echo on a terminal, else one line
/// from stdin, for a wrapper that pipes it in.
pub fn prompt() -> PasswordPrompt {
    Box::new(|| read_password("Master password: "))
}

/// The master password from the terminal, or from stdin when that is not a
/// terminal. Wiped on drop.
pub fn read_password(label: &str) -> io::Result<Zeroizing<String>> {
    if io::stdin().is_terminal() {
        return rpassword::prompt_password(label).map(Zeroizing::new);
    }
    let mut line = Zeroizing::new(String::new());
    if io::stdin().lock().read_line(&mut line)? == 0 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "stdin is not a terminal and holds no password line",
        ));
    }
    while line.ends_with(['\n', '\r']) {
        line.pop();
    }
    Ok(line)
}

/// The key material on file descriptor `fd`, read once and whole: what a
/// wrapper hands in with `3< /path/to/key`, or a supervisor that passes
/// secrets as open descriptors (systemd does not; its `LoadCredential=` is
/// a file under `$CREDENTIALS_DIRECTORY`, for `--key-file`). Through
/// `/dev/fd`, which Linux and macOS both have, so no handle is adopted
/// blind.
#[cfg(unix)]
fn read_fd(fd: u32) -> io::Result<Zeroizing<Vec<u8>>> {
    use std::io::Read;
    let mut material = Zeroizing::new(Vec::new());
    std::fs::File::open(format!("/dev/fd/{fd}"))?.read_to_end(&mut material)?;
    if material.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "the descriptor holds no key material",
        ));
    }
    Ok(material)
}

#[cfg(not(unix))]
fn read_fd(_fd: u32) -> io::Result<Zeroizing<Vec<u8>>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "--key-fd is for Unix; use --key-file here",
    ))
}
