use crate::leader::voting::Voting;
use crate::leader::Leading;
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
    /// Leader or candidate.
    ///
    /// `vote.committed==true` means it is a leader.
    Leading(Box<Leading<C, LeaderQuorumSet<C::NodeId>>>),

    /// Follower or learner.
    ///
    /// Being a voter means it is a follower.
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
    pub(crate) fn voting_mut(&mut self) -> Option<&mut Voting<C, LeaderQuorumSet<C::NodeId>>> {
        match self {
            InternalServerState::Leading(l) => l.voting_mut(),
            InternalServerState::Following => None,
        }
    }

    pub(crate) fn leading(&self) -> Option<&Leading<C, LeaderQuorumSet<C::NodeId>>> {
        match self {
            InternalServerState::Leading(l) => Some(l),
            InternalServerState::Following => None,
        }
    }

    pub(crate) fn leading_mut(&mut self) -> Option<&mut Leading<C, LeaderQuorumSet<C::NodeId>>> {
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
