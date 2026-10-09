//! The guard that installs a runtime on the current thread.

use std::sync::Arc;

use crate::executor::CURRENT;
use crate::executor::Shared;

/// Installs a runtime on this thread and removes it on drop, including during a panic.
pub(super) struct Enter;

impl Enter {
    pub(super) fn new(shared: Arc<Shared>) -> Self {
        Self::try_new(shared).expect("rt-sim: nested block_on is not supported")
    }

    /// Installs `shared` unless a runtime is already installed on this thread.
    pub(super) fn try_new(shared: Arc<Shared>) -> Option<Self> {
        CURRENT
            .try_with(|c| {
                let mut c = c.borrow_mut();
                if c.is_some() {
                    return false;
                }
                *c = Some(shared);
                true
            })
            .unwrap_or(false)
            .then_some(Enter)
    }
}

impl Drop for Enter {
    fn drop(&mut self) {
        let _ = CURRENT.try_with(|c| c.borrow_mut().take());
    }
}
