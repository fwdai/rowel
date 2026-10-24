use crate::session;
use crate::state::AppState;
use crate::timer::Timer;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};
use tauri::{AppHandle, Manager, WindowEvent};

// The bounds are the settings module's: it normalises the stored value with
// the same numbers, so what the file holds is what gets armed here.
use crate::settings::{
    DEFAULT_AUTOLOCK_SECS as DEFAULT_TIMEOUT_SECS, MAX_AUTOLOCK_SECS as MAX_TIMEOUT_SECS,
};

/// The inactivity lock.
///
/// The timer is armed for the whole of an unlocked session and re-armed by
/// every sign of the user — input in the webview (`touch_activity`), the window
/// gaining or losing focus — so it comes due `timeout_secs` after the *last*
/// one, wherever the window is. An earlier version armed only on blur and
/// disarmed on focus, which left a focused window that nobody was at unlocked
/// indefinitely: a laptop walked away from with the app in front never locked.
pub struct AutoLock {
    /// The pending lock. One timer for the whole process: every activity
    /// re-arms it rather than adding to it, so a busy user cannot pile up
    /// pending locks.
    timer: Arc<Timer>,
    /// Idle seconds before an unlocked vault seals itself. Seeded from
    /// `settings.json` at startup and re-set whenever the Settings row changes,
    /// so the value the user picked is in force from the first arming.
    timeout_secs: AtomicU64,
    /// Wall-clock reading taken when the window last lost focus, cleared when
    /// it comes back. The timer alone cannot be trusted across that gap: it
    /// measures in `Instant`s, which do not advance while an Apple machine is
    /// asleep and whose thread does not run at all while iOS has the process
    /// suspended. This is what says how long the user was really away.
    blurred_at: Mutex<Option<SystemTime>>,
}

impl Default for AutoLock {
    fn default() -> Self {
        Self {
            timer: Timer::spawn(),
            timeout_secs: AtomicU64::new(DEFAULT_TIMEOUT_SECS),
            blurred_at: Mutex::new(None),
        }
    }
}

/// What coming back owes the session: sealed, because the time away spent the
/// whole timeout, or just put back on the clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Resume {
    Lock,
    Touch,
}

impl AutoLock {
    /// The idle timeout in force, as the settings row last left it.
    fn timeout(&self) -> Duration {
        Duration::from_secs(self.timeout_secs.load(Ordering::SeqCst))
    }

    /// The window lost focus at `now`: remember when, so coming back can tell
    /// how long the user was really away.
    fn blur(&self, now: SystemTime) {
        *self.blurred_at.lock().unwrap() = Some(now);
    }

    /// The window came back at `now`. Consumes the stored blur — a resume with
    /// none, or a second resume after one was already spent, has nothing to
    /// measure and so only re-arms.
    fn resume(&self, now: SystemTime, timeout: Duration) -> Resume {
        let blurred_at = self.blurred_at.lock().unwrap().take();
        if blurred_at.is_some_and(|at| overdue(at, now, timeout)) {
            Resume::Lock
        } else {
            Resume::Touch
        }
    }
}

pub fn set_timeout(app: &AppHandle, secs: u64) {
    store_timeout(app, secs);
    // Anything already pending was armed against the old value. Re-armed
    // rather than dropped: a session that is live stays on the clock, now the
    // new one.
    touch(app);
}

/// The value alone, for a caller that re-arms afterwards (`settings::set`):
/// `touch` reads the session, and this takes no lock at all.
pub fn store_timeout(app: &AppHandle, secs: u64) {
    app.state::<AutoLock>()
        .timeout_secs
        .store(secs.clamp(1, MAX_TIMEOUT_SECS), Ordering::SeqCst);
}

/// The user did something: start the idle clock over. A no-op with no session
/// to lock, so the frontend can call it freely.
///
/// `is_live`, not `is_unlocked`: a vault whose key is out with a password
/// change or a workspace create is still a vault to lock, and the operation
/// finds out when it tries to hand the key back (`Session::adopt`).
///
/// A ring with keys in it counts too. A switch to a workspace the ring does
/// not hold ends the session and keeps the ring (`workspace_select`), so the
/// lock screen it lands on has the app open behind it — for exactly as long
/// as an open vault would be.
pub fn touch(app: &AppHandle) {
    let state = app.state::<AppState>();
    let live = state.session.lock().unwrap().is_live();
    let held = !state.keyring.lock().unwrap().is_empty();
    if !live && !held {
        return;
    }
    let state = app.state::<AutoLock>();
    let timeout = state.timeout();
    let app = app.clone();
    state.timer.arm(timeout, move || lock(&app));
}

/// Nothing left to lock. Called when a session ends, so a timer armed for the
/// session just sealed cannot come due inside the next one — an unlock a few
/// seconds later would otherwise be locked again by the old deadline.
pub fn disarm(app: &AppHandle) {
    app.state::<AutoLock>().timer.disarm();
}

/// Focus in either direction is activity: the user is switching to or from
/// the app. Backgrounding counts as a blur on every platform, so this one hook
/// covers the phone too: tao's iOS scene delegate posts `Focused(false)` from
/// `sceneWillResignActive:` and `Focused(true)` from `sceneDidBecomeActive:`
/// (tao 0.35.3, `platform_impl/ios/scene.rs:75-101`), and tauri-runtime-wry
/// forwards both unchanged off Windows (`lib.rs:522`). Sending the app to the
/// home screen therefore starts the same clock that alt-tabbing does.
///
/// Coming *back*, though, is not a fresh start if the time away already spent
/// the whole timeout: the vault is locked instead of re-armed. Without that,
/// resuming could cancel a lock that was already owed — on iOS the process is
/// frozen while backgrounded, so on thaw the main thread's `Focused(true)` and
/// the timer thread's expired wait race, and a focus event that wins would
/// re-arm a full timeout over a vault left open for hours. The reading is a
/// wall clock (`SystemTime`), not the timer's `Instant`, because a sleeping
/// Apple machine does not advance the latter at all.
pub fn handle_event(app: &AppHandle, event: &WindowEvent) {
    match event {
        WindowEvent::Focused(false) => {
            app.state::<AutoLock>().blur(SystemTime::now());
            touch(app);
        }
        WindowEvent::Focused(true) => {
            let resume = {
                let state = app.state::<AutoLock>();
                let timeout = state.timeout();
                state.resume(SystemTime::now(), timeout)
            };
            match resume {
                Resume::Lock => lock(app),
                Resume::Touch => touch(app),
            }
        }
        _ => {}
    }
}

/// Whether a window blurred at `blurred_at` has been away for its whole
/// timeout by `now`. A clock that went backwards between the two readings —
/// an NTP correction, the user changing the date — reads as no time at all,
/// so a session is never sealed on the strength of a negative interval.
fn overdue(blurred_at: SystemTime, now: SystemTime, timeout: Duration) -> bool {
    now.duration_since(blurred_at).unwrap_or_default() >= timeout
}

/// The lock an idle timer or the tray asks for, which may well find the vault
/// already sealed — a timer that comes due after a manual lock, the tray item
/// pressed twice. `session::lock` checks and seals under one guard and says
/// whether there was anything to seal, so a timer that comes due while a fresh
/// unlock is landing cannot clear the session that unlock just opened.
pub fn lock(app: &AppHandle) {
    session::lock(app);
}

#[cfg(test)]
mod tests {
    use super::*;

    const TIMEOUT: Duration = Duration::from_secs(300);
    const A_DAY: Duration = Duration::from_secs(86_400);

    // When the window lost focus. Any fixed reading will do: every test says
    // what `now` is, so nothing here depends on the real clock.
    fn blurred_at() -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_000)
    }

    #[test]
    fn a_resume_inside_the_timeout_touches() {
        let autolock = AutoLock::default();
        autolock.blur(blurred_at());
        assert_eq!(
            autolock.resume(blurred_at() + TIMEOUT / 2, TIMEOUT),
            Resume::Touch
        );
    }

    // The whole point: time the machine spent asleep or suspended counts, and
    // the timeout falling exactly due is due.
    #[test]
    fn a_resume_after_the_whole_timeout_locks() {
        let autolock = AutoLock::default();

        autolock.blur(blurred_at());
        assert_eq!(
            autolock.resume(blurred_at() + TIMEOUT, TIMEOUT),
            Resume::Lock
        );

        autolock.blur(blurred_at());
        assert_eq!(autolock.resume(blurred_at() + A_DAY, TIMEOUT), Resume::Lock);
    }

    // Focus gained with no blur behind it — the window raised at startup, a
    // platform that repeats the event — has no absence to measure.
    #[test]
    fn a_resume_without_a_blur_touches() {
        let autolock = AutoLock::default();
        assert_eq!(
            autolock.resume(blurred_at() + A_DAY, TIMEOUT),
            Resume::Touch
        );
    }

    // The resume that reads the blur consumes it, so a later focus event
    // cannot lock again on a reading already spent.
    #[test]
    fn a_second_resume_touches_because_the_blur_was_consumed() {
        let autolock = AutoLock::default();
        autolock.blur(blurred_at());
        assert_eq!(
            autolock.resume(blurred_at() + TIMEOUT, TIMEOUT),
            Resume::Lock
        );
        assert_eq!(
            autolock.resume(blurred_at() + A_DAY, TIMEOUT),
            Resume::Touch
        );
    }

    // A clock that went backwards says nothing about how long the user was
    // away, so it locks nothing; the timer is still there to come due.
    #[test]
    fn a_resume_on_a_clock_that_went_backwards_touches() {
        let autolock = AutoLock::default();
        autolock.blur(blurred_at() + A_DAY);
        assert_eq!(autolock.resume(blurred_at(), TIMEOUT), Resume::Touch);
    }
}
