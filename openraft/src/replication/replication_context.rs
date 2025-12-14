use std::fmt;
use std::sync::Arc;

use crate::Config;
use crate::RaftTypeConfig;
use crate::core::SharedRuntimeState;
use crate::core::notification::Notification;
use crate::replication::ReplicationSessionId;
use crate::type_config::alias::MpscSenderOf;
use crate::type_config::alias::WatchReceiverOf;

/// Shared context for replication tasks.
///
/// Contains the common state needed by both log replication (`ReplicationCore`)
/// and snapshot transmission (`SnapshotTransmitter`) tasks, including node identifiers,
/// session information, configuration, and the notification channel back to `RaftCore`.
#[derive(Clone)]
pub(crate) struct ReplicationContext<C>
where C: RaftTypeConfig
{
    /// This node id
    #[allow(dead_code)]
    pub(crate) id: C::NodeId,

    /// The ID of the target Raft node which replication events are to be sent to.
    pub(crate) target: C::NodeId,

    /// Identifies which session this replication belongs to.
    pub(crate) session_id: ReplicationSessionId<C>,

    /// The Raft's runtime config.
    pub(crate) config: Arc<Config>,

    /// A channel for sending events to the RaftCore.
    #[allow(clippy::type_complexity)]
    pub(crate) tx_notify: MpscSenderOf<C, Notification<C>>,

    /// Watch channel receiver for cancellation signal.
    ///
    /// When the sender is dropped, this signals that replication should stop.
    pub(crate) cancel_rx: WatchReceiverOf<C, ()>,

    /// Shared runtime statistics for recording replication metrics.
    pub(crate) runtime_stats: SharedRuntimeState,
}

impl<C> fmt::Debug for ReplicationContext<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReplicationContext")
            .field("id", &self.id)
            .field("target", &self.target)
            .field("session_id", &self.session_id)
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}
