//! Metrics recording trait for external metrics collection.
//!
//! This module defines the [`MetricsRecorder`] trait that allows applications
//! to plug in custom metrics collection backends. The trait provides a minimal
//! interface for recording Raft internal events.
//!
//! # Example
//!
//! Implement the trait, then install the recorder with [`Raft::set_metrics_recorder`]. The setter
//! is async: it sends the recorder to `RaftCore` only when its returned future is awaited.
//! `Ok(())` means that the command is queued for `RaftCore`. The recorder is installed, and
//! receives callbacks, only after `RaftCore` handles the command.
//!
//! ```no_run
//! # #![deny(unused_must_use)]
//! use std::sync::Arc;
//!
//! use openraft::metrics::MetricsRecorder;
//! # use openraft::Raft;
//! # use openraft::RaftTypeConfig;
//! # use openraft::errors::Fatal;
//! # use openraft::storage::RaftStateMachine;
//!
//! #[derive(Debug)]
//! struct MyRecorder;
//!
//! impl MetricsRecorder for MyRecorder {
//!     fn record_apply_batch(&self, entry_count: u64) { /* ... */ }
//!     fn record_append_batch(&self, entry_count: u64) { /* ... */ }
//!     fn record_write_batch(&self, entry_count: u64) { /* ... */ }
//!     fn set_current_term(&self, term: u64) { /* ... */ }
//!     fn set_last_log_index(&self, index: u64) { /* ... */ }
//!     fn set_committed_index(&self, index: u64) { /* ... */ }
//!     fn set_applied_index(&self, index: u64) { /* ... */ }
//!     fn set_snapshot_index(&self, index: u64) { /* ... */ }
//!     fn set_purged_index(&self, index: u64) { /* ... */ }
//!     fn set_server_state(&self, state: u8) { /* ... */ }
//!     fn increment_vote(&self) { /* ... */ }
//!     fn increment_heartbeat(&self) { /* ... */ }
//!     fn increment_append(&self) { /* ... */ }
//! }
//!
//! # async fn install<C, SM>(raft: &Raft<C, SM>) -> Result<(), Fatal<C>>
//! # where
//! #     C: RaftTypeConfig,
//! #     SM: RaftStateMachine<C>,
//! # {
//! // Queue the recorder for RaftCore to install.
//! raft.set_metrics_recorder(Some(Arc::new(MyRecorder))).await?;
//! # Ok(())
//! # }
//! ```
//!
//! [`Raft::set_metrics_recorder`]: crate::Raft::set_metrics_recorder

use openraft_macros::since;

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
///
/// `RaftCore` sets the gauges through [`forward_metrics()`], which calls a gauge setter only when
/// it has a value to pass. A skipped setter leaves its gauge unchanged.
pub trait MetricsRecorder: Send + Sync + std::fmt::Debug {
    // --- Histograms (batch sizes) ---

    /// Record the number of log entries the state machine receives in one `apply()` call.
    ///
    /// Called once per [`RaftStateMachine::apply()`] call, after it succeeds. The state-machine
    /// worker merges queued apply commands into one batch, so one call may cover several
    /// commands.
    ///
    /// [`RaftStateMachine::apply()`]: crate::storage::RaftStateMachine::apply
    #[since(version = "0.10.1", change = "called per `apply()` call, not per command")]
    fn record_apply_batch(&self, entry_count: u64);

    /// Record a batch of log entries being appended to storage.
    fn record_append_batch(&self, entry_count: u64);

    /// Record a batch of client write entries merged together.
    fn record_write_batch(&self, entry_count: u64);

    // --- Gauges (current state) ---

    /// Set the current Raft term.
    ///
    /// Not called when [`RaftTerm::as_u64()`] cannot convert the term to `u64`.
    fn set_current_term(&self, term: u64);

    /// Set the index of the last log entry.
    fn set_last_log_index(&self, index: u64);

    /// Set the index of the last log entry that this node knows to be committed.
    ///
    /// The index comes from [`RaftMetrics::local_committed`], which can lag
    /// [`RaftMetrics::cluster_committed`].
    fn set_committed_index(&self, index: u64);

    /// Set the index of the last applied log entry.
    fn set_applied_index(&self, index: u64);

    /// Set the index of the last snapshot.
    fn set_snapshot_index(&self, index: u64);

    /// Set the index of the last purged log entry.
    fn set_purged_index(&self, index: u64);

    /// Set the server state.
    ///
    /// States: 0=Learner, 1=Follower, 2=Candidate, 3=Leader, 4=Shutdown. When the node stops, the
    /// last call sets 4.
    #[since(version = "0.10.2", change = "called with 4, `Shutdown`, when the node stops")]
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
/// It calls [`set_server_state()`] every time, and each other setter only when it has a value to
/// pass:
///
/// - [`set_current_term()`] receives [`current_term`] when [`RaftTerm::as_u64()`] converts it. The
///   `RaftTerm` implementations for integer types return `None` for a negative term or one above
///   `u64::MAX`.
/// - [`set_last_log_index()`] receives [`last_log_index`] when it is `Some`.
/// - [`set_committed_index()`] receives the index of [`local_committed`] when it is `Some`.
/// - [`set_applied_index()`] receives the index of [`last_applied`] when it is `Some`.
/// - [`set_snapshot_index()`] receives the index of [`snapshot`] when it is `Some`.
/// - [`set_purged_index()`] receives the index of [`purged`] when it is `Some`.
///
/// Forwarding never clears a gauge: a skipped setter leaves the recorder's value as it was. For
/// example, forwarding [`RaftMetrics::new_initial()`], whose log fields are all `None`, to a
/// recorder that already holds values leaves its index gauges as they were. A call with index 0
/// reports the first log entry, not an absent value. To tell an absent value from a retained one,
/// read [`RaftMetrics`] itself, such as from [`Raft::metrics()`].
///
/// # Example
///
/// ```
/// use openraft::RaftTypeConfig;
/// use openraft::metrics::MetricsRecorder;
/// use openraft::metrics::RaftMetrics;
/// use openraft::metrics::forward_metrics;
///
/// fn on_metrics_update<C>(metrics: &RaftMetrics<C>, recorder: &dyn MetricsRecorder)
/// where C: RaftTypeConfig {
///     forward_metrics(metrics, recorder);
/// }
/// ```
///
/// [`set_server_state()`]: MetricsRecorder::set_server_state
/// [`set_current_term()`]: MetricsRecorder::set_current_term
/// [`set_last_log_index()`]: MetricsRecorder::set_last_log_index
/// [`set_committed_index()`]: MetricsRecorder::set_committed_index
/// [`set_applied_index()`]: MetricsRecorder::set_applied_index
/// [`set_snapshot_index()`]: MetricsRecorder::set_snapshot_index
/// [`set_purged_index()`]: MetricsRecorder::set_purged_index
/// [`current_term`]: RaftMetrics::current_term
/// [`last_log_index`]: RaftMetrics::last_log_index
/// [`local_committed`]: RaftMetrics::local_committed
/// [`last_applied`]: RaftMetrics::last_applied
/// [`snapshot`]: RaftMetrics::snapshot
/// [`purged`]: RaftMetrics::purged
/// [`Raft::metrics()`]: crate::Raft::metrics
pub fn forward_metrics<C: RaftTypeConfig>(metrics: &RaftMetrics<C>, recorder: &dyn MetricsRecorder) {
    if let Some(term) = metrics.current_term.as_u64() {
        recorder.set_current_term(term);
    }

    if let Some(index) = metrics.last_log_index {
        recorder.set_last_log_index(index);
    }

    if let Some(ref committed) = metrics.local_committed {
        recorder.set_committed_index(committed.index());
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
