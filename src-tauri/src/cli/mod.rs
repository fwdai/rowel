//! The command line tool shipped beside the app (`rowel-cli`, the
//! `src-tauri/crates/cli` crate): where it is, and the `rowel` command on the
//! PATH that Settings › General offers to make for it.
//!
//! A release bundle carries the binary as a Tauri sidecar, beside the app's
//! own executable (`MacOS/rowel-cli` in the `.app`, the install directory on
//! Windows and Linux); a dev build finds whatever `cargo build -p rowel-cli`
//! left in the same `target/debug`. Nothing here runs it — the tool speaks to
//! the app over the extension host's socket like any other client — so all
//! this module decides is paths and files.
//!
//! How `rowel` reaches the tool depends on the platform:
//!
//! - macOS, and Linux from a `.deb` or `.rpm`: a `rowel` symlink in a bin
//!   directory to the sidecar ([`links`]).
//! - Linux from an AppImage: the sidecar is inside the image's mount, which is
//!   gone when the app quits, so the app keeps a copy of it in its local data
//!   directory and the symlink points at that copy.
//! - Windows: the app's own executable is `rowel.exe` in the same folder as
//!   the sidecar, so that folder cannot go on the PATH — `rowel` would start
//!   the app. The app keeps a copy named `rowel.exe` in a folder of its own and
//!   puts that folder on the user's PATH ([`user_path`]).
//!
//! Both copies are refreshed at startup ([`refresh`]), so an updated app
//! never leaves an older tool behind on the PATH.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;

#[cfg(unix)]
mod links;
#[cfg(any(windows, test))]
mod path_list;
#[cfg(windows)]
mod user_path;

/// The sidecar's file name, as Tauri strips the target triple off it.
#[cfg(windows)]
pub const BINARY: &str = "rowel-cli.exe";
#[cfg(not(windows))]
pub const BINARY: &str = "rowel-cli";

/// The tool as Settings shows it: the binary agents can be pointed at (beside
/// the app, or the AppImage's kept copy), the `rowel` command on the PATH when
/// it is in place, where that command goes (or is), and whether that
/// directory is on the PATH a new terminal gets.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CliStatus {
    pub bundled: Option<String>,
    pub installed: Option<String>,
    pub install_dir: Option<String>,
    pub on_path: bool,
}

/// The tool as this process finds it. `local_data` is the app's local data
/// directory, where the copies live.
pub fn status(local_data: &Path) -> CliStatus {
    let bundled = bundled(local_data);
    platform_status(bundled.as_deref(), local_data)
}

/// Put the `rowel` command in place and say how things stand after.
pub fn install(local_data: &Path) -> io::Result<CliStatus> {
    let bundled = bundled(local_data).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "the command line tool is not bundled with this build",
        )
    })?;
    platform_install(&bundled, local_data)?;
    Ok(status(local_data))
}

/// Take the `rowel` command away again, when it is ours, and say how things
/// stand after.
pub fn uninstall(local_data: &Path) -> io::Result<CliStatus> {
    if let Some(bundled) = bundled(local_data) {
        platform_uninstall(&bundled, local_data)?;
    }
    Ok(status(local_data))
}

/// At startup: bring the copies up to date with the tool this app carries.
/// An AppImage's copy is made whether or not `rowel` was installed, since it
/// is the only path to the tool that outlives the mount; Windows's copy is
/// refreshed only once the user has installed it.
pub fn refresh(local_data: &Path) {
    let Some(beside) = std::env::current_exe()
        .ok()
        .and_then(|exe| bundled_beside(&exe))
    else {
        return;
    };
    let Some(copy) = stand_in(&beside, local_data).or_else(|| installed_copy(local_data)) else {
        return;
    };
    if let Err(e) = keep_copy(&beside, &copy) {
        log::warn!(
            "command line tool: {} could not be updated: {e}",
            copy.display()
        );
    }
}

/// The sidecar beside the executable `exe`, when it is there.
pub fn bundled_beside(exe: &Path) -> Option<PathBuf> {
    let binary = exe.parent()?.join(BINARY);
    binary.is_file().then_some(binary)
}

/// The folder the copies go in, inside the app's local data directory.
fn bin_dir(local_data: &Path) -> PathBuf {
    local_data.join("bin")
}

/// The binary agents are to run: the sidecar, or the copy that stands in for
/// it once that copy is there.
fn bundled(local_data: &Path) -> Option<PathBuf> {
    let beside = std::env::current_exe()
        .ok()
        .and_then(|exe| bundled_beside(&exe))?;
    match stand_in(&beside, local_data) {
        Some(copy) => copy.is_file().then_some(copy),
        None => Some(beside),
    }
}

/// Where the copy that stands in for the sidecar `beside` goes, when the
/// sidecar is not at a lasting path: in an AppImage.
#[cfg(not(windows))]
fn stand_in(beside: &Path, local_data: &Path) -> Option<PathBuf> {
    let appdir = std::env::var_os("APPDIR");
    in_appimage(beside, appdir.as_deref().map(Path::new)).then(|| bin_dir(local_data).join(BINARY))
}

#[cfg(windows)]
fn stand_in(_beside: &Path, _local_data: &Path) -> Option<PathBuf> {
    None
}

/// The `rowel` command as a copy, when the user has installed one.
#[cfg(windows)]
fn installed_copy(local_data: &Path) -> Option<PathBuf> {
    Some(command_copy(local_data)).filter(|copy| copy.is_file())
}

#[cfg(not(windows))]
fn installed_copy(_local_data: &Path) -> Option<PathBuf> {
    None
}

/// Windows's `rowel` command: `rowel.exe` in a folder of the app's own.
#[cfg(windows)]
fn command_copy(local_data: &Path) -> PathBuf {
    bin_dir(local_data).join("rowel.exe")
}

/// Whether `binary` runs from an AppImage's mount. The AppImage runtime
/// mounts the image and names the mount in `APPDIR`; `APPIMAGE` alone would
/// not do, since a program launched from an AppImage inherits both variables
/// and may well be installed for good.
#[cfg(any(not(windows), test))]
fn in_appimage(binary: &Path, appdir: Option<&Path>) -> bool {
    appdir.is_some_and(|dir| !dir.as_os_str().is_empty() && binary.starts_with(dir))
}

/// Make `dest` a copy of `source`, unless it already is one. The copy is
/// written beside `dest` and moved over it, so a `rowel` that is running
/// meanwhile, or a reader of the path, never sees half a binary.
fn keep_copy(source: &Path, dest: &Path) -> io::Result<()> {
    if same_contents(source, dest) {
        return Ok(());
    }
    if let Some(dir) = dest.parent() {
        fs::create_dir_all(dir)?;
    }
    let partial = dest.with_extension("partial");
    fs::copy(source, &partial)?;
    replace(&partial, dest)
}

fn same_contents(a: &Path, b: &Path) -> bool {
    let len = |path: &Path| fs::metadata(path).map(|meta| meta.len()).ok();
    len(a).is_some()
        && len(a) == len(b)
        && matches!((fs::read(a), fs::read(b)), (Ok(a), Ok(b)) if a == b)
}

#[cfg(not(windows))]
fn replace(new: &Path, dest: &Path) -> io::Result<()> {
    fs::rename(new, dest)
}

#[cfg(windows)]
fn replace(new: &Path, dest: &Path) -> io::Result<()> {
    retire(dest)?;
    fs::rename(new, dest)
}

// Windows refuses to overwrite or delete an executable that is running — an
// agent's `rowel mcp` — but lets it be renamed: the old one is moved aside,
// and deleted once nothing runs it, now or at the next retire.
#[cfg(windows)]
fn retire(path: &Path) -> io::Result<()> {
    let aside = path.with_extension("old");
    let _ = fs::remove_file(&aside);
    match fs::rename(path, &aside) {
        Ok(()) => {
            let _ = fs::remove_file(&aside);
            Ok(())
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(unix)]
fn platform_status(bundled: Option<&Path>, _local_data: &Path) -> CliStatus {
    links::status_in(bundled, &links::link_dirs(), &links::default_path())
}

#[cfg(unix)]
fn platform_install(bundled: &Path, _local_data: &Path) -> io::Result<()> {
    links::install_in(&links::link_dirs(), bundled).map(drop)
}

#[cfg(unix)]
fn platform_uninstall(bundled: &Path, _local_data: &Path) -> io::Result<()> {
    links::uninstall_in(&links::link_dirs(), bundled)
}

#[cfg(windows)]
fn platform_status(bundled: Option<&Path>, local_data: &Path) -> CliStatus {
    let dir = bin_dir(local_data);
    let on_path = user_path::read().is_ok_and(|path| path_list::has(&path, &dir));
    let copy = command_copy(local_data);
    let installed = (on_path && copy.is_file()).then_some(copy);
    CliStatus {
        bundled: bundled.map(|p| p.to_string_lossy().into_owned()),
        installed: installed.map(|p| p.to_string_lossy().into_owned()),
        install_dir: Some(dir.to_string_lossy().into_owned()),
        on_path,
    }
}

#[cfg(windows)]
fn platform_install(bundled: &Path, local_data: &Path) -> io::Result<()> {
    keep_copy(bundled, &command_copy(local_data))?;
    user_path::add(&bin_dir(local_data))
}

#[cfg(windows)]
fn platform_uninstall(_bundled: &Path, local_data: &Path) -> io::Result<()> {
    user_path::remove(&bin_dir(local_data))?;
    retire(&command_copy(local_data))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Only a binary under the mount `APPDIR` names is in the AppImage: an
    // installed app started from an AppImage's terminal inherits the
    // variable, and an empty one names nothing.
    #[test]
    fn an_appimage_is_the_mount_appdir_names() {
        let binary = Path::new("/tmp/.mount_RowelAb12/usr/bin/rowel-cli");
        assert!(in_appimage(
            binary,
            Some(Path::new("/tmp/.mount_RowelAb12"))
        ));
        assert!(!in_appimage(binary, None));
        assert!(!in_appimage(binary, Some(Path::new(""))));
        assert!(!in_appimage(
            Path::new("/usr/bin/rowel-cli"),
            Some(Path::new("/tmp/.mount_RowelAb12"))
        ));
    }

    // The copy is made when missing, left alone while it matches, and
    // replaced when the app carries another build of the tool.
    #[test]
    fn a_copy_follows_its_source() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("mount").join(BINARY);
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(&source, "1.1.0").unwrap();
        let copy = bin_dir(&dir.path().join("data")).join(BINARY);

        keep_copy(&source, &copy).unwrap();
        assert_eq!(fs::read_to_string(&copy).unwrap(), "1.1.0");
        let made = fs::metadata(&copy).unwrap().modified().unwrap();

        keep_copy(&source, &copy).unwrap();
        assert_eq!(
            fs::metadata(&copy).unwrap().modified().unwrap(),
            made,
            "not rewritten"
        );

        fs::write(&source, "1.2.0").unwrap();
        keep_copy(&source, &copy).unwrap();
        assert_eq!(fs::read_to_string(&copy).unwrap(), "1.2.0");
        assert!(!copy.with_extension("partial").exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_copy_keeps_the_binary_executable() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join(BINARY);
        fs::write(&source, "cli").unwrap();
        fs::set_permissions(&source, fs::Permissions::from_mode(0o755)).unwrap();
        let copy = bin_dir(dir.path()).join(BINARY);
        keep_copy(&source, &copy).unwrap();
        assert_eq!(
            fs::metadata(&copy).unwrap().permissions().mode() & 0o777,
            0o755
        );
    }
}
