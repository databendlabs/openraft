use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt::Debug;

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

impl<NID: NodeId> EffectiveMembership<NID> {
    pub fn new(log_id: Option<LogId<NID>>, membership: Membership<NID>) -> Self {
        let all_members = membership.build_member_ids();
        Self {
            log_id,
            membership,
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

impl<NID: NodeId> MessageSummary for EffectiveMembership<NID> {
    fn summary(&self) -> String {
        format!("{{log_id:{:?} membership:{}}}", self.log_id, self.membership.summary())
    }
}
