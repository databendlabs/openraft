//! Timers on the virtual clock.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Weak;
use std::task::Context;
use std::task::Poll;

use crate::executor;
use crate::executor::Shared;

/// Completes once virtual time reaches its deadline.
///
/// The timer gets its sequence number when the future is created, is added on first poll, and is
/// removed when the future is dropped before firing. The future must be polled in the runtime that
/// created it.
#[derive(Debug)]
pub struct SimSleep {
    deadline: u64,
    seq: u64,
    runtime: Weak<Shared>,
}

impl SimSleep {
    #[track_caller]
    pub(crate) fn until(deadline: u64) -> Self {
        let shared = executor::current();
        let seq = shared.lock().new_timer_seq();
        SimSleep {
            deadline,
            seq,
            runtime: Arc::downgrade(&shared),
        }
    }
}

impl Future for SimSleep {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let shared = executor::current();
        let runtime = Arc::downgrade(&shared);
        let same_runtime = self.runtime.ptr_eq(&runtime);
        assert!(
            same_runtime,
            "rt-sim: a sleep must be polled in the runtime that created it"
        );
        let mut st = shared.lock();

        if st.now >= self.deadline {
            return Poll::Ready(());
        }

        st.set_timer(self.deadline, self.seq, cx.waker());
        Poll::Pending
    }
}

impl Drop for SimSleep {
    fn drop(&mut self) {
        let Some(shared) = self.runtime.upgrade() else {
            return;
        };
        shared.lock().cancel_timer(self.deadline, self.seq);
    }
}

#[cfg(test)]
mod sleep_test;
