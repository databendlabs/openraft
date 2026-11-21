#[cfg(feature = "runtime-stats")]
use std::sync::Arc;
#[cfg(feature = "runtime-stats")]
use std::sync::Mutex;

#[cfg(feature = "runtime-stats")]
use base2histogram::Histogram;

/// A shared histogram for tracking replication batch sizes.
///
/// This is shared between `RaftCore` and replication tasks.
/// Only `replicate_batch` needs to be shared across tasks;
/// other runtime stats are owned directly by `RaftCore`.
/// Without `runtime-stats`, the handle stores no histogram.
#[derive(Debug, Clone)]
pub struct SharedReplicateBatch {
    #[cfg(feature = "runtime-stats")]
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
            #[cfg(feature = "runtime-stats")]
            inner: Arc::new(Mutex::new(Histogram::<()>::new())),
        }
    }

    /// Record a replication batch size if `runtime-stats` is enabled.
    pub fn record(&self, value: u64) {
        #[cfg(feature = "runtime-stats")]
        {
            let mut guard = self.inner.lock().unwrap();
            guard.record(value);
        }
        #[cfg(not(feature = "runtime-stats"))]
        let _ = value;
    }

    /// Get a clone of the histogram for reporting.
    #[cfg(feature = "runtime-stats")]
    pub(crate) fn snapshot(&self) -> Histogram {
        let guard = self.inner.lock().unwrap();
        guard.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(feature = "runtime-stats"))]
    #[test]
    fn test_disabled_handle_is_zero_sized() {
        assert_eq!(size_of::<SharedReplicateBatch>(), 0);
    }

    #[cfg(feature = "runtime-stats")]
    #[test]
    fn test_new_and_default() {
        let s1 = SharedReplicateBatch::new();
        let s2 = SharedReplicateBatch::default();
        // Both should create valid instances
        s1.record(1);
        s2.record(1);
    }

    #[cfg(feature = "runtime-stats")]
    #[test]
    fn test_clone_shares_state() {
        let s1 = SharedReplicateBatch::new();
        let s2 = s1.clone();

        s1.record(100);
        s2.record(200);
        let snapshot = s2.snapshot();

        assert_eq!(snapshot.total(), 2);
    }

    #[cfg(feature = "runtime-stats")]
    #[test]
    fn test_record_and_snapshot() {
        let state = SharedReplicateBatch::new();

        state.record(10);
        state.record(20);

        let snapshot = state.snapshot();
        assert_eq!(snapshot.total(), 2);
    }
}
