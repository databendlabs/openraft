use core::time::Duration;
use std::collections::BTreeSet;

use tokio::sync::watch;
use tokio::time::Instant;

use crate::core::ServerState;
use crate::metrics::RaftMetrics;
use crate::node::Node;
use crate::LogId;
use crate::LogIdOptionExt;
use crate::MessageSummary;
use crate::NodeId;

// Error variants related to metrics.
#[derive(Debug, thiserror::Error)]
pub enum WaitError {
    #[error("timeout after {0:?} when {1}")]
    Timeout(Duration, String),

    #[error("raft is shutting down")]
    ShuttingDown,
}

/// Wait is a wrapper of RaftMetrics channel that impls several utils to wait for metrics to satisfy some condition.
pub struct Wait<NID, N>
where
    NID: NodeId,
    N: Node,
{
    pub timeout: Duration,
    pub rx: watch::Receiver<RaftMetrics<NID, N>>,
}

impl<NID, N> Wait<NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// Wait for metrics to satisfy some condition or timeout.
    #[tracing::instrument(level = "trace", skip(self, func), fields(msg=%msg.to_string()))]
    pub async fn metrics<T>(&self, func: T, msg: impl ToString) -> Result<RaftMetrics<NID, N>, WaitError>
    where T: Fn(&RaftMetrics<NID, N>) -> bool + Send {
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
    pub async fn current_leader(&self, leader_id: NID, msg: impl ToString) -> Result<RaftMetrics<NID, N>, WaitError> {
        self.metrics(
            |x| x.current_leader == Some(leader_id),
            &format!("{} .current_leader -> {}", msg.to_string(), leader_id),
        )
        .await
    }

    /// Wait until applied exactly `want_log`(inclusive) logs or timeout.
    #[tracing::instrument(level = "trace", skip(self), fields(msg=msg.to_string().as_str()))]
    pub async fn log(&self, want_log_index: Option<u64>, msg: impl ToString) -> Result<RaftMetrics<NID, N>, WaitError> {
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
    pub async fn log_at_least(
        &self,
        want_log: Option<u64>,
        msg: impl ToString,
    ) -> Result<RaftMetrics<NID, N>, WaitError> {
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
    pub async fn state(&self, want_state: ServerState, msg: impl ToString) -> Result<RaftMetrics<NID, N>, WaitError> {
        self.metrics(
            |x| x.state == want_state,
            &format!("{} .state -> {:?}", msg.to_string(), want_state),
        )
        .await
    }

    /// Wait for `membership` to become the expected node id set or timeout.
    #[tracing::instrument(level = "trace", skip(self), fields(msg=msg.to_string().as_str()))]
    pub async fn members(
        &self,
        want_members: BTreeSet<NID>,
        msg: impl ToString,
    ) -> Result<RaftMetrics<NID, N>, WaitError> {
        self.metrics(
            |x| {
                let got = x.membership_config.nodes().map(|(nid, _)| *nid).collect::<BTreeSet<_>>();
                want_members == got
            },
            &format!("{} .members -> {:?}", msg.to_string(), want_members),
        )
        .await
    }

    /// Wait for `snapshot` to become `want_snapshot` or timeout.
    #[tracing::instrument(level = "trace", skip(self), fields(msg=msg.to_string().as_str()))]
    pub async fn snapshot(
        &self,
        want_snapshot: LogId<NID>,
        msg: impl ToString,
    ) -> Result<RaftMetrics<NID, N>, WaitError> {
        self.metrics(
            |x| x.snapshot == Some(want_snapshot),
            &format!("{} .snapshot -> {}", msg.to_string(), want_snapshot),
        )
        .await
    }
}
