//! Raft metrics for observability.
//!
//! Applications may use this data in whatever way is needed. The obvious use cases are to expose
//! these metrics to a metrics collection system like Prometheus. Applications may also
//! use this data to trigger events within higher levels of the parent application.
//!
//! Metrics are observed on a running Raft node via the `Raft::metrics()` method, which will
//! return a stream of metrics.

mod raft_metrics;
mod replication_metrics;
mod wait;

#[cfg(test)] mod replication_metrics_test;
#[cfg(test)] mod wait_test;

pub use raft_metrics::RaftMetrics;
pub use replication_metrics::ReplicationMetrics;
pub use replication_metrics::ReplicationTargetMetrics;
pub(crate) use replication_metrics::UpdateMatchedLogId;
pub use wait::Wait;
pub use wait::WaitError;
