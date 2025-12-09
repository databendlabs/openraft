use crate::RaftTypeConfig;
use crate::error::ReplicationClosed;
use crate::replication::ReplicationSessionId;
use crate::replication::replicate::Replicate;
use crate::replication::snapshot_transmitter_handle::SnapshotTransmitterHandle;
use crate::type_config::alias::JoinHandleOf;
use crate::type_config::alias::WatchSenderOf;

/// The handle to a spawned replication stream.
pub(crate) struct ReplicationHandle<C>
where C: RaftTypeConfig
{
    /// Identifies this replication session (leader vote + target node).
    pub(crate) session_id: ReplicationSessionId<C>,

    /// The channel used for communicating with the replication task.
    pub(crate) replicate_tx: WatchSenderOf<C, Replicate<C>>,

    /// Sender for the cancellation signal; dropping this stops replication.
    pub(crate) cancel_tx: WatchSenderOf<C, ()>,

    /// The spawn handle of the `ReplicationCore` task.
    pub(crate) join_handle: Option<JoinHandleOf<C, Result<(), ReplicationClosed>>>,

    /// Handle to the snapshot transmitter task, if one is running.
    pub(crate) snapshot_transmit_handle: Option<SnapshotTransmitterHandle<C>>,
}

impl<C> ReplicationHandle<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(
        session_id: ReplicationSessionId<C>,
        replicate_tx: WatchSenderOf<C, Replicate<C>>,
        cancel_tx: WatchSenderOf<C, ()>,
    ) -> Self {
        Self {
            session_id,
            join_handle: None,
            replicate_tx,
            snapshot_transmit_handle: None,
            cancel_tx,
        }
    }
}
