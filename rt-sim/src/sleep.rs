//! Timers on the virtual clock.

use std::future::Future;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;

use crate::executor;

#[derive(Debug, Clone, Copy)]
struct Registration {
    runtime: u64,
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
}

impl Future for SimSleep {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let shared = executor::current();
        let mut st = shared.lock();

        if st.now >= self.deadline {
            if let Some(registration) = self.registration.take()
                && registration.runtime == shared.id
            {
                // Normally already removed by the fire; this covers a deadline reached by a later
                // advance while this future was not the one being woken.
                st.cancel_timer(self.deadline, registration.seq);
            }
            return Poll::Ready(());
        }

        let refreshed = match self.registration {
            Some(registration) if registration.runtime == shared.id => {
                st.refresh_timer(self.deadline, registration.seq, cx.waker())
            }
            _ => false,
        };
        if !refreshed {
            let seq = st.add_timer(self.deadline, cx.waker().clone());
            self.registration = Some(Registration {
                runtime: shared.id,
                seq,
            });
        }
        Poll::Pending
    }
}

impl Drop for SimSleep {
    fn drop(&mut self) {
        let Some(registration) = self.registration.take() else {
            return;
        };
        let Some(shared) = executor::try_current() else {
            return;
        };
        if shared.id == registration.runtime {
            shared.lock().cancel_timer(self.deadline, registration.seq);
        }
    }
}
