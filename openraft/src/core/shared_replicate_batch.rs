use std::sync::Arc;
use std::sync::Mutex;

use crate::base::histogram::Histogram;

/// A shared histogram for tracking replication batch sizes.
///
/// This is shared between `RaftCore` and replication tasks.
/// Only `replicate_batch` needs to be shared across tasks;
/// other runtime stats are owned directly by `RaftCore`.
#[derive(Debug, Clone)]
pub struct SharedReplicateBatch {
    inner: Arc<Mutex<Histogram>>,
}

impl Default for SharedReplicateBatch {
    fn default() -> Self {
        Self::new()
    }
}

impl SharedReplicateBatch {
    pub(crate) fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Histogram::<()>::new())),
        }
    }

    /// Record a replication batch size.
    pub fn record(&self, value: u64) {
        let mut guard = self.inner.lock().unwrap();
        guard.record(value);
    }

    /// Get a clone of the histogram for reporting.
    #[cfg(feature = "runtime-stats")]
    pub(crate) fn snapshot(&self) -> Histogram {
        let guard = self.inner.lock().unwrap();
        guard.clone()
    }
}

#[cfg(all(test, feature = "runtime-stats"))]
mod tests {
    use super::*;

    #[test]
    fn test_new_and_default() {
        let s1 = SharedReplicateBatch::new();
        let s2 = SharedReplicateBatch::default();
        // Both should create valid instances
        s1.record(1);
        s2.record(1);
    }

    #[test]
    fn test_clone_shares_state() {
        let s1 = SharedReplicateBatch::new();
        let s2 = s1.clone();

        s1.record(100);
        let snapshot = s2.snapshot();

        assert_eq!(snapshot.total(), 1);
    }

    #[test]
    fn test_record_and_snapshot() {
        let state = SharedReplicateBatch::new();

        state.record(10);
        state.record(20);

        let snapshot = state.snapshot();
        assert_eq!(snapshot.total(), 2);
    }
}
