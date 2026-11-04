use std::collections::BTreeMap;
use std::collections::BTreeSet;

use maplit::btreemap;
use maplit::btreeset;
use openraft::errors::NodeNotFound;
use openraft::errors::Operation;
use openraft::membership::NodeRef;
use openraft::membership::NodeRole;
use openraft::membership::RaftMembership;

use crate::NodeChange;
use crate::config::Config;
use crate::quorum::Joint;
use crate::quorum::ThresholdQuorumSet;
use crate::quorum::testing::threshold;
use crate::testing::voter_role;

/// A config whose node metadata is a number, so that a test can tell which config a node's
/// metadata comes from.
type TestConfig = Config<u64, ThresholdQuorumSet<u64>>;

/// A membership of test configs.
type TestMembership = Joint<TestConfig>;

/// Returns a config whose read and write rules accept any `quorum_size` of `voters`, with `roles`
/// set.
fn config(
    quorum_size: u64,
    voters: BTreeSet<u64>,
    roles: BTreeMap<u64, NodeRole>,
    nodes: BTreeMap<u64, u64>,
) -> TestConfig {
    let rule = threshold(quorum_size, voters);
    let mut config = Config::new(rule.clone(), rule, nodes).unwrap();
    config.set_roles(roles).unwrap();
    config
}

fn node(node: &'static u64, role: NodeRole, is_voter: bool) -> NodeRef<'static, u64> {
    NodeRef {
        node,
        role,
        is_read_voter: is_voter,
        is_write_voter: is_voter,
    }
}

/// Returns every active node of `membership`, as `get_node()` returns it.
fn active_nodes(membership: &TestMembership) -> BTreeMap<u64, NodeRef<'_, u64>> {
    let mut nodes = BTreeMap::new();
    for node_id in membership.node_ids() {
        let node = membership.get_node(&node_id).unwrap();
        nodes.insert(node_id, node);
    }
    nodes
}

#[test]
fn test_new_gives_default_roles() {
    let nodes = btreemap! {1 => 10, 2 => 20, 3 => 30, 4 => 40};
    let membership = Joint {
        parts: vec![config(2, btreeset! {1, 2, 3}, BTreeMap::new(), nodes)],
    };

    let got = active_nodes(&membership);
    let want = btreemap! {
        1 => node(&10, voter_role(), true),
        2 => node(&20, voter_role(), true),
        3 => node(&30, voter_role(), true),
        4 => node(&40, NodeRole::ReceiveLogs, false),
    };
    assert_eq!(want, got);

    let unknown = membership.get_node(&5);
    assert_eq!(None, unknown);
}

#[test]
fn test_new_checks_that_voters_have_nodes() {
    let rule = threshold(2, btreeset! {1, 2, 3});

    let without_three = btreemap! {1 => 10, 2 => 20};
    let res = TestConfig::new(rule.clone(), rule.clone(), without_three);
    let not_found = NodeNotFound::new(3, Operation::None);
    assert_eq!(Err(not_found), res);

    let nodes = btreemap! {1 => 10, 2 => 20, 3 => 30, 4 => 40};
    let config = TestConfig::new(rule.clone(), rule, nodes).unwrap();
    let want = btreemap! {1 => (10, None), 2 => (20, None), 3 => (30, None), 4 => (40, None)};
    assert_eq!(want, config.nodes);
}

#[test]
fn test_set_roles_stores_only_roles_that_differ_from_default() {
    let nodes = btreemap! {1 => 10, 2 => 20, 3 => 30, 4 => 40};
    let read = threshold(2, btreeset! {1, 2});
    let write = threshold(2, btreeset! {2, 3});
    let mut config = TestConfig::new(read, write, nodes).unwrap();

    // Read-only voter 1 needs no logs, write-only voter 3 stops campaigning, and learner 4 stops
    // receiving logs. Voter 2 is set to its default, so no role is stored for it.
    let roles = btreemap! {
        1 => NodeRole::Empty,
        2 => voter_role(),
        3 => NodeRole::ReceiveLogs,
        4 => NodeRole::Empty,
    };
    config.set_roles(roles).unwrap();
    let want = btreemap! {
        1 => (10, Some(NodeRole::Empty)),
        2 => (20, None),
        3 => (30, Some(NodeRole::ReceiveLogs)),
        4 => (40, Some(NodeRole::Empty)),
    };
    assert_eq!(want, config.nodes);

    // A role is kept as given, even one without `ReceiveLogs` for write voter 2: OpenRaft rejects
    // such a membership when it is proposed. Learner 4 set to its default follows it again.
    config.set_roles(btreemap! {2 => NodeRole::Campaigner, 4 => NodeRole::ReceiveLogs}).unwrap();
    let want = btreemap! {
        1 => (10, Some(NodeRole::Empty)),
        2 => (20, Some(NodeRole::Campaigner)),
        3 => (30, Some(NodeRole::ReceiveLogs)),
        4 => (40, None),
    };
    assert_eq!(want, config.nodes);

    // Node 5 is no node of the config, so the call sets no role.
    let res = config.set_roles(btreemap! {3 => voter_role(), 5 => NodeRole::Empty});
    let not_found = NodeNotFound::new(5, Operation::None);
    assert_eq!(Err(not_found), res);
    assert_eq!(want, config.nodes);
}

#[test]
fn test_quorum_sets_hold_the_rules_of_every_config() {
    let nodes = btreemap! {1 => 10, 2 => 20, 3 => 30, 4 => 40};
    let old_rule = threshold(2, btreeset! {1, 2, 3});
    let old = TestConfig::new(old_rule.clone(), old_rule.clone(), nodes.clone()).unwrap();
    let new_read = threshold(1, btreeset! {2, 3, 4});
    let new_write = threshold(3, btreeset! {2, 3, 4});
    let new = TestConfig::new(new_read.clone(), new_write.clone(), nodes).unwrap();
    let membership = Joint { parts: vec![old, new] };

    let read = membership.read_quorum_set();
    assert_eq!(Joint::from_rules([old_rule.clone(), new_read]), read);
    let write = membership.write_quorum_set();
    assert_eq!(Joint::from_rules([old_rule, new_write]), write);
}

#[test]
fn test_supports_direct_transition_checks_cross_intersections() {
    let nodes = btreemap! {1 => 10, 2 => 20, 3 => 30, 4 => 40};
    let old = Joint {
        parts: vec![config(2, btreeset! {1, 2, 3}, BTreeMap::new(), nodes.clone())],
    };

    // A role change and a new learner keep both rules.
    let roles = btreemap! {3 => NodeRole::ReceiveLogs};
    let with_five = btreemap! {1 => 10, 2 => 20, 3 => 30, 4 => 40, 5 => 50};
    let same_rules = Joint {
        parts: vec![config(2, btreeset! {1, 2, 3}, roles, with_five)],
    };
    let direct = old.supports_direct_transition_to(&same_rules);
    assert!(direct);

    // Different rules can go directly: every quorum of size 2 meets every quorum of size 3.
    let all_three = Joint {
        parts: vec![config(3, btreeset! {1, 2, 3}, BTreeMap::new(), nodes)],
    };
    let direct = old.supports_direct_transition_to(&all_three);
    assert!(direct);

    // The joint holds the old config and the new one, so both steps through it go directly.
    let joint = old.joint_with(&all_three);
    let into_joint = old.supports_direct_transition_to(&joint);
    assert!(into_joint);
    let out_of_joint = joint.supports_direct_transition_to(&all_three);
    assert!(out_of_joint);
}

#[test]
fn test_supports_direct_transition_uses_joint_proofs() {
    let voters = btreeset! {1, 2, 3};
    let nodes = btreemap! {1 => 10, 2 => 20, 3 => 30};
    let mut configs = Vec::new();
    for (read_size, write_size) in [(1, 3), (3, 1)] {
        let read = threshold(read_size, voters.clone());
        let write = threshold(write_size, voters.clone());
        let config = TestConfig::new(read, write, nodes.clone()).unwrap();
        configs.push(config);
    }
    let joint = Joint { parts: configs };
    let roles = BTreeMap::new();
    let majority = Joint {
        parts: vec![config(2, voters, roles, nodes)],
    };
    let to_majority = joint.supports_direct_transition_to(&majority);
    assert!(to_majority);
    let from_majority = majority.supports_direct_transition_to(&joint);
    assert!(from_majority);
}

#[test]
fn test_get_node_joins_roles_of_configs_where_node_is_voter() {
    // Each config gives a node its own metadata, so the metadata tells which config it comes from.
    let c0_nodes = btreemap! {1 => 10, 2 => 20, 3 => 30, 4 => 40, 6 => 60};
    let c0 = config(2, btreeset! {1, 2, 3}, BTreeMap::new(), c0_nodes);
    let c1_nodes = btreemap! {1 => 11, 2 => 21, 3 => 31, 4 => 41, 5 => 51};
    let c1 = config(2, btreeset! {2, 3, 5}, btreemap! {3 => NodeRole::ReceiveLogs}, c1_nodes);
    let c2_nodes = btreemap! {2 => 22, 3 => 32, 4 => 42, 5 => 52};
    let c2_roles = btreemap! {
        3 => NodeRole::ReceiveLogs,
        4 => NodeRole::Empty,
        5 => NodeRole::Empty,
    };
    let c2 = config(2, btreeset! {2, 3}, c2_roles, c2_nodes);
    let membership = Joint {
        parts: vec![c0, c1, c2],
    };

    let got = active_nodes(&membership);
    let want = btreemap! {
        // Voter 1 of c0 is a learner of c1, the newest config that holds it.
        1 => node(&11, voter_role(), true),
        2 => node(&22, voter_role(), true),
        // Voter 3 stops campaigning in c1 and c2, and keeps campaigning as a voter of c0.
        3 => node(&32, voter_role(), true),
        // Learner 4 of c0 and c1 has only its role in c2.
        4 => node(&42, NodeRole::Empty, false),
        // Voter 5 of c1 keeps its role there.
        5 => node(&52, voter_role(), true),
    };
    assert_eq!(want, got);

    // Learner 6 of c0 is not active: the change to c1 removed it.
    let removed_learner = membership.get_node(&6);
    assert_eq!(None, removed_learner);
}

#[test]
fn test_joint_with_appends_the_configs_of_goal() {
    let nodes = btreemap! {1 => 10, 2 => 20, 3 => 30, 4 => 40};
    let old_config = config(2, btreeset! {1, 2, 3}, BTreeMap::new(), nodes.clone());
    let new_config = config(2, btreeset! {2, 3, 4}, BTreeMap::new(), nodes);
    let old = Joint {
        parts: vec![old_config.clone()],
    };
    let new = Joint {
        parts: vec![new_config.clone()],
    };

    let joint = RaftMembership::joint_with(&old, &new);
    let want = Joint {
        parts: vec![old_config, new_config],
    };
    assert_eq!(want, joint);

    // A side without a config has no quorum, so the joint has none either.
    let empty = TestMembership::default();
    let without_old = RaftMembership::joint_with(&empty, &new);
    assert_eq!(empty, without_old);
    let without_new = RaftMembership::joint_with(&old, &empty);
    assert_eq!(empty, without_new);
}

#[test]
fn test_drop_voters_demoted_to_learners_removes_them_from_goal() {
    let nodes = btreemap! {1 => 10, 2 => 20, 3 => 30, 4 => 40};
    let old = Joint {
        parts: vec![config(2, btreeset! {1, 2, 3}, BTreeMap::new(), nodes.clone())],
    };

    // Voters 1 and 2 leave the voters, but 2 keeps campaigning, so it is no learner of the goal.
    // Learner 4 was never a voter.
    let campaigner = btreemap! {2 => voter_role()};
    let goal = Joint {
        parts: vec![config(1, btreeset! {3}, campaigner.clone(), nodes)],
    };

    let got = old.drop_voters_demoted_to_learners(goal);
    let without_one = btreemap! {2 => 20, 3 => 30, 4 => 40};
    let want = Joint {
        parts: vec![config(1, btreeset! {3}, campaigner, without_one)],
    };
    assert_eq!(want, got);
}

#[test]
fn test_drop_demoted_voters_across_joint_configs() {
    let old_nodes = btreemap! {1 => 10, 2 => 20, 3 => 30, 4 => 40, 5 => 50};
    let first = config(1, btreeset! {1, 2}, BTreeMap::new(), old_nodes.clone());
    let last = config(1, btreeset! {2, 3, 5}, BTreeMap::new(), old_nodes);
    let old = Joint {
        parts: vec![first, last],
    };

    let goal_nodes = btreemap! {1 => 11, 2 => 21, 3 => 31, 4 => 41};
    let first = config(1, btreeset! {3}, BTreeMap::new(), goal_nodes.clone());
    let last = config(1, btreeset! {2}, BTreeMap::new(), goal_nodes);
    let goal = Joint {
        parts: vec![first, last],
    };
    let got = old.drop_voters_demoted_to_learners(goal);

    let retained = btreemap! {2 => 21, 3 => 31, 4 => 41};
    let first = config(1, btreeset! {3}, BTreeMap::new(), retained.clone());
    let last = config(1, btreeset! {2}, BTreeMap::new(), retained);
    let want = Joint {
        parts: vec![first, last],
    };
    assert_eq!(want, got);
}

#[test]
fn test_with_roles_sets_roles_of_last_config() {
    // Voter 3 stops campaigning in the old config, and voter 4 replaces voter 1 in the new one.
    let nodes = btreemap! {1 => 10, 2 => 20, 3 => 30, 4 => 40};
    let old_roles = btreemap! {3 => NodeRole::ReceiveLogs};
    let old = config(2, btreeset! {1, 2, 3}, old_roles, nodes.clone());
    let new = config(2, btreeset! {2, 3, 4}, BTreeMap::new(), nodes.clone());
    let joint = Joint {
        parts: vec![old.clone(), new],
    };

    let roles = btreemap! {1 => NodeRole::Empty, 4 => NodeRole::ReceiveLogs};
    let got = joint.clone().with_roles(roles.clone()).unwrap();
    let new_with_roles = config(2, btreeset! {2, 3, 4}, roles, nodes);
    let want = Joint {
        parts: vec![old, new_with_roles],
    };
    assert_eq!(want, got);

    let res = joint.clone().with_roles(btreemap! {5 => NodeRole::Empty});
    let not_found = NodeNotFound::new(5, Operation::None);
    assert_eq!(Err(not_found), res);

    // A role is kept as given, even one without `ReceiveLogs` for write voter 4.
    let got = joint.with_roles(btreemap! {4 => NodeRole::Campaigner}).unwrap();
    let node = got.get_node(&4).unwrap();
    assert_eq!(NodeRole::Campaigner, node.role);
}

#[test]
fn test_node_change_keeps_metadata_of_active_nodes() {
    // The change to the new config removed voter 1 and learner 4.
    let old_nodes = btreemap! {1 => 10, 2 => 20, 3 => 30, 4 => 40};
    let old = config(2, btreeset! {1, 2, 3}, BTreeMap::new(), old_nodes);
    let new = config(2, btreeset! {2, 3}, BTreeMap::new(), btreemap! {2 => 20, 3 => 30});
    let joint = Joint { parts: vec![old, new] };

    // Voter 1 of the old config is active and keeps its metadata. Learner 4 is not active, so it
    // takes the added metadata, as the unknown node 5 does. Node 6 is added and removed by the
    // same change, so it is removed.
    let change = NodeChange {
        add_nodes: btreemap! {1 => 11, 4 => 41, 5 => 51, 6 => 61},
        remove_nodes: btreeset! {3, 6},
        ..NodeChange::default()
    };
    let mut nodes = btreemap! {2 => 20, 3 => 30};
    change.edit_nodes(&mut nodes, &joint);
    let want = btreemap! {1 => 10, 2 => 20, 4 => 41, 5 => 51};
    assert_eq!(want, nodes);
}

#[test]
fn test_keep_roles_keeps_set_roles_through_status_changes() {
    // Voter 3 stops campaigning, and learner 4 stops receiving logs.
    let nodes = btreemap! {1 => 10, 2 => 20, 3 => 30, 4 => 40};
    let roles = btreemap! {3 => NodeRole::ReceiveLogs, 4 => NodeRole::Empty};
    let first = config(2, btreeset! {1, 2, 3}, roles, nodes);

    // The second config removes node 4 and makes voter 3 a learner. Node 3 keeps its set role,
    // though it equals the default for a learner.
    let without_four = btreemap! {1 => 10, 2 => 20, 3 => 30};
    let rule = threshold(1, btreeset! {1, 2});
    let mut second = TestConfig::new(rule.clone(), rule, without_four.clone()).unwrap();
    second.keep_roles(&first);
    let want = btreemap! {1 => (10, None), 2 => (20, None), 3 => (30, Some(NodeRole::ReceiveLogs))};
    assert_eq!(want, second.nodes);

    // The third config makes node 3 a voter again, and it still does not campaign.
    let rule = threshold(2, btreeset! {1, 2, 3});
    let mut third = TestConfig::new(rule.clone(), rule, without_four).unwrap();
    third.keep_roles(&second);
    let membership = Joint { parts: vec![third] };
    let got = active_nodes(&membership);
    let want = btreemap! {
        1 => node(&10, voter_role(), true),
        2 => node(&20, voter_role(), true),
        3 => node(&30, NodeRole::ReceiveLogs, true),
    };
    assert_eq!(want, got);
}
