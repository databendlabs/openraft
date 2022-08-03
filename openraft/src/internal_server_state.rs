use std::sync::Arc;

use crate::leader::Leader;
use crate::node::Node;
use crate::EffectiveMembership;
use crate::NodeId;

/// In openraft there are only two state for a server:
/// Leading(raft leader or raft candidate) and following(raft follower or raft learner):
///
/// - A leading state is able to vote(candidate in original raft) and is able to propose new log if its vote is granted
///   by quorum(leader in original raft).
///
///   In this way the leadership won't be lost when it sees a higher `vote` and needs upgrade its `vote`.
///
/// - A following state just receives replication from a leader. A follower that is one of the member will be able to
///   become leader. A following state that is not a member is just a learner.
#[derive(Clone, Debug)]
#[derive(PartialEq, Eq)]
pub(crate) enum InternalServerState<NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// Leader or candidate.
    ///
    /// `vote.committed==true` means it is a leader.
    Leading(Leader<NID, Arc<EffectiveMembership<NID, N>>>),

    /// Follower or learner.
    ///
    /// Being a voter means it is a follower.
    Following,
}

impl<NID, N> Default for InternalServerState<NID, N>
where
    NID: NodeId,
    N: Node,
{
    fn default() -> Self {
        Self::Following
    }
}

impl<NID, N> InternalServerState<NID, N>
where
    NID: NodeId,
    N: Node,
{
    pub(crate) fn leading(&self) -> Option<&Leader<NID, Arc<EffectiveMembership<NID, N>>>> {
        match self {
            InternalServerState::Leading(l) => Some(l),
            InternalServerState::Following => None,
        }
    }

    pub(crate) fn leading_mut(&mut self) -> Option<&mut Leader<NID, Arc<EffectiveMembership<NID, N>>>> {
        match self {
            InternalServerState::Leading(l) => Some(l),
            InternalServerState::Following => None,
        }
    }

    pub(crate) fn is_leading(&self) -> bool {
        match self {
            InternalServerState::Leading(_) => true,
            InternalServerState::Following => false,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn is_following(&self) -> bool {
        !self.is_leading()
    }
}
