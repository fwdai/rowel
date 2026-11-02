//! macOS: screen lock, sleep, wake and user-switch notifications.

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

/// Undocumented but long-standing; Electron and KeePassXC rely on it too.
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

    sink(Event::Watching);
}

fn observe(center: &NSNotificationCenter, name: &NSNotificationName, event: Event, sink: &Sink) {
    let sink = sink.clone();
    let block = RcBlock::new(move |_: NonNull<NSNotification>| sink(event));
    // Never removed, so the observer token is kept for the life of the process.
    // SAFETY: no object filter or queue, and the block is `Send`.
    let observer = unsafe {
        center.addObserverForName_object_queue_usingBlock(Some(name), None, None, &block)
    };
    std::mem::forget(observer);
}
