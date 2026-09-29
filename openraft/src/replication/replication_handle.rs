use rt::WatchSender;

use crate::RaftTypeConfig;
use crate::errors::ReplicationClosed;
use crate::progress::stream_id::StreamId;
use crate::replication::replicate::Replicate;
use crate::replication::snapshot_transmitter_handle::SnapshotTransmitterHandle;
use crate::type_config::alias::JoinHandleOf;
use crate::type_config::alias::WatchSenderOf;

/// The handle to a spawned replication stream.
pub(crate) struct ReplicationHandle<C>
where C: RaftTypeConfig
{
    /// Identifies this replication session (leader vote + target node).
    pub(crate) stream_id: StreamId,

    /// The channel used for communicating with the replication task.
    pub(crate) replicate_tx: WatchSenderOf<C, Replicate<C>>,

    /// Sender for the cancellation signal; dropping this stops replication.
    pub(crate) cancel_tx: WatchSenderOf<C, ()>,

    /// Sender for the backoff reset signal; sending on it ends the active backoff of replication.
    pub(crate) backoff_reset_tx: WatchSenderOf<C, ()>,

    /// The spawn handle of the `ReplicationCore` task.
    pub(crate) join_handle: Option<JoinHandleOf<C, Result<(), ReplicationClosed>>>,

    /// Handle to the snapshot transmitter task, if one is running.
    pub(crate) snapshot_transmit_handle: Option<SnapshotTransmitterHandle<C>>,
}

impl<C> ReplicationHandle<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(
        stream_id: StreamId,
        replicate_tx: WatchSenderOf<C, Replicate<C>>,
        cancel_tx: WatchSenderOf<C, ()>,
        backoff_reset_tx: WatchSenderOf<C, ()>,
    ) -> Self {
        Self {
            stream_id,
            join_handle: None,
            replicate_tx,
            snapshot_transmit_handle: None,
            cancel_tx,
            backoff_reset_tx,
        }
    }

    /// Signal the replication task to end its active backoff.
    pub(crate) fn reset_backoff(&self) {
        let send_res = self.backoff_reset_tx.send(());
        if send_res.is_err() {
            tracing::warn!(
                "Failed to reset backoff for replication stream {}: channel closed",
                self.stream_id
            );
        }
    }
}
