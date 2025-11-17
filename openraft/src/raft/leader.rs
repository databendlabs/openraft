use openraft_macros::since;

use crate::Raft;
use crate::RaftTypeConfig;
use crate::impls::leader_id_std;
use crate::type_config::alias::CommittedLeaderIdOf;
use crate::type_config::alias::InstantOf;
use crate::type_config::alias::LeaderIdOf;
use crate::vote::RaftLeaderId;

/// Information about a node when it is a leader.
///
/// This struct contains metadata about the current leader state, including
/// its identity and health indicators.
#[since(version = "0.10.0")]
#[derive(Clone, Debug)]
pub struct Leader<C>
where C: RaftTypeConfig
{
    pub(crate) raft: Raft<C>,

    /// The leader ID, including term and node ID.
    pub(crate) leader_id: LeaderIdOf<C>,

    /// The timestamp when the leader was last acknowledged by a quorum.
    ///
    /// `None` if the leader has not yet been acknowledged by a quorum.
    /// Being acknowledged means receiving a reply of AppendEntries with committed vote.
    pub(crate) last_quorum_acked: Option<InstantOf<C>>,
}

impl<C> Leader<C>
where C: RaftTypeConfig
{
    pub fn raft(&self) -> &Raft<C> {
        &self.raft
    }

    pub fn leader_id(&self) -> &LeaderIdOf<C> {
        &self.leader_id
    }

    pub fn to_committed_leader_id(&self) -> CommittedLeaderIdOf<C> {
        self.leader_id.to_committed()
    }

    /// Only when the [`CommittedLeaderIdOf`] is a single term this method is allowed.
    /// Otherwise, the user may mistakenly get the term as the entire [`CommittedLeaderIdOf`]
    pub fn term(&self) -> C::Term
    where C: RaftTypeConfig<LeaderId = leader_id_std::LeaderId<C>> {
        self.leader_id.term()
    }

    pub fn last_quorum_acked(&self) -> Option<InstantOf<C>> {
        self.last_quorum_acked
    }
}
