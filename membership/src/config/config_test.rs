use std::collections::BTreeMap;
use std::collections::BTreeSet;

use maplit::btreemap;
use maplit::btreeset;
use openraft::errors::NodeNotFound;
use openraft::errors::Operation;
use openraft::membership::NodeRef;
use openraft::membership::NodeRole;
use openraft::membership::errors::WriteVoterWithoutReceiveLogs;

use crate::config::Config;
use crate::errors::ConfigsError;
use crate::quorum::Joint;
use crate::quorum::ThresholdQuorumSet;
use crate::quorum::testing::threshold;
use crate::testing::voter_role;

/// A config whose node metadata is a number, so that a test can tell which config a node's
/// metadata comes from.
type TestConfig = Config<u64, u64, ThresholdQuorumSet<u64>>;

/// Returns a config whose read and write rules accept any `quorum_size` of `voters`.
fn config(
    quorum_size: u64,
    voters: BTreeSet<u64>,
    roles: BTreeMap<u64, NodeRole>,
    nodes: BTreeMap<u64, u64>,
) -> TestConfig {
    let rule = threshold(quorum_size, voters);
    Config::new(rule.clone(), rule, roles, nodes).unwrap()
}

fn node(node: &'static u64, role: NodeRole, is_voter: bool) -> NodeRef<'static, u64> {
    NodeRef {
        node,
        role,
        is_read_voter: is_voter,
        is_write_voter: is_voter,
    }
}

/// Returns every active node of `configs`, as `get_node()` returns it.
fn active_nodes(configs: &[TestConfig]) -> BTreeMap<u64, NodeRef<'_, u64>> {
    let mut nodes = BTreeMap::new();
    for node_id in Config::node_ids(configs) {
        let node = Config::get_node(configs, &node_id).unwrap();
        nodes.insert(node_id, node);
    }
    nodes
}

#[test]
fn test_new_gives_default_roles() {
    let nodes = btreemap! {1 => 10, 2 => 20, 3 => 30, 4 => 40};
    let configs = [config(2, btreeset! {1, 2, 3}, BTreeMap::new(), nodes)];

    let got = active_nodes(&configs);
    let want = btreemap! {
        1 => node(&10, voter_role(), true),
        2 => node(&20, voter_role(), true),
        3 => node(&30, voter_role(), true),
        4 => node(&40, NodeRole::ReceiveLogs, false),
    };
    assert_eq!(want, got);

    let unknown = Config::get_node(&configs, &5);
    assert_eq!(None, unknown);
}

#[test]
fn test_new_sets_roles_of_its_nodes() {
    // Voter 3 stops campaigning, learner 4 stops receiving logs, and node 5 is no node of the
    // config, so its role is dropped.
    let roles = btreemap! {
        3 => NodeRole::ReceiveLogs,
        4 => NodeRole::Empty,
        5 => NodeRole::Empty,
    };
    let nodes = btreemap! {1 => 10, 2 => 20, 3 => 30, 4 => 40};
    let config = config(2, btreeset! {1, 2, 3}, roles, nodes);

    let want = btreemap! {
        1 => (10, voter_role()),
        2 => (20, voter_role()),
        3 => (30, NodeRole::ReceiveLogs),
        4 => (40, NodeRole::Empty),
    };
    assert_eq!(want, config.nodes);
}

#[test]
fn test_new_checks_voters() {
    let rule = threshold(2, btreeset! {1, 2, 3});

    let without_three = btreemap! {1 => 10, 2 => 20};
    let res = TestConfig::new(rule.clone(), rule.clone(), BTreeMap::new(), without_three);
    let not_found = NodeNotFound::new(3, Operation::None);
    assert_eq!(Err(ConfigsError::NodeNotFound(not_found)), res);

    let nodes = btreemap! {1 => 10, 2 => 20, 3 => 30, 4 => 40};
    let campaigns_only = btreemap! {2 => NodeRole::Campaigner};
    let res = TestConfig::new(rule.clone(), rule.clone(), campaigns_only, nodes.clone());
    let no_logs = WriteVoterWithoutReceiveLogs { node_id: 2 };
    assert_eq!(Err(ConfigsError::WriteVoterWithoutReceiveLogs(no_logs)), res);

    // Node 4 is no voter, so it needs no log.
    let learner_without_logs = btreemap! {4 => NodeRole::Empty};
    let res = TestConfig::new(rule.clone(), rule, learner_without_logs, nodes);
    assert!(res.is_ok());
}

#[test]
fn test_quorum_sets_hold_the_rules_of_every_config() {
    let nodes = btreemap! {1 => 10, 2 => 20, 3 => 30, 4 => 40};
    let old_rule = threshold(2, btreeset! {1, 2, 3});
    let old = TestConfig::new(old_rule.clone(), old_rule.clone(), BTreeMap::new(), nodes.clone()).unwrap();
    let new_read = threshold(1, btreeset! {2, 3, 4});
    let new_write = threshold(3, btreeset! {2, 3, 4});
    let new = TestConfig::new(new_read.clone(), new_write.clone(), BTreeMap::new(), nodes).unwrap();
    let configs = [old, new];

    let read = Config::read_quorum_set(&configs);
    assert_eq!(Joint::new([old_rule.clone(), new_read]), read);
    let write = Config::write_quorum_set(&configs);
    assert_eq!(Joint::new([old_rule, new_write]), write);
}

#[test]
fn test_supports_direct_transition_needs_a_config_with_the_same_rules() {
    let nodes = btreemap! {1 => 10, 2 => 20, 3 => 30, 4 => 40};
    let old = [config(2, btreeset! {1, 2, 3}, BTreeMap::new(), nodes.clone())];

    // A role change and a new learner keep both rules.
    let roles = btreemap! {3 => NodeRole::ReceiveLogs};
    let with_five = btreemap! {1 => 10, 2 => 20, 3 => 30, 4 => 40, 5 => 50};
    let same_rules = [config(2, btreeset! {1, 2, 3}, roles, with_five)];
    let direct = Config::supports_direct_transition(&old, &same_rules);
    assert!(direct);

    // Quorum size 3 would be safe directly, because 2 + 3 exceeds the 3 voters. It changes both
    // rules, so it still goes through a joint.
    let all_three = [config(3, btreeset! {1, 2, 3}, BTreeMap::new(), nodes)];
    let direct = Config::supports_direct_transition(&old, &all_three);
    assert!(!direct);

    // The joint holds the old config and the new one, so both steps through it go directly.
    let joint = Config::joint(&old, &all_three);
    let into_joint = Config::supports_direct_transition(&old, &joint);
    assert!(into_joint);
    let out_of_joint = Config::supports_direct_transition(&joint, &all_three);
    assert!(out_of_joint);
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
    let configs = [c0, c1, c2];

    let got = active_nodes(&configs);
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
    let removed_learner = Config::get_node(&configs, &6);
    assert_eq!(None, removed_learner);
}

#[test]
fn test_joint_appends_the_configs_of_goal() {
    let nodes = btreemap! {1 => 10, 2 => 20, 3 => 30, 4 => 40};
    let old = [config(2, btreeset! {1, 2, 3}, BTreeMap::new(), nodes.clone())];
    let new = [config(2, btreeset! {2, 3, 4}, BTreeMap::new(), nodes)];

    let joint = Config::joint(&old, &new);
    let want = vec![old[0].clone(), new[0].clone()];
    assert_eq!(want, joint);

    // A side without a config has no quorum, so the joint has none either.
    let without_old = Config::joint(&[], &new);
    assert_eq!(Vec::<TestConfig>::new(), without_old);
    let without_new = Config::joint(&old, &[]);
    assert_eq!(Vec::<TestConfig>::new(), without_new);
}

#[test]
fn test_drop_voters_demoted_to_learners_removes_them_from_goal() {
    let nodes = btreemap! {1 => 10, 2 => 20, 3 => 30, 4 => 40};
    let old = [config(2, btreeset! {1, 2, 3}, BTreeMap::new(), nodes.clone())];

    // Voters 1 and 2 leave the voters, but 2 keeps campaigning, so it is no learner of the goal.
    // Learner 4 was never a voter.
    let campaigner = btreemap! {2 => voter_role()};
    let goal = vec![config(1, btreeset! {3}, campaigner.clone(), nodes)];

    let got = Config::drop_voters_demoted_to_learners(&old, goal);
    let without_one = btreemap! {2 => 20, 3 => 30, 4 => 40};
    let want = vec![config(1, btreeset! {3}, campaigner, without_one)];
    assert_eq!(want, got);
}

#[test]
fn test_with_roles_sets_roles_of_last_config() {
    // Voter 3 stops campaigning in the old config, and voter 4 replaces voter 1 in the new one.
    let nodes = btreemap! {1 => 10, 2 => 20, 3 => 30, 4 => 40};
    let old_roles = btreemap! {3 => NodeRole::ReceiveLogs};
    let old = config(2, btreeset! {1, 2, 3}, old_roles, nodes.clone());
    let new = config(2, btreeset! {2, 3, 4}, BTreeMap::new(), nodes.clone());
    let joint = vec![old.clone(), new];

    let roles = btreemap! {1 => NodeRole::Empty, 4 => NodeRole::ReceiveLogs};
    let got = Config::with_roles(joint.clone(), roles.clone()).unwrap();
    let new_with_roles = config(2, btreeset! {2, 3, 4}, roles, nodes);
    assert_eq!(vec![old, new_with_roles], got);

    let res = Config::with_roles(joint.clone(), btreemap! {5 => NodeRole::Empty});
    let not_found = NodeNotFound::new(5, Operation::None);
    assert_eq!(Err(ConfigsError::NodeNotFound(not_found)), res);

    let res = Config::with_roles(joint, btreemap! {4 => NodeRole::Campaigner});
    let no_logs = WriteVoterWithoutReceiveLogs { node_id: 4 };
    assert_eq!(Err(ConfigsError::WriteVoterWithoutReceiveLogs(no_logs)), res);
}

#[test]
fn test_edit_nodes_keeps_metadata_of_active_nodes() {
    // The change to the new config removed voter 1 and learner 4.
    let old_nodes = btreemap! {1 => 10, 2 => 20, 3 => 30, 4 => 40};
    let old = config(2, btreeset! {1, 2, 3}, BTreeMap::new(), old_nodes);
    let new = config(2, btreeset! {2, 3}, BTreeMap::new(), btreemap! {2 => 20, 3 => 30});
    let joint = [old, new];

    // Voter 1 of the old config is active and keeps its metadata. Learner 4 is not active, so it
    // takes the added metadata, as the unknown node 5 does.
    let add_nodes = btreemap! {1 => 11, 4 => 41, 5 => 51};
    let got = Config::edit_nodes(&joint, add_nodes, &btreeset! {3});
    let want = btreemap! {1 => 10, 2 => 20, 4 => 41, 5 => 51};
    assert_eq!(want, got);
}

#[test]
fn test_edit_roles_keeps_roles_that_differ_from_default() {
    let roles = btreemap! {
        1 => voter_role(),
        2 => NodeRole::ReceiveLogs,
        4 => NodeRole::ReceiveLogs,
    };
    let nodes = btreemap! {1 => 10, 2 => 20, 3 => 30, 4 => 40};
    let config = config(2, btreeset! {1, 2, 3}, roles, nodes.clone());

    // Only voter 2 has a role that differs from the default for its status.
    let got = config.edit_roles(btreemap! {4 => NodeRole::Empty}, &nodes).unwrap();
    let want = btreemap! {2 => NodeRole::ReceiveLogs, 4 => NodeRole::Empty};
    assert_eq!(want, got);

    let res = config.edit_roles(btreemap! {5 => NodeRole::Empty}, &nodes);
    let not_found = NodeNotFound::new(5, Operation::None);
    assert_eq!(Err(not_found), res);
}
