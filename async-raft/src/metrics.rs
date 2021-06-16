//! Raft metrics for observability.
//!
//! Applications may use this data in whatever way is needed. The obvious use cases are to expose
//! these metrics to a metrics collection system like Prometheus. Applications may also
//! use this data to trigger events within higher levels of the parent application.
//!
//! Metrics are observed on a running Raft node via the `Raft::metrics()` method, which will
//! return a stream of metrics.

use std::collections::HashSet;

use serde::Deserialize;
use serde::Serialize;
use thiserror::Error;
use tokio::sync::watch;
use tokio::time::Duration;

use crate::core::State;
use crate::raft::MembershipConfig;
use crate::NodeId;
use crate::RaftError;

/// A set of metrics describing the current state of a Raft node.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RaftMetrics {
    /// The ID of the Raft node.
    pub id: NodeId,
    /// The state of the Raft node.
    pub state: State,
    /// The current term of the Raft node.
    pub current_term: u64,
    /// The last log index to be appended to this Raft node's log.
    pub last_log_index: u64,
    /// The last log index to be applied to this Raft node's state machine.
    pub last_applied: u64,
    /// The current cluster leader.
    pub current_leader: Option<NodeId>,
    /// The current membership config of the cluster.
    pub membership_config: MembershipConfig,
}

impl RaftMetrics {
    pub(crate) fn new_initial(id: NodeId) -> Self {
        let membership_config = MembershipConfig::new_initial(id);
        Self {
            id,
            state: State::Follower,
            current_term: 0,
            last_log_index: 0,
            last_applied: 0,
            current_leader: None,
            membership_config,
        }
    }
}

// Error variants related to metrics.
#[derive(Debug, Error)]
pub enum WaitError {
    #[error("timeout after {0:?} when {1}")]
    Timeout(Duration, String),

    #[error("{0}")]
    RaftError(#[from] RaftError),
}

/// Wait is a wrapper of RaftMetrics channel that impls several utils to wait for metrics to satisfy some condition.
pub struct Wait {
    pub timeout: Duration,
    pub rx: watch::Receiver<RaftMetrics>,
}

impl Wait {
    /// Wait for metrics to satisfy some condition or timeout.
    #[tracing::instrument(level = "debug", skip(self, func), fields(msg=msg.to_string().as_str()))]
    pub async fn metrics<T>(&self, func: T, msg: impl ToString) -> Result<RaftMetrics, WaitError>
    where T: Fn(&RaftMetrics) -> bool + Send {
        let mut rx = self.rx.clone();
        loop {
            let latest = rx.borrow().clone();

            tracing::debug!(
                "id={} wait {:} latest: {:?}",
                latest.id,
                msg.to_string(),
                latest
            );

            if func(&latest) {
                tracing::debug!(
                    "id={} done wait {:} latest: {:?}",
                    latest.id,
                    msg.to_string(),
                    latest
                );
                return Ok(latest);
            }

            let delay = tokio::time::sleep(self.timeout);

            tokio::select! {
                _ = delay => {
                tracing::debug!( "id={} timeout wait {:} latest: {:?}", latest.id, msg.to_string(), latest );
                    return Err(WaitError::Timeout(self.timeout, format!("{} latest: {:?}", msg.to_string(), latest)));
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
                            return Err(WaitError::RaftError(RaftError::ShuttingDown));
                        }
                    }
                }
            };
        }
    }

    /// Wait for `current_leader` to become `Some(leader_id)` until timeout.
    #[tracing::instrument(level = "debug", skip(self), fields(msg=msg.to_string().as_str()))]
    pub async fn current_leader(
        &self,
        leader_id: NodeId,
        msg: impl ToString,
    ) -> Result<RaftMetrics, WaitError> {
        self.metrics(
            |x| x.current_leader == Some(leader_id),
            &format!("{} .current_leader -> {}", msg.to_string(), leader_id),
        )
        .await
    }

    /// Wait until applied upto `want_log`(inclusive) logs or timeout.
    #[tracing::instrument(level = "debug", skip(self), fields(msg=msg.to_string().as_str()))]
    pub async fn log(&self, want_log: u64, msg: impl ToString) -> Result<RaftMetrics, WaitError> {
        self.metrics(
            |x| x.last_log_index == want_log,
            &format!("{} .last_log_index -> {}", msg.to_string(), want_log),
        )
        .await?;

        self.metrics(
            |x| x.last_applied == want_log,
            &format!("{} .last_applied -> {}", msg.to_string(), want_log),
        )
        .await
    }

    /// Wait for `state` to become `want_state` or timeout.
    #[tracing::instrument(level = "debug", skip(self), fields(msg=msg.to_string().as_str()))]
    pub async fn state(
        &self,
        want_state: State,
        msg: impl ToString,
    ) -> Result<RaftMetrics, WaitError> {
        self.metrics(
            |x| x.state == want_state,
            &format!("{} .state -> {:?}", msg.to_string(), want_state),
        )
        .await
    }

    /// Wait for `membership_config.members` to become expected node set or timeout.
    #[tracing::instrument(level = "debug", skip(self), fields(msg=msg.to_string().as_str()))]
    pub async fn members(
        &self,
        want_members: HashSet<NodeId>,
        msg: impl ToString,
    ) -> Result<RaftMetrics, WaitError> {
        self.metrics(
            |x| x.membership_config.members == want_members,
            &format!(
                "{} .membership_config.members -> {:?}",
                msg.to_string(),
                want_members
            ),
        )
        .await
    }

    /// Wait for `membership_config.members_after_consensus` to become expected node set or timeout.
    #[tracing::instrument(level = "debug", skip(self), fields(msg=msg.to_string().as_str()))]
    pub async fn next_members(
        &self,
        want_members: Option<HashSet<NodeId>>,
        msg: impl ToString,
    ) -> Result<RaftMetrics, WaitError> {
        self.metrics(
            |x| x.membership_config.members_after_consensus == want_members,
            &format!(
                "{} .membership_config.members_after_consensus -> {:?}",
                msg.to_string(),
                want_members
            ),
        )
        .await
    }
}
