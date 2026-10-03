//! The on/off switch of the tick loop.

use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::task::Poll;

use futures_util::task::AtomicWaker;

/// The on/off switch of the tick loop, shared with `RuntimeConfigHandle::tick()`.
///
/// While the switch is off, the tick loop parks on `waker` instead of waking every period to
/// re-check the flag, so a disabled tick causes no wakeups. `AtomicWaker` only stores the `Waker`
/// of the task that polls `wait_enabled()`, so this works with any `AsyncRuntime`.
pub(crate) struct TickSwitch {
    enabled: AtomicBool,
    waker: AtomicWaker,
}

impl TickSwitch {
    pub(crate) fn new(enabled: bool) -> Self {
        Self {
            enabled: AtomicBool::new(enabled),
            waker: AtomicWaker::new(),
        }
    }

    pub(crate) fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// Turn the tick on or off. Turning it on wakes a parked tick loop.
    ///
    /// The flag is stored before `wake()`, and `wait_enabled()` registers its waker before it
    /// loads the flag. `AtomicWaker` orders the two sides so that one of them always observes the
    /// other, and the wakeup cannot be lost.
    pub(crate) fn set(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
        if enabled {
            self.waker.wake();
        }
    }

    /// Resolve once the switch is on.
    ///
    /// Only the tick loop waits here, since `AtomicWaker` holds a single waker. A spurious wake
    /// just re-checks the flag.
    pub(crate) async fn wait_enabled(&self) {
        std::future::poll_fn(|cx| {
            self.waker.register(cx.waker());
            if self.is_enabled() {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        })
        .await
    }
}
