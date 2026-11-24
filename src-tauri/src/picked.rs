//! A file picked on Android, made a path on disk as it is everywhere else, and
//! the plaintext files picking leaves there.
//!
//! Every reader of a chosen file, the grants (`grants`) and the webview's
//! checks by extension work on a path. The desktop dialog answers with one, and
//! the iOS dialog with a `file://` URL to a copy its picker has already made in
//! the app's own temp folder. Android's answers with a `content://` URI: a
//! document some provider serves, which `std::fs` cannot open and whose last
//! segment is an id rather than its name (`tauri-plugin-dialog` 2.8.4,
//! `DialogPlugin.kt`: `filePickerResult` resolves with the intent's URIs as
//! strings). So there the document is copied once, as iOS does, into
//! `picked/<unique id>/<its name>` under the app's cache directory, and the pick
//! goes on as that path ([`into_path`]). The directory is the internal one,
//! `Context.getCacheDir`, which no other app can read (tauri 2.12.5:
//! `PathResolver::app_cache_dir` asks `PathPlugin.getCacheDir`).
//!
//! The camera leaves files too. wry answers a file input with `capture` by
//! creating a file for the camera app to write the photo into (wry 0.57.0,
//! `RustWebChromeClient.kt`: `createImageFile`, `File.createTempFile("JPEG_" +
//! <yyyyMMdd_HHmmss> + "_", ".jpg", ...)` in
//! `getExternalFilesDir(DIRECTORY_PICTURES)`) and never removes it, nor the
//! empty one a cancelled capture leaves, on storage a USB connection reaches.
//!
//! Both are what the user picked or photographed, an export in the clear or a
//! card, and live no longer than the session they were made for: [`wipe`]
//! removes both kinds at startup and whenever a session opens or ends
//! (`session`). One rule rather than one per file, so a file made in a session
//! stays until that session ends. A pick answered after its session ended is
//! not copied, and a copy a session end overtook is removed
//! (`commands::tools::pick_file`, [`discard`]).
//!
//! The wipe runs off the thread that asks for it, and the folder it clears is
//! the one the next copy is made in, so a copy first settles any wipe still
//! owed ([`Owed`]): it is made after the wipe, never under it. A camera photo
//! has no such step of ours in front of it. wry makes the file, so a wipe not
//! yet run when the camera opens takes that photo's file and the capture has to
//! be taken again.

use std::path::{Path, PathBuf};
#[cfg(any(target_os = "android", test))]
use std::{
    fs::{self, File},
    io::{self, Read},
};

use tauri::AppHandle;
use tauri_plugin_dialog::FilePath;

use crate::error::{Error, Result};

/// The folder under the app's cache directory the copies are made in.
#[cfg(any(target_os = "android", test))]
const DIR: &str = "picked";

/// A wipe asked for and not yet done.
#[cfg(any(target_os = "android", test))]
struct Owed {
    asked: std::sync::atomic::AtomicBool,
    doing: std::sync::Mutex<()>,
}

#[cfg(any(target_os = "android", test))]
impl Owed {
    const fn new() -> Self {
        Self {
            asked: std::sync::atomic::AtomicBool::new(false),
            doing: std::sync::Mutex::new(()),
        }
    }

    fn ask(&self) {
        self.asked.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Run `wipe` if one is owed. One caller at a time, so whoever returns
    /// from here has no wipe owed or under way behind it.
    fn settle(&self, wipe: impl FnOnce()) {
        let _doing = self.doing.lock().unwrap_or_else(|e| e.into_inner());
        if self.asked.swap(false, std::sync::atomic::Ordering::SeqCst) {
            wipe();
        }
    }
}

#[cfg(target_os = "android")]
static OWED: Owed = Owed::new();

/// The dialog's answer as a path to read: on Android a document is copied in
/// first (see the module docs); anything else is the path the answer names,
/// a `file://` URL resolved to its path.
pub fn into_path(app: &AppHandle, answer: FilePath) -> Result<PathBuf> {
    match answer {
        #[cfg(target_os = "android")]
        FilePath::Url(uri) if uri.scheme() == "content" => copy_document(app, uri),
        answer => {
            #[cfg(not(target_os = "android"))]
            let _ = app;
            answer
                .into_path()
                .map_err(|_| Error::Unsupported("the picked file has no local path".into()))
        }
    }
}

/// Copy the document at `uri` into the app's cache, under the name its provider
/// gives it (`PathPlugin.getFileNameFromUri`; empty if the provider will not
/// say). Opened read-only through the content resolver (`tauri-plugin-fs`
/// 2.7.1, `src/android.rs`: `Fs::open` asks `FsPlugin.getFileDescriptor`).
/// Both are round trips to the provider, so this runs off the async runtime.
#[cfg(target_os = "android")]
fn copy_document(app: &AppHandle, uri: url::Url) -> Result<PathBuf> {
    use tauri::Manager;
    use tauri_plugin_fs::{FsExt, OpenOptions};

    settle(app);
    let name = app.path().file_name(uri.as_str()).unwrap_or_default();
    let cache = app
        .path()
        .app_cache_dir()
        .map_err(|e| Error::Other(e.to_string()))?;
    let mut options = OpenOptions::new();
    options.read(true);
    let source = app.fs().open(FilePath::Url(uri), options)?;
    copy_in(&cache, &name, source, crate::storage::MAX_BACKUP_BYTES)
}

/// Copy `source` into a folder of its own under `<cache>/picked`, as `name`,
/// and hand back where it went. `name` comes from another app, so only what
/// [`file_name`] keeps of it is used. Refused past `cap`, counted on the bytes
/// read: a provider may serve a pipe, which has no length to trust. On any
/// failure the folder goes, with whatever had been written into it.
#[cfg(any(target_os = "android", test))]
fn copy_in(cache: &Path, name: &str, source: impl Read, cap: u64) -> Result<PathBuf> {
    let name = file_name(name)
        .ok_or_else(|| Error::Unsupported("the picked file has no name to keep it under".into()))?;
    let root = cache.join(DIR);
    fs::create_dir_all(&root)?;
    // Removed when dropped, so every way out before `keep` takes it along.
    let dir = tempfile::tempdir_in(&root)?;
    let mut file = File::create_new(dir.path().join(name))?;
    let copied = io::copy(&mut source.take(cap.saturating_add(1)), &mut file)?;
    if copied > cap {
        return Err(Error::FileTooLarge);
    }
    Ok(dir.keep().join(name))
}

/// What a copy may be named out of the `name` a provider gave: its last path
/// component, so the copy cannot land outside its folder, and nothing when
/// that is empty or only dots (`.`, `..`), which name no file of their own.
#[cfg(any(target_os = "android", test))]
fn file_name(name: &str) -> Option<&str> {
    let last = name.rsplit(['/', '\\']).next()?;
    (!last.trim_start_matches('.').is_empty()).then_some(last)
}

/// Remove the copy at `path` if [`into_path`] made it; any other path is the
/// user's own file and stays.
pub fn discard(app: &AppHandle, path: &Path) {
    #[cfg(target_os = "android")]
    {
        use tauri::Manager;

        if let Ok(cache) = app.path().app_cache_dir() {
            discard_in(&cache, path);
        }
    }
    #[cfg(not(target_os = "android"))]
    let _ = (app, path);
}

/// Remove `path`'s folder if it is one [`copy_in`] made under `cache`.
#[cfg(any(target_os = "android", test))]
fn discard_in(cache: &Path, path: &Path) {
    let root = cache.join(DIR);
    if let Some(dir) = path.parent().filter(|dir| dir.parent() == Some(&*root)) {
        let _ = fs::remove_dir_all(dir);
    }
}

/// Remove every copy and every camera photo (see the module docs). Android
/// only, the one platform that leaves them. Only asks: the removal runs off the
/// calling thread, so this may be called under the session guard (`session`).
/// Best effort: what will not go now goes the next time.
pub fn wipe(app: &AppHandle) {
    #[cfg(target_os = "android")]
    {
        OWED.ask();
        let app = app.clone();
        crate::commands::detached(move || settle(&app));
    }
    #[cfg(not(target_os = "android"))]
    let _ = app;
}

/// Do the wipe if one is owed.
#[cfg(target_os = "android")]
fn settle(app: &AppHandle) {
    use tauri::Manager;

    OWED.settle(|| {
        // `PathPlugin.getPictureDir`: the folder wry has the camera write to.
        let dirs = app.path();
        wipe_in(
            dirs.app_cache_dir().ok().as_deref(),
            dirs.picture_dir().ok().as_deref(),
        );
    });
}

#[cfg(any(target_os = "android", test))]
fn wipe_in(cache: Option<&Path>, pictures: Option<&Path>) {
    if let Some(cache) = cache {
        let _ = fs::remove_dir_all(cache.join(DIR));
    }
    let Some(entries) = pictures.and_then(|dir| fs::read_dir(dir).ok()) else {
        return;
    };
    for entry in entries.flatten() {
        let file = entry.file_type().is_ok_and(|kind| kind.is_file());
        if file && entry.file_name().to_str().is_some_and(is_capture) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// Whether `name` is one wry gives a capture: its `JPEG_` prefix and `.jpg`
/// suffix around something. The middle is not checked: its timestamp is
/// formatted in the device's locale, whose digits need not be ASCII.
#[cfg(any(target_os = "android", test))]
fn is_capture(name: &str) -> bool {
    name.len() > "JPEG_.jpg".len() && name.starts_with("JPEG_") && name.ends_with(".jpg")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_keeps_only_its_last_component() {
        assert_eq!(file_name("export.csv"), Some("export.csv"));
        assert_eq!(file_name(".env.production"), Some(".env.production"));
        assert_eq!(file_name("../../databases/vault.db"), Some("vault.db"));
        assert_eq!(file_name("/data/data/app/x.rowel"), Some("x.rowel"));
        assert_eq!(file_name("..\\..\\x.rowel"), Some("x.rowel"));
    }

    #[test]
    fn a_name_that_leaves_nothing_is_refused() {
        for name in ["", ".", "..", "...", "a/..", "a/", "/"] {
            assert_eq!(file_name(name), None, "{name:?}");
        }
    }

    #[test]
    fn a_document_is_copied_into_a_folder_of_its_own() {
        let cache = tempfile::tempdir().unwrap();

        let first = copy_in(cache.path(), "../export.csv", &b"name,password\n"[..], 64).unwrap();
        let second = copy_in(cache.path(), "export.csv", &b"other"[..], 64).unwrap();

        assert_eq!(first.file_name().unwrap(), "export.csv");
        assert_eq!(
            first.parent().unwrap().parent().unwrap(),
            cache.path().join(DIR)
        );
        assert_ne!(first, second);
        assert_eq!(fs::read(&first).unwrap(), b"name,password\n");
        assert_eq!(fs::read(&second).unwrap(), b"other");
    }

    #[test]
    fn a_document_without_a_name_is_not_copied() {
        let cache = tempfile::tempdir().unwrap();
        assert!(matches!(
            copy_in(cache.path(), "..", &b"x"[..], 64),
            Err(Error::Unsupported(_))
        ));
        assert!(!cache.path().join(DIR).exists());
    }

    // Counted on the bytes read, and nothing of a refused copy stays behind.
    #[test]
    fn a_document_past_the_cap_is_refused_and_removed() {
        let cache = tempfile::tempdir().unwrap();

        assert!(copy_in(cache.path(), "exact.rowel", &[0u8; 64][..], 64).is_ok());
        let refused = copy_in(cache.path(), "huge.rowel", &[0u8; 65][..], 64);

        assert!(matches!(refused, Err(Error::FileTooLarge)));
        let left: Vec<_> = fs::read_dir(cache.path().join(DIR)).unwrap().collect();
        assert_eq!(left.len(), 1, "only the copy within the cap is left");
    }

    // Only a copy's own folder goes: a path the dialog answered with on its own
    // is the user's file.
    #[test]
    fn a_discarded_pick_leaves_no_copy_and_no_other_file_is_touched() {
        let cache = tempfile::tempdir().unwrap();
        let copy = copy_in(cache.path(), "export.csv", &b"secret"[..], 64).unwrap();
        let own = cache.path().join("export.csv");
        fs::write(&own, b"theirs").unwrap();

        discard_in(cache.path(), &copy);
        discard_in(cache.path(), &own);

        assert!(!copy.parent().unwrap().exists());
        assert!(cache.path().join(DIR).exists());
        assert!(own.exists());
    }

    #[test]
    fn a_capture_is_named_as_wry_names_it() {
        for name in [
            "JPEG_20261124_104700_8123456789012345678.jpg",
            "JPEG_20261124_104700_-42.jpg",
            "JPEG_٢٠٢٦١١٢٤_١٠٤٧٠٠_42.jpg",
        ] {
            assert!(is_capture(name), "{name}");
        }
        for name in [
            "JPEG_.jpg",
            "IMG_0042.jpg",
            "JPEG_20261124_104700_42.png",
            "jpeg_20261124_104700_42.jpg",
        ] {
            assert!(!is_capture(name), "{name}");
        }
    }

    // Both kinds go, the camera's empty file included; anything else in either
    // folder stays.
    // What keeps a new copy out from under a wipe: by the time `settle`
    // returns, the wipe that was owed has run, and it runs once.
    #[test]
    fn a_wipe_owed_is_done_once_by_whoever_settles_first() {
        let owed = Owed::new();
        let mut done = 0;
        owed.settle(|| done += 1);
        assert_eq!(done, 0, "none was asked for");

        owed.ask();
        owed.ask();
        owed.settle(|| done += 1);
        owed.settle(|| done += 1);
        assert_eq!(done, 1);
    }

    #[test]
    fn a_wipe_takes_every_copy_and_capture_and_nothing_else() {
        let cache = tempfile::tempdir().unwrap();
        let pictures = tempfile::tempdir().unwrap();
        let kept = cache.path().join("favicons.tmp");
        fs::write(&kept, b"x").unwrap();
        copy_in(cache.path(), "export.csv", &b"a"[..], 64).unwrap();
        copy_in(cache.path(), "card.jpg", &b"b"[..], 64).unwrap();
        let capture = pictures.path().join("JPEG_20261124_104700_42.jpg");
        let empty = pictures.path().join("JPEG_20261124_104812_7.jpg");
        let other = pictures.path().join("IMG_0042.jpg");
        let folder = pictures.path().join("JPEG_folder_1.jpg");
        fs::write(&capture, b"\xff\xd8\xff\xe0").unwrap();
        fs::write(&empty, b"").unwrap();
        fs::write(&other, b"\xff\xd8\xff\xe0").unwrap();
        fs::create_dir(&folder).unwrap();

        wipe_in(Some(cache.path()), Some(pictures.path()));

        assert!(!cache.path().join(DIR).exists());
        assert!(!capture.exists());
        assert!(!empty.exists());
        assert!(kept.exists());
        assert!(other.exists());
        assert!(folder.exists());
        // Nothing to wipe, or nowhere to look, is nothing to do.
        wipe_in(Some(cache.path()), Some(&pictures.path().join("gone")));
        wipe_in(None, None);
    }
}
