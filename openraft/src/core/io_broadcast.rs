use crate::RaftTypeConfig;
use crate::StorageError;
use crate::raft_state::IOId;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::WatchSenderOf;

/// The watch channels [`RaftCore`] broadcasts local IO progress on.
///
/// These are direct IO and replication progress signals. Storage reports each completed IO on
/// `completed`, and the replication tasks follow `accepted`, `submitted` and `committed` to
/// decide which entries they may read and send, and which commit to send. They are separate
/// from [`MetricsChannels`], which publishes snapshots of the node's state. Those snapshots are
/// not for reporting alone: a linearizable read waits on them for its read log id to be applied,
/// and a leader removed from the membership watches them to step down. Like any watch channel,
/// both may skip intermediate values.
///
/// [`RaftCore`]: crate::core::RaftCore
/// [`MetricsChannels`]: crate::core::MetricsChannels
pub(crate) struct IoBroadcast<C>
where C: RaftTypeConfig
{
    /// Reports a completed storage IO back to [`RaftCore`], written by `IOFlushed` callbacks.
    ///
    /// This is the one channel here that storage writes to rather than [`RaftCore`]; it is
    /// synchronous, so a callback can report completion without an async context.
    ///
    /// [`RaftCore`]: crate::core::RaftCore
    pub(crate) completed: WatchSenderOf<C, Result<IOId<C>, StorageError<C>>>,

    /// Announces an IO that is about to be submitted to storage.
    ///
    /// Sent when [`RaftCore`] executes the IO rather than when `Engine` accepts it, because
    /// `Engine` is a pure algorithm with no IO of its own. Observers use it to prepare for work
    /// that has not happened yet.
    ///
    /// [`RaftCore`]: crate::core::RaftCore
    pub(crate) accepted: WatchSenderOf<C, IOId<C>>,

    /// Announces an IO that has reached storage, so replication may read those entries.
    pub(crate) submitted: WatchSenderOf<C, IOId<C>>,

    /// Announces the committed log id to the replication tasks.
    pub(crate) committed: WatchSenderOf<C, Option<LogIdOf<C>>>,
}
