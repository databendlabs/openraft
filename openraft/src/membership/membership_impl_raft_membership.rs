use std::collections::BTreeMap;
use std::collections::BTreeSet;

use crate::Membership;
use crate::Node;
use crate::NodeId;
use crate::membership::NodeRef;
use crate::membership::NodeRole;
use crate::membership::RaftMembership;
use crate::quorum::QuorumBridge;
use crate::quorum::QuorumIntersection;

impl<NID, N> RaftMembership for Membership<NID, N>
where
    NID: NodeId,
    N: Node,
{
    type NodeId = NID;
    type Node = N;
    type QuorumSet = Vec<BTreeSet<NID>>;

    fn read_quorum_set(&self) -> Self::QuorumSet {
        self.configs.clone()
    }

    fn write_quorum_set(&self) -> Self::QuorumSet {
        self.configs.clone()
    }

    /// `Membership` uses one quorum set for reads and writes, so one
    /// [`intersects_with()`](QuorumIntersection::intersects_with) call checks both directions.
    fn supports_direct_transition_to(&self, other: &Self) -> bool {
        let intersects = self.configs.intersects_with(&other.configs);
        intersects == Some(true)
    }

    fn get_node(&self, node_id: &Self::NodeId) -> Option<NodeRef<'_, Self::Node>> {
        let node = self.nodes.get(node_id)?;
        let node_ref = NodeRef {
            node,
            role: self.node_role(node_id),
            is_read_voter: self.is_read_voter(node_id),
            is_write_voter: self.is_write_voter(node_id),
        };
        Some(node_ref)
    }

    type Iter = std::vec::IntoIter<NID>;

    fn node_ids(&self) -> Self::Iter {
        let keys = self.nodes.keys();
        let ids: Vec<NID> = keys.cloned().collect();
        ids.into_iter()
    }

    fn drop_voters_demoted_to_learners(&self, goal: Self) -> Self {
        let mut nodes = BTreeMap::new();
        for (node_id, node) in goal.nodes() {
            let was_voter = self.is_read_voter(node_id) || self.is_write_voter(node_id);
            if was_voter && goal.is_learner(node_id) {
                continue;
            }
            nodes.insert(node_id.clone(), node.clone());
        }

        Membership::new_unchecked(goal.configs, nodes)
    }

    fn joint_with(&self, goal: &Self) -> Self {
        let configs = self.configs.joint_with(goal.configs.clone());

        let mut nodes = goal.nodes.clone();
        for voter_id in self.voter_ids() {
            // Safe unwrap(): a voter of `self` is a node that `get_node()` knows.
            let node = self.get_node(&voter_id).unwrap();
            nodes.entry(voter_id).or_insert_with(|| node.clone());
        }

        Membership::new_unchecked(configs, nodes)
    }
}

impl<NID, N> Membership<NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// Returns the capabilities this majority-based membership gives a known node.
    ///
    /// Every known node receives logs, and every voter may campaign.
    fn node_role(&self, node_id: &NID) -> NodeRole {
        let mut role = NodeRole::ReceiveLogs;
        if self.is_campaigner(node_id) {
            role |= NodeRole::Campaigner;
        }
        role
    }
}

#[cfg(test)]
mod tests {
    use maplit::btreemap;
    use maplit::btreeset;

    use crate::Membership;
    use crate::membership::NodeRef;
    use crate::membership::NodeRole;
    use crate::membership::RaftMembership;

    #[test]
    fn get_node_derives_role_and_voter_flags() {
        let configs = vec![btreeset! {1, 2}, btreeset! {2, 3}];
        let m = Membership::<u64, ()>::new_with_defaults(configs, btreeset! {4});

        let voter = RaftMembership::get_node(&m, &3);
        let want = Some(NodeRef {
            node: &(),
            role: NodeRole::Campaigner | NodeRole::ReceiveLogs,
            is_read_voter: true,
            is_write_voter: true,
        });
        assert_eq!(want, voter);

        let learner = RaftMembership::get_node(&m, &4);
        let want = Some(NodeRef {
            node: &(),
            role: NodeRole::ReceiveLogs,
            is_read_voter: false,
            is_write_voter: false,
        });
        assert_eq!(want, learner);

        let absent = RaftMembership::get_node(&m, &5);
        assert_eq!(None, absent);
    }

    #[test]
    fn node_ids_lists_voters_and_learners() {
        let m = Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1, 2}], btreeset! {3});

        let ids: Vec<u64> = RaftMembership::node_ids(&m).collect();
        assert_eq!(vec![1, 2, 3], ids);
    }

    #[test]
    fn drop_voters_demoted_to_learners_drops_old_voters_outside_goal_configs() {
        let old = Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1, 2, 3}], [4]);
        // Voters 1 and 2 leave the configs, and learner 4 stays a learner.
        let goal = Membership::<u64, ()>::new_with_defaults(vec![btreeset! {3}], [1, 2, 4]);

        let res = old.drop_voters_demoted_to_learners(goal);

        let want = Membership::<u64, ()>::new_with_defaults(vec![btreeset! {3}], [4]);
        assert_eq!(want, res);
    }

    #[test]
    fn joint_with_keeps_goal_nodes_and_old_voters() {
        let old_nodes = btreemap! {1 => 10, 2 => 20, 3 => 30, 4 => 40, 5 => 50};
        let old = Membership::<u64, u64>::new(vec![btreeset! {1, 2, 3}], old_nodes).unwrap();
        // Learner 4 replaces voter 1, learner 5 leaves, and voter 2 gets a new node.
        let goal_nodes = btreemap! {2 => 21, 3 => 30, 4 => 40};
        let goal = Membership::<u64, u64>::new(vec![btreeset! {2, 3, 4}], goal_nodes).unwrap();

        let joint = old.joint_with(&goal);

        // Old voter 1 keeps its node, and voter 2 takes its node in `goal`.
        let configs = vec![btreeset! {1, 2, 3}, btreeset! {2, 3, 4}];
        let nodes = btreemap! {1 => 10, 2 => 21, 3 => 30, 4 => 40};
        let want = Membership::<u64, u64>::new(configs, nodes).unwrap();
        assert_eq!(want, joint);
    }
}
