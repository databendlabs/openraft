use std::collections::BTreeMap;
use std::collections::BTreeSet;

use openraft_macros::since;

use crate::Membership;
use crate::Node;
use crate::NodeId;
use crate::errors::MembershipError;
use crate::impls::ChangeMembers;
use crate::membership::NodeRef;
use crate::membership::NodeRole;
use crate::membership::RaftMembership;
use crate::membership::RaftMembershipEdit;

impl<NID, N> RaftMembership for Membership<NID, N>
where
    NID: NodeId,
    N: Node,
{
    type NodeId = NID;
    type Node = N;
    type QuorumSet = Vec<BTreeSet<NID>>;

    fn read_quorum_set(&self) -> &Self::QuorumSet {
        &self.configs
    }

    fn write_quorum_set(&self) -> &Self::QuorumSet {
        &self.configs
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

    /// # Panics
    ///
    /// `Membership` keeps one joint config for both quorum sets and derives every node's role
    /// from it. This panics if `read_quorum_set` differs from `write_quorum_set`, or if a
    /// supplied role differs from the derived role.
    fn new<I>(read_quorum_set: Self::QuorumSet, write_quorum_set: Self::QuorumSet, nodes: I) -> Self
    where I: IntoIterator<Item = (Self::NodeId, Self::Node, NodeRole)> {
        let same_rules = read_quorum_set == write_quorum_set;
        assert!(
            same_rules,
            "Membership uses one joint config for both read and write quorum sets"
        );

        let mut membership = Membership {
            configs: read_quorum_set,
            nodes: BTreeMap::new(),
        };
        for (node_id, node, role) in nodes {
            let derived_role = membership.node_role(&node_id);
            assert_eq!(
                role, derived_role,
                "Membership derives the role of node {node_id} from its configs"
            );
            membership.nodes.insert(node_id, node);
        }
        membership
    }
}

impl<NID, N> Membership<NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// Applies a built-in edit to the target configuration.
    ///
    /// # Panics
    ///
    /// Panics if this membership has no configuration after applying the edit.
    #[since(version = "0.10.0")]
    pub fn change(&self, change: ChangeMembers<NID, N>) -> Result<Self, MembershipError<NID>> {
        let result = change.apply(self);
        result.map_err(|error| error.map_user_defined(|never| match never {}))
    }

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
    fn new_uses_one_joint_config_for_both_quorum_sets() {
        let configs = vec![btreeset! {1, 2}, btreeset! {2, 3}];
        let voter = NodeRole::Campaigner | NodeRole::ReceiveLogs;
        let nodes = [
            (1, (), voter),
            (2, (), voter),
            (3, (), voter),
            (4, (), NodeRole::ReceiveLogs),
        ];

        let m = <Membership<u64, ()> as RaftMembership>::new(configs.clone(), configs.clone(), nodes);

        let node_map = btreemap! {1 => (), 2 => (), 3 => (), 4 => ()};
        let want = Membership::<u64, ()>::new_unchecked(configs.clone(), node_map);
        assert_eq!(want, m);

        let read_quorum_set = RaftMembership::read_quorum_set(&m);
        assert_eq!(&configs, read_quorum_set);

        let write_quorum_set = RaftMembership::write_quorum_set(&m);
        assert_eq!(&configs, write_quorum_set);
    }

    #[test]
    #[should_panic(expected = "one joint config")]
    fn new_panics_on_different_read_and_write_sets() {
        let read = vec![btreeset! {1, 2, 3}];
        let write = vec![btreeset! {1, 2, 3}, btreeset! {4, 5, 6}];
        let nodes: [(u64, (), NodeRole); 0] = [];

        <Membership<u64, ()> as RaftMembership>::new(read, write, nodes);
    }

    #[test]
    #[should_panic(expected = "derives the role of node 1")]
    fn new_panics_on_role_that_configs_do_not_give() {
        let configs = vec![btreeset! {1}];
        let nodes = [(1, (), NodeRole::ReceiveLogs)];

        <Membership<u64, ()> as RaftMembership>::new(configs.clone(), configs, nodes);
    }
}
