use std::fmt;

use crate::display_ext::DisplayInstantExt;
use crate::display_ext::DisplayOptionExt;
use crate::type_config::alias::InstantOf;
use crate::vote::CommittedVote;
use crate::LogId;
use crate::RaftTypeConfig;

/// The information for broadcasting a heartbeat.
#[derive(Debug, Clone, Copy)]
#[derive(PartialEq, Eq)]
pub struct HeartbeatEvent<C>
where C: RaftTypeConfig
{
    /// The timestamp when this heartbeat is sent.
    ///
    /// The Leader use this sending time to calculate the quorum acknowledge time, but not the
    /// receiving timestamp.
    pub(crate) time: InstantOf<C>,

    /// The vote of the Leader that submit this heartbeat.
    ///
    /// The response that matches this vote is considered as a valid response.
    /// Otherwise, it is considered as an outdated response and will be ignored.
    pub(crate) leader_vote: CommittedVote<C>,

    /// The last known committed log id of the Leader.
    ///
    /// When there are no new logs to replicate, the Leader sends a heartbeat to replicate committed
    /// log id to followers to update their committed log id.
    pub(crate) committed: Option<LogId<C::NodeId>>,
}

impl<C> HeartbeatEvent<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(time: InstantOf<C>, leader_vote: CommittedVote<C>, committed: Option<LogId<C::NodeId>>) -> Self {
        Self {
            time,
            leader_vote,
            committed,
        }
    }
}

impl<C> fmt::Display for HeartbeatEvent<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(time={}, leader_vote: {}, committed: {})",
            self.time.display(),
            self.leader_vote,
            self.committed.display()
        )
    }
}
