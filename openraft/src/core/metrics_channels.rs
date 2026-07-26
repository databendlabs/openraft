use crate::RaftTypeConfig;
use crate::core::io_flush_tracking::IoProgressSender;
use crate::metrics::RaftDataMetrics;
use crate::metrics::RaftMetrics;
use crate::metrics::RaftServerMetrics;
use crate::type_config::alias::WatchSenderOf;

/// The channels [`RaftCore`] publishes its own state on, for observers to read.
///
/// Every field here is write-only from [`RaftCore`]'s side and is updated together by
/// [`RaftCore::flush_metrics()`], so that a reader of any one of them sees a consistent snapshot.
///
/// [`RaftCore`]: crate::core::RaftCore
/// [`RaftCore::flush_metrics()`]: crate::core::RaftCore::flush_metrics
pub(crate) struct MetricsChannels<C>
where C: RaftTypeConfig
{
    /// The complete metrics, backing [`Raft::metrics()`].
    ///
    /// [`Raft::metrics()`]: crate::Raft::metrics
    pub(crate) all: WatchSenderOf<C, RaftMetrics<C>>,

    /// The subset of metrics that changes with data, backing [`Raft::data_metrics()`].
    ///
    /// [`Raft::data_metrics()`]: crate::Raft::data_metrics
    pub(crate) data: WatchSenderOf<C, RaftDataMetrics<C>>,

    /// The subset of metrics that changes with server state, backing [`Raft::server_metrics()`].
    ///
    /// [`Raft::server_metrics()`]: crate::Raft::server_metrics
    pub(crate) server: WatchSenderOf<C, RaftServerMetrics<C>>,

    /// Per-stage IO progress, backing the `Raft::watch_*_progress()` family.
    pub(crate) progress: IoProgressSender<C>,
}
