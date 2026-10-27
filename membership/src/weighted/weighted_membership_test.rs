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
use quorum_set::verify_intersection;

use crate::ThresholdMembership;
use crate::WeightedChange;
use crate::WeightedMembership;
use crate::errors::InvalidWeights;
use crate::quorum::QuorumSet;
use crate::quorum::testing::accepted;
use crate::quorum::testing::all_subsets;
use crate::testing::assert_apply_checks_roles;
use crate::testing::assert_change_through_joint_keeps_node_metadata;
use crate::testing::assert_claims_hold;
use crate::testing::assert_joint_keeps_old_roles_and_target_restores_new_roles;
use crate::testing::assert_quorum_sets_are_valid;
use crate::testing::assert_role_and_quorum_change_through_joint;
use crate::testing::assert_with_roles;
use crate::testing::voter_role;

type Membership = WeightedMembership<u64, ()>;

/// Returns a membership whose only config gives its voters these weights and this write weight,
/// with a `()` node for each voter.
fn weighted(weights: BTreeMap<u64, u64>, write_weight: u64) -> Membership {
    let nodes: BTreeSet<u64> = weights.keys().copied().collect();
    WeightedMembership::new(weights, write_weight, nodes).unwrap()
}

/// Returns a membership whose only config gives each of `voters` weight 1 and needs weight 2 to
/// write, with each node of `nodes`.
fn unit_weights<N, T>(voters: BTreeSet<u64>, nodes: T) -> WeightedMembership<u64, N>
where
    N: Node,
    T: IntoNodes<u64, N>,
{
    let pairs = voters.into_iter().map(|voter| (voter, 1));
    let weights = pairs.collect();
    WeightedMembership::new(weights, 2, nodes).unwrap()
}

#[test]
fn test_new() {
    let weights = btreemap! {1 => 2, 2 => 1, 3 => 1};
    let membership = Membership::new(weights.clone(), 2, btreeset! {1, 2, 3, 4}).unwrap();
    let want = "{configs: [Config { read: WeightedQuorumSet { weights: {1: 2, 2: 1, 3: 1}, threshold: 3 }, \
                write: WeightedQuorumSet { weights: {1: 2, 2: 1, 3: 1}, threshold: 2 }, \
                nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), 2: ((), NodeRole(Campaigner | ReceiveLogs)), \
                3: ((), NodeRole(Campaigner | ReceiveLogs)), 4: ((), NodeRole(ReceiveLogs))} }]}";
    assert_eq!(want, membership.to_string());

    // Voter 3 has no node.
    let res = Membership::new(weights, 2, btreeset! {1, 2});
    let want = Err(MembershipError::NodeNotFound(NodeNotFound::new(3, Operation::None)));
    assert_eq!(want, res);
}

#[test]
fn test_new_rejects_invalid_weights() {
    let weights = btreemap! {1 => 2, 2 => 1, 3 => 1};
    let nodes = btreeset! {1, 2, 3};

    // The write weight must lie in 1..=4, and no write weight fits an empty map.
    let cases = [(weights.clone(), 0, 4), (weights, 5, 4), (BTreeMap::new(), 1, 0)];
    for (weights, write_weight, total_weight) in cases {
        let res = Membership::new(weights, write_weight, nodes.clone());
        let invalid = InvalidWeights::WriteWeightOutOfRange {
            write_weight,
            total_weight,
        };
        assert_eq!(Err(MembershipError::UserDefined(invalid)), res);
    }

    let res = Membership::new(btreemap! {1 => 1, 2 => 0}, 1, nodes.clone());
    let invalid = InvalidWeights::ZeroWeight { node_id: 2 };
    assert_eq!(Err(MembershipError::UserDefined(invalid)), res);

    // The weights sum to u64::MAX + 1.
    let res = Membership::new(btreemap! {1 => u64::MAX, 2 => 1}, 1, nodes);
    let invalid = InvalidWeights::TotalWeightOverflow;
    assert_eq!(Err(MembershipError::UserDefined(invalid)), res);
}

#[test]
fn test_quorum_sets_accept_exact_sets_with_unequal_weights() {
    // The total weight is 7, so write weight 3 derives read weight 5.
    let membership = weighted(btreemap! {1 => 3, 2 => 2, 3 => 1, 4 => 1}, 3);
    let subsets = all_subsets(&btreeset! {1, 2, 3, 4});

    // Voter 1 alone commits an entry, while voters 3 and 4 together do not.
    let write_accepted = accepted(&membership.write_quorum_set(), &subsets);
    let want = vec![
        btreeset! {1},
        btreeset! {1, 2},
        btreeset! {1, 3},
        btreeset! {2, 3},
        btreeset! {1, 2, 3},
        btreeset! {1, 4},
        btreeset! {2, 4},
        btreeset! {1, 2, 4},
        btreeset! {1, 3, 4},
        btreeset! {2, 3, 4},
        btreeset! {1, 2, 3, 4},
    ];
    assert_eq!(want, write_accepted);

    // Voters 2, 3 and 4 weigh 4 together, so every election needs voter 1.
    let read_accepted = accepted(&membership.read_quorum_set(), &subsets);
    let want = vec![
        btreeset! {1, 2},
        btreeset! {1, 2, 3},
        btreeset! {1, 2, 4},
        btreeset! {1, 3, 4},
        btreeset! {1, 2, 3, 4},
    ];
    assert_eq!(want, read_accepted);

    // A repeated ID counts once, and an unknown ID adds nothing.
    let repeated = membership.write_quorum_set().is_quorum([3, 3, 4, 4, 9].iter());
    assert!(!repeated);
}

#[test]
fn test_boundary_write_weights() {
    let subsets = all_subsets(&btreeset! {1, 2});
    let nonempty = vec![btreeset! {1}, btreeset! {2}, btreeset! {1, 2}];
    let both = vec![btreeset! {1, 2}];

    // Write weight 1 accepts every nonempty set, so a read needs the total weight 3. Write
    // weight 3 is the reverse.
    let weights = btreemap! {1 => 2, 2 => 1};
    let cases = [(1, nonempty.clone(), both.clone()), (3, both.clone(), nonempty.clone())];
    for (write_weight, want_write, want_read) in cases {
        let membership = weighted(weights.clone(), write_weight);
        let write_accepted = accepted(&membership.write_quorum_set(), &subsets);
        assert_eq!(want_write, write_accepted, "write_weight={write_weight}");
        let read_accepted = accepted(&membership.read_quorum_set(), &subsets);
        assert_eq!(want_read, read_accepted, "write_weight={write_weight}");
    }

    // A total weight of exactly u64::MAX fits.
    let membership = weighted(btreemap! {1 => u64::MAX - 1, 2 => 1}, u64::MAX);
    let want = format!(
        "{{configs: [Config {{ read: WeightedQuorumSet {{ weights: {{1: {0}, 2: 1}}, threshold: 1 }}, \
         write: WeightedQuorumSet {{ weights: {{1: {0}, 2: 1}}, threshold: {1} }}, \
         nodes: {{1: ((), NodeRole(Campaigner | ReceiveLogs)), 2: ((), NodeRole(Campaigner | ReceiveLogs))}} }}]}}",
        u64::MAX - 1,
        u64::MAX
    );
    assert_eq!(want, membership.to_string());
    let write_accepted = accepted(&membership.write_quorum_set(), &subsets);
    assert_eq!(both, write_accepted);
    let read_accepted = accepted(&membership.read_quorum_set(), &subsets);
    assert_eq!(nonempty, read_accepted);
}

#[test]
fn test_equal_weights_reproduce_threshold_membership() {
    for voter_count in 1..=5 {
        let voters: BTreeSet<u64> = (1..=voter_count).collect();
        let subsets = all_subsets(&voters);

        for write_size in 1..=voter_count {
            let threshold = ThresholdMembership::<u64, ()>::new(voters.clone(), write_size, voters.clone()).unwrap();
            let want_read = accepted(&threshold.read_quorum_set(), &subsets);
            let want_write = accepted(&threshold.write_quorum_set(), &subsets);

            // A voter of weight 3 counts like 3 voters of weight 1, so write weight
            // `3 * write_size` needs `write_size` voters.
            for weight in [1, 3] {
                let mut weights = BTreeMap::new();
                for voter_id in &voters {
                    weights.insert(*voter_id, weight);
                }
                let membership = weighted(weights, weight * write_size);

                let context = format!("voters={voter_count}, write_size={write_size}, weight={weight}");
                let read_accepted = accepted(&membership.read_quorum_set(), &subsets);
                assert_eq!(want_read, read_accepted, "{context}");
                let write_accepted = accepted(&membership.write_quorum_set(), &subsets);
                assert_eq!(want_write, write_accepted, "{context}");
            }
        }

        // Converting a node map gives every voter weight 1 and a majority write weight.
        let mut nodes = BTreeMap::new();
        for voter_id in &voters {
            nodes.insert(*voter_id, ());
        }
        let threshold = ThresholdMembership::from(nodes.clone());
        let membership = Membership::from(nodes);
        let want_read = accepted(&threshold.read_quorum_set(), &subsets);
        let read_accepted = accepted(&membership.read_quorum_set(), &subsets);
        assert_eq!(want_read, read_accepted, "voters={voter_count}");
        let want_write = accepted(&threshold.write_quorum_set(), &subsets);
        let write_accepted = accepted(&membership.write_quorum_set(), &subsets);
        assert_eq!(want_write, write_accepted, "voters={voter_count}");
    }
}

#[test]
fn test_from_node_map() {
    let nodes: BTreeMap<u64, ()> = btreemap! {1 => (), 2 => (), 3 => (), 4 => ()};
    let membership = Membership::from(nodes);

    // Four voters of weight 1 default to write weight 3, with read weight 2.
    let want = weighted(btreemap! {1 => 1, 2 => 1, 3 => 1, 4 => 1}, 3);
    assert_eq!(want, membership);

    let empty = Membership::from(BTreeMap::new());
    assert_eq!(Membership::default(), empty);
    assert_eq!("{configs: []}", empty.to_string());
}

#[test]
fn test_get_node() {
    let membership = Membership::new(btreemap! {1 => 2, 2 => 1, 3 => 1}, 2, btreeset! {1, 2, 3, 4}).unwrap();

    // Both rules of a config weigh the same voters.
    let voter = membership.get_node(&3);
    let want = Some(NodeRef {
        node: &(),
        role: voter_role(),
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

    let ids: Vec<u64> = membership.node_ids().collect();
    assert_eq!(vec![1, 2, 3, 4], ids);
    let read_ids: Vec<u64> = membership.read_quorum_set().ids().collect();
    assert_eq!(vec![1, 2, 3], read_ids);
    let write_ids: Vec<u64> = membership.write_quorum_set().ids().collect();
    assert_eq!(vec![1, 2, 3], write_ids);
}

#[test]
fn test_direct_transitions_need_same_weights_and_write_weight() {
    let weights = btreemap! {1 => 2, 2 => 1, 3 => 1};
    let write_two = weighted(weights.clone(), 2);
    let write_three = weighted(weights, 3);

    let same = write_two.supports_direct_transition_to(&write_two);
    assert!(same);

    // Raising the write weight from 2 to 3 lowers the read weight from 3 to 2: the old write
    // quorum {1} misses the new read quorum {2,3}. Lowering it misses in the other direction.
    let raise = write_two.supports_direct_transition_to(&write_three);
    assert!(!raise);
    let lower = write_three.supports_direct_transition_to(&write_two);
    assert!(!lower);

    // New weights change both rules, so this change needs a joint even though every pair of old
    // and new quorums intersects.
    let heavier = weighted(btreemap! {1 => 3, 2 => 1, 3 => 1}, 2);
    let reads_meet_writes = verify_intersection(&write_two.read_quorum_set(), &heavier.write_quorum_set());
    assert!(reads_meet_writes);
    let writes_meet_reads = verify_intersection(&write_two.write_quorum_set(), &heavier.read_quorum_set());
    assert!(writes_meet_reads);
    let reweighted = write_two.supports_direct_transition_to(&heavier);
    assert!(!reweighted);

    // A joint goes directly to its target, and adding a learner changes no quorum.
    let joint = write_two.joint_with(&heavier);
    let to_target = joint.supports_direct_transition_to(&heavier);
    assert!(to_target);
    let add_learner = <WeightedChange<u64, ()> as RaftMembershipEdit<Membership>>::add_learner(9, ());
    let with_learner = add_learner.apply(&write_two, true).unwrap();
    let to_learner = write_two.supports_direct_transition_to(&with_learner);
    assert!(to_learner);
}

#[test]
fn test_small_memberships_go_directly_only_with_same_weights_and_write_weight() {
    // Every membership that gives each voter of a subset of {1,2,3} the weight 1 or 2, with each
    // write weight up to the total weight.
    let mut memberships = vec![];
    for voters in all_subsets(&btreeset! {1, 2, 3}) {
        let voters: Vec<u64> = voters.into_iter().collect();
        for heavy in 0..1u64 << voters.len() {
            let mut weights = BTreeMap::new();
            for (i, voter) in voters.iter().enumerate() {
                let is_heavy = heavy & (1u64 << i) != 0;
                let weight = if is_heavy { 2 } else { 1 };
                weights.insert(*voter, weight);
            }
            let total_weight: u64 = weights.values().sum();
            for write_weight in 1..=total_weight {
                let membership = weighted(weights.clone(), write_weight);
                assert_quorum_sets_are_valid(&membership);
                memberships.push(membership);
            }
        }
    }

    for old in &memberships {
        for new in &memberships {
            assert_claims_hold(old, new);

            // Both rules of a config derive from its weights and its write weight, so two configs
            // share both rules only with the same weights and the same write weight.
            let old_rules = old.write_quorum_set();
            let old_write = &old_rules.rules()[0];
            let new_rules = new.write_quorum_set();
            let new_write = &new_rules.rules()[0];
            let same_weights = old_write.weights() == new_write.weights();
            let same_write_weight = old_write.threshold() == new_write.threshold();
            let want = same_weights && same_write_weight;
            let direct = old.supports_direct_transition_to(new);
            assert_eq!(want, direct, "{old} to {new}");
        }
    }
}

#[test]
fn test_remove_heavy_voter_through_joint() {
    // Voter 1 weighs as much as the other three together; the total weight is 6.
    let old = weighted(btreemap! {1 => 3, 2 => 1, 3 => 1, 4 => 1}, 4);

    // The voters left weigh 3, below the write weight 4, so the change must lower it.
    let remove_one = WeightedChange::weights(btreemap! {2 => 1, 3 => 1, 4 => 1});
    let res = remove_one.clone().apply(&old, true);
    let invalid = InvalidWeights::WriteWeightOutOfRange {
        write_weight: 4,
        total_weight: 3,
    };
    assert_eq!(Err(MembershipError::UserDefined(invalid)), res);

    let new = remove_one.with_write_weight(2).apply(&old, true).unwrap();
    let want = "{configs: [Config { read: WeightedQuorumSet { weights: {2: 1, 3: 1, 4: 1}, threshold: 2 }, \
                write: WeightedQuorumSet { weights: {2: 1, 3: 1, 4: 1}, threshold: 2 }, \
                nodes: {1: ((), NodeRole(ReceiveLogs)), 2: ((), NodeRole(Campaigner | ReceiveLogs)), \
                3: ((), NodeRole(Campaigner | ReceiveLogs)), 4: ((), NodeRole(Campaigner | ReceiveLogs))} }]}";
    assert_eq!(want, new.to_string());

    // The old write quorum {1,2} misses the new read quorum {3,4}.
    let direct = old.supports_direct_transition_to(&new);
    assert!(!direct);

    // A quorum of the joint is a quorum of both configs, so every joint write quorum still
    // holds voter 1.
    let joint = old.joint_with(&new);
    let subsets = all_subsets(&btreeset! {1, 2, 3, 4});
    let read_accepted = accepted(&joint.read_quorum_set(), &subsets);
    let want = vec![
        btreeset! {1, 2, 3},
        btreeset! {1, 2, 4},
        btreeset! {1, 3, 4},
        btreeset! {2, 3, 4},
        btreeset! {1, 2, 3, 4},
    ];
    assert_eq!(want, read_accepted);
    let write_accepted = accepted(&joint.write_quorum_set(), &subsets);
    let want = vec![
        btreeset! {1, 2, 3},
        btreeset! {1, 2, 4},
        btreeset! {1, 3, 4},
        btreeset! {1, 2, 3, 4},
    ];
    assert_eq!(want, write_accepted);

    let want = "{configs: [Config { read: WeightedQuorumSet { weights: {1: 3, 2: 1, 3: 1, 4: 1}, threshold: 3 }, \
                write: WeightedQuorumSet { weights: {1: 3, 2: 1, 3: 1, 4: 1}, threshold: 4 }, \
                nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), 2: ((), NodeRole(Campaigner | ReceiveLogs)), \
                3: ((), NodeRole(Campaigner | ReceiveLogs)), 4: ((), NodeRole(Campaigner | ReceiveLogs))} }, Config { \
                read: WeightedQuorumSet { weights: {2: 1, 3: 1, 4: 1}, threshold: 2 }, write: WeightedQuorumSet { \
                weights: {2: 1, 3: 1, 4: 1}, threshold: 2 }, nodes: {1: ((), NodeRole(ReceiveLogs)), \
                2: ((), NodeRole(Campaigner | ReceiveLogs)), 3: ((), NodeRole(Campaigner | ReceiveLogs)), \
                4: ((), NodeRole(Campaigner | ReceiveLogs))} }]}";
    assert_eq!(want, joint.to_string());

    // Leaving the joint selects the target, where node 1 stays as a learner.
    let change = <WeightedChange<u64, ()> as RaftMembershipEdit<Membership>>::make_uniform();
    let target = change.apply(&joint, true).unwrap();
    assert_eq!(new, target);
    let direct = joint.supports_direct_transition_to(&target);
    assert!(direct);
}

#[test]
fn test_apply() {
    let nodes = btreemap! {1 => "a".to_string(), 2 => "b".to_string(), 3 => "c".to_string()};
    let weights = btreemap! {1 => 1, 2 => 1, 3 => 1};
    let heavier = btreemap! {1 => 3, 2 => 1, 3 => 1};
    let membership = WeightedMembership::new(weights.clone(), 2, nodes.clone()).unwrap();

    // A change of the weights keeps the write weight and derives the read weight from the new
    // total weight.
    let change = WeightedChange::weights(heavier.clone());
    let res = change.apply(&membership, true);
    let want = WeightedMembership::new(heavier.clone(), 2, nodes.clone());
    assert_eq!(want, res);

    // A change of the write weight keeps the weights.
    let change = WeightedChange::write_weight(3);
    let res = change.apply(&membership, true);
    let want = WeightedMembership::new(weights.clone(), 3, nodes.clone());
    assert_eq!(want, res);

    let change = WeightedChange::weights(heavier.clone()).with_write_weight(4);
    let res = change.apply(&membership, true);
    let want = WeightedMembership::new(heavier, 4, nodes.clone());
    assert_eq!(want, res);

    // A new voter needs its node, which the same change can add. A known node keeps its
    // metadata.
    let four_voters = btreemap! {1 => 1, 2 => 1, 3 => 1, 4 => 1};
    let change = WeightedChange::weights(four_voters.clone());
    let res = change.apply(&membership, true);
    let want = Err(MembershipError::from(NodeNotFound::new(4, Operation::None)));
    assert_eq!(want, res);

    let change = WeightedChange {
        add_nodes: btreemap! {1 => "z".to_string(), 4 => "d".to_string()},
        ..WeightedChange::weights(four_voters.clone())
    };
    let res = change.apply(&membership, true);
    let mut want_nodes = nodes.clone();
    want_nodes.insert(4, "d".to_string());
    let want = WeightedMembership::new(four_voters, 2, want_nodes.clone());
    assert_eq!(want, res);

    // Removing a learner drops its node. Removing a voter fails, because every voter needs a
    // node.
    let with_four = WeightedMembership::new(weights.clone(), 2, want_nodes).unwrap();
    let change = WeightedChange {
        remove_nodes: btreeset! {4},
        ..WeightedChange::default()
    };
    let res = change.apply(&with_four, true);
    assert_eq!(Ok(membership.clone()), res);

    let change = WeightedChange {
        remove_nodes: btreeset! {3},
        ..WeightedChange::default()
    };
    let res = change.apply(&membership, true);
    let want = Err(MembershipError::from(NodeNotFound::new(3, Operation::None)));
    assert_eq!(want, res);

    // Invalid weights are the edit's own errors.
    let cases = [
        (
            WeightedChange::weights(btreemap! {1 => 1, 2 => 0}),
            InvalidWeights::ZeroWeight { node_id: 2 },
        ),
        (
            WeightedChange::weights(btreemap! {1 => u64::MAX, 2 => 1}),
            InvalidWeights::TotalWeightOverflow,
        ),
        (WeightedChange::write_weight(4), InvalidWeights::WriteWeightOutOfRange {
            write_weight: 4,
            total_weight: 3,
        }),
        (WeightedChange::write_weight(0), InvalidWeights::WriteWeightOutOfRange {
            write_weight: 0,
            total_weight: 3,
        }),
    ];
    for (change, invalid) in cases {
        let res = change.apply(&membership, true);
        assert_eq!(Err(MembershipError::UserDefined(invalid)), res);
    }

    // `add_learner()` never replaces the node of a known ID.
    type Change = WeightedChange<u64, String>;
    type Target = WeightedMembership<u64, String>;
    let change = <Change as RaftMembershipEdit<Target>>::add_learner(2, "z".to_string());
    let res = change.apply(&membership, true);
    assert_eq!(Ok(membership.clone()), res);

    // A membership with no config has no target to change.
    let empty = Target::default();
    let change = WeightedChange::write_weight(1);
    let res = change.apply(&empty, true);
    let want = Err(MembershipError::EmptyMembership(EmptyMembership {}));
    assert_eq!(want, res);
}

#[test]
fn test_joint_keeps_old_roles_and_target_restores_new_roles() {
    let old = Membership::new(btreemap! {1 => 2, 2 => 1}, 2, btreeset! {1, 2, 3}).unwrap();

    // The new config stops voter 2 from campaigning.
    let new = old.clone().with_roles(btreemap! {2 => NodeRole::ReceiveLogs}).unwrap();
    let target =
        assert_joint_keeps_old_roles_and_target_restores_new_roles::<_, WeightedChange<u64, ()>>(&old, &new, 2);

    // A change of the write weight keeps the roles of the last config.
    let change = WeightedChange::write_weight(3);
    let changed = change.apply(&target, true).unwrap();
    let want = "{configs: [Config { read: WeightedQuorumSet { weights: {1: 2, 2: 1}, threshold: 1 }, \
                write: WeightedQuorumSet { weights: {1: 2, 2: 1}, threshold: 3 }, \
                nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), 2: ((), NodeRole(ReceiveLogs)), \
                3: ((), NodeRole(ReceiveLogs))} }]}";
    assert_eq!(want, changed.to_string());
}

#[test]
fn test_with_roles() {
    let want = "{configs: [Config { read: WeightedQuorumSet { weights: {1: 1, 2: 1, 3: 1}, threshold: 2 }, \
                write: WeightedQuorumSet { weights: {1: 1, 2: 1, 3: 1}, threshold: 2 }, \
                nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), 2: ((), NodeRole(Campaigner | ReceiveLogs)), \
                3: ((), NodeRole(ReceiveLogs)), 4: ((), NodeRole(Campaigner | ReceiveLogs)), \
                5: ((), NodeRole(Empty))} }]}";
    assert_with_roles(unit_weights, Membership::with_roles, want);
}

#[test]
fn test_role_and_quorum_change_through_joint() {
    let want = "{configs: [Config { read: WeightedQuorumSet { weights: {2: 1, 3: 1, 4: 1}, threshold: 2 }, \
                write: WeightedQuorumSet { weights: {2: 1, 3: 1, 4: 1}, threshold: 2 }, \
                nodes: {1: ((), NodeRole(ReceiveLogs)), 2: ((), NodeRole(Campaigner | ReceiveLogs)), \
                3: ((), NodeRole(ReceiveLogs)), 4: ((), NodeRole(Campaigner | ReceiveLogs)), \
                5: ((), NodeRole(Empty))} }]}";
    assert_role_and_quorum_change_through_joint(unit_weights, Membership::with_roles, want, |voters, roles| {
        let pairs = voters.into_iter().map(|voter| (voter, 1));
        let weights = pairs.collect();
        WeightedChange::weights(weights).with_roles(roles)
    });
}

#[test]
fn test_change_through_joint_keeps_node_metadata() {
    assert_change_through_joint_keeps_node_metadata(unit_weights, |voters, remove_nodes| {
        let pairs = voters.into_iter().map(|voter| (voter, 1));
        let weights = pairs.collect();
        WeightedChange {
            remove_nodes,
            ..WeightedChange::weights(weights)
        }
    });
}

#[test]
fn test_apply_checks_roles() {
    assert_apply_checks_roles(
        unit_weights,
        Membership::with_roles,
        |add_nodes, remove_nodes, roles| WeightedChange {
            add_nodes,
            remove_nodes,
            roles,
            ..WeightedChange::default()
        },
    );
}

#[cfg(feature = "serde")]
#[test]
fn test_serde() {
    let membership = Membership::new(btreemap! {1 => 2, 2 => 1, 3 => 1}, 2, btreeset! {1, 2, 3, 4}).unwrap();

    let json = serde_json::to_string(&membership).unwrap();
    let want = r#"{"configs":[{"read":{"weights":{"1":2,"2":1,"3":1},"threshold":3},"write":{"weights":{"1":2,"2":1,"3":1},"threshold":2},"nodes":{"1":[null,3],"2":[null,3],"3":[null,3],"4":[null,2]}}]}"#;
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
    let old = weighted(btreemap! {1 => 2, 2 => 1, 3 => 1}, 2);
    let roles = btreemap! {3 => NodeRole::ReceiveLogs};
    let heavier = weighted(btreemap! {1 => 3, 2 => 1, 3 => 1}, 4);
    let new = heavier.with_roles(roles).unwrap();
    let joint = old.joint_with(&new);

    let binary = bincode::serialize(&joint).unwrap();
    let decoded = bincode::deserialize::<Membership>(&binary).unwrap();
    assert_eq!(joint, decoded);

    let json = serde_json::to_string(&joint).unwrap();
    let want = r#"{"configs":[{"read":{"weights":{"1":2,"2":1,"3":1},"threshold":3},"write":{"weights":{"1":2,"2":1,"3":1},"threshold":2},"nodes":{"1":[null,3],"2":[null,3],"3":[null,3]}},{"read":{"weights":{"1":3,"2":1,"3":1},"threshold":2},"write":{"weights":{"1":3,"2":1,"3":1},"threshold":4},"nodes":{"1":[null,3],"2":[null,3],"3":[null,2]}}]}"#;
    assert_eq!(want, json);
    let decoded = serde_json::from_str::<Membership>(&json).unwrap();
    assert_eq!(joint, decoded);
}
