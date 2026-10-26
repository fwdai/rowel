//! The `rowel` symlink on macOS and Linux: pure functions over the
//! directories handed in, so the tests run them over a temporary tree.

use std::io;
use std::path::{Path, PathBuf};

use super::{CliStatus, BINARY};

/// The name the tool goes by on the PATH. The app's own binary is `rowel`
/// too, but it is never on the PATH: the link points at the sidecar.
pub const LINK: &str = "rowel";

/// The status over what was found: `bundled`, the directories a link may be
/// in, and the directories on the PATH a new terminal gets.
pub fn status_in(bundled: Option<&Path>, dirs: &[PathBuf], path: &[PathBuf]) -> CliStatus {
    let installed = bundled.and_then(|bundled| installed_in(dirs, bundled));
    let install_dir = installed
        .as_deref()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .or_else(|| install_dir(dirs));
    let on_path = install_dir
        .as_deref()
        .is_some_and(|dir| path.iter().any(|entry| same_file(entry, dir)));
    CliStatus {
        bundled: bundled.map(|p| p.to_string_lossy().into_owned()),
        installed: installed.map(|p| p.to_string_lossy().into_owned()),
        install_dir: install_dir.map(|p| p.to_string_lossy().into_owned()),
        on_path,
    }
}

/// The directories a `rowel` link may live in, most preferred first: the
/// system's `/usr/local/bin`, which is on every shell's PATH, then the user's
/// own `~/.local/bin`, which the XDG base directory spec puts there and which
/// needs no privilege to write — but which is on macOS's PATH only when the
/// user's shell profile puts it there.
pub fn link_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![PathBuf::from("/usr/local/bin")];
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".local").join("bin"));
    }
    dirs
}

/// The directories on the PATH a terminal opened now starts with. On macOS
/// that is `/etc/paths` and `/etc/paths.d`, which every login shell reads
/// through `path_helper` — the app's own PATH is launchd's, which has neither
/// bin directory. On Linux the app inherits the session's PATH, which is the
/// one the user's terminals get.
#[cfg(target_os = "macos")]
pub fn default_path() -> Vec<PathBuf> {
    let mut files = vec![PathBuf::from("/etc/paths")];
    if let Ok(entries) = std::fs::read_dir("/etc/paths.d") {
        files.extend(entries.flatten().map(|entry| entry.path()));
    }
    files
        .iter()
        .filter_map(|file| std::fs::read_to_string(file).ok())
        .flat_map(|text| paths_in(&text))
        .collect()
}

#[cfg(not(target_os = "macos"))]
pub fn default_path() -> Vec<PathBuf> {
    std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default()
}

/// The directories a `path_helper` file lists, one a line.
#[cfg(any(target_os = "macos", test))]
fn paths_in(text: &str) -> Vec<PathBuf> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .collect()
}

/// The `rowel` link to `bundled` among `dirs`, if one of them holds it. A
/// `rowel` that is something else — another tool of that name, a link to an
/// older install elsewhere — is not ours and is not reported as installed.
pub fn installed_in(dirs: &[PathBuf], bundled: &Path) -> Option<PathBuf> {
    dirs.iter()
        .map(|dir| dir.join(LINK))
        .find(|link| std::fs::read_link(link).is_ok_and(|target| same_file(&target, bundled)))
}

/// Where a link would go: the first of `dirs` that exists and can be written
/// to, else the last of them, which is made on demand. `None` with no
/// directories to choose from.
pub fn install_dir(dirs: &[PathBuf]) -> Option<PathBuf> {
    dirs.iter()
        .find(|dir| writable(dir))
        .or(dirs.last())
        .cloned()
}

/// Make `dir/rowel` a link to `bundled`, in the directory [`install_dir`]
/// picks among `dirs`, making it (owner-writable, world-readable, like a bin
/// directory) if it is not there. A `rowel` already there is replaced only
/// if it is ours — a link to a `rowel-cli`, from an earlier install of a
/// since moved or updated app; a file, or a link to anything else, is
/// refused by its path, since it is not ours to overwrite.
pub fn install_in(dirs: &[PathBuf], bundled: &Path) -> io::Result<PathBuf> {
    let dir = install_dir(dirs).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "there is no home directory to link the command line tool into",
        )
    })?;
    make_bin_dir(&dir)?;
    let link = dir.join(LINK);
    match std::fs::symlink_metadata(&link) {
        Ok(meta) if meta.file_type().is_symlink() && is_ours(&link) => std::fs::remove_file(&link)?,
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!(
                    "{} exists and is not a link to a Rowel command line tool; move it aside first",
                    link.display()
                ),
            ))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    std::os::unix::fs::symlink(bundled, &link)?;
    Ok(link)
}

/// Remove every `rowel` link among `dirs` that points at `bundled`. Any
/// other `rowel` is not ours and stays.
pub fn uninstall_in(dirs: &[PathBuf], bundled: &Path) -> io::Result<()> {
    for link in dirs.iter().map(|dir| dir.join(LINK)) {
        if std::fs::read_link(&link).is_ok_and(|target| same_file(&target, bundled)) {
            std::fs::remove_file(&link)?;
        }
    }
    Ok(())
}

/// Whether the link at `link` is one of ours: it points at a `rowel-cli`,
/// wherever that one is or was.
fn is_ours(link: &Path) -> bool {
    std::fs::read_link(link)
        .ok()
        .and_then(|target| target.file_name().map(|name| name == BINARY))
        .unwrap_or(false)
}

fn same_file(a: &Path, b: &Path) -> bool {
    a == b || matches!((a.canonicalize(), b.canonicalize()), (Ok(a), Ok(b)) if a == b)
}

fn writable(dir: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(path) = std::ffi::CString::new(dir.as_os_str().as_bytes()) else {
        return false;
    };
    // `access(2)` answers for the real user, which is who the link is for.
    dir.is_dir() && unsafe { libc::access(path.as_ptr(), libc::W_OK) } == 0
}

fn make_bin_dir(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o755)
        .create(dir)
}

#[cfg(test)]
mod tests {
    use super::super::bundled_beside;
    use super::*;

    fn tree() -> (tempfile::TempDir, PathBuf, Vec<PathBuf>) {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("Rowel.app/Contents/MacOS");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::write(app.join("rowel"), "app").unwrap();
        let dirs = vec![
            dir.path().join("usr/local/bin"),
            dir.path().join("home/.local/bin"),
        ];
        (dir, app, dirs)
    }

    // A dev build with no sidecar built has nothing to offer; one that ran
    // `cargo build -p rowel-cli` finds it beside itself.
    #[test]
    fn the_tool_is_the_binary_beside_the_executable() {
        let (_dir, app, dirs) = tree();
        let exe = app.join("rowel");
        assert_eq!(bundled_beside(&exe), None);
        assert_eq!(
            status_in(None, &dirs, &[]),
            CliStatus {
                bundled: None,
                installed: None,
                install_dir: Some(dirs[1].to_string_lossy().into_owned()),
                on_path: false,
            },
            "nowhere exists yet, so the user's own directory is where a link would go"
        );

        std::fs::write(app.join(BINARY), "cli").unwrap();
        assert_eq!(bundled_beside(&exe), Some(app.join(BINARY)));
    }

    // The system directory is preferred when it is there and writable; the
    // user's own is made when it is not. A link already there is replaced,
    // a file of the same name is refused.
    #[test]
    fn install_links_into_the_first_writable_directory_and_replaces_only_a_link() {
        let (_dir, app, dirs) = tree();
        let bundled = app.join(BINARY);
        std::fs::write(&bundled, "cli").unwrap();

        let link = install_in(&dirs, &bundled).unwrap();
        assert_eq!(
            link,
            dirs[1].join(LINK),
            "no system directory: the user's own, made"
        );
        assert_eq!(std::fs::read_link(&link).unwrap(), bundled);
        assert!(dirs[1].is_dir());
        let status = status_in(Some(&bundled), &dirs, &dirs[..1]);
        assert_eq!(status.installed.as_deref(), Some(link.to_str().unwrap()));
        assert_eq!(
            status.install_dir.as_deref(),
            Some(dirs[1].to_str().unwrap())
        );
        assert!(
            !status.on_path,
            "the user's own directory, which this PATH lacks"
        );
        assert!(status_in(Some(&bundled), &dirs, &dirs).on_path);

        // The system directory appears: it is preferred from now on, and the
        // link there replaces an older one of ours, pointing at the tool of
        // an app that has since moved.
        std::fs::create_dir_all(&dirs[0]).unwrap();
        std::os::unix::fs::symlink(app.join("old").join(BINARY), dirs[0].join(LINK)).unwrap();
        assert_eq!(installed_in(&dirs, &bundled), Some(dirs[1].join(LINK)));
        let link = install_in(&dirs, &bundled).unwrap();
        assert_eq!(link, dirs[0].join(LINK));
        assert_eq!(std::fs::read_link(&link).unwrap(), bundled);
        assert_eq!(installed_in(&dirs, &bundled), Some(link));

        // A link under the name to anything else — someone's own `rowel` —
        // is not ours to replace, and neither is a file.
        std::fs::remove_file(dirs[0].join(LINK)).unwrap();
        std::os::unix::fs::symlink(app.join("someone-elses-rowel"), dirs[0].join(LINK)).unwrap();
        let err = install_in(&dirs, &bundled).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(
            std::fs::read_link(dirs[0].join(LINK)).unwrap(),
            app.join("someone-elses-rowel"),
            "left as it was"
        );
        std::fs::remove_file(dirs[0].join(LINK)).unwrap();
        std::fs::write(dirs[0].join(LINK), "someone else's").unwrap();
        let err = install_in(&dirs, &bundled).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert!(err
            .to_string()
            .contains(dirs[0].join(LINK).to_str().unwrap()));
        assert_eq!(
            installed_in(&dirs, &bundled),
            Some(dirs[1].join(LINK)),
            "the user's own link still counts"
        );
    }

    // A system directory the user cannot write to is passed over for their
    // own, as it is for a user who is not an administrator.
    #[test]
    fn a_read_only_system_directory_is_passed_over() {
        use std::os::unix::fs::PermissionsExt;
        let (_dir, app, dirs) = tree();
        let bundled = app.join(BINARY);
        std::fs::write(&bundled, "cli").unwrap();
        std::fs::create_dir_all(&dirs[0]).unwrap();
        std::fs::set_permissions(&dirs[0], std::fs::Permissions::from_mode(0o555)).unwrap();
        // Root writes anywhere, and then this says nothing.
        if writable(&dirs[0]) {
            return;
        }

        assert_eq!(install_dir(&dirs), Some(dirs[1].clone()));
        let link = install_in(&dirs, &bundled).unwrap();
        assert_eq!(link, dirs[1].join(LINK));
        std::fs::set_permissions(&dirs[0], std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    // Remove takes away the links to this tool and nothing else of the name.
    #[test]
    fn uninstall_removes_only_links_to_the_tool() {
        let (_dir, app, dirs) = tree();
        let bundled = app.join(BINARY);
        std::fs::write(&bundled, "cli").unwrap();
        let link = install_in(&dirs, &bundled).unwrap();
        std::fs::create_dir_all(&dirs[0]).unwrap();
        std::os::unix::fs::symlink(app.join("someone-elses-rowel"), dirs[0].join(LINK)).unwrap();

        uninstall_in(&dirs, &bundled).unwrap();
        assert!(std::fs::symlink_metadata(&link).is_err());
        assert_eq!(
            std::fs::read_link(dirs[0].join(LINK)).unwrap(),
            app.join("someone-elses-rowel")
        );
        assert_eq!(status_in(Some(&bundled), &dirs, &[]).installed, None);
        uninstall_in(&dirs, &bundled).unwrap();
    }

    #[test]
    fn a_path_helper_file_is_one_directory_a_line() {
        assert_eq!(
            paths_in("/usr/local/bin\n/usr/bin\n\n  /opt/homebrew/bin  \n"),
            ["/usr/local/bin", "/usr/bin", "/opt/homebrew/bin"].map(PathBuf::from)
        );
    }
}
