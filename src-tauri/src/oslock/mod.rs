//! The operating system locking — the screen lock, sleep, a switch to another
//! user — heard so the vault can seal with it. Desktop only.
//!
//! The vault seals whenever the OS locks, whatever idle timeout is chosen: a
//! vault still open behind the OS password just typed would be the odd one
//! out. The "follow the OS" timeout then makes that the only lock short of the
//! day-long idle ceiling (`autolock`).
//!
//! One listener per platform, each reporting what it hears to the same sink:
//!
//! - macOS: the distributed `com.apple.screenIsLocked`, and the workspace's
//!   sleep, wake and session-resign notifications (`macos`).
//! - Windows: the session-change and power broadcasts, to a hidden window of
//!   the listener's own (`win32`).
//! - Linux: logind on the system bus — the session's lock, its `LockedHint`
//!   and `Active`, and the manager's sleep (`logind`).
//!
//! A display that only blanks is not a lock, and none of them reports one.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

use tauri::AppHandle;

#[cfg(target_os = "linux")]
mod logind;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod win32;

/// What a listener heard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// The listener is in place: from here on the OS locking is heard.
    Watching,
    /// It went away (the bus connection under it closed): from here on the
    /// OS locking is not heard. Never on macOS, whose observers stay put.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    Lost,
    /// The session locked: the lock shortcut, the OS's own idle lock, a lid
    /// closed with a password to come back to.
    Locked,
    /// The user switched away: to another user's session, another virtual
    /// terminal, a remote session disconnected.
    SwitchedAway,
    /// The machine is going to sleep.
    Sleep,
    /// It woke. Sealed too: a sleep whose notice was missed — the process
    /// suspended before it got to it — is still a sleep.
    Wake,
}

/// Where a listener reports, from whichever thread the platform delivers on.
type Sink = Arc<dyn Fn(Event) + Send + Sync>;

/// Start listening, for the life of the process. Called once, from `setup`, on
/// the main thread: the macOS observers deliver on the thread that made them.
pub fn watch(app: &AppHandle) {
    let app = app.clone();
    let sink: Sink = Arc::new(move |event| {
        // Reached from the platform's own callbacks — an Objective-C block, a
        // window procedure — which a panic must not unwind through.
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
    // Another desktop OS has no listener: the timeouts stay the only lock.
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    drop(sink);
}
