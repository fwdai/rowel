//! macOS: what the system posts when the screen locks, the machine sleeps or
//! wakes, and the user switches to another account.

use std::ptr::NonNull;

use block2::RcBlock;
use objc2_app_kit::{
    NSWorkspace, NSWorkspaceDidWakeNotification, NSWorkspaceSessionDidResignActiveNotification,
    NSWorkspaceWillSleepNotification,
};
use objc2_foundation::{
    NSDistributedNotificationCenter, NSNotification, NSNotificationCenter, NSNotificationName,
    NSString,
};

use super::{Event, Sink};

/// Posted when the screen locks: the lock shortcut, the Apple menu, a hot
/// corner, the screen saver or the display sleeping with a password required
/// to come back. In no header, but loginwindow has posted it since Mac OS X
/// 10.x, and it is what Electron's `lock-screen` and KeePassXC listen for.
const SCREEN_LOCKED: &str = "com.apple.screenIsLocked";

pub(super) fn watch(sink: Sink) {
    let distributed = NSDistributedNotificationCenter::defaultCenter();
    let locked = NSString::from_str(SCREEN_LOCKED);
    observe(&distributed, &locked, Event::Locked, &sink);

    let workspace = NSWorkspace::sharedWorkspace().notificationCenter();
    // SAFETY: AppKit's constants, there for the life of the process.
    let (resigned, sleep, wake) = unsafe {
        (
            NSWorkspaceSessionDidResignActiveNotification,
            NSWorkspaceWillSleepNotification,
            NSWorkspaceDidWakeNotification,
        )
    };
    observe(&workspace, resigned, Event::SwitchedAway, &sink);
    observe(&workspace, sleep, Event::Sleep, &sink);
    observe(&workspace, wake, Event::Wake, &sink);

    // Registering cannot fail, so this is heard from now on.
    sink(Event::Watching);
}

fn observe(center: &NSNotificationCenter, name: &NSNotificationName, event: Event, sink: &Sink) {
    let sink = sink.clone();
    let block = RcBlock::new(move |_: NonNull<NSNotification>| sink(event));
    // No queue: the block runs on the thread that posts, which for every one
    // of these is the main thread. The observer returned is what would remove
    // the registration; it is never removed, so it is kept for the life of the
    // process.
    //
    // SAFETY: no object to match, no queue, and a block that is `Send` (it
    // holds the sink, which is `Send + Sync`, and a `Copy` event).
    let observer = unsafe {
        center.addObserverForName_object_queue_usingBlock(Some(name), None, None, &block)
    };
    std::mem::forget(observer);
}
