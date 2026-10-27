//! Membership helpers and checks shared by unit tests.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::error::Error;

use maplit::btreemap;
use maplit::btreeset;
use openraft::errors::MembershipError;
use openraft::errors::NodeNotFound;
use openraft::errors::Operation;
use openraft::membership::NodeRef;
use openraft::membership::NodeRole;
use openraft::membership::RaftMembership;
use openraft::membership::RaftMembershipEdit;
use openraft::membership::errors::WriteVoterWithoutReceiveLogs;
use quorum_set::verify_intersection;

use crate::quorum::testing::assert_quorum_set_contract;

/// Returns the role of a voter whose config sets no role for it.
pub(crate) fn voter_role() -> NodeRole {
    NodeRole::Campaigner | NodeRole::ReceiveLogs
}

/// Returns whether every read quorum of each of `old` and `new` meets every write quorum of the
/// other, checked on every subset of their IDs.
pub(crate) fn direct_transition_is_safe<M>(old: &M, new: &M) -> bool
where M: RaftMembership {
    let read_meets_write = verify_intersection(&old.read_quorum_set(), &new.write_quorum_set());
    let write_meets_read = verify_intersection(&old.write_quorum_set(), &new.read_quorum_set());
    read_meets_write && write_meets_read
}

/// Asserts that both quorum sets of `membership` keep the `QuorumSet` contract, and that every
/// read quorum meets every write quorum, as `membership` claims.
pub(crate) fn assert_quorum_sets_are_valid<M>(membership: &M)
where M: RaftMembership<NodeId = u64> {
    assert_quorum_set_contract(&membership.read_quorum_set());
    assert_quorum_set_contract(&membership.write_quorum_set());
    assert_read_write_quorums_intersect(membership);
}

/// Asserts every claim that OpenRaft trusts on the way from `old` to `new`, checked on every
/// subset of their IDs: a direct transition from `old` to `new`, the read-write intersection of
/// their joint, and the two steps through the joint, which OpenRaft must be able to take.
pub(crate) fn assert_claims_hold<M>(old: &M, new: &M)
where M: RaftMembership {
    let direct = old.supports_direct_transition_to(new);
    if direct {
        let safe = direct_transition_is_safe(old, new);
        assert!(safe, "{old} claims a direct transition to {new}");
    }

    let joint = old.joint_with(new);
    assert_read_write_quorums_intersect(&joint);

    let into_joint = old.supports_direct_transition_to(&joint);
    assert!(into_joint, "{old} to {joint}");
    let safe = direct_transition_is_safe(old, &joint);
    assert!(safe, "{old} to {joint}");

    let out_of_joint = joint.supports_direct_transition_to(new);
    assert!(out_of_joint, "{joint} to {new}");
    let safe = direct_transition_is_safe(&joint, new);
    assert!(safe, "{joint} to {new}");
}

/// Asserts that the joint of `old` and `new` keeps the role [`voter_role()`] that `old` gives
/// voter `node_id`, and that the `make_uniform()` change of `C` reduces the joint to `new`, which
/// gives the voter only [`NodeRole::ReceiveLogs`]. Returns the result of `make_uniform()`.
pub(crate) fn assert_joint_keeps_old_roles_and_target_restores_new_roles<M, C>(old: &M, new: &M, node_id: u64) -> M
where
    M: RaftMembership<NodeId = u64>,
    C: RaftMembershipEdit<M>,
{
    let joint = old.joint_with(new);
    let in_joint = joint.get_node(&node_id).unwrap();
    assert_eq!(voter_role(), in_joint.role);

    let change = C::make_uniform();
    let target = change.apply(&joint, true).unwrap();
    assert_eq!(new, &target);
    let in_target = target.get_node(&node_id).unwrap();
    assert_eq!(NodeRole::ReceiveLogs, in_target.role);
    target
}

/// Asserts that `with_roles` sets the role of each node that it names, keeps every other role,
/// and rejects an unknown node and a write voter without [`NodeRole::ReceiveLogs`].
///
/// `build(voters, nodes)` returns a membership whose only config has the voters `voters` and a
/// `()` node for each of `nodes`. `with_roles` is the `with_roles()` method of that membership
/// type. The test builds voters 1, 2 and 3 and learners 4 and 5.
pub(crate) fn assert_with_roles<M, E, B, W>(build: B, with_roles: W)
where
    M: RaftMembership<NodeId = u64, Node = ()>,
    E: Error + PartialEq + 'static,
    B: Fn(BTreeSet<u64>, BTreeSet<u64>) -> M,
    W: Fn(M, BTreeMap<u64, NodeRole>) -> Result<M, MembershipError<u64, E>>,
{
    let membership = build(btreeset! {1, 2, 3}, btreeset! {1, 2, 3, 4, 5});

    // Voter 3 stops campaigning, learner 4 gets `Campaigner` too, and learner 5 stops receiving
    // logs.
    let roles = btreemap! {
        3 => NodeRole::ReceiveLogs,
        4 => voter_role(),
        5 => NodeRole::Empty,
    };
    let configured = with_roles(membership.clone(), roles).unwrap();
    let cases = [
        (1, voter_role(), true),
        (2, voter_role(), true),
        (3, NodeRole::ReceiveLogs, true),
        (4, voter_role(), false),
        (5, NodeRole::Empty, false),
    ];
    for (node_id, role, is_voter) in cases {
        let got = configured.get_node(&node_id);
        let want = Some(NodeRef {
            node: &(),
            role,
            is_read_voter: is_voter,
            is_write_voter: is_voter,
        });
        assert_eq!(want, got, "node {node_id}");
    }
    // A later call keeps the roles of the nodes that it does not name, and can set a role back to
    // the default.
    let res = with_roles(configured, btreemap! {3 => voter_role(), 5 => NodeRole::ReceiveLogs});
    let want = with_roles(membership.clone(), btreemap! {4 => voter_role()});
    assert_eq!(want, res);

    // Node 9 is unknown, and write voter 2 would store no entry.
    let res = with_roles(membership.clone(), btreemap! {9 => NodeRole::ReceiveLogs});
    let not_found = NodeNotFound::new(9, Operation::None);
    assert_eq!(Err(MembershipError::NodeNotFound(not_found)), res);

    let res = with_roles(membership, btreemap! {2 => NodeRole::Campaigner});
    let no_logs = WriteVoterWithoutReceiveLogs { node_id: 2 };
    assert_eq!(Err(MembershipError::WriteVoterWithoutReceiveLogs(no_logs)), res);
}

/// Asserts that a change of both the voters and the roles goes through a joint. The joint gives
/// each node its new role, joined with its old role where it is an old voter, and
/// `make_uniform()` restores the new roles.
///
/// `build` and `with_roles` are as in [`assert_with_roles()`]. `change_voters(voters, roles)`
/// returns a change to the rules that `build` gives `voters`, which also sets `roles`.
pub(crate) fn assert_role_and_quorum_change_through_joint<M, C, B, W, F>(build: B, with_roles: W, change_voters: F)
where
    M: RaftMembership<NodeId = u64, Node = ()>,
    C: RaftMembershipEdit<M>,
    B: Fn(BTreeSet<u64>, BTreeSet<u64>) -> M,
    W: Fn(M, BTreeMap<u64, NodeRole>) -> Result<M, MembershipError<u64, C::Error>>,
    F: Fn(BTreeSet<u64>, BTreeMap<u64, NodeRole>) -> C,
{
    let membership = build(btreeset! {1, 2, 3}, btreeset! {1, 2, 3, 4, 5});
    let old = with_roles(membership, btreemap! {1 => NodeRole::ReceiveLogs}).unwrap();

    // Learner 4 replaces voter 1, voter 3 stops campaigning, and learner 5 stops receiving logs.
    let roles = btreemap! {3 => NodeRole::ReceiveLogs, 5 => NodeRole::Empty};
    let change = change_voters(btreeset! {2, 3, 4}, roles);
    let new = change.apply(&old, true).unwrap();
    let direct = old.supports_direct_transition_to(&new);
    assert!(!direct);

    // The joint gives each node its new role, joined with its old role where it is an old voter,
    // and keeps learner 5.
    let joint = old.joint_with(&new);
    let cases = [
        (1, NodeRole::ReceiveLogs, true),
        (2, voter_role(), true),
        (3, voter_role(), true),
        (4, voter_role(), true),
        (5, NodeRole::Empty, false),
    ];
    for (node_id, role, is_voter) in cases {
        let got = joint.get_node(&node_id);
        let want = Some(NodeRef {
            node: &(),
            role,
            is_read_voter: is_voter,
            is_write_voter: is_voter,
        });
        assert_eq!(want, got, "node {node_id}");
    }
    let ids: Vec<u64> = joint.node_ids().collect();
    assert_eq!(vec![1, 2, 3, 4, 5], ids);

    // The target restores the new roles.
    let change = C::make_uniform();
    let target = change.apply(&joint, true).unwrap();
    assert_eq!(new, target);
    let cases = [
        (1, NodeRole::ReceiveLogs, false),
        (2, voter_role(), true),
        (3, NodeRole::ReceiveLogs, true),
        (4, voter_role(), true),
        (5, NodeRole::Empty, false),
    ];
    for (node_id, role, is_voter) in cases {
        let got = target.get_node(&node_id);
        let want = Some(NodeRef {
            node: &(),
            role,
            is_read_voter: is_voter,
            is_write_voter: is_voter,
        });
        assert_eq!(want, got, "node {node_id}");
    }
    let ids: Vec<u64> = target.node_ids().collect();
    assert_eq!(vec![1, 2, 3, 4, 5], ids);

    // A removed voter with a non-learner role stays even when learners are not retained.
    for role in [NodeRole::Campaigner, NodeRole::Empty] {
        let change = change_voters(btreeset! {2, 3, 4}, btreemap! {1 => role});
        let target = change.apply(&old, false).unwrap();
        let got = target.get_node(&1);
        let want = Some(NodeRef {
            node: &(),
            role,
            is_read_voter: false,
            is_write_voter: false,
        });
        assert_eq!(want, got);
    }
}

/// Asserts that `apply()` sets a role only for a node of its result, and keeps every write voter
/// receiving logs.
///
/// `build` and `with_roles` are as in [`assert_with_roles()`]. `change_nodes(add_nodes,
/// remove_nodes, roles)` returns a change that keeps the rules, adds `add_nodes`, removes
/// `remove_nodes` and sets `roles`.
pub(crate) fn assert_apply_checks_roles<M, C, B, W, F>(build: B, with_roles: W, change_nodes: F)
where
    M: RaftMembership<NodeId = u64, Node = ()>,
    C: RaftMembershipEdit<M>,
    C::Error: PartialEq,
    B: Fn(BTreeSet<u64>, BTreeSet<u64>) -> M,
    W: Fn(M, BTreeMap<u64, NodeRole>) -> Result<M, MembershipError<u64, C::Error>>,
    F: Fn(BTreeMap<u64, ()>, BTreeSet<u64>, BTreeMap<u64, NodeRole>) -> C,
{
    let membership = build(btreeset! {1, 2, 3}, btreeset! {1, 2, 3, 4});

    // A role must name a node of the result: node 9 is unknown, and the change removes node 4.
    let change = change_nodes(btreemap! {}, btreeset! {}, btreemap! {9 => NodeRole::ReceiveLogs});
    let res = change.apply(&membership, true);
    let not_found = NodeNotFound::new(9, Operation::None);
    let want = Err(MembershipError::from(not_found));
    assert_eq!(want, res);

    let change = change_nodes(btreemap! {}, btreeset! {4}, btreemap! {4 => NodeRole::Empty});
    let res = change.apply(&membership, true);
    let not_found = NodeNotFound::new(4, Operation::None);
    let want = Err(MembershipError::from(not_found));
    assert_eq!(want, res);

    // A node that the change adds may take a role.
    let change = change_nodes(btreemap! {5 => ()}, btreeset! {}, btreemap! {5 => NodeRole::Empty});
    let res = change.apply(&membership, true);
    let want = build(btreeset! {1, 2, 3}, btreeset! {1, 2, 3, 4, 5});
    let want = with_roles(want, btreemap! {5 => NodeRole::Empty});
    assert_eq!(want, res);

    // Write voter 2 must keep receiving logs.
    let change = change_nodes(btreemap! {}, btreeset! {}, btreemap! {2 => NodeRole::Campaigner});
    let res = change.apply(&membership, true);
    let no_logs = WriteVoterWithoutReceiveLogs { node_id: 2 };
    assert_eq!(Err(MembershipError::WriteVoterWithoutReceiveLogs(no_logs)), res);
}

/// Asserts that `get_node()` returns the metadata of every node, before, during and after a change
/// through a joint, and after a serde round trip of the joint. The target keeps the voter that the
/// change removes only when the change retains it.
///
/// `build(voters, nodes)` returns a membership whose only config has the voters `voters` and the
/// nodes `nodes`. `change_voters(voters, remove_nodes)` returns a change to the rules that `build`
/// gives `voters`, which also removes `remove_nodes`.
pub(crate) fn assert_change_through_joint_keeps_node_metadata<M, C, B, F>(build: B, change_voters: F)
where
    M: RaftMembership<NodeId = u64, Node = u64>,
    C: RaftMembershipEdit<M>,
    B: Fn(BTreeSet<u64>, BTreeMap<u64, u64>) -> M,
    F: Fn(BTreeSet<u64>, BTreeSet<u64>) -> C,
{
    // Each node has its own metadata, so a node that a step loses or replaces shows.
    let nodes = btreemap! {1 => 10, 2 => 20, 3 => 30, 4 => 40, 5 => 50};
    let old = build(btreeset! {1, 2, 3}, nodes.clone());
    let got = node_metadata(&old);
    assert_eq!(nodes, got);

    // Learner 4 replaces voter 1, and the change removes learner 5. `apply()` keeps voter 1 only
    // when retaining removed voters.
    let change = change_voters(btreeset! {2, 3, 4}, btreeset! {5});
    let retained = change.apply(&old, true).unwrap();
    let change = change_voters(btreeset! {2, 3, 4}, btreeset! {5});
    let dropped = change.apply(&old, false).unwrap();

    let cases = [
        (true, retained, btreemap! {1 => 10, 2 => 20, 3 => 30, 4 => 40}),
        (false, dropped, btreemap! {2 => 20, 3 => 30, 4 => 40}),
    ];
    for (retain, goal, want_target) in cases {
        // In both modes the joint keeps old voter 1 with its metadata, but not old learner 5.
        let joint = old.joint_with(&goal);
        let got = node_metadata(&joint);
        let want = btreemap! {1 => 10, 2 => 20, 3 => 30, 4 => 40};
        assert_eq!(want, got);

        #[cfg(feature = "serde")]
        {
            let json = serde_json::to_string(&joint).unwrap();
            let decoded = serde_json::from_str::<M>(&json).unwrap();
            assert_eq!(joint, decoded);

            let binary = bincode::serialize(&joint).unwrap();
            let decoded = bincode::deserialize::<M>(&binary).unwrap();
            assert_eq!(joint, decoded);
        }

        // Leaving the joint selects the target of the change, which holds voter 1 only when the
        // change retains it.
        let change = C::make_uniform();
        let target = change.apply(&joint, retain).unwrap();
        assert_eq!(goal, target);
        let got = node_metadata(&target);
        assert_eq!(want_target, got);
    }
}

/// Asserts that every read quorum of `membership` meets every write quorum, as it claims.
fn assert_read_write_quorums_intersect<M>(membership: &M)
where M: RaftMembership {
    let claim = membership.read_write_quorums_intersect();
    assert_eq!(Some(true), claim, "{membership}");
    let read = membership.read_quorum_set();
    let write = membership.write_quorum_set();
    let exact = verify_intersection(&read, &write);
    assert!(exact, "{membership}");
}

/// Returns the metadata of each node of `membership`, as `get_node()` returns it.
fn node_metadata<M>(membership: &M) -> BTreeMap<u64, u64>
where M: RaftMembership<NodeId = u64, Node = u64> {
    let mut nodes = BTreeMap::new();
    for node_id in membership.node_ids() {
        let node = membership.get_node(&node_id).unwrap();
        nodes.insert(node_id, *node.node);
    }
    nodes
}
