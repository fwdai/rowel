//! Documents the OS asks the app to open: a double-clicked `.rowel` backup or
//! `.swftx` legacy vault, the two associations `tauri.conf.json` declares.
//!
//! The request arrives three ways. macOS delivers it as `RunEvent::Opened`,
//! whether Finder launched the app for it or the app was already running.
//! Windows and Linux put the path on the command line — of this process at
//! launch, or of the second instance the single-instance plugin folds into this
//! one. All three end in [`accept`].
//!
//! The webview may not be listening yet; at launch it never is. So the path is
//! parked in `AppState::pending_open` as well as announced as `file:opened`.
//! The frontend collects the parked one once it has subscribed
//! (`commands::app::take_opened_file`) and follows the event from then on, so
//! the two arrivals look the same to it.

#[cfg(desktop)]
use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager};

#[cfg(desktop)]
use crate::grants::{PathGrants, Purpose};
use crate::state::AppState;

/// The extensions `bundle.fileAssociations` registers.
#[cfg(desktop)]
const EXTENSIONS: [&str; 2] = [
    crate::commands::vault::BACKUP_EXTENSION,
    crate::commands::vault::LEGACY_BACKUP_EXTENSION,
];

/// Park and announce the first document in `paths` that is ours. Anything
/// else is ignored: a flag on the command line, a directory, a file of some
/// other kind the OS was told to open with us anyway.
///
/// The document is granted as a backup (see `grants`) before the webview hears
/// of it: the user opened that file with the app, which is as much a choice as
/// picking it from the dialog, and the restore or merge it leads to reads only
/// granted paths.
#[cfg(desktop)]
pub fn accept(app: &AppHandle, paths: impl IntoIterator<Item = PathBuf>) {
    let Some(path) = paths.into_iter().find(|p| is_ours(p)) else {
        return;
    };
    app.state::<PathGrants>().grant(&path, Purpose::Backup);
    let path = path.to_string_lossy().into_owned();
    *app.state::<AppState>().pending_open.lock().unwrap() = Some(path.clone());
    crate::events::file_opened(app, &path);
    // A document opened while the app is up brings the window forward; at
    // launch the reveal choreography does that itself.
    crate::window::raise(app);
}

/// The parked path, handed over once. See the module docs.
pub fn take(app: &AppHandle) -> Option<String> {
    app.state::<AppState>().pending_open.lock().unwrap().take()
}

/// Command-line arguments as paths, resolved against `cwd` — the arguments of
/// a second instance are relative to *its* working directory, which the
/// single-instance plugin passes along.
#[cfg(desktop)]
pub fn from_args(args: impl IntoIterator<Item = String>, cwd: &Path) -> Vec<PathBuf> {
    args.into_iter()
        .map(|arg| {
            let path = PathBuf::from(arg);
            if path.is_absolute() {
                path
            } else {
                cwd.join(path)
            }
        })
        .collect()
}

/// The `file://` URLs among those macOS handed over, as paths. Anything on
/// another scheme is not a document.
#[cfg(target_os = "macos")]
pub fn from_urls(urls: &[tauri::Url]) -> Vec<PathBuf> {
    urls.iter()
        .filter_map(|url| url.to_file_path().ok())
        .collect()
}

#[cfg(desktop)]
fn is_ours(path: &Path) -> bool {
    has_our_extension(path) && path.is_file()
}

#[cfg(desktop)]
fn has_our_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| EXTENSIONS.iter().any(|ours| ext.eq_ignore_ascii_case(ours)))
}

#[cfg(all(test, desktop))]
mod tests {
    use super::*;

    #[test]
    fn recognises_both_associations_whatever_the_case() {
        for name in ["vault.rowel", "Backup.ROWEL", "old.swftx", "x.SwFtX"] {
            assert!(has_our_extension(Path::new(name)), "{name}");
        }
        for name in [
            "notes.txt",
            "rowel",
            ".rowel.bak",
            "archive.rowel.zip",
            "--flag",
        ] {
            assert!(!has_our_extension(Path::new(name)), "{name}");
        }
    }

    // A relative argument belongs to the directory the launching process was
    // in; an absolute one is left alone.
    #[test]
    fn resolves_relative_arguments_against_the_launch_directory() {
        let cwd = Path::new("/home/me/Downloads");
        let paths = from_args(
            ["backup.rowel".to_string(), "/tmp/other.rowel".to_string()],
            cwd,
        );
        assert_eq!(
            paths,
            [
                PathBuf::from("/home/me/Downloads/backup.rowel"),
                PathBuf::from("/tmp/other.rowel"),
            ]
        );
    }

    // Only a file that is actually there counts: a stray flag, or a directory
    // that happens to end in `.rowel`, is not a document to open.
    #[test]
    fn a_document_has_to_exist_as_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("vault.rowel");
        std::fs::write(&file, b"pack").unwrap();
        let folder = dir.path().join("folder.rowel");
        std::fs::create_dir(&folder).unwrap();

        assert!(is_ours(&file));
        assert!(!is_ours(&folder));
        assert!(!is_ours(&dir.path().join("missing.rowel")));
    }
}
