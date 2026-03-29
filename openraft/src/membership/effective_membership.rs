use std::collections::BTreeSet;
use std::fmt;
use std::fmt::Debug;
use std::sync::Arc;

use display_more::DisplayOptionExt;
use openraft_macros::since;

use crate::Membership;
use crate::StoredMembership;
use crate::log_id::LogId;
use crate::log_id::raft_log_id::RaftLogId;
use crate::log_id::raft_log_id_ext::RaftLogIdExt;
use crate::node::Node;
use crate::node::NodeId;
use crate::quorum::Joint;
use crate::quorum::QuorumSet;
use crate::vote::RaftCommittedLeaderId;

/// The currently active membership config.
///
/// It includes:
/// - the id of the log that sets this membership config,
/// - and the config.
///
/// An active config is just the last seen config in raft spec.
#[since(
    version = "0.10.0",
    change = "from `EffectiveMembership<C>` to `EffectiveMembership<CLID, NID, N>`"
)]
#[derive(Clone, Eq)]
pub struct EffectiveMembership<CLID, NID, N>
where
    CLID: RaftCommittedLeaderId,
    NID: NodeId,
    N: Node,
{
    stored_membership: Arc<StoredMembership<CLID, NID, N>>,

    /// The quorum set built from `membership`.
    quorum_set: Joint<NID, Vec<NID>, Vec<Vec<NID>>>,

    /// Cache of the union of all members
    voter_ids: BTreeSet<NID>,
}

impl<CLID, NID, N> Default for EffectiveMembership<CLID, NID, N>
where
    CLID: RaftCommittedLeaderId,
    NID: NodeId,
    N: Node,
{
    fn default() -> Self {
        Self {
            stored_membership: Arc::new(StoredMembership::<CLID, NID, N>::default()),
            quorum_set: Joint::default(),
            voter_ids: Default::default(),
        }
    }
}

impl<CLID, NID, N> Debug for EffectiveMembership<CLID, NID, N>
where
    CLID: RaftCommittedLeaderId,
    NID: NodeId,
    N: Node,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EffectiveMembership")
            .field("log_id", self.log_id())
            .field("membership", self.membership())
            .field("voter_ids", &self.voter_ids)
            .finish()
    }
}

impl<CLID, NID, N> PartialEq for EffectiveMembership<CLID, NID, N>
where
    CLID: RaftCommittedLeaderId,
    NID: NodeId,
    N: Node,
{
    fn eq(&self, other: &Self) -> bool {
        self.stored_membership == other.stored_membership && self.voter_ids == other.voter_ids
    }
}

impl<CLID, NID, N, LID> From<(&LID, Membership<NID, N>)> for EffectiveMembership<CLID, NID, N>
where
    CLID: RaftCommittedLeaderId,
    NID: NodeId,
    N: Node,
    LID: RaftLogId<CommittedLeaderId = CLID>,
{
    fn from(v: (&LID, Membership<NID, N>)) -> Self {
        EffectiveMembership::new(Some(v.0.to_log_id()), v.1)
    }
}

impl<CLID, NID, N> EffectiveMembership<CLID, NID, N>
where
    CLID: RaftCommittedLeaderId,
    NID: NodeId,
    N: Node,
{
    pub(crate) fn new_arc(log_id: Option<LogId<CLID>>, membership: Membership<NID, N>) -> Arc<Self> {
        Arc::new(Self::new(log_id, membership))
    }

    /// Create a new EffectiveMembership from a log ID and membership configuration.
    pub fn new(log_id: Option<LogId<CLID>>, membership: Membership<NID, N>) -> Self {
        let voter_ids = membership.voter_ids().collect();

        let configs = membership.get_joint_config();
        let mut joint = vec![];
        for c in configs {
            joint.push(c.iter().cloned().collect::<Vec<_>>());
        }

        let quorum_set = Joint::from(joint);

        Self {
            stored_membership: Arc::new(StoredMembership::<CLID, NID, N>::new(log_id, membership)),
            quorum_set,
            voter_ids,
        }
    }

    pub(crate) fn new_from_stored_membership(stored: StoredMembership<CLID, NID, N>) -> Self {
        Self::new(stored.log_id().clone(), stored.membership().clone())
    }

    pub(crate) fn stored_membership(&self) -> &Arc<StoredMembership<CLID, NID, N>> {
        &self.stored_membership
    }

    /// Get the log ID at which this membership was stored.
    pub fn log_id(&self) -> &Option<LogId<CLID>> {
        self.stored_membership.log_id()
    }

    /// Get the membership configuration.
    pub fn membership(&self) -> &Membership<NID, N> {
        self.stored_membership.membership()
    }
}

/// Membership API
impl<CLID, NID, N> EffectiveMembership<CLID, NID, N>
where
    CLID: RaftCommittedLeaderId,
    NID: NodeId,
    N: Node,
{
    #[allow(dead_code)]
    pub(crate) fn is_voter(&self, nid: &NID) -> bool {
        self.membership().is_voter(nid)
    }

    /// Returns an Iterator of all voter node ids. Learners are not included.
    pub fn voter_ids(&self) -> impl Iterator<Item = NID> + '_ {
        self.voter_ids.iter().cloned()
    }

    /// Returns an Iterator of all learner node ids. Voters are not included.
    pub(crate) fn learner_ids(&self) -> impl Iterator<Item = NID> + '_ {
        self.membership().learner_ids()
    }

    /// Get the node (either voter or learner) by node id.
    pub fn get_node(&self, node_id: &NID) -> Option<&N> {
        self.membership().get_node(node_id)
    }

    /// Returns an Iterator of all nodes (voters and learners).
    pub fn nodes(&self) -> impl Iterator<Item = (&NID, &N)> {
        self.membership().nodes()
    }

    /// Returns reference to the joint config.
    ///
    /// Membership is defined by a joint of multiple configs.
    /// Each config is a vec of node-id.
    pub fn get_joint_config(&self) -> &Vec<Vec<NID>> {
        self.quorum_set.children()
    }
}

impl<CLID, NID, N> fmt::Display for EffectiveMembership<CLID, NID, N>
where
    CLID: RaftCommittedLeaderId,
    NID: NodeId,
    N: Node,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "EffectiveMembership{{log_id: {}, membership: {}}}",
            self.log_id().display(),
            self.membership()
        )
    }
}

/// Implement node-id joint quorum set.
impl<CLID, NID, N> QuorumSet<NID> for EffectiveMembership<CLID, NID, N>
where
    CLID: RaftCommittedLeaderId,
    NID: NodeId,
    N: Node,
{
    type Iter = std::collections::btree_set::IntoIter<NID>;

    fn is_quorum<'a, I: Iterator<Item = &'a NID> + Clone>(&self, ids: I) -> bool {
        self.quorum_set.is_quorum(ids)
    }

    fn ids(&self) -> Self::Iter {
        self.quorum_set.ids()
    }
}
