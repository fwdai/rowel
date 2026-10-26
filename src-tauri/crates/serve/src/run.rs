//! The process: find the data directory and the lockbox, refuse plainly
//! when its agent access is off, unlock, listen, serve until SIGTERM or
//! SIGINT, seal, exit. The password itself is the caller's to obtain —
//! `rowel serve` prompts on its terminal and hands it in as data — so this
//! crate never touches a TTY.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use zeroize::Zeroizing;

use rowel_broker::{server, socket};
use rowel_core::workspace::Registry;

use crate::config::{self, Config};
use crate::host::{HeadlessHost, Overrides};
use crate::unlock::{self, Unlock, UnlockError};
use crate::{control, listen};

/// Asks the person at the terminal for the master password: what the CLI
/// supplies, and what is asked again after an idle lock in password mode.
/// `Sync` because the second ask runs on a thread of its own, so a stop
/// signal is still heard while it waits for the terminal.
pub type PasswordPrompt = Box<dyn Fn() -> io::Result<Zeroizing<String>> + Send + Sync>;

pub struct ServeOptions {
    /// The data directory; the app's when `None`, so the extension on this
    /// box finds the socket where it looks.
    pub data_dir: Option<PathBuf>,
    /// The lockbox to serve; the registry's active one when `None`.
    pub lockbox: Option<String>,
    /// The config file; `<data dir>/serve.json` when `None`.
    pub config: Option<PathBuf>,
    /// How to unlock; `None` asks `prompt`.
    pub unlock: Option<Unlock>,
    pub prompt: Option<PasswordPrompt>,
    /// The WebSocket address, over the config's.
    pub ws: Option<String>,
    pub no_socket: bool,
    pub idle_lock_ms: Option<u64>,
    pub ask_timeout_ms: Option<u64>,
}

/// Run the broker until it is told to stop. The exit code: 0 after a clean
/// stop, 1 when it could not start, 2 for a usage mistake.
pub fn run(options: ServeOptions) -> i32 {
    match serve(options) {
        Ok(()) => 0,
        Err(Exit::Usage(message)) => {
            eprintln!("rowel serve: {message}");
            2
        }
        Err(Exit::Failed(message)) => {
            eprintln!("rowel serve: {message}");
            1
        }
    }
}

enum Exit {
    Usage(String),
    Failed(String),
}

fn failed(e: impl std::fmt::Display) -> Exit {
    Exit::Failed(e.to_string())
}

fn serve(options: ServeOptions) -> Result<(), Exit> {
    let root = options
        .data_dir
        .clone()
        .or_else(socket::root_dir)
        .ok_or_else(|| Exit::Failed("no data directory for this user; pass --data-dir".into()))?;
    let registry = Registry::load(&root);
    let lockbox = options
        .lockbox
        .clone()
        .unwrap_or_else(|| registry.active.clone());
    let workspace = registry
        .workspaces
        .iter()
        .find(|w| w.id == lockbox)
        .ok_or_else(|| {
            Exit::Failed(format!(
                "no lockbox {lockbox} in {}",
                root.join(rowel_core::workspace::REGISTRY_FILE).display()
            ))
        })?;
    if !workspace.agents {
        return Err(Exit::Failed(format!(
            "agents are off for lockbox {lockbox}; turn them on with: rowel admin agents on --lockbox {lockbox}{}",
            options
                .data_dir
                .as_ref()
                .map(|dir| format!(" --data-dir {}", dir.display()))
                .unwrap_or_default()
        )));
    }
    let config_path = options
        .config
        .clone()
        .unwrap_or_else(|| config::path_in(&root));
    let config = Config::load(&config_path).map_err(failed)?;

    let idle_lock_ms = options.idle_lock_ms.or(config.idle_lock_ms);
    if idle_lock_ms.is_some() && matches!(options.unlock, Some(Unlock::Material(_))) {
        return Err(Exit::Usage(
            "an idle lock cannot be lifted in --key-fd mode: the material was read once and is gone after the first seal; use --key-file, or leave the idle lock off".into(),
        ));
    }
    let key_file = match &options.unlock {
        Some(Unlock::KeyFile(path)) => Some(path.clone()),
        _ => None,
    };
    let password_mode = matches!(options.unlock, None | Some(Unlock::Password(_)));

    // One broker per data directory, the app or this one, settled before the
    // password is asked for and before any socket is bound or replaced.
    let _lock = server::Lock::take(&root).map_err(|e| match e {
        server::LockError::Held => Exit::Failed(format!(
            "{e}; quit it, or point rowel serve at another --data-dir"
        )),
        other => failed(other),
    })?;

    let host = HeadlessHost::new(
        &root,
        &lockbox,
        &config_path,
        Overrides {
            ask_timeout_ms: options.ask_timeout_ms,
            idle_lock_ms: options.idle_lock_ms,
        },
        key_file,
        None,
    )
    .map_err(failed)?;

    let prompt = options.prompt;
    let opened = match &options.unlock {
        Some(unlock) => unlock::open(&root, &lockbox, unlock).map_err(failed)?,
        None => {
            let prompt = prompt.as_ref().ok_or_else(|| {
                Exit::Usage(
                    "no way to unlock: pass --key-file or --key-fd, or run from a terminal".into(),
                )
            })?;
            prompt_and_open(&root, &lockbox, prompt)?
        }
    };
    host.unlock(opened);
    eprintln!("rowel serve: lockbox {lockbox} unlocked");

    // The data directory's socket, unless told otherwise.
    if config.listen.socket && !options.no_socket {
        let listener = server::bind(&root).map_err(failed)?;
        let served = host.clone();
        thread::spawn(move || listen::serve_socket(listener, served));
        eprintln!("rowel serve: listening on the socket in {}", root.display());
    }

    // The control socket, for `rowel approve` and `rowel admin`; the lock
    // above is what makes replacing a file there safe.
    let control = control::bind(&root).map_err(failed)?;
    let controlled = host.clone();
    thread::spawn(move || control::serve(control, controlled));

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(failed)?;

    // The WebSocket, from the command line over the config.
    if let Some(address) = options.ws.clone().or(config.listen.ws.clone()) {
        let tls = config
            .listen
            .tls
            .as_ref()
            .map(listen::tls_acceptor)
            .transpose()
            .map_err(failed)?;
        let listener = runtime
            .block_on(listen::bind_ws(&address))
            .map_err(failed)?;
        let bound = listener.local_addr().map_err(failed)?;
        eprintln!(
            "rowel serve: listening on {}://{bound}",
            if tls.is_some() { "wss" } else { "ws" }
        );
        runtime.spawn(listen::serve_ws(listener, tls, host.clone()));
    }

    // One thread ticking every second: expiry, the idle lock, the files
    // watched. An idle lock in password mode is reported to the main thread,
    // which owns the terminal.
    let (resealed, idle) = std::sync::mpsc::channel::<()>();
    let ticking = host.clone();
    thread::spawn(move || loop {
        thread::sleep(Duration::from_secs(1));
        if ticking.tick() && resealed.send(()).is_err() {
            break;
        }
    });

    runtime.block_on(until_stopped(
        &host,
        &root,
        &lockbox,
        password_mode,
        prompt,
        idle,
    ));
    host.seal();
    eprintln!("rowel serve: sealed, exiting");
    Ok(())
}

/// Wait for SIGTERM or SIGINT, prompting for the password again after an
/// idle lock when that is the mode. The signal future lives across the whole
/// wait, and the prompt runs on a thread of its own, so `systemctl stop` or
/// Ctrl-C at the password prompt still seals and exits instead of waiting
/// for a line that is not coming.
async fn until_stopped(
    host: &HeadlessHost,
    root: &Path,
    lockbox: &str,
    password_mode: bool,
    prompt: Option<PasswordPrompt>,
    idle: Receiver<()>,
) {
    let stop = stop_signal();
    tokio::pin!(stop);
    let mut prompt = prompt.filter(|_| password_mode).map(Arc::new);
    loop {
        tokio::select! {
            _ = &mut stop => return,
            sealed = idle_sealed(&idle) => {
                if !sealed {
                    // The ticker is gone, so nothing more will arrive here;
                    // only the signal can end the wait now.
                    stop.await;
                    return;
                }
                let Some(ask) = prompt.clone() else { continue };
                let (done, reopened) = tokio::sync::oneshot::channel();
                let (asked_root, asked_lockbox) = (root.to_path_buf(), lockbox.to_string());
                thread::spawn(move || {
                    let _ = done.send(prompt_and_open(&asked_root, &asked_lockbox, &ask));
                });
                tokio::select! {
                    _ = &mut stop => return,
                    reopened = reopened => match reopened {
                        Ok(Ok(opened)) => {
                            host.unlock(opened);
                            eprintln!("rowel serve: lockbox {lockbox} unlocked");
                        }
                        Ok(Err(Exit::Usage(message)) | Err(Exit::Failed(message))) => {
                            eprintln!("rowel serve: {message}; the lockbox stays sealed");
                            prompt = None;
                        }
                        Err(_) => prompt = None,
                    }
                }
            }
        }
    }
}

/// Whether the ticker reported an idle seal: `true` on one, `false` once the
/// ticker is gone. Polled rather than awaited, because the ticker is a std
/// thread.
async fn idle_sealed(idle: &Receiver<()>) -> bool {
    loop {
        match idle.try_recv() {
            Ok(()) => return true,
            Err(TryRecvError::Disconnected) => return false,
            Err(TryRecvError::Empty) => tokio::time::sleep(Duration::from_millis(250)).await,
        }
    }
}

/// Ask for the password and open with it, three tries.
fn prompt_and_open(
    root: &Path,
    lockbox: &str,
    prompt: &PasswordPrompt,
) -> Result<unlock::Opened, Exit> {
    for attempt in 1..=3 {
        let password = prompt().map_err(|e| Exit::Failed(format!("no password: {e}")))?;
        match unlock::open(root, lockbox, &Unlock::Password(password)) {
            Ok(opened) => return Ok(opened),
            Err(UnlockError::WrongKey) if attempt < 3 => eprintln!("rowel serve: wrong password"),
            Err(e) => return Err(failed(e)),
        }
    }
    Err(Exit::Failed("wrong password".into()))
}

#[cfg(unix)]
async fn stop_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut term = match signal(SignalKind::terminate()) {
        Ok(term) => term,
        Err(e) => {
            log::warn!("rowel serve: no SIGTERM handler: {e}");
            let _ = tokio::signal::ctrl_c().await;
            return;
        }
    };
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
}

#[cfg(not(unix))]
async fn stop_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
