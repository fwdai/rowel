//! Where the broker listens, and where the extension's proxy finds it: the
//! data directory, found without Tauri, and the socket in it, named so that a
//! debug build and a release build — or a test — never share one.

use std::io;
use std::path::{Path, PathBuf};

use interprocess::local_socket::{prelude::*, Listener, ListenerOptions, Name, Stream};

/// The bundle identifier, which is the app-data directory's name on every
/// desktop platform. The proxy has no `AppHandle` to ask, so it is spelled out
/// here; a test holds it to `tauri.conf.json`.
pub const IDENTIFIER: &str = "app.rowel.desktop";
pub const SOCKET_FILE: &str = "browser.sock";

/// The app's data directory, resolved the way `storage::root_dir` resolves it
/// but without Tauri: the debug override, then the platform's app-data
/// directory under the identifier, with a `dev` subdirectory in debug builds.
/// Its socket is the one Chrome and Firefox reach, through the proxy.
pub fn root_dir() -> Option<PathBuf> {
    if cfg!(debug_assertions) {
        if let Ok(dir) = std::env::var("ROWEL_DB_DIR") {
            return Some(PathBuf::from(dir));
        }
    }
    let dir = dirs::data_dir()?.join(IDENTIFIER);
    Some(rowel_core::layout::dev_subdir(dir))
}

/// The most a Unix socket's path may be on macOS: `sun_path` is 104 bytes,
/// the terminating NUL included.
pub const SUN_PATH_MAX: usize = 104;

/// The socket the app listens on in `root`: a file in it on Unix, and on
/// Windows a named pipe whose name carries a digest of `root`, so a debug
/// build and a release build — or a test — never share one.
pub fn socket_name(root: &Path) -> io::Result<Name<'static>> {
    #[cfg(unix)]
    {
        use interprocess::local_socket::GenericFilePath;
        root.join(SOCKET_FILE).to_fs_name::<GenericFilePath>()
    }
    #[cfg(windows)]
    {
        use interprocess::local_socket::GenericNamespaced;
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(root.to_string_lossy().as_bytes());
        format!("{IDENTIFIER}.browser.{}", hex::encode(&digest[..8]))
            .to_ns_name::<GenericNamespaced>()
    }
}

/// A listener at `name`, replacing a socket already there (the caller holds
/// the lock that makes one a dead process's). On Windows the pipe is created
/// under the owner-only descriptor the core gives files
/// (`owner_only::current_user_sddl`): owned by the current user, openable by
/// that user and SYSTEM alone, where the default would let any local user
/// open it for reading. On Unix the caller makes the socket file owner-only.
pub fn listen(name: Name<'_>) -> io::Result<Listener> {
    let options = ListenerOptions::new().name(name).try_overwrite(true);
    #[cfg(windows)]
    let options = {
        use interprocess::os::windows::{
            local_socket::ListenerOptionsExt, security_descriptor::SecurityDescriptor, ToWtf16,
        };
        let sddl = rowel_core::owner_only::current_user_sddl()?
            .to_wtf_16()
            .map_err(io::Error::other)?;
        options.security_descriptor(SecurityDescriptor::deserialize(&sddl)?)
    };
    options.create_sync()
}

/// A connection to the broker serving `root`: [`connect_to`] its socket.
pub fn connect(root: &Path) -> io::Result<Stream> {
    connect_to(socket_name(root)?)
}

/// A connection to the socket `name`, for a client that is about to send it
/// keys or a password: the relay the browser launches, the `rowel` CLI. On
/// Windows the pipe must be the current user's, or the connection is refused
/// before a byte crosses it: a pipe's name is public, and a pipe of that name
/// another user created first would otherwise be handed whatever the client
/// sends. [`listen`] creates it owned by the user. On Unix the socket is a
/// file in the data directory, and only whoever can write there can put
/// another in its place.
pub fn connect_to(name: Name<'_>) -> io::Result<Stream> {
    let stream = Stream::connect(name)?;
    #[cfg(windows)]
    {
        use std::os::windows::io::AsHandle;
        let Stream::NamedPipe(pipe) = &stream;
        if !rowel_core::owner_only::owned_by_current_user(pipe.as_handle())? {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "the pipe is another user's, not this user's broker",
            ));
        }
    }
    Ok(stream)
}
