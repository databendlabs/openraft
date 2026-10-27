use std::collections::BTreeMap;

use openraft::Node;
use openraft::NodeId;
use openraft::errors::MembershipError;
use openraft::membership::NodeRef;
use openraft::membership::NodeRole;
use openraft::membership::RaftMembership;
use openraft::membership::RaftMembershipEdit;
use openraft_macros::since;

use crate::ThresholdChange;
use crate::ThresholdMembership;
use crate::quorum::Joint;
use crate::quorum::ThresholdQuorumSet;

impl<NID, N> RaftMembership for ThresholdMembership<NID, N>
where
    NID: NodeId,
    N: Node,
{
    type NodeId = NID;
    type Node = N;
    type QuorumSet = Joint<ThresholdQuorumSet<NID>>;

    fn read_quorum_set(&self) -> &Self::QuorumSet {
        &self.read
    }

    fn write_quorum_set(&self) -> &Self::QuorumSet {
        &self.write
    }

    fn get_node(&self, node_id: &Self::NodeId) -> Option<NodeRef<'_, Self::Node>> {
        let node = self.nodes.get(node_id)?;
        let is_voter = self.is_voter(node_id);
        let node_ref = NodeRef {
            node,
            role: self.node_role(node_id),
            is_read_voter: is_voter,
            is_write_voter: is_voter,
        };
        Some(node_ref)
    }

    type Iter = std::vec::IntoIter<NID>;

    fn node_ids(&self) -> Self::Iter {
        let keys = self.nodes.keys();
        let ids: Vec<NID> = keys.cloned().collect();
        ids.into_iter()
    }

    /// # Panics
    ///
    /// The read rule and the write rule of a config share one voter set, and every role comes
    /// from the configs. This panics if rule i of `read_quorum_set` and rule i of
    /// `write_quorum_set` hold different voters, if their sizes fail the checks of
    /// [`ThresholdMembership::new()`], or if a supplied role differs from the role that the
    /// configs give.
    fn new<I>(read_quorum_set: Self::QuorumSet, write_quorum_set: Self::QuorumSet, nodes: I) -> Self
    where I: IntoIterator<Item = (Self::NodeId, Self::Node, NodeRole)> {
        let configs = Self::configs_from_rules(&read_quorum_set, &write_quorum_set);
        let configs = configs.unwrap_or_else(|e| panic!("invalid ThresholdMembership quorum sets: {e}"));

        let mut membership = Self::from_configs(configs, BTreeMap::new());
        for (node_id, node, role) in nodes {
            let derived_role = membership.node_role(&node_id);
            assert_eq!(
                role, derived_role,
                "ThresholdMembership derives the role of node {node_id} from its configs"
            );
            membership.nodes.insert(node_id, node);
        }
        membership
    }
}

impl<NID, N> ThresholdMembership<NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// Applies an edit through [`RaftMembershipEdit::apply()`].
    ///
    /// Returns [`MembershipError::EmptyMembership`] when there is no configuration and
    /// [`MembershipError::UserDefined`] when the requested size does not fit the voters.
    #[since(version = "0.10.0", change = "delegates to RaftMembershipEdit::apply()")]
    #[since(version = "0.10.0")]
    pub fn change(&self, change: ThresholdChange<NID, N>) -> Result<Self, MembershipError<NID>> {
        change.apply(self)
    }

    /// Returns the capabilities of a known node: every known node receives logs, and every voter
    /// may campaign.
    fn node_role(&self, node_id: &NID) -> NodeRole {
        let mut role = NodeRole::ReceiveLogs;
        if self.is_voter(node_id) {
            role |= NodeRole::Campaigner;
        }
        role
    }
}
