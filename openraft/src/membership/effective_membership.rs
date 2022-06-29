use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt::Debug;

use crate::entry::RaftEntry;
use crate::quorum::Joint;
use crate::quorum::QuorumSet;
use crate::raft_types::RaftLogId;
use crate::LogId;
use crate::Membership;
use crate::MessageSummary;
use crate::Node;
use crate::NodeId;

/// The currently active membership config.
///
/// It includes:
/// - the id of the log that sets this membership config,
/// - and the config.
///
/// An active config is just the last seen config in raft spec.
#[derive(Clone, Default, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct EffectiveMembership<NID: NodeId> {
    /// The id of the log that applies this membership config
    pub log_id: Option<LogId<NID>>,

    pub membership: Membership<NID>,

    /// The quorum set built from `membership`.
    #[serde(skip)]
    node_id_quorum_set: Joint<NID, Vec<NID>, Vec<Vec<NID>>>,

    // --- TODO: use node index as QuorumSet element and progress element
    // /// The quorum set built from `membership`.
    // #[serde(skip)]
    // node_index_quorum_set: Joint<usize, Vec<usize>, Vec<Vec<usize>>>,
    //
    // /// Sorted node ids for mapping node-id to node-index
    // #[serde(skip)]
    // member_node_ids: Vec<usize>,
    // ---
    /// Cache of union of all members
    all_members: BTreeSet<NID>,
}

impl<NID: NodeId> Debug for EffectiveMembership<NID> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EffectiveMembership")
            .field("log_id", &self.log_id)
            .field("membership", &self.membership)
            .field("all_members", &self.all_members)
            .finish()
    }
}

impl<NID: NodeId> PartialEq for EffectiveMembership<NID> {
    fn eq(&self, other: &Self) -> bool {
        self.log_id == other.log_id && self.membership == other.membership && self.all_members == other.all_members
    }
}

impl<NID: NodeId, LID: RaftLogId<NID>> From<(&LID, Membership<NID>)> for EffectiveMembership<NID> {
    fn from(v: (&LID, Membership<NID>)) -> Self {
        EffectiveMembership::new(Some(*v.0.get_log_id()), v.1)
    }
}

/// Build a EffectiveMembership from a membership config entry
impl<NID: NodeId, Ent: RaftEntry<NID>> From<&Ent> for EffectiveMembership<NID> {
    fn from(v: &Ent) -> Self {
        EffectiveMembership::new(Some(*v.get_log_id()), v.get_membership().unwrap().clone())
    }
}

impl<NID: NodeId> EffectiveMembership<NID> {
    pub fn new(log_id: Option<LogId<NID>>, membership: Membership<NID>) -> Self {
        let all_members = membership.build_member_ids();

        let configs = membership.get_configs();
        let mut vec_configs = vec![];
        for c in configs {
            vec_configs.push(c.iter().copied().collect::<Vec<_>>());
        }

        let node_id_quorum_set = Joint::from(vec_configs);

        // let mut member_node_ids = node_id_quorum_set.ids().collect::<Vec<_>>();
        // member_node_ids.sort();
        //
        // fn node_id_to_index(nid: &NID) -> usize {
        //     member_node_ids.iter().position(|&e| e == nid)
        // }
        //
        // let mut node_index_configs = vec![];
        // for c in configs {
        //     node_index_configs.push(c.iter().map(node_id_to_index).collect::<Vec<_>>());
        // }
        // let node_index_quorum_set = Joint::from(node_index_configs);

        Self {
            log_id,
            membership,
            node_id_quorum_set,
            // node_index_quorum_set,
            // member_node_ids,
            all_members,
        }
    }

    pub(crate) fn is_single(&self) -> bool {
        // an empty membership contains no nodes.
        self.all_members().len() <= 1
    }

    pub(crate) fn all_members(&self) -> &BTreeSet<NID> {
        &self.all_members
    }

    /// Node id of all nodes(members and learners) in this config.
    pub(crate) fn node_ids(&self) -> impl Iterator<Item = &NID> {
        self.membership.node_ids()
    }

    /// Returns reference to all configs.
    ///
    /// Membership is defined by a joint of multiple configs.
    /// Each config is a set of node-id.
    /// This method returns a immutable reference to the vec of node-id sets, without node infos.
    pub fn get_configs(&self) -> &Vec<BTreeSet<NID>> {
        self.membership.get_configs()
    }

    /// Get a the node info by node id.
    pub fn get_node(&self, node_id: &NID) -> Option<&Node> {
        self.membership.get_node(node_id)
    }

    /// Get all node infos.
    pub fn get_nodes(&self) -> &BTreeMap<NID, Option<Node>> {
        self.membership.get_nodes()
    }
}

impl<NID: NodeId> MessageSummary<EffectiveMembership<NID>> for EffectiveMembership<NID> {
    fn summary(&self) -> String {
        format!("{{log_id:{:?} membership:{}}}", self.log_id, self.membership.summary())
    }
}

/// Implement node-id joint quorum set.
impl<NID: NodeId> QuorumSet<NID> for EffectiveMembership<NID> {
    type Iter = std::collections::btree_set::IntoIter<NID>;

    fn is_quorum<'a, I: Iterator<Item = &'a NID> + Clone>(&self, ids: I) -> bool {
        self.node_id_quorum_set.is_quorum(ids)
    }

    fn ids(&self) -> Self::Iter {
        self.node_id_quorum_set.ids()
    }
}
