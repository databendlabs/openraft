//! Wakers that put a task back on the run queue.

use std::sync::Arc;
use std::sync::Weak;
use std::task::Wake;
use std::task::Waker;

use crate::executor::Shared;
use crate::executor::TaskId;

struct TaskWaker {
    id: TaskId,
    shared: Weak<Shared>,
}

impl Wake for TaskWaker {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        if let Some(shared) = self.shared.upgrade() {
            shared.lock().wake(self.id);
        }
    }
}

pub(super) fn waker_for(shared: &Arc<Shared>, id: TaskId) -> Waker {
    Waker::from(Arc::new(TaskWaker {
        id,
        shared: Arc::downgrade(shared),
    }))
}
