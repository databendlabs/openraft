//! Raft metrics for observability.
//!
//! Applications may use this data in whatever way is needed. The obvious use cases are to expose
//! these metrics to a metrics collection system like Prometheus. Applications may also
//! use this data to trigger events within higher levels of the parent application.
//!
//! [`Raft::metrics()`] returns a watch receiver of the latest [`RaftMetrics`] snapshot that the
//! Raft node published. The runtime configured as [`RaftTypeConfig::AsyncRuntime`] provides the
//! receiver type, and the [`WatchReceiver`] trait provides the methods that read the snapshot and
//! wait for a newer one. [`Raft::wait()`] returns a [`Wait`] that waits until a snapshot satisfies
//! a condition.
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
//! Each snapshot that the Raft node publishes replaces the previous one, so a reader observes the
//! latest state but can miss intermediate changes. Application events are therefore reactions to
//! observed snapshots. To act on a change, such as this node becoming the leader, compare each
//! observed snapshot with the last one the application acted on, because a notification of a new
//! snapshot says neither which field changed nor whether any did.
//!
//! [`Raft::metrics()`]: crate::Raft::metrics
//! [`Raft::wait()`]: crate::Raft::wait
//! [`RaftTypeConfig::AsyncRuntime`]: crate::RaftTypeConfig::AsyncRuntime
//! [`WatchReceiver`]: crate::async_runtime::WatchReceiver

mod metric;
mod raft_metrics;
mod replication_wait_error;
mod wait;

mod metric_display;
pub mod recorder;
mod serde_instant;
mod wait_condition;
#[cfg(test)]
mod wait_test;

use std::collections::BTreeMap;

pub use metric::Metric;
pub use raft_metrics::RaftDataMetrics;
pub use raft_metrics::RaftMetrics;
pub use raft_metrics::RaftServerMetrics;
pub use recorder::MetricsRecorder;
pub use recorder::forward_metrics;
pub use replication_wait_error::ReplicationWaitError;
pub use serde_instant::SerdeInstant;
pub use wait::Wait;
pub use wait::WaitError;
pub(crate) use wait_condition::Condition;

use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::NodeIdOf;
use crate::type_config::alias::SerdeInstantOf;

pub(crate) type ReplicationMetrics<C> = BTreeMap<NodeIdOf<C>, Option<LogIdOf<C>>>;
/// Heartbeat metrics, a mapping between a node's ID and the time of the last
/// acknowledged heartbeat or replication to this node.
pub(crate) type HeartbeatMetrics<C> = BTreeMap<NodeIdOf<C>, Option<SerdeInstantOf<C>>>;
