//! The photos the webview's camera input leaves behind on Android.
//!
//! wry answers a file input with `capture` by creating a file for the camera
//! app to write the photo into, and hands that to the input (wry 0.57.0,
//! `RustWebChromeClient.kt`: `createImageFile` makes it with
//! `File.createTempFile("JPEG_" + <yyyyMMdd_HHmmss> + "_", ".jpg", ...)` in
//! `getExternalFilesDir(DIRECTORY_PICTURES)`). It never removes it, nor the
//! empty one a cancelled capture leaves. That photo is a card or a passport in
//! the clear, on storage that a USB connection reaches, and before scoped
//! storage any app allowed to read external storage. So what wry leaves there is
//! swept: after a capture's bytes have reached Rust (`attachment_add_bytes`,
//! `scan_image_bytes`), and at startup for whatever a capture that never got
//! that far left.

use tauri::AppHandle;

/// Remove, off the calling thread, every photo wry's capture has left. Android
/// only: elsewhere the webview's camera input writes nothing the app keeps.
pub fn sweep(app: &AppHandle) {
    #[cfg(target_os = "android")]
    {
        use tauri::Manager;

        let app = app.clone();
        crate::commands::detached(move || {
            // `PathPlugin.getPictureDir`: the same
            // `getExternalFilesDir(DIRECTORY_PICTURES)` wry writes to.
            if let Ok(dir) = app.path().picture_dir() {
                remove_captures(&dir);
            }
        });
    }
    #[cfg(not(target_os = "android"))]
    let _ = app;
}

/// Remove the files in `dir` named the way wry names a capture. Best effort: a
/// photo that will not go now is tried again on the next sweep.
#[cfg(any(target_os = "android", test))]
fn remove_captures(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let is_file = entry.file_type().is_ok_and(|t| t.is_file());
        if is_file && entry.file_name().to_str().is_some_and(is_capture) {
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

        remove_captures(dir.path());

        assert!(!capture.exists());
        assert!(!cancelled.exists());
        assert!(other.exists());
        assert!(folder.exists());
    }

    #[test]
    fn a_directory_that_is_not_there_is_nothing_to_sweep() {
        let dir = tempfile::tempdir().unwrap();
        remove_captures(&dir.path().join("Pictures"));
    }
}
