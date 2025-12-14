use std::sync::Arc;
use std::sync::Mutex;

use super::RuntimeStats;

/// A cheaply cloneable wrapper for shared access to [`RuntimeStats`].
///
/// This allows both `RaftCore` and the state machine worker to record
/// statistics concurrently.
#[derive(Debug, Clone)]
pub struct SharedRuntimeState {
    inner: Arc<Mutex<RuntimeStats>>,
}

impl Default for SharedRuntimeState {
    fn default() -> Self {
        Self::new()
    }
}

impl SharedRuntimeState {
    pub(crate) fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(RuntimeStats::new())),
        }
    }

    /// Access runtime stats mutably with a closure and return its result.
    ///
    /// The closure receives a mutable reference to [`RuntimeStats`] and can
    /// read or modify its fields. The return value of the closure is passed through.
    pub fn with_mut<F, R>(&self, f: F) -> R
    where F: FnOnce(&mut RuntimeStats) -> R {
        let mut guard = self.inner.lock().unwrap();
        f(&mut guard)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_and_default() {
        let s1 = SharedRuntimeState::new();
        let s2 = SharedRuntimeState::default();
        // Both should create valid instances
        s1.with_mut(|_| ());
        s2.with_mut(|_| ());
    }

    #[test]
    fn test_clone_shares_state() {
        let s1 = SharedRuntimeState::new();
        let s2 = s1.clone();

        s1.with_mut(|s| s.apply_batch.record(100));
        let count = s2.with_mut(|s| s.apply_batch.total());

        assert_eq!(count, 1);
    }

    #[test]
    fn test_with_mut_returns_value() {
        let state = SharedRuntimeState::new();

        state.with_mut(|s| {
            s.apply_batch.record(10);
            s.apply_batch.record(20);
        });

        let total = state.with_mut(|s| s.apply_batch.total());
        assert_eq!(total, 2);
    }
}
