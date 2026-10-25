//! The command line tool shipped beside the app (`rowel-cli`, the
//! `src-tauri/crates/cli` crate): where it is, and the `rowel` link on the
//! PATH that Settings › General offers to make for it.
//!
//! A release bundle carries the binary as a Tauri sidecar, beside the app's
//! own executable (`MacOS/rowel-cli` in the `.app`, the install directory on
//! Windows and Linux); a dev build finds whatever `cargo build -p rowel-cli`
//! left in the same `target/debug`. Nothing here runs it — the tool speaks to
//! the app over the extension host's socket like any other client — so all
//! this module decides is paths: pure functions over directories handed in,
//! so the tests run them over a temporary tree.

use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;

/// The sidecar's file name, as Tauri strips the target triple off it.
#[cfg(windows)]
pub const BINARY: &str = "rowel-cli.exe";
#[cfg(not(windows))]
pub const BINARY: &str = "rowel-cli";

/// The name the tool goes by on the PATH. The app's own binary is `rowel`
/// too, but it is never on the PATH: the link points at the sidecar.
pub const LINK: &str = "rowel";

/// The tool as Settings shows it: the binary beside the app when there is
/// one, the link to it on the PATH when one is in place, and where a link
/// would go (or is).
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CliStatus {
    pub bundled: Option<String>,
    pub installed: Option<String>,
    pub install_dir: Option<String>,
}

/// The tool as this process finds it: beside its own executable, linked from
/// one of the user's bin directories.
pub fn status() -> CliStatus {
    let bundled = std::env::current_exe()
        .ok()
        .and_then(|exe| bundled_beside(&exe));
    let dirs = link_dirs();
    status_in(bundled.as_deref(), &dirs)
}

/// Put the `rowel` link in place and say how things stand after.
pub fn install() -> io::Result<CliStatus> {
    let exe = std::env::current_exe()?;
    let bundled = bundled_beside(&exe).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "the command line tool is not bundled with this build",
        )
    })?;
    let dirs = link_dirs();
    install_in(&dirs, &bundled)?;
    Ok(status_in(Some(&bundled), &dirs))
}

/// The sidecar beside the executable `exe`, when it is there.
pub fn bundled_beside(exe: &Path) -> Option<PathBuf> {
    let binary = exe.parent()?.join(BINARY);
    binary.is_file().then_some(binary)
}

/// [`status`] over what was found: `bundled`, and the directories a link may
/// be in.
pub fn status_in(bundled: Option<&Path>, dirs: &[PathBuf]) -> CliStatus {
    let installed = bundled.and_then(|bundled| installed_in(dirs, bundled));
    let install_dir = installed
        .as_deref()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .or_else(|| install_dir(dirs));
    CliStatus {
        bundled: bundled.map(|p| p.to_string_lossy().into_owned()),
        installed: installed.map(|p| p.to_string_lossy().into_owned()),
        install_dir: install_dir.map(|p| p.to_string_lossy().into_owned()),
    }
}

/// The directories a `rowel` link may live in, most preferred first: the
/// system's `/usr/local/bin`, which is on every shell's PATH, then the user's
/// own `~/.local/bin`, which the XDG base directory spec puts there and which
/// needs no privilege to write. None on Windows, where there is no link to
/// make: the user adds the bundled binary's folder to PATH.
pub fn link_dirs() -> Vec<PathBuf> {
    if cfg!(windows) {
        return Vec::new();
    }
    let mut dirs = vec![PathBuf::from("/usr/local/bin")];
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".local").join("bin"));
    }
    dirs
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
            io::ErrorKind::Unsupported,
            "there is no directory to link the command line tool into on this platform; add the bundled binary's folder to PATH",
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
    symlink(bundled, &link)?;
    Ok(link)
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

#[cfg(unix)]
fn writable(dir: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(path) = std::ffi::CString::new(dir.as_os_str().as_bytes()) else {
        return false;
    };
    // `access(2)` answers for the real user, which is who the link is for.
    dir.is_dir() && unsafe { libc::access(path.as_ptr(), libc::W_OK) } == 0
}

#[cfg(not(unix))]
fn writable(dir: &Path) -> bool {
    dir.is_dir()
}

#[cfg(unix)]
fn make_bin_dir(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o755)
        .create(dir)
}

#[cfg(not(unix))]
fn make_bin_dir(dir: &Path) -> io::Result<()> {
    std::fs::create_dir_all(dir)
}

#[cfg(unix)]
fn symlink(target: &Path, link: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

// Windows has symlinks, but making one takes a privilege most users do not
// hold, and `link_dirs` offers no directory there anyway: this is never
// reached, and says so rather than guessing at a privilege.
#[cfg(not(unix))]
fn symlink(_target: &Path, _link: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "add the bundled binary's folder to PATH",
    ))
}

#[cfg(all(test, unix))]
mod tests {
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
            status_in(None, &dirs),
            CliStatus {
                bundled: None,
                installed: None,
                install_dir: Some(dirs[1].to_string_lossy().into_owned()),
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
        let status = status_in(Some(&bundled), &dirs);
        assert_eq!(status.installed.as_deref(), Some(link.to_str().unwrap()));
        assert_eq!(
            status.install_dir.as_deref(),
            Some(dirs[1].to_str().unwrap())
        );

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
}
