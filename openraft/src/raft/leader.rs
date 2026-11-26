use openraft_macros::since;

use crate::NodeId;
use crate::Raft;
use crate::RaftTypeConfig;
use crate::impls::leader_id_std;
use crate::storage::RaftStateMachine;
use crate::type_config::alias::CommittedLeaderIdOf;
use crate::type_config::alias::InstantOf;
use crate::type_config::alias::LeaderIdOf;
use crate::vote::RaftLeaderId;
use crate::vote::RaftTerm;

/// Information about a node when it is a leader.
///
/// This struct contains metadata about the current leader state, including
/// its identity and health indicators.
#[since(version = "0.10.0")]
pub struct Leader<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    pub(crate) raft: Raft<C, SM>,

    /// The leader ID, including term and node ID.
    pub(crate) leader_id: LeaderIdOf<C>,

    /// The send time of the latest RPCs that a write quorum has acknowledged.
    ///
    /// `None` if no such time is available. A leader that alone forms a write quorum has this time
    /// from the moment it becomes leader, without any response. See
    /// [`Leader::last_quorum_acked()`].
    pub(crate) last_quorum_acked: Option<InstantOf<C>>,
}

impl<C, SM> Clone for Leader<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    fn clone(&self) -> Self {
        Self {
            raft: self.raft.clone(),
            leader_id: self.leader_id.clone(),
            last_quorum_acked: self.last_quorum_acked,
        }
    }
}

impl<C, SM> std::fmt::Debug for Leader<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Leader")
            .field("raft", &self.raft)
            .field("leader_id", &self.leader_id)
            .field("last_quorum_acked", &self.last_quorum_acked)
            .finish()
    }
}

impl<C, SM> Leader<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    /// Returns the [`Raft`] handle of this node.
    pub fn raft(&self) -> &Raft<C, SM> {
        &self.raft
    }

    /// Returns the leader id, which holds the term and this node's id.
    pub fn leader_id(&self) -> &LeaderIdOf<C> {
        &self.leader_id
    }

    /// Returns the committed form of the leader id, which the log ids of this leader's entries
    /// carry.
    pub fn to_committed_leader_id(&self) -> CommittedLeaderIdOf<C> {
        self.leader_id.to_committed()
    }

    /// Returns the send time of the latest RPCs that a write quorum has acknowledged, or `None` if
    /// no such time is available, for example before a write quorum has acknowledged any RPC of
    /// this leader.
    ///
    /// Each node of the write quorum, other than this leader, acknowledged an RPC that this leader
    /// sent at or after the returned time. This leader counts toward the quorum without a response
    /// when it is a write voter. The acknowledgements arrived at least one round trip after this
    /// time. A leader that alone forms a write quorum needs no response: the value starts at the
    /// time it became leader, and advances to the send time of each later RPC that another node
    /// acknowledges.
    ///
    /// The value is a snapshot taken by the [`Raft::as_leader()`] call that returned this
    /// `Leader`.
    pub fn last_quorum_acked(&self) -> Option<InstantOf<C>> {
        self.last_quorum_acked
    }
}

/// `Term` and `NID` are extracted as separate type parameters to avoid a rustc cycle error
/// that occurs when using `C::Term` or `C::NodeId` inside an associated type equality constraint
/// (e.g., `LeaderId = LeaderId<C::Term, C::NodeId>`).
impl<Term, NID, C, SM> Leader<C, SM>
where
    Term: RaftTerm,
    NID: NodeId,
    C: RaftTypeConfig<Term = Term, NodeId = NID, LeaderId = leader_id_std::LeaderId<Term, NID>>,
    SM: RaftStateMachine<C>,
{
    /// Only when the [`CommittedLeaderIdOf`] is a single term this method is allowed.
    /// Otherwise, the user may mistakenly get the term as the entire [`CommittedLeaderIdOf`]
    pub fn term(&self) -> C::Term {
        self.leader_id.term()
    }
}
