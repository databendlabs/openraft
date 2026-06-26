use std::fmt;

use display_more::DisplayOptionExt;
use openraft_macros::since;

use crate::RaftTypeConfig;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::VoteOf;

/// A request to transfer leadership from the current leader to another node.
#[since(version = "0.10.0", change = "added last_log_id")]
#[derive(Clone, Debug)]
#[derive(PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct TransferLeaderRequest<C>
where C: RaftTypeConfig
{
    /// The vote of the Leader that is transferring the leadership.
    pub(crate) from_leader: VoteOf<C>,

    /// The assigned node to be the next Leader.
    pub(crate) to_node_id: C::NodeId,

    /// The last log id the `to_node_id` node should at least have to become Leader.
    pub(crate) last_log_id: Option<LogIdOf<C>>,
}

impl<C> TransferLeaderRequest<C>
where C: RaftTypeConfig
{
    /// Create a new transfer leader request.
    #[since(version = "0.10.0", change = "added last_log_id arg")]
    pub fn new(from: VoteOf<C>, to: C::NodeId, last_log_id: Option<LogIdOf<C>>) -> Self {
        Self {
            from_leader: from,
            to_node_id: to,
            last_log_id,
        }
    }

    /// From which Leader the leadership is transferred.
    pub fn from_leader(&self) -> &VoteOf<C> {
        &self.from_leader
    }

    /// To which node the leadership is transferred.
    pub fn to_node_id(&self) -> &C::NodeId {
        &self.to_node_id
    }

    /// The last log id on the `to_node_id` node should at least have to become Leader.
    ///
    /// This is the last log id on the Leader when the leadership is transferred.
    #[since(version = "0.10.0")]
    pub fn last_log_id(&self) -> Option<&LogIdOf<C>> {
        self.last_log_id.as_ref()
    }
}

/// Result of a delivered transfer-leader request.
#[since(version = "0.10.0")]
pub type TransferLeaderResponse<C> = Result<(), TransferLeaderError<C>>;

/// Non-fatal reason a transfer-leader request was rejected by the target node.
#[since(version = "0.10.0")]
#[derive(Clone, Debug)]
#[derive(PartialEq, Eq)]
#[derive(thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub enum TransferLeaderError<C>
where C: RaftTypeConfig
{
    /// The target observed a different vote before it could accept the transfer.
    #[error("transfer leader rejected because vote changed; expected: {expected:?}, actual: {actual:?}")]
    VoteChanged {
        /// The leader vote carried by the transfer request.
        expected: VoteOf<C>,
        /// The vote currently observed by the target.
        actual: VoteOf<C>,
    },

    /// The target has not flushed the leader's expected log yet.
    #[error("transfer leader rejected because log is not flushed; expected: {expected:?}, actual: {actual:?}")]
    LogNotFlushed {
        /// The last log id from the transferring leader.
        expected: Option<LogIdOf<C>>,
        /// The target's local last log id.
        actual: Option<LogIdOf<C>>,
    },
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
