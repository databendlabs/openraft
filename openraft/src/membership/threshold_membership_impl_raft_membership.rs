use std::collections::BTreeMap;
use std::sync::Arc;

use crate::Membership;
use crate::Node;
use crate::NodeId;
use crate::errors::EmptyMembership;
use crate::errors::MembershipError;
use crate::membership::NodeRef;
use crate::membership::NodeRole;
use crate::membership::RaftMembership;
use crate::membership::ThresholdChange;
use crate::membership::ThresholdMembership;
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

    type Edit = ThresholdChange<NID, N>;

    /// Applies `change.members` to the voters and the nodes of the target config, through the
    /// same rules as [`Membership`]. Keeps the write size unless `change` sets it, then derives
    /// the read size from the new voter count.
    ///
    /// Returns [`MembershipError::EmptyMembership`] for a membership with no config. Returns
    /// [`InvalidThresholds`](crate::membership::InvalidThresholds) inside
    /// [`MembershipError::UserDefined`] when the write size does not fit the new voters.
    fn change(&self, change: ThresholdChange<NID, N>) -> Result<Self, MembershipError<NID>> {
        tracing::debug!("{}: change: {}", func_name!(), change);

        let Some(last_write) = self.write.rules().last() else {
            return Err(EmptyMembership {}.into());
        };

        let (voters, nodes) = match change.members {
            None => (last_write.id_set().clone(), self.nodes.clone()),
            Some(members) => {
                let last_voters = last_write.id_set().as_ref().clone();
                let current = Membership::new_unchecked(vec![last_voters], self.nodes.clone());
                let Membership { mut configs, nodes } = current.compute_target_membership(members);

                // Safe unwrap(): `compute_target_membership()` keeps one config when it gets one.
                let voters = configs.pop().unwrap();
                (Arc::new(voters), nodes)
            }
        };

        let write_size = change.write_size.unwrap_or(last_write.quorum_size());
        let voter_count = voters.len() as u64;
        let write_size = Self::check_write_size(voter_count, write_size)?;

        let target = Self::from_configs([(voters, write_size)], nodes);

        tracing::debug!("target membership: {}", target);

        Ok(target)
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
