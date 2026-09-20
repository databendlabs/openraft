//! The single-threaded executor behind [`SimRuntime`](crate::SimRuntime).
//!
//! The `AsyncRuntime` trait is made of static functions (`spawn`, `sleep`, `Instant::now`), so the
//! executor is reached through a thread-local that [`block_on`] installs for the duration of a run.
//!
//! Scheduling is FIFO: a task is polled in the order it became runnable. When nothing is runnable,
//! virtual time jumps to the earliest pending timer; timers sharing a deadline fire in creation
//! order, keyed by `(deadline, seq)`. Every scheduling decision and timer fire is appended to a
//! trace, so two runs can be compared line by line.

mod enter;
mod shared;
mod state;
mod task_future;
mod task_waker;

use std::cell::RefCell;
use std::future::Future;
use std::sync::Arc;
use std::task::Context;
use std::task::Poll;

pub(crate) use shared::Shared;

use crate::executor::enter::Enter;
use crate::executor::task_waker::waker_for;

/// Identifies a task within one runtime. The future passed to `block_on` is task 0.
pub(crate) type TaskId = u64;

const MAIN_TASK: TaskId = 0;

/// Virtual time starts one day in, so code that subtracts a timeout from `now` does not underflow.
pub(crate) const EPOCH_NANOS: u64 = 86_400 * 1_000_000_000;

thread_local! {
    static CURRENT: RefCell<Option<Arc<Shared>>> = const { RefCell::new(None) };
}

/// The runtime installed on this thread. Panics outside `block_on`.
#[track_caller]
pub(crate) fn current() -> Arc<Shared> {
    try_current().expect("rt-sim: this must be called inside SimRuntime::block_on")
}

pub(crate) fn try_current() -> Option<Arc<Shared>> {
    CURRENT.try_with(|c| c.borrow().clone()).ok().flatten()
}

/// Runs `future` to completion, polling spawned tasks and advancing virtual time as needed.
pub(crate) fn block_on<F>(shared: &Arc<Shared>, future: F) -> F::Output
where F: Future {
    let _enter = Enter::new(shared.clone());
    #[cfg(feature = "sim-log")]
    let _log = crate::sim_log::enter();
    let mut future = std::pin::pin!(future);
    let main_waker = waker_for(shared, MAIN_TASK);

    {
        let mut st = shared.lock();
        st.current = None;
        st.trace(|_| "block_on".to_string());
        st.enqueue(MAIN_TASK);
    }

    loop {
        let next = shared.lock().pop_runnable();
        match next {
            Some(MAIN_TASK) => {
                shared.lock().begin_poll(MAIN_TASK);
                let polled = future.as_mut().poll(&mut Context::from_waker(&main_waker));
                if let Poll::Ready(output) = polled {
                    let mut st = shared.lock();
                    st.current = None;
                    st.trace(|_| format!("ready t{MAIN_TASK}"));
                    return output;
                }
            }
            Some(id) => poll_task(shared, id),
            None => {
                if !fire_next_timers(shared) {
                    let st = shared.lock();
                    panic!(
                        "rt-sim: deadlock at virtual time {}ns: the main future is pending, no task is runnable and \
                         no timer is pending ({} spawned tasks are blocked)",
                        st.now - EPOCH_NANOS,
                        st.tasks.len()
                    );
                }
            }
        }
    }
}

fn poll_task(shared: &Arc<Shared>, id: TaskId) {
    let taken = {
        let mut st = shared.lock();
        match st.tasks.get_mut(&id).and_then(Option::take) {
            Some(task) => {
                st.begin_poll(id);
                Some(task)
            }
            None => None,
        }
    };
    let Some(mut task) = taken else { return };

    let waker = waker_for(shared, id);
    let polled = task.as_mut().poll(&mut Context::from_waker(&waker));

    let mut st = shared.lock();
    st.current = None;
    match polled {
        Poll::Ready(()) => {
            st.tasks.remove(&id);
            st.trace(|_| format!("ready t{id}"));
            drop(st);
            // Dropped outside the lock: the task may own timers whose drop needs it.
            drop(task);
        }
        Poll::Pending => {
            if let Some(slot) = st.tasks.get_mut(&id) {
                *slot = Some(task);
            }
        }
    }
}

/// Jumps virtual time to the earliest pending timer and wakes every timer due at that instant, in
/// `(deadline, seq)` order. Returns `false` if there is no timer.
fn fire_next_timers(shared: &Arc<Shared>) -> bool {
    let wakers = {
        let mut st = shared.lock();
        st.current = None;
        let Some(&(deadline, _)) = st.timers.keys().next() else {
            return false;
        };
        if deadline > st.now {
            st.trace(|st| format!("advance {} -> {}", st.now - EPOCH_NANOS, deadline - EPOCH_NANOS));
            st.now = deadline;
            st.polls_at_instant = 0;
        }
        let now = st.now;
        let mut wakers = Vec::new();
        while let Some(entry) = st.timers.first_entry() {
            let (deadline, _) = *entry.key();
            if deadline > now {
                break;
            }
            let (key, waker) = entry.remove_entry();
            st.trace(|_| format!("fire ({},{})", key.0 - EPOCH_NANOS, key.1));
            wakers.push(waker);
        }
        wakers
    };
    for waker in wakers {
        waker.wake();
    }
    true
}
