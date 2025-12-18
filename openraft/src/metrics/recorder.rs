//! Metrics recording trait for external metrics collection.
//!
//! This module defines the [`MetricsRecorder`] trait that allows applications
//! to plug in custom metrics collection backends. The trait provides a minimal
//! interface for recording Raft internal events.
//!
//! # Example
//!
//! ```ignore
//! use std::sync::Arc;
//! use openraft::metrics::MetricsRecorder;
//!
//! #[derive(Debug)]
//! struct MyRecorder;
//!
//! impl MetricsRecorder for MyRecorder {
//!     fn record_apply_batch(&self, entry_count: u64) { /* ... */ }
//!     fn record_append_batch(&self, entry_count: u64) { /* ... */ }
//!     fn record_replicate_batch(&self, entry_count: u64) { /* ... */ }
//!     fn record_write_batch(&self, entry_count: u64) { /* ... */ }
//!     fn set_current_term(&self, term: u64) { /* ... */ }
//!     fn set_last_log_index(&self, index: u64) { /* ... */ }
//!     fn set_applied_index(&self, index: u64) { /* ... */ }
//!     fn set_snapshot_index(&self, index: u64) { /* ... */ }
//!     fn set_purged_index(&self, index: u64) { /* ... */ }
//!     fn set_server_state(&self, state: u8) { /* ... */ }
//!     fn increment_vote(&self) { /* ... */ }
//!     fn increment_heartbeat(&self) { /* ... */ }
//!     fn increment_append(&self) { /* ... */ }
//! }
//!
//! // Install in Raft instance
//! raft.set_metrics_recorder(Some(Arc::new(MyRecorder)));
//! ```

use crate::RaftTypeConfig;
use crate::core::ServerState;
use crate::metrics::RaftMetrics;
use crate::vote::RaftTerm;

/// Trait for recording Raft metrics to external systems.
///
/// Implement this trait to collect metrics from Raft internals and export them
/// to your preferred metrics backend (e.g., Prometheus, OpenTelemetry, StatsD).
///
/// All methods are called from the RaftCore task and should return quickly to
/// avoid blocking Raft operations.
pub trait MetricsRecorder: Send + Sync + std::fmt::Debug {
    // --- Histograms (batch sizes) ---

    /// Record a batch of log entries being applied to the state machine.
    fn record_apply_batch(&self, entry_count: u64);

    /// Record a batch of log entries being appended to storage.
    fn record_append_batch(&self, entry_count: u64);

    /// Record a batch of log entries in a replication RPC.
    fn record_replicate_batch(&self, entry_count: u64);

    /// Record a batch of client write entries merged together.
    fn record_write_batch(&self, entry_count: u64);

    // --- Gauges (current state) ---

    /// Set the current Raft term.
    fn set_current_term(&self, term: u64);

    /// Set the index of the last log entry.
    fn set_last_log_index(&self, index: u64);

    /// Set the index of the last applied log entry.
    fn set_applied_index(&self, index: u64);

    /// Set the index of the last snapshot.
    fn set_snapshot_index(&self, index: u64);

    /// Set the index of the last purged log entry.
    fn set_purged_index(&self, index: u64);

    /// Set the server state.
    ///
    /// States: 0=Learner, 1=Follower, 2=Candidate, 3=Leader, 4=Shutdown
    fn set_server_state(&self, state: u8);

    // --- Counters (operations) ---

    /// Increment the vote operation counter.
    fn increment_vote(&self);

    /// Increment the heartbeat operation counter.
    fn increment_heartbeat(&self);

    /// Increment the append entries operation counter.
    fn increment_append(&self);
}

/// Forward gauge metrics from `RaftMetrics` to a `MetricsRecorder`.
///
/// This helper function extracts the relevant fields from `RaftMetrics` and
/// calls the corresponding setter methods on the recorder. It only forwards
/// gauge metrics (current state values), not counters or histograms.
///
/// # Example
///
/// ```ignore
/// use openraft::metrics::{RaftMetrics, MetricsRecorder, forward_metrics};
///
/// fn on_metrics_update<C: RaftTypeConfig>(metrics: &RaftMetrics<C>, recorder: &dyn MetricsRecorder) {
///     forward_metrics(metrics, recorder);
/// }
/// ```
pub fn forward_metrics<C: RaftTypeConfig>(metrics: &RaftMetrics<C>, recorder: &dyn MetricsRecorder) {
    if let Some(term) = metrics.current_term.as_u64() {
        recorder.set_current_term(term);
    }

    if let Some(index) = metrics.last_log_index {
        recorder.set_last_log_index(index);
    }

    if let Some(ref applied) = metrics.last_applied {
        recorder.set_applied_index(applied.index());
    }

    if let Some(ref snapshot) = metrics.snapshot {
        recorder.set_snapshot_index(snapshot.index());
    }

    if let Some(ref purged) = metrics.purged {
        recorder.set_purged_index(purged.index());
    }

    recorder.set_server_state(match metrics.state {
        ServerState::Learner => 0,
        ServerState::Follower => 1,
        ServerState::Candidate => 2,
        ServerState::Leader => 3,
        ServerState::Shutdown => 4,
    });
}
