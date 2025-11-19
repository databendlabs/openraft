use crate::LogId;
use crate::RaftTypeConfig;
use crate::replication::log_state::LogState;

/// Tracks the log replication state for a follower from the leader's perspective.
///
/// This struct maintains the progress of log replication, including which logs have been matched,
/// the search range for finding matching logs, and the stream identifier for error isolation.
#[derive(Clone, Debug)]
#[derive(PartialEq, Eq)]
pub(crate) struct ReplicationState<C>
where C: RaftTypeConfig
{
    /// Replication stream identifier.
    ///
    /// Used until a replication error occurs and is reported to RaftCore.
    /// A new stream_id is generated after each error to isolate delayed messages from previous
    /// streams.
    pub(crate) stream_id: u64,

    /// The last purged log id.
    ///
    /// Log entries at or below this id are absent on the leader and require snapshot replication.
    pub(crate) purged: Option<LogId<C>>,

    /// Log state on the leader node.
    ///
    /// - `committed`: the leader's committed log id.
    /// - `last`: unused.
    pub(crate) local: LogState<C>,

    /// Known log state on the remote follower.
    ///
    /// - `committed`: unused.
    /// - `last`: the last matching log id, i.e., the follower has all logs up to this id.
    pub(crate) remote: LogState<C>,

    /// Upper bound (exclusive) of the log index range on the follower that may match the leader.
    pub(crate) searching_end: u64,
}

impl<C> ReplicationState<C> where C: RaftTypeConfig {}
