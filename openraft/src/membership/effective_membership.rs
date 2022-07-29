use std::collections::BTreeSet;
use std::fmt::Debug;

use crate::entry::RaftEntry;
use crate::membership::NodeRole;
use crate::quorum::Joint;
use crate::quorum::QuorumSet;
use crate::raft_types::RaftLogId;
use crate::LogId;
use crate::Membership;
use crate::MessageSummary;
use crate::Node;
use crate::NodeType;

type JoinQuorumSet<NID> = Joint<NID, Vec<NID>, Vec<Vec<NID>>>;
/// The currently active membership config.
///
/// It includes:
/// - the id of the log that sets this membership config,
/// - and the config.
///
/// An active config is just the last seen config in raft spec.
#[derive(Clone, Default, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct EffectiveMembership<NT>
where NT: NodeType
{
    /// The id of the log that applies this membership config
    pub log_id: Option<LogId<NT::NodeId>>,

    pub membership: Membership<NT>,

    /// The quorum set built from `membership`.
    // #[serde(skip_serialize)]
    // #[serde(deserialize_wit="")]
    quorum_set: JoinQuorumSet<NT::NodeId>,

    /// Cache of union of all members
    voter_ids: BTreeSet<NT::NodeId>,
}

impl<NT> Debug for EffectiveMembership<NT>
where NT: NodeType
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EffectiveMembership")
            .field("log_id", &self.log_id)
            .field("membership", &self.membership)
            .field("all_members", &self.voter_ids)
            .finish()
    }
}

impl<NT> PartialEq for EffectiveMembership<NT>
where NT: NodeType
{
    fn eq(&self, other: &Self) -> bool {
        self.log_id == other.log_id && self.membership == other.membership && self.voter_ids == other.voter_ids
    }
}

impl<NT, LID> From<(&LID, Membership<NT>)> for EffectiveMembership<NT>
where
    NT: NodeType,
    LID: RaftLogId<NT::NodeId>,
{
    fn from(v: (&LID, Membership<NT>)) -> Self {
        EffectiveMembership::new(Some(*v.0.get_log_id()), v.1)
    }
}

/// Build a EffectiveMembership from a membership config entry
impl<NT, Ent> From<&Ent> for EffectiveMembership<NT>
where
    NT: NodeType,
    Ent: RaftEntry<NT>,
{
    fn from(v: &Ent) -> Self {
        EffectiveMembership::new(Some(*v.get_log_id()), v.get_membership().unwrap().clone())
    }
}

impl<NT> EffectiveMembership<NT>
where NT: NodeType
{
    pub fn new(log_id: Option<LogId<NT::NodeId>>, membership: Membership<NT>) -> Self {
        let voter_ids = membership.voter_ids().collect();

        let configs = membership.get_joint_config();
        let mut joint = vec![];
        for c in configs {
            joint.push(c.iter().copied().collect::<Vec<_>>());
        }

        let quorum_set = Joint::from(joint);

        Self {
            log_id,
            membership,
            quorum_set,
            voter_ids,
        }
    }
}

/// Membership API
impl<NT> EffectiveMembership<NT>
where NT: NodeType
{
    /// Return if a node is a voter or learner, or not in this membership config at all.
    pub(crate) fn get_node_role(&self, nid: &NT::NodeId) -> Option<NodeRole> {
        if self.voter_ids.contains(nid) {
            Some(NodeRole::Voter)
        } else if self.contains(nid) {
            Some(NodeRole::Learner)
        } else {
            None
        }
    }

    #[allow(dead_code)]
    pub(crate) fn is_voter(&self, nid: &NT::NodeId) -> bool {
        self.membership.is_voter(nid)
    }

    /// Returns an Iterator of all voter node ids. Learners are not included.
    pub fn voter_ids(&self) -> impl Iterator<Item = NT::NodeId> + '_ {
        self.voter_ids.iter().copied()
    }

    /// Returns an Iterator of all learner node ids. Voters are not included.
    #[allow(dead_code)]
    pub(crate) fn learner_ids(&self) -> impl Iterator<Item = NT::NodeId> + '_ {
        self.membership.learner_ids()
    }

    /// Returns if a voter or learner exists in this membership.
    pub(crate) fn contains(&self, id: &NT::NodeId) -> bool {
        self.membership.contains(id)
    }

    /// Get a the node(either voter or learner) by node id.
    pub fn get_node(&self, node_id: &NT::NodeId) -> Option<&Node<NT>> {
        self.membership.get_node(node_id)
    }

    /// Returns an Iterator of all nodes(voters and learners).
    pub fn nodes(&self) -> impl Iterator<Item = (&NT::NodeId, &Option<Node<NT>>)> {
        self.membership.nodes()
    }

    /// Returns reference to the joint config.
    ///
    /// Membership is defined by a joint of multiple configs.
    /// Each config is a vec of node-id.
    pub fn get_joint_config(&self) -> &Vec<Vec<NT::NodeId>> {
        self.quorum_set.children()
    }
}

impl<NT> MessageSummary<EffectiveMembership<NT>> for EffectiveMembership<NT>
where NT: NodeType
{
    fn summary(&self) -> String {
        format!("{{log_id:{:?} membership:{}}}", self.log_id, self.membership.summary())
    }
}

/// Implement node-id joint quorum set.
impl<NT> QuorumSet<NT::NodeId> for EffectiveMembership<NT>
where NT: NodeType
{
    type Iter = std::collections::btree_set::IntoIter<NT::NodeId>;

    fn is_quorum<'a, I: Iterator<Item = &'a NT::NodeId> + Clone>(&self, ids: I) -> bool {
        self.quorum_set.is_quorum(ids)
    }

    fn ids(&self) -> Self::Iter {
        self.quorum_set.ids()
    }
}
