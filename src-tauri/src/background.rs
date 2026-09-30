//! iOS: the app in the background.
//!
//! iOS ends a suspended process that holds a lock on a file in a shared
//! container — the `0xdead10cc` termination, which TestFlight reports as a
//! crash — and an idle WAL connection to the vault holds one on its `-shm`
//! for as long as it is open. The vault is in the App Group container so the
//! AutoFill extension can open it, so an unlocked app taken to the background
//! was ended at suspension, and again whenever the extension opened the vault
//! behind it. On `UIApplicationDidEnterBackground` the session parks its
//! store (`Session::set_background`): the connection closes, and until
//! `UIApplicationWillEnterForeground` each use opens and closes one of its
//! own. The key stays in memory: the vault is as unlocked as it was, only its
//! connection is gone.
//!
//! The parking runs off the main thread, under a background task: a command
//! that holds the session — a sync merge landing as the user swipes away —
//! finishes first, and the process is not suspended until the store is
//! parked. A store that is away on a lease (a password change, a workspace
//! being made or restored) is out of the session's reach, so the task stays
//! open until the lease comes back — its return parks the store — or the
//! session ends; iOS's own expiry ends it if that takes too long, which
//! leaves things as they were before this module and no worse.
//! Each notification takes a turn; a worker that finds a newer turn
//! by the time it holds the session does nothing, so a quick swipe away and
//! back cannot leave the store parked in the foreground, or held open in the
//! background.

use std::ptr::NonNull;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use block2::RcBlock;
use objc2::MainThreadMarker;
use objc2_foundation::{
    NSNotification, NSNotificationCenter, NSNotificationName, NSOperationQueue,
};
use objc2_ui_kit::{
    UIApplication, UIApplicationDidEnterBackgroundNotification,
    UIApplicationWillEnterForegroundNotification, UIBackgroundTaskIdentifier,
    UIBackgroundTaskInvalid,
};
use tauri::{AppHandle, Manager};

use crate::state::AppState;

// The notifications in the order they came, so a worker can tell it has been
// overtaken.
static TURN: AtomicU64 = AtomicU64::new(0);

// How often a worker looks again for a lease to have come back.
const LEASE_POLL: Duration = Duration::from_millis(50);

/// Start following the app's background and foreground transitions, for the
/// life of the process.
pub fn watch(app: &AppHandle) {
    observe(
        unsafe { UIApplicationDidEnterBackgroundNotification },
        app.clone(),
        true,
    );
    observe(
        unsafe { UIApplicationWillEnterForegroundNotification },
        app.clone(),
        false,
    );
}

fn observe(name: &NSNotificationName, app: AppHandle, background: bool) {
    // Posted on the main thread; the block runs there, synchronously.
    let block = RcBlock::new(move |_: NonNull<NSNotification>| {
        let turn = TURN.fetch_add(1, Ordering::SeqCst) + 1;
        let task = if background { Task::begin() } else { None };
        let app = app.clone();
        std::thread::spawn(move || {
            let state = app.state::<AppState>();
            loop {
                let lease_out = {
                    let mut session = state.session.lock().unwrap_or_else(|e| e.into_inner());
                    if TURN.load(Ordering::SeqCst) != turn {
                        break;
                    }
                    session.set_background(background);
                    background && session.is_held_out()
                };
                // The store is away on a lease, where parking cannot reach
                // it: keep the task, and so the process, until it is back —
                // its return parks it (`Session::adopt`) — or the session has
                // ended. The parking above is repeated harmlessly meanwhile.
                if !lease_out {
                    break;
                }
                std::thread::sleep(LEASE_POLL);
            }
            if let Some(task) = task {
                task.end();
            }
        });
    });
    // The observer returned is what would remove the registration. It is
    // never removed, so it is kept for the life of the process.
    let observer = unsafe {
        NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
            Some(name),
            None,
            None,
            &block,
        )
    };
    std::mem::forget(observer);
}

/// A background task: the time iOS gives the process past
/// `didEnterBackground` to finish something before it is suspended. Ended
/// once by whichever comes first, the work finishing or iOS's expiry.
struct Task {
    id: Arc<Mutex<Option<UIBackgroundTaskIdentifier>>>,
}

impl Task {
    // From the main thread, where the application object is reached; `None`
    // when this is not it, or iOS refused the task — the work then runs
    // without the time, which is how it ran before.
    fn begin() -> Option<Self> {
        let mtm = MainThreadMarker::new()?;
        let id: Arc<Mutex<Option<UIBackgroundTaskIdentifier>>> = Arc::default();
        let on_expiry = {
            let id = id.clone();
            RcBlock::new(move || finish(&id))
        };
        let task = UIApplication::sharedApplication(mtm)
            .beginBackgroundTaskWithExpirationHandler(Some(&*on_expiry));
        if task == unsafe { UIBackgroundTaskInvalid } {
            return None;
        }
        *id.lock().unwrap_or_else(|e| e.into_inner()) = Some(task);
        Some(Self { id })
    }

    // From the worker: the application object is reached on the main thread,
    // so the end is posted there.
    fn end(self) {
        let id = self.id;
        let block = RcBlock::new(move || finish(&id));
        unsafe { NSOperationQueue::mainQueue().addOperationWithBlock(&block) };
    }
}

// End the task, if it is still to be ended. On the main thread, from either
// the expiry handler or the posted end.
fn finish(id: &Mutex<Option<UIBackgroundTaskIdentifier>>) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    if let Some(task) = id.lock().unwrap_or_else(|e| e.into_inner()).take() {
        UIApplication::sharedApplication(mtm).endBackgroundTask(task);
    }
}
