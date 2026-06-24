use crate::RaftTypeConfig;
use crate::progress::inflight_id::InflightId;
use crate::type_config::alias::JoinHandleOf;
use crate::type_config::alias::WatchSenderOf;

/// Handle to a running `SnapshotTransmitter` task.
///
/// Dropping this handle cancels the snapshot transmission.
pub(crate) struct SnapshotTransmitterHandle<C>
where C: RaftTypeConfig
{
    /// The spawn handle of the `SnapshotTransmitter` task.
    pub(crate) _join_handle: JoinHandleOf<C, ()>,

    /// Dropping this sender signals the task to cancel.
    pub(crate) _tx_cancel: WatchSenderOf<C, ()>,

    /// The inflight id of this snapshot transmission.
    ///
    /// Used by RaftCore to match completion notifications against the currently stored handle.
    pub(crate) inflight_id: InflightId,
}
