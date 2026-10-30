use openraft::Node;
use openraft::NodeId;
use openraft::membership::NodeRef;
use openraft::membership::NodeRole;
use openraft::membership::QuorumIntersection;
use openraft::membership::RaftMembership;

use crate::ThresholdMembership;
use crate::quorum::Joint;
use crate::quorum::QuorumBridge;
use crate::quorum::ThresholdQuorumSet;

impl<NID, N> RaftMembership for ThresholdMembership<NID, N>
where
    NID: NodeId,
    N: Node,
{
    type NodeId = NID;
    type Node = N;
    type QuorumSet = Joint<ThresholdQuorumSet<NID>>;

    fn read_quorum_set(&self) -> Self::QuorumSet {
        self.read.clone()
    }

    fn write_quorum_set(&self) -> Self::QuorumSet {
        self.write.clone()
    }

    fn supports_direct_transition_to(&self, other: &Self) -> bool {
        let read_meets_write = self.read.intersects_with(&other.write);
        if read_meets_write != Some(true) {
            return false;
        }

        let write_meets_read = self.write.intersects_with(&other.read);
        write_meets_read == Some(true)
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

    fn joint_with(&self, goal: &Self) -> Self {
        let read = self.read.joint_with(goal.read.clone());
        let write = self.write.joint_with(goal.write.clone());

        let mut nodes = goal.nodes.clone();
        for (node_id, node) in &self.nodes {
            if self.is_voter(node_id) {
                nodes.entry(node_id.clone()).or_insert_with(|| node.clone());
            }
        }

        Self { read, write, nodes }
    }
}

impl<NID, N> ThresholdMembership<NID, N>
where
    NID: NodeId,
    N: Node,
{
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
