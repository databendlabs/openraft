//! Raft metrics for observability.
//!
//! Applications may use this data in whatever way is needed. The obvious use cases are to expose
//! these metrics to a metrics collection system like Prometheus. Applications may also
//! use this data to trigger events within higher levels of the parent application.
//!
//! Metrics are observed on a running Raft node via the `Raft::metrics()` method, which will
//! return a stream of metrics.

use std::collections::BTreeSet;
use std::collections::HashMap;
use std::sync::Arc;

use serde::Deserialize;
use serde::Serialize;
use thiserror::Error;
use tokio::sync::watch;
use tokio::time::Duration;
use tokio::time::Instant;

use crate::core::EffectiveMembership;
use crate::core::State;
use crate::error::Fatal;
use crate::raft_types::LogIdOptionExt;
use crate::LogId;
use crate::Membership;
use crate::MessageSummary;
use crate::RaftTypeConfig;
use crate::ReplicationMetrics;

/// A set of metrics describing the current state of a Raft node.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RaftMetrics<C: RaftTypeConfig> {
    pub running_state: Result<(), Fatal<C>>,

    /// The ID of the Raft node.
    pub id: C::NodeId,
    /// The state of the Raft node.
    pub state: State,
    /// The current term of the Raft node.
    pub current_term: u64,
    /// The last log index has been appended to this Raft node's log.
    pub last_log_index: Option<u64>,
    /// The last log index has been applied to this Raft node's state machine.
    pub last_applied: Option<LogId<C>>,
    /// The current cluster leader.
    pub current_leader: Option<C::NodeId>,
    /// The current membership config of the cluster.
    pub membership_config: Arc<EffectiveMembership<C>>,

    /// The id of the last log included in snapshot.
    /// If there is no snapshot, it is (0,0).
    pub snapshot: Option<LogId<C>>,

    /// The metrics about the leader. It is Some() only when this node is leader.
    pub leader_metrics: Option<Arc<LeaderMetrics<C>>>,
}

impl<C: RaftTypeConfig> MessageSummary for RaftMetrics<C> {
    fn summary(&self) -> String {
        format!("Metrics{{id:{},{:?}, term:{}, last_log:{:?}, last_applied:{:?}, leader:{:?}, membership:{}, snapshot:{:?}, replication:{}",
            self.id,
            self.state,
            self.current_term,
            self.last_log_index,
            self.last_applied,
            self.current_leader,
            self.membership_config.summary(),
            self.snapshot,
            self.leader_metrics.as_ref().map(|x| x.summary()).unwrap_or_default(),
        )
    }
}

/// The metrics about the leader. It is Some() only when this node is leader.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaderMetrics<C: RaftTypeConfig> {
    /// Replication metrics of all known replication target: voters and learners
    pub replication: HashMap<C::NodeId, ReplicationMetrics<C>>,
}

impl<C: RaftTypeConfig> MessageSummary for LeaderMetrics<C> {
    fn summary(&self) -> String {
        let mut res = vec!["LeaderMetrics{".to_string()];
        for (i, (k, v)) in self.replication.iter().enumerate() {
            if i > 0 {
                res.push(",".to_string());
            }
            res.push(format!("{}:{}", k, v.summary()));
        }

        res.push("}".to_string());
        res.join("")
    }
}

impl<C: RaftTypeConfig> RaftMetrics<C> {
    pub(crate) fn new_initial(id: C::NodeId) -> Self {
        let membership_config = Membership::new_initial(id);
        Self {
            running_state: Ok(()),
            id,
            state: State::Follower,
            current_term: 0,
            last_log_index: None,
            last_applied: None,
            current_leader: None,
            membership_config: Arc::new(EffectiveMembership::new(LogId::default(), membership_config)),
            snapshot: None,
            leader_metrics: None,
        }
    }
}

// Error variants related to metrics.
#[derive(Debug, Error)]
pub enum WaitError {
    #[error("timeout after {0:?} when {1}")]
    Timeout(Duration, String),

    #[error("raft is shutting down")]
    ShuttingDown,
}

/// Wait is a wrapper of RaftMetrics channel that impls several utils to wait for metrics to satisfy some condition.
pub struct Wait<C: RaftTypeConfig> {
    pub timeout: Duration,
    pub rx: watch::Receiver<RaftMetrics<C>>,
}

impl<C: RaftTypeConfig> Wait<C> {
    /// Wait for metrics to satisfy some condition or timeout.
    #[tracing::instrument(level = "trace", skip(self, func), fields(msg=%msg.to_string()))]
    pub async fn metrics<T>(&self, func: T, msg: impl ToString) -> Result<RaftMetrics<C>, WaitError>
    where T: Fn(&RaftMetrics<C>) -> bool + Send {
        let timeout_at = Instant::now() + self.timeout;

        let mut rx = self.rx.clone();
        loop {
            let latest = rx.borrow().clone();

            tracing::debug!(
                "id={} wait {:} latest: {}",
                latest.id,
                msg.to_string(),
                latest.summary()
            );

            if func(&latest) {
                tracing::debug!(
                    "id={} done wait {:} latest: {}",
                    latest.id,
                    msg.to_string(),
                    latest.summary()
                );
                return Ok(latest);
            }

            let now = Instant::now();
            if now >= timeout_at {
                return Err(WaitError::Timeout(
                    self.timeout,
                    format!("{} latest: {:?}", msg.to_string(), latest),
                ));
            }

            let sleep_time = timeout_at - now;
            tracing::debug!(?sleep_time, "wait timeout");
            let delay = tokio::time::sleep(sleep_time);

            tokio::select! {
                _ = delay => {
                tracing::debug!( "id={} timeout wait {:} latest: {}", latest.id, msg.to_string(), latest.summary() );
                    return Err(WaitError::Timeout(self.timeout, format!("{} latest: {}", msg.to_string(), latest.summary())));
                }
                changed = rx.changed() => {
                    match changed {
                        Ok(_) => {
                            // metrics changed, continue the waiting loop
                        },
                        Err(err) => {
                            tracing::debug!(
                                "id={} error: {:?}; wait {:} latest: {:?}",
                                latest.id,
                                err,
                                msg.to_string(),
                                latest
                            );

                            return Err(WaitError::ShuttingDown);
                        }
                    }
                }
            };
        }
    }

    /// Wait for `current_leader` to become `Some(leader_id)` until timeout.
    #[tracing::instrument(level = "trace", skip(self), fields(msg=msg.to_string().as_str()))]
    pub async fn current_leader(&self, leader_id: C::NodeId, msg: impl ToString) -> Result<RaftMetrics<C>, WaitError> {
        self.metrics(
            |x| x.current_leader == Some(leader_id),
            &format!("{} .current_leader -> {}", msg.to_string(), leader_id),
        )
        .await
    }

    /// Wait until applied exactly `want_log`(inclusive) logs or timeout.
    #[tracing::instrument(level = "trace", skip(self), fields(msg=msg.to_string().as_str()))]
    pub async fn log(&self, want_log_index: Option<u64>, msg: impl ToString) -> Result<RaftMetrics<C>, WaitError> {
        self.metrics(
            |x| x.last_log_index == want_log_index,
            &format!("{} .last_log_index -> {:?}", msg.to_string(), want_log_index),
        )
        .await?;

        self.metrics(
            |x| x.last_applied.index() == want_log_index,
            &format!("{} .last_applied -> {:?}", msg.to_string(), want_log_index),
        )
        .await
    }

    /// Wait until applied at least `want_log`(inclusive) logs or timeout.
    #[tracing::instrument(level = "trace", skip(self), fields(msg=msg.to_string().as_str()))]
    pub async fn log_at_least(&self, want_log: Option<u64>, msg: impl ToString) -> Result<RaftMetrics<C>, WaitError> {
        self.metrics(
            |x| x.last_log_index >= want_log,
            &format!("{} .last_log_index >= {:?}", msg.to_string(), want_log),
        )
        .await?;

        self.metrics(
            |x| x.last_applied.index() >= want_log,
            &format!("{} .last_applied >= {:?}", msg.to_string(), want_log),
        )
        .await
    }

    /// Wait for `state` to become `want_state` or timeout.
    #[tracing::instrument(level = "trace", skip(self), fields(msg=msg.to_string().as_str()))]
    pub async fn state(&self, want_state: State, msg: impl ToString) -> Result<RaftMetrics<C>, WaitError> {
        self.metrics(
            |x| x.state == want_state,
            &format!("{} .state -> {:?}", msg.to_string(), want_state),
        )
        .await
    }

    /// Wait for `membership_config.members` to become expected node set or timeout.
    #[tracing::instrument(level = "trace", skip(self), fields(msg=msg.to_string().as_str()))]
    pub async fn members(
        &self,
        want_members: BTreeSet<C::NodeId>,
        msg: impl ToString,
    ) -> Result<RaftMetrics<C>, WaitError> {
        self.metrics(
            |x| x.membership_config.membership.get_ith_config(0).cloned().unwrap() == want_members,
            &format!("{} .membership_config.members -> {:?}", msg.to_string(), want_members),
        )
        .await
    }

    // TODO(xp): remove this
    /// Wait for `membership_config.members_after_consensus` to become expected node set or timeout.
    #[tracing::instrument(level = "trace", skip(self), fields(msg=msg.to_string().as_str()))]
    pub async fn next_members(
        &self,
        want_members: Option<BTreeSet<C::NodeId>>,
        msg: impl ToString,
    ) -> Result<RaftMetrics<C>, WaitError> {
        self.metrics(
            |x| x.membership_config.membership.get_ith_config(1) == want_members.as_ref(),
            &format!("{} .membership_config.next -> {:?}", msg.to_string(), want_members),
        )
        .await
    }

    /// Wait for `snapshot` to become `want_snapshot` or timeout.
    #[tracing::instrument(level = "trace", skip(self), fields(msg=msg.to_string().as_str()))]
    pub async fn snapshot(&self, want_snapshot: LogId<C>, msg: impl ToString) -> Result<RaftMetrics<C>, WaitError> {
        self.metrics(
            |x| x.snapshot == Some(want_snapshot),
            &format!("{} .snapshot -> {:?}", msg.to_string(), want_snapshot),
        )
        .await
    }
}
