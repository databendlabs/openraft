use crate::RaftTypeConfig;
use crate::core::io_flush_tracking::IoProgressSender;
use crate::metrics::RaftDataMetrics;
use crate::metrics::RaftMetrics;
use crate::metrics::RaftServerMetrics;
use crate::type_config::alias::WatchSenderOf;

/// The channels [`RaftCore`] publishes its own state on, for observers to read.
///
/// The fields are senders that [`RaftCore`] owns. [`RaftCore::flush_metrics()`] is the routine
/// publisher: it publishes the IO progress first, and then `report_metrics()` updates `data` and
/// `server` when they changed, and sends `all` last, so a reader that sees `all` change finds the
/// other two updated. `RaftCore::main()` is the terminal publisher: when it stops, after its last
/// `flush_metrics()`, it publishes the shutdown state. Each channel holds a consistent snapshot of
/// its own values; the channels are not updated as one.
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
