use crate::base::histogram::Histogram;

/// Runtime statistics for Raft operations.
///
/// This is a volatile structure that is not persisted. It accumulates
/// statistics from the time the Raft node starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimeStats {
    /// Histogram tracking the distribution of log entry counts in Apply commands.
    ///
    /// This tracks how many log entries are included in each apply command sent
    /// to the state machine, helping identify batch size patterns and I/O efficiency.
    pub(crate) apply_batch_size: Histogram,
}

impl Default for RuntimeStats {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeStats {
    pub(crate) fn new() -> Self {
        Self {
            apply_batch_size: Histogram::new(),
        }
    }
}
