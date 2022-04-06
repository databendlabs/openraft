use core::time::Duration;
use std::collections::BTreeSet;

use tokio::sync::watch;
use tokio::time::Instant;

use crate::core::State;
use crate::metrics::RaftMetrics;
use crate::raft::RaftTypeConfig;
use crate::LogId;
use crate::LogIdOptionExt;
use crate::MessageSummary;

// Error variants related to metrics.
#[derive(Debug, thiserror::Error)]
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
    /// TODO(xp): this method wait for a uniform config. There should be method waiting for a generalized config.
    #[tracing::instrument(level = "trace", skip(self), fields(msg=msg.to_string().as_str()))]
    pub async fn members(
        &self,
        want_members: BTreeSet<C::NodeId>,
        msg: impl ToString,
    ) -> Result<RaftMetrics<C>, WaitError> {
        self.metrics(
            |x| {
                let configs = x.membership_config.get_configs();
                let first = configs.get(0);
                first == Some(&want_members)
            },
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
            |x| x.membership_config.membership.get_configs().get(1) == want_members.as_ref(),
            &format!("{} .membership_config.next -> {:?}", msg.to_string(), want_members),
        )
        .await
    }

    /// Wait for `snapshot` to become `want_snapshot` or timeout.
    #[tracing::instrument(level = "trace", skip(self), fields(msg=msg.to_string().as_str()))]
    pub async fn snapshot(
        &self,
        want_snapshot: LogId<C::NodeId>,
        msg: impl ToString,
    ) -> Result<RaftMetrics<C>, WaitError> {
        self.metrics(
            |x| x.snapshot == Some(want_snapshot),
            &format!("{} .snapshot -> {:?}", msg.to_string(), want_snapshot),
        )
        .await
    }
}
