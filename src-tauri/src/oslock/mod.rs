//! OS screen lock, sleep and user-switch listeners, one per desktop platform.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

use tauri::AppHandle;

#[cfg(target_os = "linux")]
mod logind;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod win32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Watching,
    /// The listener is gone (its bus connection closed); never on macOS.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    Lost,
    Locked,
    SwitchedAway,
    Sleep,
    Wake,
}

type Sink = Arc<dyn Fn(Event) + Send + Sync>;

/// Called once from `setup`, on the main thread the macOS observers deliver on.
pub fn watch(app: &AppHandle) {
    let app = app.clone();
    let sink: Sink = Arc::new(move |event| {
        // Called from Objective-C blocks and window procedures: never unwind.
        if catch_unwind(AssertUnwindSafe(|| crate::autolock::os_event(&app, event))).is_err() {
            log::warn!("the OS lock listener's {event:?} panicked");
        }
    });
    #[cfg(target_os = "linux")]
    logind::watch(sink);
    #[cfg(target_os = "macos")]
    macos::watch(sink);
    #[cfg(target_os = "windows")]
    win32::watch(sink);
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    drop(sink);
}
