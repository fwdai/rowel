//! Where the broker listens, and where the extension's proxy finds it: the
//! data directory, found without Tauri, and the socket in it, named so that a
//! debug build and a release build — or a test — never share one.

use std::io;
use std::path::{Path, PathBuf};

use interprocess::local_socket::{prelude::*, Name};

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
