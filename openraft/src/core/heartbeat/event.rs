use std::fmt;

use display_more::DisplayOptionExt;

use crate::RaftTypeConfig;
use crate::proposer::SendStamp;
use crate::type_config::alias::InstantOf;
use crate::type_config::alias::LogIdOf;

/// The information for broadcasting a heartbeat.
#[derive(Debug, Clone)]
#[derive(PartialEq, Eq)]
pub(crate) struct HeartbeatEvent<C>
where C: RaftTypeConfig
{
    /// The timestamp when this heartbeat is sent, and the heartbeat round of this broadcast.
    ///
    /// The Leader uses this sending time to calculate the quorum acknowledge time, but not the
    /// receiving timestamp.
    pub(crate) time: SendStamp<InstantOf<C>>,

    /// The last known matching log id that has been confirmed replicated to the target follower.
    ///
    /// This is used as `prev_log_id` in heartbeat AppendEntries to ensure the follower has this
    /// log id, avoiding false conflict responses that could be misinterpreted as log reversion.
    pub(crate) matching: Option<LogIdOf<C>>,

    /// The log id that the Leader sends as `leader_commit` in the heartbeat AppendEntries: the
    /// bound up to which followers may apply.
    ///
    /// It is the cluster-committed log id, or with relaxed reads the smaller of that and the
    /// Leader's applied log id, so that no follower applies an entry before the Leader does. For
    /// example, with the cluster committed through 22 and the Leader applied through 21, it is 22
    /// without relaxed reads and 21 with them. See `RaftState::follower_apply_bound()`.
    ///
    /// When there are no new logs to replicate, the heartbeat carries this bound to followers, so
    /// that they can update their committed log id.
    pub(crate) cluster_committed: Option<LogIdOf<C>>,
}

impl<C> fmt::Display for HeartbeatEvent<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(time={}, matching: {}, cluster_committed: {})",
            self.time,
            self.matching.display(),
            self.cluster_committed.display()
        )
    }
}
