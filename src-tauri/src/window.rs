use crate::grants::{PathGrants, Purpose};
use crate::{autolock, commands, scan};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, DragDropEvent, Manager, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_opener::OpenerExt;

const MAIN: &str = "main";

// How long to wait for the frontend to say it is up before showing the window
// anyway, so a bundle that never runs (dev server down, a throwing script)
// degrades to the resting mascot index.html paints on its own rather than to no
// window at all.
const SHOW_FALLBACK: std::time::Duration = std::time::Duration::from_secs(3);

// One-shot latch, one main window per process: whichever reveal path wins —
// `app_ready` or the fallback timer — the other becomes a no-op, so a window the
// user has since closed to the tray never pops back up on its own.
static REVEALED: AtomicBool = AtomicBool::new(false);

// Build the main window from the frozen config, adding the boot payload, the
// reveal timing, the per-OS tweaks and the navigation locking that
// tauri.conf.json can't express (config sets create:false). Chrome itself is
// native everywhere: Windows and Linux get the system frame, macOS the
// hidden-inset title bar from config.
//
// Config also creates the window hidden — an empty window would otherwise sit on
// screen through the whole bundle load, and with an overlay title bar that reads
// as bare traffic lights floating over nothing. The frontend calls `app_ready`
// the moment it has armed the splash, so the window appears exactly when the
// choreography starts rather than at some unrelated page-load milestone.
pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let config = app
        .config()
        .app
        .windows
        .iter()
        .find(|w| w.label == MAIN)
        .expect("main window missing from tauri.conf.json")
        .clone();

    let handle = app.clone();
    let builder = WebviewWindowBuilder::from_config(app, &config)?
        .on_navigation(move |url| navigate(&handle, url))
        // The theme and the language, before any script of ours runs, so the
        // first frame is themed and the catalogue starts loading without a round
        // trip in front of it (see `commands::app::boot_script`).
        .initialization_script(commands::app::boot_script(app));

    let window = builder.build()?;
    #[cfg(target_os = "ios")]
    cover_safe_area(&window)?;
    // The native zoom animation leaves the page frozen until it ends; this
    // swaps in one the webview can keep up with (see `zoom`).
    #[cfg(target_os = "macos")]
    crate::zoom::install(window.ns_window()?);
    let handle = app.clone();
    window.on_window_event(move |event| {
        autolock::handle_event(&handle, event);
        // The OS delivers a drop as this window event, and tauri raises the
        // webview's `drag-drop` from the same one — so the paths are granted
        // here before the frontend can ask to read them (see `grants`).
        //
        // A drop has no dialog to frame it, so what the user chose it as comes
        // from the file itself — by the same test the frontend routes the drop
        // with: an extension the scanner opens makes it an image, anything else
        // an env file, which may be named anything (`read_env_file` applies its
        // own name-or-content check to those). So a dropped photo cannot be
        // read back as text, nor a dropped `.env` be OCRed. Either may still be
        // attached to the open entry (see `PathGrants::grant_drop`).
        if let WindowEvent::DragDrop(DragDropEvent::Drop { paths, .. }) = event {
            let grants = handle.state::<PathGrants>();
            for path in paths {
                let purpose = if scan::is_image(path) {
                    Purpose::Image
                } else {
                    Purpose::Env
                };
                grants.grant_drop(path, purpose);
            }
        }
        // Coming back to the foreground is how a consent flow the user walked
        // away from gets noticed (see `commands::sync::on_resume`).
        #[cfg(mobile)]
        if let WindowEvent::Focused(true) = event {
            crate::commands::sync::on_resume(&handle);
        }
    });

    let fallback = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(SHOW_FALLBACK);
        reveal(&fallback);
    });

    Ok(())
}

// Give the page the whole screen. WKWebView's scroll view inherits UIKit's
// automatic safe-area content insets, which take the notch and the home
// indicator out of the page's layout viewport (874 → 778pt on an iPhone 17)
// even though `viewport-fit=cover` paints under them — so a `height: 100%`
// shell ended 96pt above the bottom edge, tab bar and all. With the adjustment
// off the layout viewport is the screen, and the compact chrome keeps clear of
// the edges with `env(safe-area-inset-*)`, which UIKit still reports.
#[cfg(target_os = "ios")]
fn cover_safe_area(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    window.with_webview(|webview| unsafe {
        use objc2::{msg_send, runtime::AnyObject};
        // UIScrollViewContentInsetAdjustmentNever
        const NEVER: isize = 2;
        let webview = webview.inner() as *mut AnyObject;
        let scroll_view: *mut AnyObject = msg_send![webview, scrollView];
        let () = msg_send![scroll_view, setContentInsetAdjustmentBehavior: NEVER];
    })
}

// Block in-app navigation to external sites; open them in the OS browser instead.
fn navigate(app: &AppHandle, url: &tauri::Url) -> bool {
    let external = matches!(url.scheme(), "http" | "https")
        && !matches!(url.host_str(), Some(h) if h == "localhost" || h.ends_with(".localhost"));
    if external {
        let _ = app.opener().open_url(url.as_str(), None::<&str>);
        return false;
    }
    true
}

/// First reveal at startup, run at most once (see `REVEALED`). Called by the
/// `app_ready` command and by the fallback timer in `create`.
pub fn reveal(app: &AppHandle) {
    if !REVEALED.swap(true, Ordering::SeqCst) {
        show(app);
    }
}

/// Bring the window forward for a document the OS handed over while the app is
/// up. Before the first reveal it does nothing: the launch choreography shows
/// the window itself, and showing it here would put it up bare, ahead of the
/// splash it is meant to appear with.
#[cfg(desktop)]
pub fn raise(app: &AppHandle) {
    if REVEALED.load(Ordering::SeqCst) {
        show(app);
    }
}

// Show and focus the main window (tray "Open Rowel" and second-instance launch).
pub fn show(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN) {
        let _ = window.show();
        // Minimization is a desktop window state; the API doesn't exist on mobile.
        #[cfg(desktop)]
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}
