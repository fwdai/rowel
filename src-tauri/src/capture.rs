//! The photos the webview's camera input leaves behind on Android.
//!
//! wry answers a file input with `capture` by creating a file for the camera
//! app to write the photo into, and hands that to the input (wry 0.57.0,
//! `RustWebChromeClient.kt`: `createImageFile` makes it with
//! `File.createTempFile("JPEG_" + <yyyyMMdd_HHmmss> + "_", ".jpg", ...)` in
//! `getExternalFilesDir(DIRECTORY_PICTURES)`). It never removes it, nor the
//! empty one a cancelled capture leaves. That photo is a card or a passport in
//! the clear, on storage that a USB connection reaches, and before scoped
//! storage any app allowed to read external storage.
//!
//! So a capture is swept once the webview is done with it: the capture input
//! asks (`capture_settled`) when it has read the photo or given up on it —
//! taken and handed over, refused, dropped across a lock, or the camera
//! cancelled — whatever then becomes of the bytes. The camera is one capture
//! at a time, and a sweep takes only what was there when it was asked for, so
//! a capture begun after it, its file still being written, is left alone. At
//! startup a sweep takes whatever a capture the app did not live to settle
//! left behind.

#[cfg(any(target_os = "android", test))]
use std::time::SystemTime;

use tauri::AppHandle;

/// Remove, off the calling thread, every photo wry's capture had left by now.
/// Android only: elsewhere the webview's camera input writes nothing the app
/// keeps.
pub fn sweep(app: &AppHandle) {
    #[cfg(target_os = "android")]
    {
        use tauri::Manager;

        let asked = SystemTime::now();
        let app = app.clone();
        crate::commands::detached(move || {
            // `PathPlugin.getPictureDir`: the same
            // `getExternalFilesDir(DIRECTORY_PICTURES)` wry writes to.
            if let Ok(dir) = app.path().picture_dir() {
                remove_captures(&dir, asked);
            }
        });
    }
    #[cfg(not(target_os = "android"))]
    let _ = app;
}

/// Remove the files in `dir` named the way wry names a capture and last
/// written no later than `asked`: one written since belongs to a capture begun
/// after the sweep was asked for, which the camera may still be writing. Best
/// effort: a photo that will not go now is tried again on the next sweep.
#[cfg(any(target_os = "android", test))]
fn remove_captures(dir: &std::path::Path, asked: SystemTime) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let settled = meta.modified().is_ok_and(|written| written <= asked);
        if meta.is_file() && settled && entry.file_name().to_str().is_some_and(is_capture) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Whether `name` is one wry gives a capture: its `JPEG_` prefix and `.jpg`
/// suffix around something. What is between them is not checked further: the
/// timestamp is formatted in the device's locale, whose digits need not be
/// ASCII ones, and nothing but wry writes into that directory.
#[cfg(any(target_os = "android", test))]
fn is_capture(name: &str) -> bool {
    name.len() > "JPEG_.jpg".len() && name.starts_with("JPEG_") && name.ends_with(".jpg")
}

#[cfg(test)]
mod tests {
    use super::*;

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
            "rowel-export.csv",
        ] {
            assert!(!is_capture(name), "{name}");
        }
    }

    // Only wry's photos go, and only files: anything else in the directory is
    // left where it is.
    #[test]
    fn a_sweep_takes_the_captures_and_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let capture = dir.path().join("JPEG_20261124_104700_42.jpg");
        let cancelled = dir.path().join("JPEG_20261124_104812_7.jpg");
        let other = dir.path().join("IMG_0042.jpg");
        let folder = dir.path().join("JPEG_folder_1.jpg");
        std::fs::write(&capture, b"\xff\xd8\xff\xe0").unwrap();
        std::fs::write(&cancelled, b"").unwrap();
        std::fs::write(&other, b"\xff\xd8\xff\xe0").unwrap();
        std::fs::create_dir(&folder).unwrap();

        remove_captures(dir.path(), SystemTime::now());

        assert!(!capture.exists());
        assert!(!cancelled.exists());
        assert!(other.exists());
        assert!(folder.exists());
    }

    // A capture begun after the sweep was asked for — wry's empty file, which
    // the camera has yet to write — is not the sweep's to take.
    #[test]
    fn a_sweep_leaves_a_capture_begun_after_it_was_asked_for() {
        let dir = tempfile::tempdir().unwrap();
        let settled = dir.path().join("JPEG_20261124_104700_42.jpg");
        let in_progress = dir.path().join("JPEG_20261124_104830_43.jpg");
        std::fs::write(&settled, b"\xff\xd8\xff\xe0").unwrap();
        let asked = SystemTime::now();
        let later = asked + std::time::Duration::from_secs(10);
        std::fs::File::create(&in_progress)
            .unwrap()
            .set_modified(later)
            .unwrap();

        remove_captures(dir.path(), asked);

        assert!(!settled.exists());
        assert!(in_progress.exists());
    }

    #[test]
    fn a_directory_that_is_not_there_is_nothing_to_sweep() {
        let dir = tempfile::tempdir().unwrap();
        remove_captures(&dir.path().join("Pictures"), SystemTime::now());
    }
}
