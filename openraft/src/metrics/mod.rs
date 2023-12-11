//! Raft metrics for observability.
//!
//! Applications may use this data in whatever way is needed. The obvious use cases are to expose
//! these metrics to a metrics collection system like Prometheus. Applications may also
//! use this data to trigger events within higher levels of the parent application.
//!
//! Metrics are observed on a running Raft node via the [`Raft::metrics() ->
//! watch::Receiver<RaftMetrics>`](`crate::Raft::metrics`) method, which will return a stream of
//! metrics.
//!
//!
//! ## [`RaftMetrics`]
//!
//! [`RaftMetrics`] contains useful information such as:
//!
//! - Server state(leader/follower/learner/candidate) of this raft node,
//! - The current leader,
//! - Last log and applied log.
//! - Replication state, if this node is a Leader,
//! - Snapshot state,
//! - etc.
//!
//! Metrics can be used as a trigger of application events, as a monitoring data
//! source, etc.
//!
//! Metrics is not a stream thus it only guarantees to provide the latest state but
//! not every change of the state.
//! Because internally, `watch::channel()` only stores one last state.

mod metric;
mod raft_metrics;
mod wait;

mod metric_display;
mod wait_condition;
#[cfg(test)] mod wait_test;

use std::collections::BTreeMap;

pub use metric::Metric;
pub use raft_metrics::RaftMetrics;
pub use wait::Wait;
pub use wait::WaitError;
pub(crate) use wait_condition::Condition;

use crate::LogId;

pub(crate) type ReplicationMetrics<NID> = BTreeMap<NID, Option<LogId<NID>>>;
