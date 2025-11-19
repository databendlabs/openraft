//! Log state on the leader for replicating log entries to a follower.

use crate::LogId;
use crate::RaftTypeConfig;

#[derive(Clone, Debug)]
#[derive(PartialEq, Eq)]
pub(crate) struct LogState<C>
where C: RaftTypeConfig
{
    /// The last known committed log id on a node.
    pub(crate) committed: Option<LogId<C>>,

    /// The last log id on a node.
    pub(crate) last: Option<LogId<C>>,
}
