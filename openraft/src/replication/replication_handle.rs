use crate::RaftTypeConfig;
use crate::error::ReplicationClosed;
use crate::replication::ReplicationSessionId;
use crate::replication::request::Replicate;
use crate::replication::snapshot_transmitter_handle::SnapshotTransmitterHandle;
use crate::type_config::alias::JoinHandleOf;
use crate::type_config::alias::MpscUnboundedSenderOf;
use crate::type_config::alias::WatchSenderOf;

/// The handle to a spawned replication stream.
pub(crate) struct ReplicationHandle<C>
where C: RaftTypeConfig
{
    /// Identifies this replication session (leader vote + target node).
    pub(crate) session_id: ReplicationSessionId<C>,

    /// The spawn handle of the `ReplicationCore` task.
    pub(crate) join_handle: JoinHandleOf<C, Result<(), ReplicationClosed>>,

    /// The channel used for communicating with the replication task.
    pub(crate) tx_repl: MpscUnboundedSenderOf<C, Replicate<C>>,

    /// Handle to the snapshot transmitter task, if one is running.
    pub(crate) snapshot_transmit_handle: Option<SnapshotTransmitterHandle<C>>,

    /// Sender for the cancellation signal; dropping this stops replication.
    pub(crate) _cancel_tx: WatchSenderOf<C, ()>,
}
