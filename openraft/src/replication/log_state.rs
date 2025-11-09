//! Log state on the leader for replicating log entries to a follower.

use crate::LogId;
use crate::RaftTypeConfig;
use crate::type_config::alias::LogIdOf;

/// Tracks the log replication state for a follower from the leader's perspective.
///
/// This struct maintains the progress of log replication, including which logs have been matched,
/// the search range for finding matching logs, and the stream identifier for error isolation.
#[derive(Clone, Debug)]
#[derive(PartialEq, Eq)]
pub(crate) struct LogState<C>
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

    /// The last known committed log id on the leader.
    pub(crate) committed: Option<LogId<C>>,

    /// The last matching log id on the follower.
    pub(crate) matching: Option<LogIdOf<C>>,

    /// Upper bound (exclusive) of the log index range on the follower that may match the leader.
    pub(crate) searching_end: u64,

    /// The last log id on this leader node.
    pub(crate) last_log_id: Option<LogId<C>>,
}
