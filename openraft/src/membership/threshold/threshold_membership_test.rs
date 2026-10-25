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
use crate::membership::ThresholdMembershipError;
use crate::quorum::Joint;
use crate::quorum::QuorumSet;
use crate::quorum::ThresholdQuorumSet;
use crate::quorum::threshold_quorum_set_test::all_subsets;
use crate::type_config::alias::MembershipOf;
use crate::type_config::alias::MembershipStateOf;
use crate::type_config::alias::StoredMembershipOf;

/// Returns a membership whose only config holds `voters` with this write size and `()` nodes.
fn threshold(voters: BTreeSet<u64>, write_size: u64) -> ThresholdMembership<u64, ()> {
    let nodes = voters.clone();
    ThresholdMembership::new(voters, write_size, nodes).unwrap()
}

/// Returns the error that `ThresholdMembership` returns for sizes that do not fit its voters.
fn invalid_thresholds(voter_count: u64, write_size: u64) -> ThresholdMembershipError<u64> {
    let invalid = InvalidThresholds {
        voter_count,
        write_size,
    };
    ThresholdMembershipError::InvalidThresholds(invalid)
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

    // Each write size determines the minimum intersecting read size.
    for (read_size, write_size) in [(4, 2), (2, 4), (3, 3), (5, 1)] {
        let membership = threshold(voters.clone(), write_size);

        let got = membership.to_string();
        let want =
            format!("{{voters:[{{1:(),2:(),3:(),4:(),5:()}} read={read_size} write={write_size}], learners:[]}}");
        assert_eq!(want, got);

        let shared = shares_voter_sets(&membership);
        assert!(shared);
    }

    for write_size in [0, 6, u64::MAX] {
        let res = ThresholdMembership::<u64, ()>::new(voters.clone(), write_size, voters.clone());
        let want = Err(invalid_thresholds(5, write_size));
        assert_eq!(want, res);
    }

    let res = ThresholdMembership::<u64, ()>::new(BTreeSet::new(), 1, BTreeSet::new());
    let want = Err(invalid_thresholds(0, 1));
    assert_eq!(want, res);

    // Voter 3 has no node.
    let res = ThresholdMembership::<u64, ()>::new(btreeset! {1, 2, 3}, 2, btreeset! {1, 2});
    let want = Err(ThresholdMembershipError::NodeNotFound(NodeNotFound::new(
        3,
        Operation::None,
    )));
    assert_eq!(want, res);
}

#[test]
fn test_every_write_size_derives_minimum_intersecting_read_quorums() {
    for voter_count in 1..=6 {
        let voters: BTreeSet<u64> = (1..=voter_count).collect();
        let subsets = all_subsets(&voters);
        for write_size in 1..=voter_count {
            let membership = threshold(voters.clone(), write_size);
            let minimum_read = (voter_count - write_size + 1) as usize;
            let read_subsets = subsets.iter().filter(|ids| ids.len() >= minimum_read);
            let expected_reads: Vec<_> = read_subsets.cloned().collect();
            let actual_reads = accepted(membership.read_quorum_set(), &subsets);
            assert_eq!(
                expected_reads, actual_reads,
                "voters={voter_count}, write_size={write_size}"
            );
            let write_subsets = subsets.iter().filter(|ids| ids.len() >= write_size as usize);
            let expected_writes: Vec<_> = write_subsets.cloned().collect();
            let actual_writes = accepted(membership.write_quorum_set(), &subsets);
            assert_eq!(
                expected_writes, actual_writes,
                "voters={voter_count}, write_size={write_size}"
            );
        }
    }
}

#[test]
fn test_quorum_sets_accept_read_size_and_write_size_voters() {
    let voters = btreeset! {1, 2, 3, 4, 5};
    let subsets = all_subsets(&voters);
    let membership = threshold(voters, 2);

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
    let membership = threshold(btreeset! {1, 2, 3, 4, 5}, 4);

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
    let m42 = threshold(voters.clone(), 2);
    let m33 = threshold(voters.clone(), 3);
    let m24 = threshold(voters, 4);

    // Changing the write size also changes the read size, so one direction can miss a quorum.
    let direct = m42.supports_direct_transition_to(&m33);
    assert!(!direct);

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
    let m42 = threshold(voters.clone(), 2);

    // A change of the sizes keeps the voters.
    let res = m42.change(ThresholdChange::write_size(4));
    let want = Ok(threshold(voters.clone(), 4));
    assert_eq!(want, res);

    // A change of the members keeps the write size and derives the new read size.
    let remove_five = ChangeMembers::RemoveVoters(btreeset! {5});
    let res = m42.change(ThresholdChange::members(remove_five));
    let want = ThresholdMembership::new(btreeset! {1, 2, 3, 4}, 2, voters.clone());
    let want = want.map_err(MembershipError::from);
    assert_eq!(want, res);

    let change = <ThresholdChange<u64, ()> as RaftMembershipEdit<ThresholdMembership<u64, ()>>>::add_learner(6, ());
    let res: Result<_, MembershipError<u64>> = change.apply(&m42);
    let want = ThresholdMembership::new(voters.clone(), 2, btreeset! {1, 2, 3, 4, 5, 6});
    let want = want.map_err(MembershipError::from);
    assert_eq!(want, res);

    // Growing the voter set keeps the write size and raises the read size automatically.
    let m22 = ThresholdMembership::<u64, ()>::new(btreeset! {1, 2, 3}, 2, voters.clone()).unwrap();
    let add_four_five = ThresholdChange::members(ChangeMembers::AddVoterIds(btreeset! {4, 5}));

    let res = m22.change(add_four_five.clone());
    let want = Ok(threshold(voters.clone(), 2));
    assert_eq!(want, res);

    let res = m22.change(add_four_five.with_write_size(3));
    let want = Ok(threshold(voters.clone(), 3));
    assert_eq!(want, res);

    // Removing too many voters needs an explicit reduction in the write size.
    let m24 = threshold(voters.clone(), 4);
    let remove = ThresholdChange::members(ChangeMembers::RemoveVoters(btreeset! {4, 5}));
    let res = m24.change(remove.clone());
    let want = Err(MembershipError::from(invalid_thresholds(3, 4)));
    assert_eq!(want, res);
    let res = m24.change(remove.with_write_size(3));
    let want = ThresholdMembership::new(btreeset! {1, 2, 3}, 3, voters.clone());
    let want = want.map_err(MembershipError::from);
    assert_eq!(want, res);

    // Leaving a joint membership keeps its target and the write size of the target.
    let joint = m42.joint_with(&threshold(voters.clone(), 4));
    let change = <ThresholdChange<u64, ()> as RaftMembershipEdit<ThresholdMembership<u64, ()>>>::make_uniform();
    let res: Result<_, MembershipError<u64>> = change.apply(&joint);
    let want = Ok(threshold(voters, 4));
    assert_eq!(want, res);

    // A membership with no config has no target to change.
    let empty = ThresholdMembership::<u64, ()>::default();
    let res = empty.change(ThresholdChange::write_size(1));
    let want = Err(MembershipError::EmptyMembership(EmptyMembership {}));
    assert_eq!(want, res);
}

#[test]
fn test_raft_membership_new_rebuilds_joint_membership() {
    let nodes = btreeset! {1, 2, 3, 4};
    let old = ThresholdMembership::<u64, ()>::new(btreeset! {1, 2, 3}, 2, nodes.clone()).unwrap();
    let new = ThresholdMembership::<u64, ()>::new(btreeset! {2, 3, 4}, 1, nodes).unwrap();
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
#[should_panic(expected = "read_size=3 must equal voter_count + 1 - write_size=2")]
fn test_raft_membership_new_rejects_non_minimal_read_size() {
    let three = NonZeroU64::new(3).unwrap();
    let rule = ThresholdQuorumSet::new(three, btreeset! {1, 2, 3, 4});
    let read = Joint::new([rule.clone()]);
    let write = Joint::new([rule]);
    let nodes: [(u64, (), NodeRole); 0] = [];

    <ThresholdMembership<u64, ()> as RaftMembership>::new(read, write, nodes);
}

#[test]
fn test_get_node() {
    let membership = ThresholdMembership::<u64, ()>::new(btreeset! {1, 2, 3}, 2, btreeset! {1, 2, 3, 4}).unwrap();

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
    // Four voters default to write=3, with the minimum intersecting read=2.
    let nodes: BTreeMap<u64, ()> = btreemap! {1 => (), 2 => (), 3 => (), 4 => ()};
    let membership = ThresholdMembership::from(nodes);

    let want = threshold(btreeset! {1, 2, 3, 4}, 3);
    assert_eq!(want, membership);

    let empty = ThresholdMembership::<u64, ()>::from(BTreeMap::new());
    let want = ThresholdMembership::default();
    assert_eq!(want, empty);
}

#[test]
fn test_display() {
    let voters = btreeset! {1, 2, 3, 4, 5};
    let nodes = btreeset! {1, 2, 3, 4, 5, 6};
    let m42 = ThresholdMembership::<u64, ()>::new(voters.clone(), 2, nodes.clone()).unwrap();

    let got = m42.to_string();
    assert_eq!(
        "{voters:[{1:(),2:(),3:(),4:(),5:()} read=4 write=2], learners:[6:()]}",
        got
    );

    // Config i pairs read rule i with write rule i, so the two configs print different sizes.
    let m24 = ThresholdMembership::<u64, ()>::new(voters, 4, nodes).unwrap();
    let joint = m42.joint_with(&m24);
    let got = joint.to_string();
    let want = "{voters:[{1:(),2:(),3:(),4:(),5:()} read=4 write=2,{1:(),2:(),3:(),4:(),5:()} read=2 write=4], \
                learners:[6:()]}";
    assert_eq!(want, got);

    let change = ThresholdChange::<u64, ()>::write_size(4);
    let got = change.to_string();
    assert_eq!("{members: None, write_size: 4}", got);
}

#[cfg(feature = "serde")]
#[test]
fn test_serde() {
    let membership = threshold(btreeset! {1, 2, 3, 4, 5}, 2);

    let json = serde_json::to_string(&membership).unwrap();
    let want = r#"{"write":{"rules":[{"ids":[1,2,3,4,5],"quorum_size":2}]},"nodes":{"1":null,"2":null,"3":null,"4":null,"5":null}}"#;
    assert_eq!(want, json);

    // Decoding gives the two rules of a config one voter set again.
    let decoded = serde_json::from_str::<ThresholdMembership<u64, ()>>(&json).unwrap();
    assert_eq!(membership, decoded);
    let shared = shares_voter_sets(&decoded);
    assert!(shared);

    let empty = r#"{"write":{"rules":[]},"nodes":{}}"#;
    let decoded = serde_json::from_str::<ThresholdMembership<u64, ()>>(empty).unwrap();
    let want = ThresholdMembership::default();
    assert_eq!(want, decoded);
}

#[cfg(feature = "serde")]
#[test]
fn test_binary_roundtrip_rebuilds_joint_read_rules() {
    let first = threshold(btreeset! {1, 2, 3, 4}, 3);
    let second = threshold(btreeset! {2, 3, 4, 5, 6}, 2);
    let joint = first.joint_with(&second);
    let binary = bincode::serialize(&joint).unwrap();
    let decoded = bincode::deserialize::<ThresholdMembership<u64, ()>>(&binary).unwrap();
    assert_eq!(joint, decoded);
    let shared = shares_voter_sets(&decoded);
    assert!(shared);
}

#[cfg(feature = "serde")]
#[test]
fn test_serde_rejects_invalid_configs() {
    // serde_json appends the position of the error to the message of the decoder.
    let decode_error = |json: &str| {
        let err = serde_json::from_str::<ThresholdMembership<u64, ()>>(json).unwrap_err();
        err.to_string()
    };

    // The write size exceeds the number of voters.
    let large_size = r#"{"write":{"rules":[{"ids":[1,2,3],"quorum_size":4}]},"nodes":{"1":null,"2":null,"3":null}}"#;
    let message = decode_error(large_size);
    let invalid = InvalidThresholds {
        voter_count: 3,
        write_size: 4,
    };
    let reports_sizes = message.starts_with(&invalid.to_string());
    assert!(reports_sizes, "{message}");

    // A separately supplied read size must not be silently ignored.
    let explicit_read = r#"{"read":{"rules":[]},"write":{"rules":[]},"nodes":{}}"#;
    let message = decode_error(explicit_read);
    let reports_read = message.starts_with("unknown field `read`");
    assert!(reports_read, "{message}");

    // Voter 3 has no node.
    let missing_node = r#"{"write":{"rules":[{"ids":[1,2,3],"quorum_size":2}]},"nodes":{"1":null,"2":null}}"#;
    let message = decode_error(missing_node);
    let node_not_found = NodeNotFound::new(3, Operation::None);
    let reports_node = message.starts_with(&node_not_found.to_string());
    assert!(reports_node, "{message}");
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[error("{0}")]
struct TestErrorSource(String);

impl crate::errors::ErrorSource for TestErrorSource {
    fn from_error<E>(error: &E) -> Self
    where E: std::error::Error + 'static {
        Self(format!("wrapped: {error}"))
    }

    fn from_string(message: impl ToString) -> Self {
        Self(message.to_string())
    }
}

crate::declare_raft_types!(
    CustomErrorConfig:
        D = u64,
        R = (),
        Node = (),
        Membership = ThresholdMembership<Self::NodeId, Self::Node>,
        MembershipEdit = ThresholdChange<Self::NodeId, Self::Node>,
        ErrorSource = TestErrorSource,
        AsyncRuntime = openraft_rt_tokio::TokioRuntime,
);

crate::declare_raft_types!(
    DefaultMembershipConfig:
        D = u64,
        R = (),
        Node = (),
        ErrorSource = TestErrorSource,
        AsyncRuntime = openraft_rt_tokio::TokioRuntime,
);

#[test]
fn constructor_returns_concrete_validation_error() {
    type Membership = MembershipOf<CustomErrorConfig>;
    let voters = btreeset! {1, 2, 3};
    let result = Membership::new(voters.clone(), 4, voters);
    let expected = Err(invalid_thresholds(3, 4));
    assert_eq!(expected, result);
}

#[test]
fn membership_change_preserves_configured_error_source() {
    type Membership = MembershipOf<CustomErrorConfig>;
    let membership = Membership::from(btreemap! {1 => (), 2 => (), 3 => ()});
    let stored = Arc::new(StoredMembershipOf::<CustomErrorConfig>::new(None, membership));
    let state = MembershipStateOf::<CustomErrorConfig>::new(stored.clone(), stored);
    let result = state.next_membership::<_, TestErrorSource>(ThresholdChange::write_size(4), true, false);
    let source = TestErrorSource(
        "wrapped: invalid threshold: write_size=4, voter_count=3; need 1 <= write_size <= voter_count".into(),
    );
    let error = crate::errors::ChangeMembershipError::UserDefined(source);
    let expected = Err(error);
    assert_eq!(expected, result);
}

#[test]
fn default_constructor_returns_missing_node() {
    type Membership = MembershipOf<DefaultMembershipConfig>;
    let result = Membership::new(vec![btreeset! {1, 2}], btreemap! {1 => ()});
    let expected = NodeNotFound::new(2, Operation::None);
    let expected = Err(expected);
    assert_eq!(expected, result);
}

#[cfg(feature = "serde")]
#[test]
fn configured_error_source_preserves_error_serialization() {
    let source = TestErrorSource("custom".into());
    let error: MembershipError<u64, TestErrorSource> = MembershipError::UserDefined(source);
    let json = serde_json::to_string(&error).unwrap();
    let decoded = serde_json::from_str::<MembershipError<u64, TestErrorSource>>(&json).unwrap();
    assert_eq!(error, decoded);
}

#[test]
fn edit_preserves_structural_errors_with_custom_source() {
    let membership = ThresholdMembership::<u64, ()>::default();
    let change = ThresholdChange::write_size(1);
    let result: Result<_, MembershipError<u64, TestErrorSource>> = change.apply(&membership);
    let expected = Err(MembershipError::EmptyMembership(EmptyMembership {}));
    assert_eq!(expected, result);

    let result = ThresholdMembership::<u64, ()>::new(btreeset! {1, 2}, 1, btreeset! {1});
    let error = result.unwrap_err();
    let actual: MembershipError<u64, TestErrorSource> = error.into();
    let missing = NodeNotFound::new(2, Operation::None);
    let expected = MembershipError::NodeNotFound(missing);
    assert_eq!(expected, actual);
}
