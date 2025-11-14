use std::fmt;

use crate::LogId;
use crate::RaftTypeConfig;
use crate::display_ext::DisplayInstantExt;
use crate::display_ext::DisplayOptionExt;
use crate::replication::ReplicationSessionId;
use crate::type_config::alias::InstantOf;
use crate::type_config::alias::LogIdOf;

/// The information for broadcasting a heartbeat.
#[derive(Debug, Clone)]
#[derive(PartialEq, Eq)]
pub(crate) struct HeartbeatEvent<C>
where C: RaftTypeConfig
{
    /// The timestamp when this heartbeat is sent.
    ///
    /// The Leader uses this sending time to calculate the quorum acknowledge time, but not the
    /// receiving timestamp.
    pub(crate) time: InstantOf<C>,

    /// The vote of the Leader that submits this heartbeat and the log id of the cluster config.
    ///
    /// The response that matches this session id is considered as a valid response.
    /// Otherwise, it is considered as an outdated response from older leader or older cluster
    /// membership config and will be ignored.
    pub(crate) session_id: ReplicationSessionId<C>,

    /// The last known matching log id that has been confirmed replicated to the target follower.
    ///
    /// This is used as `prev_log_id` in heartbeat AppendEntries to ensure the follower has this
    /// log id, avoiding false conflict responses that could be misinterpreted as log reversion.
    pub(crate) matching: Option<LogId<C>>,

    /// The last known committed log id of the Leader.
    ///
    /// When there are no new logs to replicate, the Leader sends a heartbeat to replicate committed
    /// log id to followers to update their committed log id.
    pub(crate) committed: Option<LogIdOf<C>>,
}

impl<C> fmt::Display for HeartbeatEvent<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(time={}, leader_vote: {}, matching: {}, committed: {})",
            self.time.display(),
            self.session_id,
            self.matching.display(),
            self.committed.display()
        )
    }
}
