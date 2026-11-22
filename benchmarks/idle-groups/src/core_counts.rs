//! The counters of `RuntimeStats` that show how often the members wake for a timer and publish
//! their metrics.

use crate::Raft;

/// Counters of `RuntimeStats`, summed over many members.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct CoreCounts {
    /// The deadlines reached: the heartbeats, elections and pending-read checks that fired.
    pub(crate) fires: u64,

    /// The wakes that `RaftCore` gave a timer's task to re-arm it.
    pub(crate) rearms: u64,

    /// The `RaftMetrics` sends: one per metrics report.
    pub(crate) metrics: u64,

    /// The `RaftDataMetrics` sends: the metrics reports that changed it.
    pub(crate) data_metrics: u64,

    /// The `RaftServerMetrics` sends: the metrics reports that changed it.
    pub(crate) server_metrics: u64,
}

impl CoreCounts {
    /// Read the counters of every member in `groups` and sum them.
    ///
    /// Returns `None` without the `runtime-stats` feature, since `Raft::runtime_stats()` needs it.
    pub(crate) async fn read(groups: &[Vec<Raft>]) -> Option<Self> {
        #[cfg(feature = "runtime-stats")]
        {
            use futures::future::join_all;
            use openraft::stats::NotificationName;

            let fired = [
                NotificationName::Tick,
                NotificationName::ElectionDeadlineReached,
                NotificationName::PendingReadDeadlineReached,
            ];

            let reads = groups.iter().flatten().map(|raft| raft.runtime_stats());
            let mut sum = Self::default();
            for stats in join_all(reads).await {
                let stats = stats.expect("read the runtime stats of a member");
                sum.fires += fired.iter().map(|name| stats.notification_counts[name.index()]).sum::<u64>();
                sum.rearms += stats.heartbeat_rearm_wakes + stats.election_rearm_wakes + stats.pending_read_rearm_wakes;
                sum.metrics += stats.metrics_sends;
                sum.data_metrics += stats.data_metrics_sends;
                sum.server_metrics += stats.server_metrics_sends;
            }
            Some(sum)
        }

        #[cfg(not(feature = "runtime-stats"))]
        {
            let _ = groups;
            None
        }
    }

    /// The counts from `before` to `self`.
    pub(crate) fn since(&self, before: &Self) -> Self {
        Self {
            fires: self.fires - before.fires,
            rearms: self.rearms - before.rearms,
            metrics: self.metrics - before.metrics,
            data_metrics: self.data_metrics - before.data_metrics,
            server_metrics: self.server_metrics - before.server_metrics,
        }
    }
}
