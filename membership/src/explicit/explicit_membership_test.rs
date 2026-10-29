use std::collections::BTreeMap;
use std::collections::BTreeSet;

use maplit::btreemap;
use maplit::btreeset;
use openraft::Node;
use openraft::errors::EmptyMembership;
use openraft::errors::MembershipError;
use openraft::errors::NodeNotFound;
use openraft::errors::Operation;
use openraft::membership::IntoNodes;
use openraft::membership::NodeRef;
use openraft::membership::NodeRole;
use openraft::membership::RaftMembership;
use openraft::membership::RaftMembershipEdit;
use openraft::membership::errors::WriteVoterWithoutReceiveLogs;
use quorum_set::verify_intersection;

use crate::ExplicitChange;
use crate::ExplicitMembership;
use crate::errors::InvalidQuorumLists;
use crate::quorum::ExplicitQuorumSet;
use crate::quorum::QuorumSet;
use crate::quorum::testing::accepted;
use crate::quorum::testing::all_subsets;
use crate::quorum::testing::holding_one_of;
use crate::testing::assert_apply_checks_roles;
use crate::testing::assert_change_through_joint_keeps_node_metadata;
use crate::testing::assert_claims_hold;
use crate::testing::assert_joint_keeps_old_roles_and_target_restores_new_roles;
use crate::testing::assert_quorum_sets_are_valid;
use crate::testing::assert_role_and_quorum_change_through_joint;
use crate::testing::assert_with_roles;
use crate::testing::voter_role;

type Membership = ExplicitMembership<u64, ()>;

/// Returns a membership whose only config lists these quorums, with a `()` node for each voter.
fn explicit(read: Vec<BTreeSet<u64>>, write: Vec<BTreeSet<u64>>) -> Membership {
    let mut nodes = BTreeSet::new();
    for quorum in read.iter().chain(&write) {
        nodes.extend(quorum);
    }
    ExplicitMembership::new(read, write, nodes).unwrap()
}

/// Returns every subset of `voters` that holds `voters.len() / 2 + 1` voters.
fn majorities(voters: &BTreeSet<u64>) -> Vec<BTreeSet<u64>> {
    let size = voters.len() / 2 + 1;
    let mut majorities = all_subsets(voters);
    majorities.retain(|subset| subset.len() == size);
    majorities
}

/// Returns a membership whose only config lists the majorities of `voters` as its read quorums and
/// its write quorums, with each node of `nodes`.
fn majority<N, T>(voters: BTreeSet<u64>, nodes: T) -> ExplicitMembership<u64, N>
where
    N: Node,
    T: IntoNodes<u64, N>,
{
    let quorums = majorities(&voters);
    ExplicitMembership::new(quorums.clone(), quorums, nodes).unwrap()
}

#[test]
fn test_new() {
    let read = vec![btreeset! {1, 2}, btreeset! {2, 3}, btreeset! {1, 2, 3}];
    let write = vec![btreeset! {2}];
    let membership = ExplicitMembership::<u64, ()>::new(read, write, btreeset! {1, 2, 3, 4}).unwrap();
    let want = "{configs: [Config { read: ExplicitQuorumSet { quorums: [{1, 2}, {2, 3}] }, write: ExplicitQuorumSet { \
                quorums: [{2}] }, nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), \
                2: ((), NodeRole(Campaigner | ReceiveLogs)), 3: ((), NodeRole(Campaigner | ReceiveLogs)), \
                4: ((), NodeRole(ReceiveLogs))} }]}";
    assert_eq!(want, membership.to_string());

    let cases = [
        (vec![], vec![btreeset! {1}], InvalidQuorumLists::NoReadQuorum),
        (vec![btreeset! {1}], vec![], InvalidQuorumLists::NoWriteQuorum),
        (
            vec![BTreeSet::new()],
            vec![btreeset! {1}],
            InvalidQuorumLists::EmptyReadQuorum,
        ),
        (
            vec![btreeset! {1}],
            vec![BTreeSet::new()],
            InvalidQuorumLists::EmptyWriteQuorum,
        ),
    ];
    for (read, write, invalid) in cases {
        let res = Membership::new(read, write, btreeset! {1});
        assert_eq!(Err(MembershipError::UserDefined(invalid)), res);
    }

    // {1,2} meets both write quorums, but {3} misses {1,4}.
    let read = [btreeset! {1, 2}, btreeset! {3}];
    let write = [btreeset! {2, 3}, btreeset! {1, 4}];
    let res = Membership::new(read, write, btreeset! {1, 2, 3, 4});
    let disjoint = InvalidQuorumLists::DisjointQuorums {
        read: btreeset! {3},
        write: btreeset! {1, 4},
    };
    assert_eq!(Err(MembershipError::UserDefined(disjoint)), res);

    // Voter 3 has no node.
    let res = Membership::new([btreeset! {1, 3}], [btreeset! {1, 2}], btreeset! {1, 2});
    let want = Err(MembershipError::NodeNotFound(NodeNotFound::new(3, Operation::None)));
    assert_eq!(want, res);
}

#[test]
fn test_quorum_sets_accept_exactly_the_sets_holding_a_listed_quorum() {
    let read = vec![btreeset! {1, 3}, btreeset! {1, 4}, btreeset! {2, 3}, btreeset! {2, 4}];
    let write = vec![btreeset! {1, 2}, btreeset! {3, 4}];
    let membership = explicit(read.clone(), write.clone());
    let subsets = all_subsets(&btreeset! {1, 2, 3, 4});

    let read_accepted = accepted(&membership.read_quorum_set(), &subsets);
    let want = holding_one_of(&read, &subsets);
    assert_eq!(9, want.len());
    assert_eq!(want, read_accepted);

    let write_accepted = accepted(&membership.write_quorum_set(), &subsets);
    let want = holding_one_of(&write, &subsets);
    assert_eq!(7, want.len());
    assert_eq!(want, write_accepted);

    // Read quorums {1,3} and {2,4} are disjoint, and so are write quorums {1,2} and {3,4}.
    let accepts = membership.read_quorum_set().is_quorum([2, 4].iter());
    assert!(accepts);
    let accepts = membership.write_quorum_set().is_quorum([3, 4].iter());
    assert!(accepts);
}

#[test]
fn test_get_node_reports_asymmetric_voter_sets() {
    // Node 1 only votes, node 3 only stores entries, and node 4 is a learner.
    let membership = Membership::new([btreeset! {1, 2}], [btreeset! {2, 3}], btreeset! {1, 2, 3, 4}).unwrap();

    let cases = [(1, true, false), (2, true, true), (3, false, true)];
    for (node_id, is_read_voter, is_write_voter) in cases {
        let got = membership.get_node(&node_id);
        let want = Some(NodeRef {
            node: &(),
            role: voter_role(),
            is_read_voter,
            is_write_voter,
        });
        assert_eq!(want, got, "node {node_id}");
    }

    let learner = membership.get_node(&4);
    let want = Some(NodeRef {
        node: &(),
        role: NodeRole::ReceiveLogs,
        is_read_voter: false,
        is_write_voter: false,
    });
    assert_eq!(want, learner);

    let absent = membership.get_node(&5);
    assert_eq!(None, absent);

    let ids: Vec<u64> = membership.node_ids().collect();
    assert_eq!(vec![1, 2, 3, 4], ids);
    let read_ids: Vec<u64> = membership.read_quorum_set().ids().collect();
    assert_eq!(vec![1, 2], read_ids);
    let write_ids: Vec<u64> = membership.write_quorum_set().ids().collect();
    assert_eq!(vec![2, 3], write_ids);

    // Voters 1 and 3 exchange read/write roles. The joint keeps both roles for each voter.
    let target = Membership::new([btreeset! {2, 3}], [btreeset! {1, 2}], btreeset! {1, 2, 3, 4}).unwrap();
    let target = target.with_roles(btreemap! {1 => NodeRole::ReceiveLogs, 3 => NodeRole::Empty}).unwrap();
    let joint = membership.joint_with(&target);
    for node_id in 1..=3 {
        let got = joint.get_node(&node_id);
        let want = Some(NodeRef {
            node: &(),
            role: voter_role(),
            is_read_voter: true,
            is_write_voter: true,
        });
        assert_eq!(want, got, "node {node_id}");
    }
    let uniform_change = ExplicitChange::make_uniform();
    let uniform = uniform_change.apply(&joint, true).unwrap();
    assert_eq!(target, uniform);
}

#[test]
fn test_from_node_map_requires_every_voter() {
    let nodes: BTreeMap<u64, ()> = btreemap! {1 => (), 2 => (), 3 => ()};
    let membership = Membership::from(nodes);

    let want = explicit(vec![btreeset! {1, 2, 3}], vec![btreeset! {1, 2, 3}]);
    assert_eq!(want, membership);

    let empty = Membership::from(BTreeMap::new());
    assert_eq!(Membership::default(), empty);
    assert_eq!("{configs: []}", empty.to_string());
}

#[test]
fn test_direct_transitions_need_the_same_lists() {
    let one_two = explicit(vec![btreeset! {1, 2}], vec![btreeset! {1, 2}]);
    let two_three = explicit(vec![btreeset! {2, 3}], vec![btreeset! {2, 3}]);
    let three_four = explicit(vec![btreeset! {3, 4}], vec![btreeset! {3, 4}]);

    let same = one_two.supports_direct_transition_to(&one_two);
    assert!(same);
    // Every old quorum meets every new quorum in voter 2, but the lists change, so the change
    // still goes through a joint.
    let shares_two = one_two.supports_direct_transition_to(&two_three);
    assert!(!shares_two);
    let disjoint = one_two.supports_direct_transition_to(&three_four);
    assert!(!disjoint);

    // Each check below holds in one direction only: reads on {2} miss writes on {1}.
    let read_two = explicit(vec![btreeset! {2}], vec![btreeset! {1, 2}]);
    let write_one = explicit(vec![btreeset! {1, 2}], vec![btreeset! {1}]);
    let old_reads_miss = read_two.supports_direct_transition_to(&write_one);
    assert!(!old_reads_miss);
    let new_reads_miss = write_one.supports_direct_transition_to(&read_two);
    assert!(!new_reads_miss);

    // A joint goes directly to its target, and adding a learner changes no quorum.
    let joint = one_two.joint_with(&three_four);
    let to_target = joint.supports_direct_transition_to(&three_four);
    assert!(to_target);
    let add_learner = <ExplicitChange<u64, ()> as RaftMembershipEdit<Membership>>::add_learner(9, ());
    let with_learner = add_learner.apply(&one_two, true).unwrap();
    let to_learner = one_two.supports_direct_transition_to(&with_learner);
    assert!(to_learner);
}

#[test]
fn test_small_memberships_go_directly_only_with_the_same_lists() {
    // Each single quorum of voters {1,2,3}, a chain of two quorums, and the majorities.
    let mut lists = vec![];
    for quorum in all_subsets(&btreeset! {1, 2, 3}) {
        if !quorum.is_empty() {
            lists.push(vec![quorum]);
        }
    }
    lists.push(vec![btreeset! {1, 2}, btreeset! {2, 3}]);
    lists.push(vec![btreeset! {1, 2}, btreeset! {1, 3}, btreeset! {2, 3}]);

    // Every membership of these lists whose read quorums meet its write quorums.
    let mut memberships = vec![];
    for read in &lists {
        for write in &lists {
            let read_rule = ExplicitQuorumSet::new(read.clone()).unwrap();
            let write_rule = ExplicitQuorumSet::new(write.clone()).unwrap();
            let intersect = verify_intersection(&read_rule, &write_rule);
            if intersect {
                let membership = explicit(read.clone(), write.clone());
                assert_quorum_sets_are_valid(&membership);
                memberships.push(membership);
            }
        }
    }

    for old in &memberships {
        for new in &memberships {
            assert_claims_hold(old, new);

            let same_read = old.read_quorum_set() == new.read_quorum_set();
            let same_write = old.write_quorum_set() == new.write_quorum_set();
            let want = same_read && same_write;
            let direct = old.supports_direct_transition_to(new);
            assert_eq!(want, direct, "{old} to {new}");
        }
    }
}

#[test]
fn test_joint_quorums_satisfy_both_configs() {
    let old = explicit(vec![btreeset! {1, 2}], vec![btreeset! {1, 2}]);
    let new = explicit(vec![btreeset! {3}], vec![btreeset! {2, 3}, btreeset! {3, 4}]);
    let joint = old.joint_with(&new);
    let subsets = all_subsets(&btreeset! {1, 2, 3, 4});

    let read_accepted = accepted(&joint.read_quorum_set(), &subsets);
    let want = holding_one_of(&[btreeset! {1, 2, 3}], &subsets);
    assert_eq!(want, read_accepted);

    let write_accepted = accepted(&joint.write_quorum_set(), &subsets);
    let want = holding_one_of(&[btreeset! {1, 2, 3}, btreeset! {1, 2, 3, 4}], &subsets);
    assert_eq!(want, write_accepted);

    let want = "{configs: [Config { read: ExplicitQuorumSet { quorums: [{1, 2}] }, write: ExplicitQuorumSet { \
                quorums: [{1, 2}] }, nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), \
                2: ((), NodeRole(Campaigner | ReceiveLogs))} }, Config { read: ExplicitQuorumSet { quorums: [{3}] }, \
                write: ExplicitQuorumSet { quorums: [{2, 3}, {3, 4}] }, \
                nodes: {2: ((), NodeRole(Campaigner | ReceiveLogs)), 3: ((), NodeRole(Campaigner | ReceiveLogs)), \
                4: ((), NodeRole(Campaigner | ReceiveLogs))} }]}";
    assert_eq!(want, joint.to_string());
}

#[test]
fn test_apply() {
    let nodes = btreemap! {1 => "a".to_string(), 2 => "b".to_string(), 3 => "c".to_string()};
    let all = btreeset! {1, 2, 3};
    let membership = ExplicitMembership::new([all.clone()], [all.clone()], nodes.clone()).unwrap();

    // A change of one list keeps the other.
    let singles = vec![btreeset! {1}, btreeset! {2}, btreeset! {3}];
    let change = ExplicitChange::read_quorums(singles.clone());
    let res = change.apply(&membership, true);
    let want = ExplicitMembership::new(singles.clone(), [all.clone()], nodes.clone());
    assert_eq!(want, res);

    let change = ExplicitChange::write_quorums(singles.clone());
    let res = change.apply(&membership, true);
    let want = ExplicitMembership::new([all.clone()], singles.clone(), nodes.clone());
    assert_eq!(want, res);

    let pairs = vec![btreeset! {1, 2}, btreeset! {2, 3}];
    let change = ExplicitChange::quorums(pairs.clone(), vec![btreeset! {2}]);
    let res = change.apply(&membership, true);
    let want = ExplicitMembership::new(pairs, [btreeset! {2}], nodes.clone());
    assert_eq!(want, res);

    // Adding nodes keeps the metadata of a known node; removing a learner drops it.
    let change = ExplicitChange {
        add_nodes: btreemap! {1 => "z".to_string(), 4 => "d".to_string()},
        ..ExplicitChange::default()
    };
    let with_four = change.apply(&membership, true).unwrap();
    let mut want_nodes = nodes.clone();
    want_nodes.insert(4, "d".to_string());
    let want = ExplicitMembership::new([all.clone()], [all.clone()], want_nodes).unwrap();
    assert_eq!(want, with_four);

    let change = ExplicitChange {
        remove_nodes: btreeset! {4},
        ..ExplicitChange::default()
    };
    let res = change.apply(&with_four, true);
    assert_eq!(Ok(membership.clone()), res);

    // Removing a voter fails, because every voter needs a node.
    let change = ExplicitChange {
        remove_nodes: btreeset! {3},
        ..ExplicitChange::default()
    };
    let res = change.apply(&membership, true);
    let want = Err(MembershipError::from(NodeNotFound::new(3, Operation::None)));
    assert_eq!(want, res);

    // Invalid lists are the edit's own errors.
    let change = ExplicitChange::write_quorums(vec![]);
    let res = change.apply(&membership, true);
    let want = Err(MembershipError::UserDefined(InvalidQuorumLists::NoWriteQuorum));
    assert_eq!(want, res);

    let change = ExplicitChange::quorums(vec![btreeset! {1}], vec![btreeset! {2}]);
    let res = change.apply(&membership, true);
    let disjoint = InvalidQuorumLists::DisjointQuorums {
        read: btreeset! {1},
        write: btreeset! {2},
    };
    assert_eq!(Err(MembershipError::UserDefined(disjoint)), res);

    // `add_learner()` never replaces the node of a known ID.
    type Change = ExplicitChange<u64, String>;
    type Target = ExplicitMembership<u64, String>;
    let change = <Change as RaftMembershipEdit<Target>>::add_learner(2, "z".to_string());
    let res = change.apply(&membership, true);
    assert_eq!(Ok(membership.clone()), res);

    // A membership with no config has no target to change.
    let empty = Target::default();
    let change = ExplicitChange::write_quorums(singles);
    let res = change.apply(&empty, true);
    let want = Err(MembershipError::EmptyMembership(EmptyMembership {}));
    assert_eq!(want, res);
}

#[test]
fn test_make_uniform_selects_target() {
    let old = Membership::new([btreeset! {1, 2}], [btreeset! {1, 2}], btreeset! {1, 2, 3}).unwrap();

    // Voter 1 of the old config stays as a learner of the target only when the target holds it:
    // OpenRaft removes it from the target unless the change retains removed voters.
    let with_learner = Membership::new([btreeset! {2, 3}], [btreeset! {3}], btreeset! {1, 2, 3}).unwrap();
    let without_learner = Membership::new([btreeset! {2, 3}], [btreeset! {3}], btreeset! {2, 3}).unwrap();
    for new in [with_learner, without_learner] {
        let joint = old.joint_with(&new);

        let change = <ExplicitChange<u64, ()> as RaftMembershipEdit<Membership>>::make_uniform();
        let target = change.apply(&joint, true).unwrap();
        assert_eq!(new, target);
        let direct = joint.supports_direct_transition_to(&target);
        assert!(direct);
    }
}

#[test]
fn test_joint_keeps_old_roles_and_target_restores_new_roles() {
    let nodes = btreeset! {1, 2, 3};
    let old = Membership::new([btreeset! {1, 2}], [btreeset! {1, 2}], nodes.clone()).unwrap();

    // The new config stops voter 2 from campaigning.
    let new = old.clone().with_roles(btreemap! {2 => NodeRole::ReceiveLogs}).unwrap();
    let target =
        assert_joint_keeps_old_roles_and_target_restores_new_roles::<_, ExplicitChange<u64, ()>>(&old, &new, 2);

    let want = "{configs: [Config { read: ExplicitQuorumSet { quorums: [{1, 2}] }, write: ExplicitQuorumSet { \
                quorums: [{1, 2}] }, nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), \
                2: ((), NodeRole(ReceiveLogs)), 3: ((), NodeRole(ReceiveLogs))} }]}";
    assert_eq!(want, target.to_string());
}

#[test]
fn test_with_roles() {
    let want = "{configs: [Config { read: ExplicitQuorumSet { quorums: [{1, 2}, {1, 3}, {2, 3}] }, \
                write: ExplicitQuorumSet { quorums: [{1, 2}, {1, 3}, {2, 3}] }, \
                nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), 2: ((), NodeRole(Campaigner | ReceiveLogs)), \
                3: ((), NodeRole(ReceiveLogs)), 4: ((), NodeRole(Campaigner | ReceiveLogs)), \
                5: ((), NodeRole(Empty))} }]}";
    assert_with_roles(majority, Membership::with_roles, want);
}

#[test]
fn test_only_write_voters_need_receive_logs() {
    // Node 1 only votes, node 2 votes and stores entries, and node 3 only stores entries.
    let read = [btreeset! {1, 2}];
    let membership = Membership::new(read.clone(), [btreeset! {2, 3}], btreeset! {1, 2, 3}).unwrap();

    // Read voter 1 never stores an entry, and write voter 3 stores entries but never campaigns.
    let roles = btreemap! {1 => NodeRole::Empty, 3 => NodeRole::ReceiveLogs};
    let configured = membership.with_roles(roles).unwrap();
    let cases = [
        (1, NodeRole::Empty, true, false),
        (2, voter_role(), true, true),
        (3, NodeRole::ReceiveLogs, false, true),
    ];
    for (node_id, role, is_read_voter, is_write_voter) in cases {
        let got = configured.get_node(&node_id);
        let want = Some(NodeRef {
            node: &(),
            role,
            is_read_voter,
            is_write_voter,
        });
        assert_eq!(want, got, "node {node_id}");
    }

    let res = configured.clone().with_roles(btreemap! {3 => NodeRole::Campaigner});
    let no_logs = WriteVoterWithoutReceiveLogs { node_id: 3 };
    assert_eq!(Err(MembershipError::WriteVoterWithoutReceiveLogs(no_logs)), res);

    // A change that makes node 1 a write voter must also give it `ReceiveLogs`.
    let write = vec![btreeset! {1, 2}, btreeset! {2, 3}];
    let change = ExplicitChange::write_quorums(write.clone());
    let res = change.apply(&configured, true);
    let no_logs = WriteVoterWithoutReceiveLogs { node_id: 1 };
    assert_eq!(Err(MembershipError::WriteVoterWithoutReceiveLogs(no_logs)), res);

    let roles = btreemap! {1 => NodeRole::ReceiveLogs};
    let change = ExplicitChange::write_quorums(write.clone()).with_roles(roles);
    let res = change.apply(&configured, true);
    let want = Membership::new(read, write, btreeset! {1, 2, 3}).unwrap();
    let want = want.with_roles(btreemap! {1 => NodeRole::ReceiveLogs, 3 => NodeRole::ReceiveLogs});
    assert_eq!(want, res);
}

#[test]
fn test_role_and_quorum_change_through_joint() {
    let want = "{configs: [Config { read: ExplicitQuorumSet { quorums: [{2, 3}, {2, 4}, {3, 4}] }, \
                write: ExplicitQuorumSet { quorums: [{2, 3}, {2, 4}, {3, 4}] }, \
                nodes: {1: ((), NodeRole(ReceiveLogs)), 2: ((), NodeRole(Campaigner | ReceiveLogs)), \
                3: ((), NodeRole(ReceiveLogs)), 4: ((), NodeRole(Campaigner | ReceiveLogs)), \
                5: ((), NodeRole(Empty))} }]}";
    assert_role_and_quorum_change_through_joint(majority, Membership::with_roles, want, |voters, roles| {
        let quorums = majorities(&voters);
        ExplicitChange::quorums(quorums.clone(), quorums).with_roles(roles)
    });
}

#[test]
fn test_change_through_joint_keeps_node_metadata() {
    assert_change_through_joint_keeps_node_metadata(majority, |voters, remove_nodes| {
        let quorums = majorities(&voters);
        ExplicitChange {
            remove_nodes,
            ..ExplicitChange::quorums(quorums.clone(), quorums)
        }
    });
}

#[test]
fn test_apply_checks_roles() {
    assert_apply_checks_roles(majority, Membership::with_roles, |add_nodes, remove_nodes, roles| {
        ExplicitChange {
            add_nodes,
            remove_nodes,
            roles,
            ..ExplicitChange::default()
        }
    });
}

#[cfg(feature = "serde")]
#[test]
fn test_serde() {
    let membership = Membership::new([btreeset! {1, 2}], [btreeset! {2, 3}], btreeset! {1, 2, 3, 4}).unwrap();

    let json = serde_json::to_string(&membership).unwrap();
    let want = r#"{"configs":[{"read":{"quorums":[[1,2]]},"write":{"quorums":[[2,3]]},"nodes":{"1":[null,3],"2":[null,3],"3":[null,3],"4":[null,2]}}]}"#;
    assert_eq!(want, json);
    let decoded = serde_json::from_str::<Membership>(&json).unwrap();
    assert_eq!(membership, decoded);

    let empty = r#"{"configs":[]}"#;
    let decoded = serde_json::from_str::<Membership>(empty).unwrap();
    assert_eq!(Membership::default(), decoded);
}

#[cfg(feature = "serde")]
#[test]
fn test_binary_roundtrip_keeps_joint_configs_and_roles() {
    let old = explicit(vec![btreeset! {1, 2}], vec![btreeset! {1, 2}]);
    let roles = btreemap! {3 => NodeRole::ReceiveLogs};
    let three = explicit(vec![btreeset! {1, 2}], vec![btreeset! {2, 3}]);
    let new = three.with_roles(roles).unwrap();
    let joint = old.joint_with(&new);

    let binary = bincode::serialize(&joint).unwrap();
    let decoded = bincode::deserialize::<Membership>(&binary).unwrap();
    assert_eq!(joint, decoded);

    let json = serde_json::to_string(&joint).unwrap();
    let decoded = serde_json::from_str::<Membership>(&json).unwrap();
    assert_eq!(joint, decoded);
}
