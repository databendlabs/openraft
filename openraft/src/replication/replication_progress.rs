use crate::RaftTypeConfig;
use crate::type_config::alias::LogIdOf;

/// Tracks the log replication state for a follower from the leader's perspective.
#[derive(Clone, Debug)]
#[derive(PartialEq, Eq)]
pub(crate) struct ReplicationProgress<C>
where C: RaftTypeConfig
{
    /// The leader's committed log id to replicate to the follower.
    pub(crate) local_committed: Option<LogIdOf<C>>,

    /// The last log id known to match on the follower.
    ///
    /// All logs up to and including this id are confirmed to exist on the follower.
    pub(crate) remote_matched: Option<LogIdOf<C>>,
}

impl<C> ReplicationProgress<C> where C: RaftTypeConfig {}
