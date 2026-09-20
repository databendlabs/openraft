//! Timers on the virtual clock.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Weak;
use std::task::Context;
use std::task::Poll;

use crate::executor;
use crate::executor::Shared;

#[derive(Debug)]
struct Registration {
    runtime: Weak<Shared>,
    seq: u64,
}

/// Completes once virtual time reaches its deadline.
///
/// The timer is registered on first poll and removed when the future is dropped before firing.
#[derive(Debug)]
pub struct SimSleep {
    deadline: u64,
    registration: Option<Registration>,
}

impl SimSleep {
    pub(crate) fn until(deadline: u64) -> Self {
        SimSleep {
            deadline,
            registration: None,
        }
    }

    fn cancel_registration(&mut self) {
        let Some(registration) = self.registration.take() else {
            return;
        };
        let Some(shared) = registration.runtime.upgrade() else {
            return;
        };
        shared.lock().cancel_timer(self.deadline, registration.seq);
    }
}

impl Future for SimSleep {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let shared = executor::current();
        let runtime = Arc::downgrade(&shared);
        if let Some(registration) = self.registration.as_ref() {
            let same_runtime = registration.runtime.ptr_eq(&runtime);
            if !same_runtime {
                self.cancel_registration();
            }
        }
        let mut st = shared.lock();

        if st.now >= self.deadline {
            if let Some(registration) = self.registration.take() {
                // Normally already removed by the fire; this covers a deadline reached by a later
                // advance while this future was not the one being woken.
                st.cancel_timer(self.deadline, registration.seq);
            }
            return Poll::Ready(());
        }

        let refreshed = match self.registration.as_ref() {
            Some(registration) => st.refresh_timer(self.deadline, registration.seq, cx.waker()),
            None => false,
        };
        if !refreshed {
            let seq = st.add_timer(self.deadline, cx.waker().clone());
            self.registration = Some(Registration { runtime, seq });
        }
        Poll::Pending
    }
}

impl Drop for SimSleep {
    fn drop(&mut self) {
        self.cancel_registration();
    }
}

#[cfg(test)]
mod sleep_test;
