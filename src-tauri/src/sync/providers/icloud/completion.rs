//! A one-shot future a completion handler resolves.
//!
//! CloudKit answers every call on a private serial queue, through a block. The
//! store's methods are `async`, driven with `block_on` on a blocking-pool
//! thread (see `sync::store`), so what they need is a future that the block can
//! complete from CloudKit's thread: the block holds one clone, the awaiting
//! method the other. Nothing is spun or polled in a loop; the block wakes the
//! task.
//!
//! The first value wins. CloudKit calls each handler once, but a handler that
//! did call twice would otherwise have the second value sit in the slot for
//! nobody, and a `resolve` after the await has returned is harmless either way.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll, Waker};

pub(super) struct Completion<T>(Arc<Mutex<Slot<T>>>);

struct Slot<T> {
    value: Option<T>,
    waker: Option<Waker>,
}

impl<T> Clone for Completion<T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<T> Completion<T> {
    pub(super) fn new() -> Self {
        Self(Arc::new(Mutex::new(Slot {
            value: None,
            waker: None,
        })))
    }

    /// Hand the value to whoever awaits, waking them if they already are.
    /// A second call is ignored: the first answer is the answer.
    pub(super) fn resolve(&self, value: T) {
        let mut slot = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if slot.value.is_some() {
            return;
        }
        slot.value = Some(value);
        let waker = slot.waker.take();
        // Woken outside the lock: the task may poll on this very thread.
        drop(slot);
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

impl<T> Future for Completion<T> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<T> {
        let mut slot = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        match slot.value.take() {
            Some(value) => Poll::Ready(value),
            None => {
                slot.waker = Some(cx.waker().clone());
                Poll::Pending
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;
    use tauri::async_runtime::block_on;

    // CloudKit may answer before the caller gets as far as awaiting: the value
    // waits in the slot rather than being lost.
    #[test]
    fn a_value_resolved_before_the_await_is_still_delivered() {
        let done = Completion::new();
        done.resolve(7);
        assert_eq!(block_on(done), 7);
    }

    // The ordinary case: the handler fires on another thread while the task is
    // parked, and the task wakes with the value.
    #[test]
    fn a_value_resolved_from_another_thread_wakes_the_task() {
        let done = Completion::new();
        let handler = done.clone();
        let fired = thread::spawn(move || {
            thread::sleep(Duration::from_millis(50));
            handler.resolve("answered");
        });
        assert_eq!(block_on(done), "answered");
        fired.join().unwrap();
    }

    #[test]
    fn the_first_answer_wins() {
        let done = Completion::new();
        done.resolve(1);
        done.resolve(2);
        assert_eq!(block_on(done), 1);
    }
}
