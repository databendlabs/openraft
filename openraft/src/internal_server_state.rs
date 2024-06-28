use crate::leader::Leader;
use crate::quorum::Joint;
use crate::RaftTypeConfig;

/// The quorum set type used by `Leader`.
pub(crate) type LeaderQuorumSet<NID> = Joint<NID, Vec<NID>, Vec<Vec<NID>>>;

/// In openraft there are only two state for a server:
/// Leading(raft leader or raft candidate) and following(raft follower or raft learner):
///
/// - A leading state is able to vote(candidate in original raft) and is able to propose new log if
///   its vote is granted by quorum(leader in original raft).
///
///   In this way the leadership won't be lost when it sees a higher `vote` and needs upgrade its
/// `vote`.
///
/// - A following state just receives replication from a leader. A follower that is one of the
///   member will be able to become leader. A following state that is not a member is just a
///   learner.
#[derive(Clone, Debug)]
#[derive(PartialEq, Eq)]
// TODO(9): Make InternalServerState an Option, separate Leading(Proposer) role and
//          Following(Acceptor) role
pub(crate) enum InternalServerState<C>
where C: RaftTypeConfig
{
    /// Leader state.
    ///
    /// `vote.committed==true` means it is a leader.
    Leader(Box<Leader<C, LeaderQuorumSet<C::NodeId>>>),

    /// Follower or Learner state.
    ///
    /// It is a follower if it is a voter of the cluster, otherwise it is a learner.
    Following,
}

impl<C> Default for InternalServerState<C>
where C: RaftTypeConfig
{
    fn default() -> Self {
        Self::Following
    }
}

impl<C> InternalServerState<C>
where C: RaftTypeConfig
{
    pub(crate) fn leader_ref(&self) -> Option<&Leader<C, LeaderQuorumSet<C::NodeId>>> {
        match self {
            InternalServerState::Leader(l) => Some(l),
            InternalServerState::Following => None,
        }
    }

    pub(crate) fn leader_mut(&mut self) -> Option<&mut Leader<C, LeaderQuorumSet<C::NodeId>>> {
        match self {
            InternalServerState::Leader(l) => Some(l),
            InternalServerState::Following => None,
        }
    }

    pub(crate) fn is_leader(&self) -> bool {
        match self {
            InternalServerState::Leader(_) => true,
            InternalServerState::Following => false,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn is_following(&self) -> bool {
        !self.is_leader()
    }
}
