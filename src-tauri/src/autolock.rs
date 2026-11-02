use crate::session::{self, Epoch, Session};
use crate::state::AppState;
use crate::timer::Timer;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
    /// "Follow the OS": the idle timer runs to the day-long ceiling instead.
    follows_os: AtomicBool,
    /// While the OS locking is not heard, `follows_os` falls back to `timeout_secs`.
    os_lock: Mutex<OsLockStatus>,
    /// Bumped by every arm and disarm, under the session guard. A firing timer
    /// callback can't be disarmed, so a deadline locks only if still current.
    arming: AtomicU64,
    /// Wall-clock reading taken when the window last lost focus, cleared when
    /// it comes back. The timer alone cannot be trusted across that gap: it
    /// measures in `Instant`s, which do not advance while an Apple machine is
    /// asleep and whose thread does not run at all while iOS has the process
    /// suspended. This is what says how long the user was really away. Paired
    /// with the session open at the time.
    blurred_at: Mutex<Option<(SystemTime, Epoch)>>,
}

impl Default for AutoLock {
    fn default() -> Self {
        Self {
            timer: Timer::spawn(),
            timeout_secs: AtomicU64::new(DEFAULT_TIMEOUT_SECS),
            follows_os: AtomicBool::new(false),
            os_lock: Mutex::new(OsLockStatus::default()),
            arming: AtomicU64::new(0),
            blurred_at: Mutex::new(None),
        }
    }
}

/// Sent as `oslock:status` and read once by the webview after it subscribes;
/// `seq` lets it keep the newer of the two.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OsLockStatus {
    pub supported: bool,
    pub seq: u64,
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
        Duration::from_secs(idle_secs(
            self.timeout_secs.load(Ordering::SeqCst),
            self.follows_os.load(Ordering::SeqCst),
            self.os_lock_status().supported,
        ))
    }

    fn os_lock_status(&self) -> OsLockStatus {
        *self.os_lock.lock().unwrap()
    }

    /// `None` when nothing changed.
    #[cfg_attr(not(desktop), allow(dead_code))]
    fn set_os_watched(&self, watched: bool) -> Option<OsLockStatus> {
        let mut status = self.os_lock.lock().unwrap();
        if status.supported == watched {
            return None;
        }
        *status = OsLockStatus {
            supported: watched,
            seq: status.seq + 1,
        };
        Some(*status)
    }

    /// The window lost focus at `now`: remember when, so coming back can tell
    /// how long the user was really away.
    fn blur(&self, now: SystemTime, open: Epoch) {
        *self.blurred_at.lock().unwrap() = Some((now, open));
    }

    /// The window came back at `now`. Consumes the stored blur — a resume with
    /// none, or a second resume after one was already spent, has nothing to
    /// measure and so only re-arms. A session opened since keeps its own clock.
    fn resume(&self, now: SystemTime, open: Epoch, timeout: Duration) -> Resume {
        let blurred = self.blurred_at.lock().unwrap().take();
        if blurred.is_some_and(|(at, left)| left == open && overdue(at, now, timeout)) {
            Resume::Lock
        } else {
            Resume::Touch
        }
    }

    fn arm_with(&self, after: Duration, fire: impl FnOnce(u64) + Send + 'static) {
        let arming = self.arming.fetch_add(1, Ordering::SeqCst) + 1;
        self.timer.arm(after, move || fire(arming));
    }

    fn disarm(&self) {
        self.arming.fetch_add(1, Ordering::SeqCst);
        self.timer.disarm();
    }

    fn in_force(&self, arming: u64) -> bool {
        self.arming.load(Ordering::SeqCst) == arming
    }
}

/// Following the OS still idles out after a day, for an OS that never locks.
fn idle_secs(chosen: u64, follows_os: bool, os_watched: bool) -> u64 {
    if follows_os && os_watched {
        MAX_TIMEOUT_SECS
    } else {
        chosen
    }
}

pub fn set_timeout(app: &AppHandle, secs: u64, follows_os: bool) {
    store_timeout(app, secs, follows_os);
    // Anything already pending was armed against the old value. Re-armed
    // rather than dropped: a session that is live stays on the clock, now the
    // new one.
    touch(app);
}

/// The values alone, for a caller that re-arms afterwards (`settings::set`):
/// `touch` reads the session, and this takes no lock at all.
pub fn store_timeout(app: &AppHandle, secs: u64, follows_os: bool) {
    let state = app.state::<AutoLock>();
    state
        .timeout_secs
        .store(secs.clamp(1, MAX_TIMEOUT_SECS), Ordering::SeqCst);
    state.follows_os.store(follows_os, Ordering::SeqCst);
}

pub fn os_lock_status(app: &AppHandle) -> OsLockStatus {
    app.state::<AutoLock>().os_lock_status()
}

/// The OS locking, a user switch, sleep and wake seal the vault whatever the
/// timeout (wake catches a sleep noticed too late).
#[cfg(desktop)]
pub fn os_event(app: &AppHandle, event: crate::oslock::Event) {
    use crate::oslock::Event;
    match event {
        Event::Watching | Event::Lost => {
            let changed = app
                .state::<AutoLock>()
                .set_os_watched(event == Event::Watching);
            if let Some(status) = changed {
                touch(app);
                crate::events::os_lock_status(app, status);
            }
        }
        Event::Locked | Event::SwitchedAway | Event::Sleep | Event::Wake => {
            if session::lock(app) {
                log::info!("vault sealed with the OS ({event:?})");
            }
        }
    }
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
    let session = state.session.lock().unwrap();
    let held = !state.keyring.lock().unwrap().is_empty();
    if !session.is_live() && !held {
        return;
    }
    arm(app);
}

/// Only under the session guard, with a session to lock.
pub fn arm(app: &AppHandle) {
    let state = app.state::<AutoLock>();
    let app = app.clone();
    state.arm_with(state.timeout(), move |arming| idle(&app, arming));
}

/// Nothing left to lock. Called when a session ends, so a timer armed for the
/// session just sealed cannot come due inside the next one — an unlock a few
/// seconds later would otherwise be locked again by the old deadline.
pub fn disarm(app: &AppHandle) {
    app.state::<AutoLock>().disarm();
}

fn idle(app: &AppHandle, arming: u64) {
    let state = app.state::<AutoLock>();
    if session::lock_if(app, |_| state.in_force(arming)) {
        log::info!("vault sealed after its idle timeout");
    }
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
            let open = app.state::<AppState>().session.lock().unwrap().epoch();
            app.state::<AutoLock>().blur(SystemTime::now(), open);
            touch(app);
        }
        WindowEvent::Focused(true) => {
            let now = SystemTime::now();
            let state = app.state::<AutoLock>();
            let owed = |session: &Session| {
                state.resume(now, session.epoch(), state.timeout()) == Resume::Lock
            };
            if !session::lock_if(app, owed) {
                touch(app);
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

/// The tray's lock.
#[cfg(desktop)]
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

    fn open() -> Epoch {
        Session::default().epoch()
    }
    fn opened_since() -> Epoch {
        let mut session = Session::default();
        session.clear_for_test();
        session.epoch()
    }

    // The Touch ID sheet takes focus while the vault opens behind it.
    #[test]
    fn a_resume_does_not_lock_a_session_opened_since_the_window_was_left() {
        let autolock = AutoLock::default();
        autolock.blur(blurred_at(), open());
        assert_eq!(
            autolock.resume(blurred_at() + A_DAY, opened_since(), TIMEOUT),
            Resume::Touch
        );
        assert_eq!(
            autolock.resume(blurred_at() + A_DAY, open(), TIMEOUT),
            Resume::Touch
        );
    }

    #[test]
    fn a_resume_inside_the_timeout_touches() {
        let autolock = AutoLock::default();
        autolock.blur(blurred_at(), open());
        assert_eq!(
            autolock.resume(blurred_at() + TIMEOUT / 2, open(), TIMEOUT),
            Resume::Touch
        );
    }

    // The whole point: time the machine spent asleep or suspended counts, and
    // the timeout falling exactly due is due.
    #[test]
    fn a_resume_after_the_whole_timeout_locks() {
        let autolock = AutoLock::default();

        autolock.blur(blurred_at(), open());
        assert_eq!(
            autolock.resume(blurred_at() + TIMEOUT, open(), TIMEOUT),
            Resume::Lock
        );

        autolock.blur(blurred_at(), open());
        assert_eq!(
            autolock.resume(blurred_at() + A_DAY, open(), TIMEOUT),
            Resume::Lock
        );
    }

    // Focus gained with no blur behind it — the window raised at startup, a
    // platform that repeats the event — has no absence to measure.
    #[test]
    fn a_resume_without_a_blur_touches() {
        let autolock = AutoLock::default();
        assert_eq!(
            autolock.resume(blurred_at() + A_DAY, open(), TIMEOUT),
            Resume::Touch
        );
    }

    // The resume that reads the blur consumes it, so a later focus event
    // cannot lock again on a reading already spent.
    #[test]
    fn a_second_resume_touches_because_the_blur_was_consumed() {
        let autolock = AutoLock::default();
        autolock.blur(blurred_at(), open());
        assert_eq!(
            autolock.resume(blurred_at() + TIMEOUT, open(), TIMEOUT),
            Resume::Lock
        );
        assert_eq!(
            autolock.resume(blurred_at() + A_DAY, open(), TIMEOUT),
            Resume::Touch
        );
    }

    // A clock that went backwards says nothing about how long the user was
    // away, so it locks nothing; the timer is still there to come due.
    #[test]
    fn a_resume_on_a_clock_that_went_backwards_touches() {
        let autolock = AutoLock::default();
        autolock.blur(blurred_at() + A_DAY, open());
        assert_eq!(
            autolock.resume(blurred_at(), open(), TIMEOUT),
            Resume::Touch
        );
    }

    #[test]
    fn the_chosen_timeout_is_the_idle_lock() {
        assert_eq!(idle_secs(300, false, false), 300);
        assert_eq!(idle_secs(300, false, true), 300);
    }

    #[test]
    fn following_a_heard_os_idles_out_after_a_day() {
        assert_eq!(idle_secs(300, true, true), MAX_TIMEOUT_SECS);
    }

    #[test]
    fn an_os_that_is_not_heard_is_not_followed() {
        assert_eq!(idle_secs(300, true, false), 300);
    }

    #[test]
    fn the_timeout_follows_the_os_only_while_it_is_heard() {
        let autolock = AutoLock::default();
        autolock.timeout_secs.store(300, Ordering::SeqCst);
        autolock.follows_os.store(true, Ordering::SeqCst);
        assert_eq!(autolock.timeout(), Duration::from_secs(300));

        autolock.set_os_watched(true);
        assert_eq!(autolock.timeout(), Duration::from_secs(MAX_TIMEOUT_SECS));

        autolock.set_os_watched(false);
        assert_eq!(autolock.timeout(), Duration::from_secs(300));
    }

    #[test]
    fn each_change_of_the_listener_is_counted_and_a_repeat_is_not() {
        let autolock = AutoLock::default();
        assert_eq!(autolock.os_lock_status(), OsLockStatus::default());

        let up = OsLockStatus {
            supported: true,
            seq: 1,
        };
        assert_eq!(autolock.set_os_watched(true), Some(up));
        assert_eq!(autolock.set_os_watched(true), None);
        assert_eq!(autolock.os_lock_status(), up);

        let lost = OsLockStatus {
            supported: false,
            seq: 2,
        };
        assert_eq!(autolock.set_os_watched(false), Some(lost));
        assert_eq!(autolock.os_lock_status(), lost);
    }

    // What `idle` decides, under the session guard.
    fn would_lock(state: &AppState, autolock: &AutoLock, arming: u64) -> bool {
        let session = state.session.lock().unwrap();
        session::lock_applies(&session, &state.keyring, |_| autolock.in_force(arming))
    }

    fn unlocked() -> (AppState, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let key = crate::crypto::VaultKey::legacy_from_password("first");
        let store =
            crate::store::SqliteStore::open(&dir.path().join("vault.db"), &*key.sqlcipher_key())
                .unwrap();
        let state = AppState::default();
        state.session.lock().unwrap().set(key, store, false);
        (state, dir)
    }

    // A firing deadline is held where it would wait for the guard while `step`
    // changes the clock under it.
    fn deadline_overtaken_by(step: impl FnOnce(&AppState, &AutoLock)) -> bool {
        let (state, _dir) = unlocked();
        let state = std::sync::Arc::new(state);
        let autolock = std::sync::Arc::new(AutoLock::default());
        let (due, is_due) = std::sync::mpsc::channel();
        let (release, released) = std::sync::mpsc::channel::<()>();
        let (answer, answered) = std::sync::mpsc::channel();
        {
            let _session = state.session.lock().unwrap();
            let (state, held) = (state.clone(), autolock.clone());
            autolock.arm_with(Duration::from_millis(10), move |arming| {
                due.send(()).unwrap();
                released.recv().unwrap();
                answer.send(would_lock(&state, &held, arming)).unwrap();
            });
        }
        is_due.recv_timeout(Duration::from_secs(5)).unwrap();
        {
            let _session = state.session.lock().unwrap();
            step(&state, &autolock);
        }
        release.send(()).unwrap();
        answered.recv_timeout(Duration::from_secs(5)).unwrap()
    }

    #[test]
    fn a_deadline_already_firing_locks_nothing_once_activity_re_arms() {
        assert!(!deadline_overtaken_by(|_, autolock| {
            autolock.arm_with(Duration::from_secs(3600), |_| {});
        }));
    }

    #[test]
    fn a_deadline_already_firing_locks_nothing_once_a_session_reopens() {
        assert!(!deadline_overtaken_by(|_, autolock| {
            autolock.disarm();
            autolock.arm_with(Duration::from_secs(3600), |_| {});
        }));
    }

    #[test]
    fn a_deadline_nothing_overtook_locks() {
        assert!(deadline_overtaken_by(|_, _| {}));
    }

    #[test]
    fn a_deadline_over_nothing_open_locks_nothing() {
        let state = AppState::default();
        let autolock = AutoLock::default();
        autolock.arm_with(Duration::from_secs(3600), |_| {});
        let arming = autolock.arming.load(Ordering::SeqCst);
        assert!(!would_lock(&state, &autolock, arming));
    }
}
