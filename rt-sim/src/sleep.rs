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
mod tests {
    use std::future::poll_fn;
    use std::time::Duration;

    use openraft_rt::AsyncRuntime;

    use super::*;
    use crate::SimRuntime;

    /// Creates a sleep in `rt` and polls it once, so its timer is pending.
    fn pending_sleep(rt: &mut SimRuntime, deadline: u64) -> SimSleep {
        rt.block_on(poll_fn(|cx| {
            let mut sleep = SimSleep::until(deadline);
            let polled = Pin::new(&mut sleep).poll(cx);
            assert_eq!(polled, Poll::Pending);
            Poll::Ready(sleep)
        }))
    }

    fn timer_events(rt: &SimRuntime) -> Vec<String> {
        let trace = rt.take_trace();
        let events = trace.into_iter().filter(|event| {
            let is_timer = event.starts_with("timer");
            let is_fire = event.starts_with("fire ");
            is_timer || is_fire
        });
        events.collect()
    }

    #[test]
    fn drop_outside_runtime_cancels_timer() {
        let mut rt = SimRuntime::with_seed(0);
        rt.record_trace(true);
        let sleep = pending_sleep(&mut rt, executor::EPOCH_NANOS + 1);
        drop(sleep);
        rt.block_on(async {
            SimRuntime::sleep(Duration::from_nanos(2)).await;
        });
        let events = timer_events(&rt);
        let expected = [
            "timer+ (1,0) by t0",
            "timer- (1,0) by exec",
            "timer+ (2,1) by t0",
            "fire (2,1)",
        ];
        assert_eq!(events, expected);
    }

    #[test]
    fn drop_in_another_runtime_cancels_only_owner_timer() {
        let mut owner = SimRuntime::with_seed(0);
        let mut other = SimRuntime::with_seed(0);
        owner.record_trace(true);
        other.record_trace(true);
        let sleep = pending_sleep(&mut owner, executor::EPOCH_NANOS + 1);
        let other_sleep = pending_sleep(&mut other, executor::EPOCH_NANOS + 1);
        other.block_on(async {
            drop(sleep);
            other_sleep.await;
        });
        let events = timer_events(&owner);
        let expected = ["timer+ (1,0) by t0", "timer- (1,0) by exec"];
        assert_eq!(events, expected);
        let events = timer_events(&other);
        let expected = ["timer+ (1,0) by t0", "fire (1,0)"];
        assert_eq!(events, expected);
    }

    #[test]
    #[should_panic(expected = "rt-sim: a sleep must be polled in the runtime that created it")]
    fn polling_in_another_runtime_panics() {
        let mut first = SimRuntime::with_seed(0);
        let mut second = SimRuntime::with_seed(0);
        let mut sleep = pending_sleep(&mut first, executor::EPOCH_NANOS + 1);
        second.block_on(&mut sleep);
    }

    #[test]
    fn sleep_does_not_keep_runtime_alive() {
        let mut rt = SimRuntime::with_seed(0);
        let owner = rt.block_on(async {
            let shared = executor::current();
            Arc::downgrade(&shared)
        });
        let sleep = pending_sleep(&mut rt, executor::EPOCH_NANOS + 1);
        drop(rt);
        let owner_alive = owner.upgrade().is_some();
        assert!(!owner_alive);
        drop(sleep);
    }
}
