use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::num::NonZeroU64;
use std::sync::Arc;

use maplit::btreemap;
use maplit::btreeset;

use crate::ChangeMembers;
use crate::errors::EmptyMembership;
use crate::errors::MembershipError;
use crate::errors::NodeNotFound;
use crate::errors::Operation;
use crate::membership::InvalidThresholds;
use crate::membership::NodeRef;
use crate::membership::NodeRole;
use crate::membership::RaftMembership;
use crate::membership::RaftMembershipEdit;
use crate::membership::RaftMembershipExt;
use crate::membership::ThresholdChange;
use crate::membership::ThresholdMembership;
use crate::quorum::Joint;
use crate::quorum::QuorumSet;
use crate::quorum::ThresholdQuorumSet;
use crate::quorum::threshold_quorum_set_test::all_subsets;

/// Returns a membership whose only config holds `voters` with the two sizes. Each voter has a `()`
/// node.
fn threshold(voters: BTreeSet<u64>, read_size: u64, write_size: u64) -> ThresholdMembership<u64, ()> {
    let nodes = voters.clone();
    ThresholdMembership::new(voters, read_size, write_size, nodes).unwrap()
}

/// Returns the error that `ThresholdMembership` returns for sizes that do not fit its voters.
fn invalid_thresholds(voter_count: u64, read_size: u64, write_size: u64) -> MembershipError<u64> {
    let invalid = InvalidThresholds {
        voter_count,
        read_size,
        write_size,
    };
    MembershipError::from(invalid)
}

/// Returns the sets in `subsets` that `quorum_set` accepts, in their order.
fn accepted(quorum_set: &Joint<ThresholdQuorumSet<u64>>, subsets: &[BTreeSet<u64>]) -> Vec<BTreeSet<u64>> {
    let mut accepted = vec![];
    for subset in subsets {
        let is_quorum = quorum_set.is_quorum(subset.iter());
        if is_quorum {
            accepted.push(subset.clone());
        }
    }
    accepted
}

/// Returns whether the read rule and the write rule of each config share one voter set.
fn shares_voter_sets(membership: &ThresholdMembership<u64, ()>) -> bool {
    for (read, write) in membership.configs() {
        let shared = Arc::ptr_eq(read.id_set(), write.id_set());
        if !shared {
            return false;
        }
    }
    true
}

#[test]
fn test_new() {
    let voters = btreeset! {1, 2, 3, 4, 5};

    // Every read quorum intersects every write quorum exactly when read_size + write_size > 5.
    for (read_size, write_size) in [(4, 2), (2, 4), (3, 3), (5, 1)] {
        let membership = threshold(voters.clone(), read_size, write_size);

        let got = membership.to_string();
        let want =
            format!("{{voters:[{{1:(),2:(),3:(),4:(),5:()}} read={read_size} write={write_size}], learners:[]}}");
        assert_eq!(want, got);

        let shared = shares_voter_sets(&membership);
        assert!(shared);
    }

    for (read_size, write_size) in [(2, 3), (0, 5), (6, 1)] {
        let res = ThresholdMembership::<u64, ()>::new(voters.clone(), read_size, write_size, voters.clone());
        let want = Err(invalid_thresholds(5, read_size, write_size));
        assert_eq!(want, res);
    }

    // Voter 3 has no node.
    let res = ThresholdMembership::<u64, ()>::new(btreeset! {1, 2, 3}, 2, 2, btreeset! {1, 2});
    let want = Err(MembershipError::NodeNotFound(NodeNotFound::new(3, Operation::None)));
    assert_eq!(want, res);
}

#[test]
fn test_quorum_sets_accept_read_size_and_write_size_voters() {
    let voters = btreeset! {1, 2, 3, 4, 5};
    let subsets = all_subsets(&voters);
    let membership = threshold(voters, 4, 2);

    // 5 subsets hold 4 voters, and 1 holds all 5.
    let read_accepted = accepted(membership.read_quorum_set(), &subsets);
    let want = subsets.iter().filter(|subset| subset.len() >= 4).cloned().collect::<Vec<_>>();
    assert_eq!(6, want.len());
    assert_eq!(want, read_accepted);

    // Only the empty set and the 5 single voters are not write quorums.
    let write_accepted = accepted(membership.write_quorum_set(), &subsets);
    let want = subsets.iter().filter(|subset| subset.len() >= 2).cloned().collect::<Vec<_>>();
    assert_eq!(26, want.len());
    assert_eq!(want, write_accepted);
}

#[test]
fn test_read_quorums_may_be_disjoint() {
    let membership = threshold(btreeset! {1, 2, 3, 4, 5}, 2, 4);

    // A read quorum must intersect only the write quorums, so the disjoint {1,2} and {3,4} are
    // both read quorums.
    let first = membership.read_quorum_set().is_quorum([1, 2].iter());
    assert!(first);
    let second = membership.read_quorum_set().is_quorum([3, 4].iter());
    assert!(second);
}

#[test]
fn test_transition_between_sizes() {
    let voters = btreeset! {1, 2, 3, 4, 5};
    let subsets = all_subsets(&voters);
    let m42 = threshold(voters.clone(), 4, 2);
    let m43 = threshold(voters.clone(), 4, 3);
    let m24 = threshold(voters, 2, 4);

    // A quorum of 4 voters meets every quorum of 2 or 3 of the same 5 voters.
    let direct = m42.supports_direct_transition_to(&m43);
    assert!(direct);

    // Write quorum {1,2} of (4,2) misses read quorum {3,4} of (2,4).
    let direct = m42.supports_direct_transition_to(&m24);
    assert!(!direct);

    // A quorum of the joint is a quorum of both configs, so it needs 4 voters for reads and for
    // writes.
    let joint = m42.joint_with(&m24);
    let want = subsets.iter().filter(|subset| subset.len() >= 4).cloned().collect::<Vec<_>>();
    let read_accepted = accepted(joint.read_quorum_set(), &subsets);
    assert_eq!(want, read_accepted);
    let write_accepted = accepted(joint.write_quorum_set(), &subsets);
    assert_eq!(want, write_accepted);
}

#[test]
fn test_change() {
    let voters = btreeset! {1, 2, 3, 4, 5};
    let m42 = threshold(voters.clone(), 4, 2);

    // A change of the sizes keeps the voters.
    let res = m42.change(ThresholdChange::sizes(2, 4));
    let want = Ok(threshold(voters.clone(), 2, 4));
    assert_eq!(want, res);

    // A change of the members keeps the sizes. Removed voter 5 stays as a learner.
    let remove_five = ChangeMembers::RemoveVoters(btreeset! {5});
    let res = m42.change(ThresholdChange::members(remove_five));
    let want = ThresholdMembership::new(btreeset! {1, 2, 3, 4}, 4, 2, voters.clone());
    assert_eq!(want, res);

    let res = m42.change(ThresholdChange::add_learner(6, ()));
    let want = ThresholdMembership::new(voters.clone(), 4, 2, btreeset! {1, 2, 3, 4, 5, 6});
    assert_eq!(want, res);

    // Growing 3 voters with (2,2) to 5 voters needs new sizes, because 2 + 2 is not more than 5.
    let m22 = ThresholdMembership::<u64, ()>::new(btreeset! {1, 2, 3}, 2, 2, voters.clone()).unwrap();
    let add_four_five = ThresholdChange::members(ChangeMembers::AddVoterIds(btreeset! {4, 5}));

    let res = m22.change(add_four_five.clone());
    let want = Err(invalid_thresholds(5, 2, 2));
    assert_eq!(want, res);

    let res = m22.change(add_four_five.with_sizes(3, 3));
    let want = Ok(threshold(voters.clone(), 3, 3));
    assert_eq!(want, res);

    // Leaving a joint membership keeps its target and the sizes of the target.
    let joint = m42.joint_with(&threshold(voters.clone(), 2, 4));
    let res = joint.change(ThresholdChange::leave_joint());
    let want = Ok(threshold(voters, 2, 4));
    assert_eq!(want, res);

    // A membership with no config has no target to change.
    let empty = ThresholdMembership::<u64, ()>::default();
    let res = empty.change(ThresholdChange::sizes(1, 1));
    let want = Err(MembershipError::EmptyMembership(EmptyMembership {}));
    assert_eq!(want, res);
}

#[test]
fn test_raft_membership_new_rebuilds_joint_membership() {
    let nodes = btreeset! {1, 2, 3, 4};
    let old = ThresholdMembership::<u64, ()>::new(btreeset! {1, 2, 3}, 2, 2, nodes.clone()).unwrap();
    let new = ThresholdMembership::<u64, ()>::new(btreeset! {2, 3, 4}, 3, 1, nodes).unwrap();
    let joint = old.joint_with(&new);

    let mut nodes = vec![];
    for node_id in joint.node_ids() {
        let node = joint.get_node(&node_id).unwrap();
        nodes.push((node_id, *node.node, node.role));
    }
    let read = joint.read_quorum_set().clone();
    let write = joint.write_quorum_set().clone();

    let rebuilt = <ThresholdMembership<u64, ()> as RaftMembership>::new(read, write, nodes);
    assert_eq!(joint, rebuilt);

    let shared = shares_voter_sets(&rebuilt);
    assert!(shared);
}

#[test]
#[should_panic(expected = "a read rule holds the voters {1, 2, 3}, but its write rule holds {1, 2, 4}")]
fn test_raft_membership_new_panics_on_different_voters() {
    let two = NonZeroU64::new(2).unwrap();
    let read_rule = ThresholdQuorumSet::new(two, btreeset! {1, 2, 3});
    let write_rule = ThresholdQuorumSet::new(two, btreeset! {1, 2, 4});
    let read = Joint::new([read_rule]);
    let write = Joint::new([write_rule]);
    let nodes: [(u64, (), NodeRole); 0] = [];

    <ThresholdMembership<u64, ()> as RaftMembership>::new(read, write, nodes);
}

#[test]
fn test_get_node() {
    let membership = ThresholdMembership::<u64, ()>::new(btreeset! {1, 2, 3}, 2, 2, btreeset! {1, 2, 3, 4}).unwrap();

    let voter = membership.get_node(&3);
    let want = Some(NodeRef {
        node: &(),
        role: NodeRole::Campaigner | NodeRole::ReceiveLogs,
        is_read_voter: true,
        is_write_voter: true,
    });
    assert_eq!(want, voter);

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

    let ids = membership.node_ids().collect::<Vec<_>>();
    assert_eq!(vec![1, 2, 3, 4], ids);
}

#[test]
fn test_from_node_map() {
    // Every node becomes a voter, and both sizes are a majority of the 4 voters.
    let nodes: BTreeMap<u64, ()> = btreemap! {1 => (), 2 => (), 3 => (), 4 => ()};
    let membership = ThresholdMembership::from(nodes);

    let want = threshold(btreeset! {1, 2, 3, 4}, 3, 3);
    assert_eq!(want, membership);
}

#[test]
fn test_display() {
    let voters = btreeset! {1, 2, 3, 4, 5};
    let nodes = btreeset! {1, 2, 3, 4, 5, 6};
    let m42 = ThresholdMembership::<u64, ()>::new(voters.clone(), 4, 2, nodes.clone()).unwrap();

    let got = m42.to_string();
    assert_eq!(
        "{voters:[{1:(),2:(),3:(),4:(),5:()} read=4 write=2], learners:[6:()]}",
        got
    );

    // Config i pairs read rule i with write rule i, so the two configs print different sizes.
    let m24 = ThresholdMembership::<u64, ()>::new(voters, 2, 4, nodes).unwrap();
    let joint = m42.joint_with(&m24);
    let got = joint.to_string();
    let want = "{voters:[{1:(),2:(),3:(),4:(),5:()} read=4 write=2,{1:(),2:(),3:(),4:(),5:()} read=2 write=4], \
                learners:[6:()]}";
    assert_eq!(want, got);

    let change = ThresholdChange::<u64, ()>::sizes(2, 4);
    let got = change.to_string();
    assert_eq!("{members: None, read_size: 2, write_size: 4}", got);
}

#[cfg(feature = "serde")]
#[test]
fn test_serde() {
    let membership = threshold(btreeset! {1, 2, 3, 4, 5}, 4, 2);

    let json = serde_json::to_string(&membership).unwrap();
    let want = r#"{"read":{"rules":[{"ids":[1,2,3,4,5],"quorum_size":4}]},"write":{"rules":[{"ids":[1,2,3,4,5],"quorum_size":2}]},"nodes":{"1":null,"2":null,"3":null,"4":null,"5":null}}"#;
    assert_eq!(want, json);

    // Decoding gives the two rules of a config one voter set again.
    let decoded = serde_json::from_str::<ThresholdMembership<u64, ()>>(&json).unwrap();
    assert_eq!(membership, decoded);
    let shared = shares_voter_sets(&decoded);
    assert!(shared);

    let empty = r#"{"read":{"rules":[]},"write":{"rules":[]},"nodes":{}}"#;
    let decoded = serde_json::from_str::<ThresholdMembership<u64, ()>>(empty).unwrap();
    let want = ThresholdMembership::default();
    assert_eq!(want, decoded);
}

#[cfg(feature = "serde")]
#[test]
fn test_serde_rejects_invalid_configs() {
    // serde_json appends the position of the error to the message of the decoder.
    let decode_error = |json: &str| {
        let err = serde_json::from_str::<ThresholdMembership<u64, ()>>(json).unwrap_err();
        err.to_string()
    };

    // 2 + 3 is not more than 5 voters.
    let small_sizes = r#"{"read":{"rules":[{"ids":[1,2,3,4,5],"quorum_size":2}]},"write":{"rules":[{"ids":[1,2,3,4,5],"quorum_size":3}]},"nodes":{"1":null,"2":null,"3":null,"4":null,"5":null}}"#;
    let message = decode_error(small_sizes);
    let invalid = InvalidThresholds {
        voter_count: 5,
        read_size: 2,
        write_size: 3,
    };
    let reports_sizes = message.starts_with(&invalid.to_string());
    assert!(reports_sizes, "{message}");

    let different_voters = r#"{"read":{"rules":[{"ids":[1,2,3],"quorum_size":2}]},"write":{"rules":[{"ids":[1,2,4],"quorum_size":2}]},"nodes":{"1":null,"2":null,"3":null,"4":null}}"#;
    let message = decode_error(different_voters);
    let reports_voters =
        message.starts_with("a read rule holds the voters {1, 2, 3}, but its write rule holds {1, 2, 4}");
    assert!(reports_voters, "{message}");

    // Voter 3 has no node.
    let missing_node = r#"{"read":{"rules":[{"ids":[1,2,3],"quorum_size":2}]},"write":{"rules":[{"ids":[1,2,3],"quorum_size":2}]},"nodes":{"1":null,"2":null}}"#;
    let message = decode_error(missing_node);
    let node_not_found = NodeNotFound::new(3, Operation::None);
    let reports_node = message.starts_with(&node_not_found.to_string());
    assert!(reports_node, "{message}");
}
