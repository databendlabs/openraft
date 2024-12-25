use std::fmt;

use crate::display_ext::DisplayOptionExt;
use crate::LogId;
use crate::RaftTypeConfig;
use crate::Vote;

#[derive(Clone, Debug)]
#[derive(PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct TransferLeaderRequest<C>
where C: RaftTypeConfig
{
    /// The vote of the Leader that is transferring the leadership.
    pub(crate) from_leader: Vote<C>,

    /// The assigned node to be the next Leader.
    pub(crate) to_node_id: C::NodeId,

    /// The last log id the `to_node_id` node should at least have to become Leader.
    pub(crate) last_log_id: Option<LogId<C>>,
}

impl<C> TransferLeaderRequest<C>
where C: RaftTypeConfig
{
    pub fn new(from: Vote<C>, to: C::NodeId, last_log_id: Option<LogId<C>>) -> Self {
        Self {
            from_leader: from,
            to_node_id: to,
            last_log_id,
        }
    }

    /// From which Leader the leadership is transferred.
    pub fn from_leader(&self) -> &Vote<C> {
        &self.from_leader
    }

    /// To which node the leadership is transferred.
    pub fn to_node_id(&self) -> &C::NodeId {
        &self.to_node_id
    }

    /// The last log id on the `to_node_id` node should at least have to become Leader.
    ///
    /// This is the last log id on the Leader when the leadership is transferred.
    pub fn last_log_id(&self) -> Option<&LogId<C>> {
        self.last_log_id.as_ref()
    }
}

impl<C> fmt::Display for TransferLeaderRequest<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(from_leader={}, to={}, last_log_id={})",
            self.from_leader,
            self.to_node_id,
            self.last_log_id.display()
        )
    }
}
