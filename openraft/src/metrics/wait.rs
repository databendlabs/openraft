use core::time::Duration;
use std::collections::BTreeSet;

use futures::FutureExt;

use crate::OptionalSend;
use crate::RaftTypeConfig;
use crate::async_runtime::watch::WatchReceiver;
use crate::core::ServerState;
use crate::metrics::Condition;
use crate::metrics::Metric;
use crate::metrics::RaftMetrics;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::VoteOf;
use crate::type_config::alias::WatchReceiverOf;

/// Error variants related to waiting for metrics conditions.
#[derive(Debug, thiserror::Error)]
pub enum WaitError {
    /// Timeout occurred while waiting for a condition.
    #[error("timeout after {0:?} when {1}")]
    Timeout(Duration, String),

    /// Raft node is shutting down.
    #[error("raft is shutting down")]
    ShuttingDown,
}

/// Wait is a wrapper of RaftMetrics channel that impls several utils to wait for metrics to satisfy
/// some condition.
pub struct Wait<C: RaftTypeConfig> {
    /// The timeout duration for waiting operations.
    pub timeout: Duration,
    /// The metrics receiver channel.
    pub rx: WatchReceiverOf<C, RaftMetrics<C>>,
}

impl<C> Wait<C>
where C: RaftTypeConfig
{
    /// Wait for metrics to satisfy some condition or timeout.
    #[tracing::instrument(level = "trace", skip(self, func), fields(msg=%msg.to_string()))]
    pub async fn metrics<T>(&self, func: T, msg: impl ToString) -> Result<RaftMetrics<C>, WaitError>
    where T: Fn(&RaftMetrics<C>) -> bool + OptionalSend {
        let timeout_at = C::now() + self.timeout;

        let mut rx = self.rx.clone();
        loop {
            let latest = rx.borrow_watched().clone();

            tracing::debug!("id={} wait {:} latest: {}", latest.id, msg.to_string(), latest);

            if func(&latest) {
                tracing::debug!("id={} done wait {:} latest: {}", latest.id, msg.to_string(), latest);
                return Ok(latest);
            }

            let now = C::now();
            if now >= timeout_at {
                return Err(WaitError::Timeout(
                    self.timeout,
                    format!("{} latest: {}", msg.to_string(), latest),
                ));
            }

            let sleep_time = timeout_at - now;
            tracing::debug!(?sleep_time, "wait timeout");
            let delay = C::sleep(sleep_time);

            futures::select_biased! {
                _ = delay.fuse() => {
                    tracing::debug!( "id={} timeout wait {:} latest: {}", latest.id, msg.to_string(), latest );
                    return Err(WaitError::Timeout(self.timeout, format!("{} latest: {}", msg.to_string(), latest)));
                }
                changed = rx.changed().fuse() => {
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
            }
        }
    }

    /// Wait for `vote` to become `want` or timeout.
    #[tracing::instrument(level = "trace", skip(self), fields(msg=msg.to_string().as_str()))]
    pub async fn vote(&self, want: VoteOf<C>, msg: impl ToString) -> Result<RaftMetrics<C>, WaitError> {
        self.eq(Metric::Vote(want), msg).await
    }

    /// Wait for `current_leader` to become `Some(leader_id)` until timeout.
    #[tracing::instrument(level = "trace", skip(self), fields(msg=msg.to_string().as_str()))]
    pub async fn current_leader(&self, leader_id: C::NodeId, msg: impl ToString) -> Result<RaftMetrics<C>, WaitError> {
        self.metrics(
            |m| m.current_leader.as_ref() == Some(&leader_id),
            &format!("{} .current_leader == {}", msg.to_string(), leader_id),
        )
        .await
    }

    /// Wait until applied exactly `want_log`(inclusive) logs or timeout.
    #[deprecated(since = "0.9.0", note = "use `log_index()` and `applied_index()` instead")]
    #[tracing::instrument(level = "trace", skip(self), fields(msg=msg.to_string().as_str()))]
    pub async fn log(&self, want_log_index: Option<u64>, msg: impl ToString) -> Result<RaftMetrics<C>, WaitError> {
        self.eq(Metric::LastLogIndex(want_log_index), msg.to_string()).await?;
        self.eq(Metric::AppliedIndex(want_log_index), msg.to_string()).await
    }

    /// Wait until applied at least `want_log`(inclusive) logs or timeout.
    #[deprecated(
        since = "0.9.0",
        note = "use `log_index_at_least()` and `applied_index_at_least()` instead"
    )]
    #[tracing::instrument(level = "trace", skip(self), fields(msg=msg.to_string().as_str()))]
    pub async fn log_at_least(&self, want_log: Option<u64>, msg: impl ToString) -> Result<RaftMetrics<C>, WaitError> {
        self.ge(Metric::LastLogIndex(want_log), msg.to_string()).await?;
        self.ge(Metric::AppliedIndex(want_log), msg.to_string()).await
    }

    /// Block until the last log index becomes exactly `index` (inclusive) or timeout.
    #[tracing::instrument(level = "trace", skip(self), fields(msg=msg.to_string().as_str()))]
    pub async fn log_index(&self, index: Option<u64>, msg: impl ToString) -> Result<RaftMetrics<C>, WaitError> {
        self.eq(Metric::LastLogIndex(index), msg).await
    }

    /// Block until the last log index becomes at least `index` (inclusive) or timeout.
    #[tracing::instrument(level = "trace", skip(self), fields(msg=msg.to_string().as_str()))]
    pub async fn log_index_at_least(
        &self,
        index: Option<u64>,
        msg: impl ToString,
    ) -> Result<RaftMetrics<C>, WaitError> {
        self.ge(Metric::LastLogIndex(index), msg).await
    }

    /// Block until the applied index becomes exactly `index` (inclusive) or timeout.
    #[tracing::instrument(level = "trace", skip(self), fields(msg=msg.to_string().as_str()))]
    pub async fn applied_index(&self, index: Option<u64>, msg: impl ToString) -> Result<RaftMetrics<C>, WaitError> {
        self.eq(Metric::AppliedIndex(index), msg).await
    }

    /// Block until the last applied log index become at least `index` (inclusive) or timeout.
    /// Note that this also implies `last_log_id >= index`.
    #[tracing::instrument(level = "trace", skip(self), fields(msg=msg.to_string().as_str()))]
    pub async fn applied_index_at_least(
        &self,
        index: Option<u64>,
        msg: impl ToString,
    ) -> Result<RaftMetrics<C>, WaitError> {
        self.ge(Metric::AppliedIndex(index), msg).await
    }

    /// Wait for `state` to become `want_state` or timeout.
    #[tracing::instrument(level = "trace", skip(self), fields(msg=msg.to_string().as_str()))]
    pub async fn state(&self, want_state: ServerState, msg: impl ToString) -> Result<RaftMetrics<C>, WaitError> {
        self.metrics(
            |m| m.state == want_state,
            &format!("{} .state == {:?}", msg.to_string(), want_state),
        )
        .await
    }

    /// Wait for `membership` to become the expected node id set or timeout.
    #[deprecated(since = "0.9.0", note = "use `voter_ids()` instead")]
    #[tracing::instrument(level = "trace", skip(self), fields(msg=msg.to_string().as_str()))]
    pub async fn members(
        &self,
        want_members: BTreeSet<C::NodeId>,
        msg: impl ToString,
    ) -> Result<RaftMetrics<C>, WaitError> {
        self.metrics(
            |m| {
                let got = m.membership_config.membership().voter_ids().collect::<BTreeSet<_>>();
                want_members == got
            },
            &format!("{} .members -> {:?}", msg.to_string(), want_members),
        )
        .await
    }

    /// Block until membership contains exact the expected `voter_ids` or timeout.
    #[tracing::instrument(level = "trace", skip_all, fields(msg=msg.to_string().as_str()))]
    pub async fn voter_ids(
        &self,
        voter_ids: impl IntoIterator<Item = C::NodeId>,
        msg: impl ToString,
    ) -> Result<RaftMetrics<C>, WaitError> {
        let want = voter_ids.into_iter().collect::<BTreeSet<_>>();

        tracing::debug!("block until voter_ids == {:?}", want);

        self.metrics(
            |m| {
                let got = m.membership_config.membership().voter_ids().collect();
                want == got
            },
            &format!("{} .members == {:?}", msg.to_string(), want),
        )
        .await
    }

    /// Wait for `snapshot` to become `snapshot_last_log_id` or timeout.
    #[tracing::instrument(level = "trace", skip(self), fields(msg=msg.to_string().as_str()))]
    pub async fn snapshot(
        &self,
        snapshot_last_log_id: LogIdOf<C>,
        msg: impl ToString,
    ) -> Result<RaftMetrics<C>, WaitError> {
        self.eq(Metric::Snapshot(Some(snapshot_last_log_id)), msg).await
    }

    /// Wait for `purged` to become `want` or timeout.
    #[tracing::instrument(level = "trace", skip(self), fields(msg=msg.to_string().as_str()))]
    pub async fn purged(&self, want: Option<LogIdOf<C>>, msg: impl ToString) -> Result<RaftMetrics<C>, WaitError> {
        self.eq(Metric::Purged(want), msg).await
    }

    /// Block until a metric becomes greater than or equal the specified value or timeout.
    ///
    /// For example, to await until the term becomes 2 or greater:
    /// ```ignore
    /// my_raft.wait(None).ge(Metric::Term(2), "become term 2").await?;
    /// ```
    pub async fn ge(&self, metric: Metric<C>, msg: impl ToString) -> Result<RaftMetrics<C>, WaitError> {
        self.until(Condition::ge(metric), msg).await
    }

    /// Block until a metric becomes equal to the specified value or timeout.
    ///
    /// For example, to await until the term becomes exact 2:
    /// ```ignore
    /// my_raft.wait(None).eq(Metric::Term(2), "become term 2").await?;
    /// ```
    pub async fn eq(&self, metric: Metric<C>, msg: impl ToString) -> Result<RaftMetrics<C>, WaitError> {
        self.until(Condition::eq(metric), msg).await
    }

    /// Block until a metric satisfies the specified condition or timeout.
    #[tracing::instrument(level = "trace", skip_all, fields(cond=cond.to_string(), msg=msg.to_string().as_str()))]
    pub(crate) async fn until(&self, cond: Condition<C>, msg: impl ToString) -> Result<RaftMetrics<C>, WaitError> {
        self.metrics(
            |raft_metrics| match &cond {
                Condition::GE(expect) => raft_metrics >= expect,
                Condition::EQ(expect) => raft_metrics == expect,
            },
            &format!("{} .{}", msg.to_string(), cond),
        )
        .await
    }
}
