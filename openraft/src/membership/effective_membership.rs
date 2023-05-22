use std::collections::BTreeSet;
use std::fmt::Debug;
use std::sync::Arc;

use crate::entry::RaftEntry;
use crate::log_id::RaftLogId;
use crate::node::Node;
use crate::quorum::Joint;
use crate::quorum::QuorumSet;
use crate::LogId;
use crate::Membership;
use crate::MessageSummary;
use crate::NodeId;
use crate::StoredMembership;

/// The currently active membership config.
///
/// It includes:
/// - the id of the log that sets this membership config,
/// - and the config.
///
/// An active config is just the last seen config in raft spec.
#[derive(Clone, Default, Eq)]
pub struct EffectiveMembership<NID, N>
where
    N: Node,
    NID: NodeId,
{
    stored_membership: Arc<StoredMembership<NID, N>>,

    /// The quorum set built from `membership`.
    quorum_set: Joint<NID, Vec<NID>, Vec<Vec<NID>>>,

    /// Cache of union of all members
    voter_ids: BTreeSet<NID>,
}

impl<NID, N> Debug for EffectiveMembership<NID, N>
where
    N: Node,
    NID: NodeId,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EffectiveMembership")
            .field("log_id", self.log_id())
            .field("membership", self.membership())
            .field("voter_ids", &self.voter_ids)
            .finish()
    }
}

impl<NID, N> PartialEq for EffectiveMembership<NID, N>
where
    N: Node,
    NID: NodeId,
{
    fn eq(&self, other: &Self) -> bool {
        self.stored_membership == other.stored_membership && self.voter_ids == other.voter_ids
    }
}

impl<NID, N, LID> From<(&LID, Membership<NID, N>)> for EffectiveMembership<NID, N>
where
    N: Node,
    NID: NodeId,
    LID: RaftLogId<NID>,
{
    fn from(v: (&LID, Membership<NID, N>)) -> Self {
        EffectiveMembership::new(Some(*v.0.get_log_id()), v.1)
    }
}

/// Build a EffectiveMembership from a membership config entry
impl<NID, N, Ent> From<&Ent> for EffectiveMembership<NID, N>
where
    N: Node,
    NID: NodeId,
    Ent: RaftEntry<NID, N>,
{
    fn from(v: &Ent) -> Self {
        EffectiveMembership::new(Some(*v.get_log_id()), v.get_membership().unwrap().clone())
    }
}

impl<NID, N> EffectiveMembership<NID, N>
where
    N: Node,
    NID: NodeId,
{
    pub(crate) fn new_arc(log_id: Option<LogId<NID>>, membership: Membership<NID, N>) -> Arc<Self> {
        Arc::new(Self::new(log_id, membership))
    }

    pub fn new(log_id: Option<LogId<NID>>, membership: Membership<NID, N>) -> Self {
        let voter_ids = membership.voter_ids().collect();

        let configs = membership.get_joint_config();
        let mut joint = vec![];
        for c in configs {
            joint.push(c.iter().copied().collect::<Vec<_>>());
        }

        let quorum_set = Joint::from(joint);

        Self {
            stored_membership: Arc::new(StoredMembership::new(log_id, membership)),
            quorum_set,
            voter_ids,
        }
    }

    pub(crate) fn new_from_stored_membership(stored: StoredMembership<NID, N>) -> Self {
        Self::new(*stored.log_id(), stored.membership().clone())
    }

    pub(crate) fn stored_membership(&self) -> &Arc<StoredMembership<NID, N>> {
        &self.stored_membership
    }

    pub fn log_id(&self) -> &Option<LogId<NID>> {
        self.stored_membership.log_id()
    }

    pub fn membership(&self) -> &Membership<NID, N> {
        self.stored_membership.membership()
    }
}

/// Membership API
impl<NID, N> EffectiveMembership<NID, N>
where
    N: Node,
    NID: NodeId,
{
    #[allow(dead_code)]
    pub(crate) fn is_voter(&self, nid: &NID) -> bool {
        self.membership().is_voter(nid)
    }

    /// Returns an Iterator of all voter node ids. Learners are not included.
    pub fn voter_ids(&self) -> impl Iterator<Item = NID> + '_ {
        self.voter_ids.iter().copied()
    }

    /// Returns an Iterator of all learner node ids. Voters are not included.
    pub(crate) fn learner_ids(&self) -> impl Iterator<Item = NID> + '_ {
        self.membership().learner_ids()
    }

    /// Get a the node(either voter or learner) by node id.
    pub fn get_node(&self, node_id: &NID) -> Option<&N> {
        self.membership().get_node(node_id)
    }

    /// Returns an Iterator of all nodes(voters and learners).
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

impl<NID, N> MessageSummary<EffectiveMembership<NID, N>> for EffectiveMembership<NID, N>
where
    N: Node,
    NID: NodeId,
{
    fn summary(&self) -> String {
        format!(
            "{{log_id:{:?} membership:{}}}",
            self.log_id(),
            self.membership().summary()
        )
    }
}

/// Implement node-id joint quorum set.
impl<NID, N> QuorumSet<NID> for EffectiveMembership<NID, N>
where
    N: Node,
    NID: NodeId,
{
    type Iter = std::collections::btree_set::IntoIter<NID>;

    fn is_quorum<'a, I: Iterator<Item = &'a NID> + Clone>(&self, ids: I) -> bool {
        self.quorum_set.is_quorum(ids)
    }

    fn ids(&self) -> Self::Iter {
        self.quorum_set.ids()
    }
}
