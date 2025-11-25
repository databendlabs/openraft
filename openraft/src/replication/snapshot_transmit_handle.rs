use crate::RaftTypeConfig;
use crate::type_config::alias::JoinHandleOf;
use crate::type_config::alias::WatchSenderOf;

pub(crate) struct SnapshotTransmitHandle<C>
where C: RaftTypeConfig
{
    /// The spawn handle of the `ReplicationCore` task.
    pub(crate) _join_handle: JoinHandleOf<C, ()>,

    /// Drop to notify the task to cancel
    pub(crate) _tx_cancel: WatchSenderOf<C, ()>,
}
