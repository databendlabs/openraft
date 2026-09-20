//! The lock around one runtime's state, shared with its wakers and timers.

use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::MutexGuard;

use crate::executor::TaskId;
use crate::executor::enter::Enter;
use crate::executor::state::State;
use crate::executor::task_future::TaskFuture;

/// Everything one runtime owns, shared with the wakers and timers of its tasks.
pub(crate) struct Shared {
    state: Mutex<State>,
}

impl Shared {
    pub(crate) fn new(seed: u64) -> Arc<Self> {
        Arc::new(Shared {
            state: Mutex::new(State::new(seed)),
        })
    }

    /// Locks the state. A panic inside a task never happens under this lock, so poisoning only
    /// follows a panic in the executor itself; the state is still consistent enough to drop.
    pub(crate) fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn set_seed(&self, seed: u64) {
        self.lock().seed = seed;
    }

    pub(crate) fn set_record(&self, record: bool) {
        self.lock().record = record;
    }

    pub(crate) fn take_trace(&self) -> Vec<String> {
        std::mem::take(&mut self.lock().trace)
    }

    /// Drops every remaining task with this runtime installed, so drop code that reads the clock
    /// or cancels timers still works. Tasks spawned while dropping are dropped too.
    pub(crate) fn drop_tasks(self: &Arc<Self>) {
        let _enter = Enter::try_new(self.clone());
        #[cfg(feature = "sim-log")]
        let _log = crate::sim_log::enter();
        loop {
            let tasks: Vec<_> = {
                let mut st = self.lock();
                // A panicked poll leaves `current` set; label the drops below as `exec`.
                st.current = None;
                st.run_queue.clear();
                std::mem::take(&mut st.tasks).into_values().flatten().collect()
            };
            if tasks.is_empty() {
                break;
            }
            drop(tasks);
        }
    }

    /// Adds a task and makes it runnable.
    pub(crate) fn spawn(&self, future: Pin<Box<dyn TaskFuture>>) -> TaskId {
        let mut st = self.lock();
        let id = st.next_task_id;
        st.next_task_id += 1;
        st.tasks.insert(id, Some(future));
        st.trace(|st| format!("spawn t{id} by {}", st.who()));
        st.enqueue(id);
        id
    }
}
