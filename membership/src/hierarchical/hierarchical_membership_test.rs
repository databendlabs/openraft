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

use crate::HierarchicalChange;
use crate::HierarchicalMembership;
use crate::errors::InvalidGroups;
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
use crate::testing::direct_transition_is_safe;
use crate::testing::voter_role;

type Membership = HierarchicalMembership<u64, ()>;
type Change = HierarchicalChange<u64, ()>;

/// Returns the groups `groups`, keyed by group ID.
fn groups(groups: &[(&str, &[u64])]) -> BTreeMap<String, BTreeSet<u64>> {
    let mut map = BTreeMap::new();
    for (group, voters) in groups {
        let voters: BTreeSet<u64> = voters.iter().copied().collect();
        map.insert(group.to_string(), voters);
    }
    map
}

/// Returns the sizes `sizes`, keyed by group ID.
fn sizes(sizes: &[(&str, u64)]) -> BTreeMap<String, u64> {
    let mut map = BTreeMap::new();
    for (group, size) in sizes {
        map.insert(group.to_string(), *size);
    }
    map
}

/// Returns the groups `a={1,2,3}`, `b={4,5,6}` and `c={7,8,9}`.
fn three_by_three() -> BTreeMap<String, BTreeSet<u64>> {
    groups(&[("a", &[1, 2, 3]), ("b", &[4, 5, 6]), ("c", &[7, 8, 9])])
}

/// Returns a membership whose only config places its voters in `groups`, with a `()` node for
/// each voter.
fn hierarchical(
    groups: BTreeMap<String, BTreeSet<u64>>,
    write_groups: u64,
    write_sizes: BTreeMap<String, u64>,
) -> Membership {
    let mut nodes = BTreeSet::new();
    for voters in groups.values() {
        nodes.extend(voters);
    }
    HierarchicalMembership::new(groups, write_groups, write_sizes, nodes).unwrap()
}

/// Returns a membership whose only config places `voters` in group a, where a write needs 2 of
/// them, with each node of `nodes`.
fn one_group<N, T>(voters: BTreeSet<u64>, nodes: T) -> HierarchicalMembership<u64, N>
where
    N: Node,
    T: IntoNodes<u64, N>,
{
    let groups = btreemap! {"a".to_string() => voters};
    HierarchicalMembership::new(groups, 1, sizes(&[("a", 2)]), nodes).unwrap()
}

/// Returns the voters of [`three_by_three()`], where reads and writes each need 2 voters in each
/// of 2 groups.
fn nine_voters() -> Membership {
    hierarchical(three_by_three(), 2, sizes(&[("a", 2), ("b", 2), ("c", 2)]))
}

/// Returns `old` with voter 3 moved from group a to group b. In the result, voter 3 receives logs
/// but never starts an election.
fn move_voter_3_to_b(old: &Membership) -> Membership {
    let moved = groups(&[("a", &[1, 2]), ("b", &[3, 4, 5, 6]), ("c", &[7, 8, 9])]);
    let change = Change::groups(moved);
    let moved = change.apply(old).unwrap();

    let roles = btreemap! {3 => NodeRole::ReceiveLogs};
    moved.with_roles(roles).unwrap()
}

/// Returns the sets in `subsets` that hold `sizes[group]` voters of each of at least
/// `quorum_groups` groups of `groups`, in their order.
fn reaching(
    groups: &BTreeMap<String, BTreeSet<u64>>,
    quorum_groups: u64,
    sizes: &BTreeMap<String, u64>,
    subsets: &[BTreeSet<u64>],
) -> Vec<BTreeSet<u64>> {
    let mut reaching = vec![];
    for subset in subsets {
        let mut reached = 0;
        for (group, voters) in groups {
            let held = voters.intersection(subset).count() as u64;
            if held >= sizes[group] {
                reached += 1;
            }
        }
        if reached >= quorum_groups {
            reaching.push(subset.clone());
        }
    }
    reaching
}

#[test]
fn test_new() {
    let two_groups = groups(&[("a", &[1, 2]), ("b", &[3, 4])]);
    let res = Membership::new(
        two_groups.clone(),
        2,
        sizes(&[("a", 1), ("b", 2)]),
        btreeset! {1, 2, 3, 4, 5},
    );
    let membership = res.unwrap();
    let want = "{configs: [Config { read: HierarchicalQuorumSet { groups: {\"a\": Group { voters: {1, 2}, \
                quorum_size: 2 }, \"b\": Group { voters: {3, 4}, quorum_size: 1 }}, quorum_groups: 1 }, \
                write: HierarchicalQuorumSet { groups: {\"a\": Group { voters: {1, 2}, quorum_size: 1 }, \
                \"b\": Group { voters: {3, 4}, quorum_size: 2 }}, quorum_groups: 2 }, \
                nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), 2: ((), NodeRole(Campaigner | ReceiveLogs)), \
                3: ((), NodeRole(Campaigner | ReceiveLogs)), 4: ((), NodeRole(Campaigner | ReceiveLogs)), \
                5: ((), NodeRole(ReceiveLogs))} }]}";
    let display = membership.to_string();
    assert_eq!(want, display);

    // The groups and the write thresholds get the checks of the write rule.
    let shared_voter = groups(&[("a", &[1, 2]), ("b", &[2])]);
    let res = Membership::new(shared_voter, 1, sizes(&[("a", 1), ("b", 1)]), btreeset! {1, 2});
    let shared = InvalidGroups::SharedVoter {
        node_id: 2,
        first: "a".to_string(),
        second: "b".to_string(),
    };
    assert_eq!(Err(MembershipError::UserDefined(shared)), res);

    let res = Membership::new(
        two_groups.clone(),
        3,
        sizes(&[("a", 1), ("b", 2)]),
        btreeset! {1, 2, 3, 4},
    );
    let invalid = InvalidGroups::InvalidQuorumGroups {
        quorum_groups: 3,
        group_count: 2,
    };
    assert_eq!(Err(MembershipError::UserDefined(invalid)), res);

    let res = Membership::new(
        two_groups.clone(),
        2,
        sizes(&[("a", 1), ("b", 3)]),
        btreeset! {1, 2, 3, 4},
    );
    let invalid = InvalidGroups::InvalidQuorumSize {
        group: "b".to_string(),
        quorum_size: 3,
        voter_count: 2,
    };
    assert_eq!(Err(MembershipError::UserDefined(invalid)), res);

    // The write sizes name exactly the groups.
    let res = Membership::new(
        two_groups.clone(),
        2,
        sizes(&[("a", 1), ("b", 2), ("c", 1)]),
        btreeset! {1, 2, 3, 4},
    );
    let unknown = InvalidGroups::UnknownGroup { group: "c".to_string() };
    assert_eq!(Err(MembershipError::UserDefined(unknown)), res);

    let res = Membership::new(two_groups.clone(), 2, sizes(&[("a", 1)]), btreeset! {1, 2, 3, 4});
    let missing = InvalidGroups::MissingQuorumSize { group: "b".to_string() };
    assert_eq!(Err(MembershipError::UserDefined(missing)), res);

    // Voter 4 has no node.
    let res = Membership::new(two_groups, 2, sizes(&[("a", 1), ("b", 2)]), btreeset! {1, 2, 3});
    let want = Err(MembershipError::NodeNotFound(NodeNotFound::new(4, Operation::None)));
    assert_eq!(want, res);
}

#[test]
fn test_nine_voters_need_two_voters_in_each_of_two_groups() {
    let membership = nine_voters();
    let subsets = all_subsets(&btreeset! {1, 2, 3, 4, 5, 6, 7, 8, 9});

    // 4 of the 8 subsets of a group hold 2 of its voters, so 3 * 4^3 sets reach exactly 2 groups
    // and 4^3 sets reach all 3.
    let want = reaching(&three_by_three(), 2, &sizes(&[("a", 2), ("b", 2), ("c", 2)]), &subsets);
    let want_count = want.len();
    assert_eq!(256, want_count);

    let read_accepted = accepted(&membership.read_quorum_set(), &subsets);
    assert_eq!(want, read_accepted);
    let write_accepted = accepted(&membership.write_quorum_set(), &subsets);
    assert_eq!(want, write_accepted);

    let want = "{configs: [Config { read: HierarchicalQuorumSet { groups: {\"a\": Group { voters: {1, 2, 3}, \
                quorum_size: 2 }, \"b\": Group { voters: {4, 5, 6}, quorum_size: 2 }, \"c\": Group { \
                voters: {7, 8, 9}, quorum_size: 2 }}, quorum_groups: 2 }, write: HierarchicalQuorumSet { \
                groups: {\"a\": Group { voters: {1, 2, 3}, quorum_size: 2 }, \"b\": Group { voters: {4, 5, 6}, \
                quorum_size: 2 }, \"c\": Group { voters: {7, 8, 9}, quorum_size: 2 }}, quorum_groups: 2 }, \
                nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), 2: ((), NodeRole(Campaigner | ReceiveLogs)), \
                3: ((), NodeRole(Campaigner | ReceiveLogs)), 4: ((), NodeRole(Campaigner | ReceiveLogs)), \
                5: ((), NodeRole(Campaigner | ReceiveLogs)), 6: ((), NodeRole(Campaigner | ReceiveLogs)), \
                7: ((), NodeRole(Campaigner | ReceiveLogs)), 8: ((), NodeRole(Campaigner | ReceiveLogs)), \
                9: ((), NodeRole(Campaigner | ReceiveLogs))} }]}";
    let display = membership.to_string();
    assert_eq!(want, display);
}

#[test]
fn test_nine_voters_survive_a_lost_group_or_a_lost_voter_per_group() {
    let membership = nine_voters();
    let read = membership.read_quorum_set();
    let write = membership.write_quorum_set();

    let cases = [
        // Group c is lost: groups a and b keep all their voters.
        (btreeset! {1, 2, 3, 4, 5, 6}, true),
        // Group c and voter 1 of group a are lost.
        (btreeset! {2, 3, 4, 5, 6}, true),
        // One voter of each group is lost.
        (btreeset! {2, 3, 5, 6, 8, 9}, true),
        // Group c and two voters of group a are lost: only group b keeps at least 2 voters.
        (btreeset! {3, 4, 5, 6}, false),
        // Two voters of group a and two of group b are lost: only group c keeps at least 2 voters.
        (btreeset! {1, 4, 7, 8, 9}, false),
    ];
    for (live, want) in cases {
        let reads = read.is_quorum(live.iter());
        assert_eq!(want, reads, "live voters {live:?}");
        let writes = write.is_quorum(live.iter());
        assert_eq!(want, writes, "live voters {live:?}");
    }
}

#[test]
fn test_moving_a_voter_between_groups_goes_through_a_joint() {
    let old = nine_voters();
    let subsets = all_subsets(&btreeset! {1, 2, 3, 4, 5, 6, 7, 8, 9});

    // Voter 3 moves from group a to group b, and every group keeps its write size 2.
    let moved = groups(&[("a", &[1, 2]), ("b", &[3, 4, 5, 6]), ("c", &[7, 8, 9])]);
    let change = Change::groups(moved.clone());
    let new = change.apply(&old).unwrap();

    // A read quorum reaches 2 groups, with 1 voter of a, 3 of b or 2 of c.
    let new_reads = reaching(&moved, 2, &sizes(&[("a", 1), ("b", 3), ("c", 2)]), &subsets);
    let read_accepted = accepted(&new.read_quorum_set(), &subsets);
    assert_eq!(new_reads, read_accepted);
    let new_writes = reaching(&moved, 2, &sizes(&[("a", 2), ("b", 2), ("c", 2)]), &subsets);
    let write_accepted = accepted(&new.write_quorum_set(), &subsets);
    assert_eq!(new_writes, write_accepted);

    // The groups changed, so the configs share no rules and OpenRaft appends a joint first.
    let direct = old.supports_direct_transition_to(&new);
    assert!(!direct);

    // A joint quorum is a quorum of both configs.
    let joint = old.joint_with(&new);
    let old_quorums = reaching(&three_by_three(), 2, &sizes(&[("a", 2), ("b", 2), ("c", 2)]), &subsets);

    let mut both_reads = old_quorums.clone();
    both_reads.retain(|subset| new_reads.contains(subset));
    let joint_reads = accepted(&joint.read_quorum_set(), &subsets);
    assert_eq!(both_reads, joint_reads);

    let mut both_writes = old_quorums;
    both_writes.retain(|subset| new_writes.contains(subset));
    let joint_writes = accepted(&joint.write_quorum_set(), &subsets);
    assert_eq!(both_writes, joint_writes);

    // The joint keeps the rules of both configs, and `make_uniform()` selects the new groups.
    let old_rules = old.write_quorum_set();
    let new_rules = new.write_quorum_set();
    let want = vec![old_rules.rules()[0].clone(), new_rules.rules()[0].clone()];
    let joint_rules = joint.write_quorum_set();
    assert_eq!(want, joint_rules.rules());

    let change = <Change as RaftMembershipEdit<Membership>>::make_uniform();
    let target = change.apply(&joint).unwrap();
    assert_eq!(new, target);
    let direct = joint.supports_direct_transition_to(&target);
    assert!(direct);
}

#[test]
fn test_get_node() {
    let two_groups = groups(&[("a", &[1, 2]), ("b", &[3])]);
    let membership = Membership::new(two_groups, 1, sizes(&[("a", 2), ("b", 1)]), btreeset! {1, 2, 3, 4}).unwrap();

    for node_id in [1, 2, 3] {
        let got = membership.get_node(&node_id);
        let want = Some(NodeRef {
            node: &(),
            role: voter_role(),
            is_read_voter: true,
            is_write_voter: true,
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
    let read = membership.read_quorum_set();
    let read_ids: Vec<u64> = read.ids().collect();
    assert_eq!(vec![1, 2, 3], read_ids);
    let write = membership.write_quorum_set();
    let write_ids: Vec<u64> = write.ids().collect();
    assert_eq!(vec![1, 2, 3], write_ids);
}

#[test]
fn test_from_node_map_puts_every_node_in_one_group() {
    let nodes: BTreeMap<u64, ()> = btreemap! {1 => (), 2 => (), 3 => (), 4 => ()};
    let membership = Membership::from(nodes);

    // 4 voters: writes need 3 of them, and reads need 2.
    let want = hierarchical(groups(&[("default", &[1, 2, 3, 4])]), 1, sizes(&[("default", 3)]));
    assert_eq!(want, membership);
    let want = "{configs: [Config { read: HierarchicalQuorumSet { groups: {\"default\": Group { voters: {1, 2, 3, 4}, \
                quorum_size: 2 }}, quorum_groups: 1 }, write: HierarchicalQuorumSet { groups: {\"default\": Group { \
                voters: {1, 2, 3, 4}, quorum_size: 3 }}, quorum_groups: 1 }, \
                nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), 2: ((), NodeRole(Campaigner | ReceiveLogs)), \
                3: ((), NodeRole(Campaigner | ReceiveLogs)), 4: ((), NodeRole(Campaigner | ReceiveLogs))} }]}";
    let display = membership.to_string();
    assert_eq!(want, display);

    let empty = Membership::from(BTreeMap::new());
    let want = Membership::default();
    assert_eq!(want, empty);
    let display = empty.to_string();
    assert_eq!("{configs: []}", display);
}

#[test]
fn test_direct_transitions_need_intersection_in_both_directions() {
    let membership = nine_voters();

    let same = membership.supports_direct_transition_to(&membership);
    assert!(same);

    // Writes to 3 groups meet every old read, but new reads of 1 group miss old writes to the
    // other 2 groups.
    let change = Change::write_groups(3);
    let three_groups = change.apply(&membership).unwrap();
    let to_three = membership.supports_direct_transition_to(&three_groups);
    assert!(!to_three);
    let from_three = three_groups.supports_direct_transition_to(&membership);
    assert!(!from_three);

    // Writes of all 3 voters of group a meet every old read, but new reads of 1 voter of group a
    // miss old writes that hold the other 2.
    let change = Change::write_sizes(sizes(&[("a", 3)]));
    let all_of_a = change.apply(&membership).unwrap();
    let to_all_of_a = membership.supports_direct_transition_to(&all_of_a);
    assert!(!to_all_of_a);

    // A joint goes directly to its target, and adding a learner changes no quorum.
    let joint = membership.joint_with(&three_groups);
    let to_target = joint.supports_direct_transition_to(&three_groups);
    assert!(to_target);

    let add_learner = <Change as RaftMembershipEdit<Membership>>::add_learner(10, ());
    let with_learner = add_learner.apply(&membership).unwrap();
    let to_learner = membership.supports_direct_transition_to(&with_learner);
    assert!(to_learner);
}

#[test]
fn test_direct_transitions_are_exact_within_one_layout() {
    // Every layout of voters {1,2,3,4} in one group, or in two groups where group a holds voter 1.
    let voters = btreeset! {1, 2, 3, 4};
    let mut layouts = vec![groups(&[("a", &[1, 2, 3, 4])])];
    for others in all_subsets(&btreeset! {2, 3, 4}) {
        let mut a = others;
        a.insert(1);
        let b: BTreeSet<u64> = voters.difference(&a).copied().collect();
        if b.is_empty() {
            continue;
        }
        layouts.push(btreemap! {"a".to_string() => a, "b".to_string() => b});
    }

    // Every membership of these layouts: each number of groups that a write reaches, with each
    // write size of each group.
    let mut memberships = vec![];
    for layout in &layouts {
        let mut all_sizes = vec![BTreeMap::new()];
        for (group, group_voters) in layout {
            let mut extended = vec![];
            for sizes in &all_sizes {
                for size in 1..=group_voters.len() as u64 {
                    let mut sizes = sizes.clone();
                    sizes.insert(group.clone(), size);
                    extended.push(sizes);
                }
            }
            all_sizes = extended;
        }

        let group_count = layout.len() as u64;
        for write_groups in 1..=group_count {
            for write_sizes in &all_sizes {
                let membership = hierarchical(layout.clone(), write_groups, write_sizes.clone());
                assert_quorum_sets_are_valid(&membership);
                memberships.push((layout, membership));
            }
        }
    }

    for (old_layout, old) in &memberships {
        for (new_layout, new) in &memberships {
            assert_claims_hold(old, new);

            // Each read rule is the dual of its write rule, so within one layout a direct
            // transition is safe only between the same rules, which the check requires. Across
            // layouts the rules always differ.
            let same_layout = old_layout == new_layout;
            let safe = direct_transition_is_safe(old, new);
            let want = same_layout && safe;
            let direct = old.supports_direct_transition_to(new);
            assert_eq!(want, direct, "{old} to {new}");
        }
    }
}

#[test]
fn test_apply_keeps_write_sizes_of_groups_that_keep_their_ids() {
    let two_groups = groups(&[("a", &[1, 2]), ("b", &[3, 4])]);
    let membership = hierarchical(two_groups.clone(), 2, sizes(&[("a", 1), ("b", 2)]));

    // A change of one threshold keeps the others.
    let change = Change::write_groups(1);
    let res = change.apply(&membership);
    let want = hierarchical(two_groups.clone(), 1, sizes(&[("a", 1), ("b", 2)]));
    assert_eq!(Ok(want), res);

    let change = Change::write_sizes(sizes(&[("b", 1)]));
    let res = change.apply(&membership);
    let want = hierarchical(two_groups, 2, sizes(&[("a", 1), ("b", 1)]));
    assert_eq!(Ok(want), res);

    // Voter 2 moves to group b, and both groups keep their write sizes.
    let moved = groups(&[("a", &[1]), ("b", &[2, 3, 4])]);
    let change = Change::groups(moved.clone());
    let res = change.apply(&membership);
    let want = hierarchical(moved, 2, sizes(&[("a", 1), ("b", 2)]));
    assert_eq!(Ok(want), res);

    // Voter 3 moves to group a, which leaves group b fewer voters than its write size.
    let shrunk = groups(&[("a", &[1, 2, 3]), ("b", &[4])]);
    let change = Change::groups(shrunk.clone());
    let res = change.apply(&membership);
    let invalid = InvalidGroups::InvalidQuorumSize {
        group: "b".to_string(),
        quorum_size: 2,
        voter_count: 1,
    };
    assert_eq!(Err(MembershipError::UserDefined(invalid)), res);

    let change = Change::groups(shrunk.clone()).with_write_sizes(sizes(&[("b", 1)]));
    let res = change.apply(&membership);
    let want = hierarchical(shrunk, 2, sizes(&[("a", 1), ("b", 1)]));
    assert_eq!(Ok(want), res);

    // A group with a new ID needs a write size.
    let with_c = groups(&[("a", &[1]), ("b", &[3, 4]), ("c", &[2])]);
    let change = Change::groups(with_c.clone());
    let res = change.apply(&membership);
    let missing = InvalidGroups::MissingQuorumSize { group: "c".to_string() };
    assert_eq!(Err(MembershipError::UserDefined(missing)), res);

    let change = Change::groups(with_c.clone()).with_write_sizes(sizes(&[("c", 1)]));
    let res = change.apply(&membership);
    let want = hierarchical(with_c, 2, sizes(&[("a", 1), ("b", 2), ("c", 1)]));
    assert_eq!(Ok(want), res);

    // A write size for a group that the result lacks is an error.
    let change = Change::write_sizes(sizes(&[("d", 1)]));
    let res = change.apply(&membership);
    let unknown = InvalidGroups::UnknownGroup { group: "d".to_string() };
    assert_eq!(Err(MembershipError::UserDefined(unknown)), res);

    // Removing group b needs a smaller `write_groups`, and voters 3 and 4 stay as learners.
    let only_a = groups(&[("a", &[1, 2])]);
    let change = Change::groups(only_a.clone());
    let res = change.apply(&membership);
    let invalid = InvalidGroups::InvalidQuorumGroups {
        quorum_groups: 2,
        group_count: 1,
    };
    assert_eq!(Err(MembershipError::UserDefined(invalid)), res);

    let change = Change::groups(only_a.clone()).with_write_groups(1);
    let res = change.apply(&membership);
    let want = Membership::new(only_a, 1, sizes(&[("a", 1)]), btreeset! {1, 2, 3, 4});
    assert_eq!(want, res);
}

#[test]
fn test_apply_edits_nodes() {
    let nodes = btreemap! {1 => "n1".to_string(), 2 => "n2".to_string(), 3 => "n3".to_string()};
    let one_group = groups(&[("a", &[1, 2, 3])]);
    let membership = HierarchicalMembership::new(one_group.clone(), 1, sizes(&[("a", 2)]), nodes.clone()).unwrap();

    // Adding nodes keeps the metadata of a known node; removing a learner drops it.
    let change = HierarchicalChange {
        add_nodes: btreemap! {1 => "z".to_string(), 4 => "n4".to_string()},
        ..HierarchicalChange::default()
    };
    let with_four = change.apply(&membership).unwrap();
    let mut want_nodes = nodes;
    want_nodes.insert(4, "n4".to_string());
    let want = HierarchicalMembership::new(one_group, 1, sizes(&[("a", 2)]), want_nodes).unwrap();
    assert_eq!(want, with_four);

    let change = HierarchicalChange {
        remove_nodes: btreeset! {4},
        ..HierarchicalChange::default()
    };
    let res = change.apply(&with_four);
    let want = membership.clone();
    assert_eq!(Ok(want), res);

    // Removing a voter fails, because every voter needs a node.
    let change = HierarchicalChange {
        remove_nodes: btreeset! {3},
        ..HierarchicalChange::default()
    };
    let res = change.apply(&membership);
    let not_found = NodeNotFound::new(3, Operation::None);
    let want = Err(MembershipError::from(not_found));
    assert_eq!(want, res);

    // `add_learner()` never replaces the node of a known ID.
    type StringChange = HierarchicalChange<u64, String>;
    type Target = HierarchicalMembership<u64, String>;
    let change = <StringChange as RaftMembershipEdit<Target>>::add_learner(2, "z".to_string());
    let res = change.apply(&membership);
    assert_eq!(Ok(membership), res);

    // A membership with no config has no target to change.
    let empty = Target::default();
    let change = StringChange::write_groups(1);
    let res = change.apply(&empty);
    let want = Err(MembershipError::EmptyMembership(EmptyMembership {}));
    assert_eq!(want, res);
}

#[test]
fn test_make_uniform_selects_target_and_keeps_old_voters_as_learners() {
    let old = nine_voters();

    // Group c leaves, and a write quorum still needs both remaining groups.
    let two_groups = groups(&[("a", &[1, 2, 3]), ("b", &[4, 5, 6])]);
    let change = Change::groups(two_groups.clone());
    let new = change.apply(&old).unwrap();
    let joint = old.joint_with(&new);

    let change = <Change as RaftMembershipEdit<Membership>>::make_uniform();
    let target = change.apply(&joint).unwrap();

    // Voters 7, 8 and 9 of the old config stay as learners of the target.
    let nodes = btreeset! {1, 2, 3, 4, 5, 6, 7, 8, 9};
    let want = Membership::new(two_groups, 2, sizes(&[("a", 2), ("b", 2)]), nodes).unwrap();
    assert_eq!(want, target);
    let direct = joint.supports_direct_transition_to(&target);
    assert!(direct);
}

#[test]
fn test_joint_keeps_old_roles_and_target_restores_new_roles() {
    let old = nine_voters();
    let new = move_voter_3_to_b(&old);
    assert_joint_keeps_old_roles_and_target_restores_new_roles::<_, Change>(&old, &new, 3);
}

#[test]
fn test_with_roles() {
    let want = "{configs: [Config { read: HierarchicalQuorumSet { groups: {\"a\": Group { voters: {1, 2, 3}, \
                quorum_size: 2 }}, quorum_groups: 1 }, write: HierarchicalQuorumSet { groups: {\"a\": Group { \
                voters: {1, 2, 3}, quorum_size: 2 }}, quorum_groups: 1 }, \
                nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), 2: ((), NodeRole(Campaigner | ReceiveLogs)), \
                3: ((), NodeRole(ReceiveLogs)), 4: ((), NodeRole(Campaigner | ReceiveLogs)), \
                5: ((), NodeRole(Empty))} }]}";
    assert_with_roles(one_group, Membership::with_roles, want);
}

#[test]
fn test_role_and_quorum_change_through_joint() {
    let want = "{configs: [Config { read: HierarchicalQuorumSet { groups: {\"a\": Group { voters: {2, 3, 4}, \
                quorum_size: 2 }}, quorum_groups: 1 }, write: HierarchicalQuorumSet { groups: {\"a\": Group { \
                voters: {2, 3, 4}, quorum_size: 2 }}, quorum_groups: 1 }, nodes: {1: ((), NodeRole(ReceiveLogs)), \
                2: ((), NodeRole(Campaigner | ReceiveLogs)), 3: ((), NodeRole(ReceiveLogs)), \
                4: ((), NodeRole(Campaigner | ReceiveLogs)), 5: ((), NodeRole(Empty))} }]}";
    assert_role_and_quorum_change_through_joint(one_group, Membership::with_roles, want, |voters, roles| {
        let groups = btreemap! {"a".to_string() => voters};
        Change::groups(groups).with_roles(roles)
    });
}

#[test]
fn test_change_through_joint_keeps_node_metadata() {
    assert_change_through_joint_keeps_node_metadata(one_group, |voters, remove_nodes| {
        let groups = btreemap! {"a".to_string() => voters};
        HierarchicalChange {
            remove_nodes,
            ..HierarchicalChange::groups(groups)
        }
    });
}

#[test]
fn test_apply_checks_roles() {
    assert_apply_checks_roles(one_group, Membership::with_roles, |add_nodes, remove_nodes, roles| {
        Change {
            add_nodes,
            remove_nodes,
            roles,
            ..Change::default()
        }
    });
}

#[cfg(feature = "serde")]
#[test]
fn test_serde() {
    let two_groups = groups(&[("a", &[1, 2]), ("b", &[3])]);
    let membership = Membership::new(two_groups, 1, sizes(&[("a", 2), ("b", 1)]), btreeset! {1, 2, 3, 4}).unwrap();

    let json = serde_json::to_string(&membership).unwrap();
    let want = r#"{"configs":[{"read":{"groups":{"a":{"voters":[1,2],"quorum_size":1},"b":{"voters":[3],"quorum_size":1}},"quorum_groups":2},"write":{"groups":{"a":{"voters":[1,2],"quorum_size":2},"b":{"voters":[3],"quorum_size":1}},"quorum_groups":1},"nodes":{"1":[null,3],"2":[null,3],"3":[null,3],"4":[null,2]}}]}"#;
    assert_eq!(want, json);
    let decoded = serde_json::from_str::<Membership>(&json).unwrap();
    assert_eq!(membership, decoded);

    let empty = r#"{"configs":[]}"#;
    let decoded = serde_json::from_str::<Membership>(empty).unwrap();
    let want = Membership::default();
    assert_eq!(want, decoded);
}

#[cfg(feature = "serde")]
#[test]
fn test_binary_roundtrip_keeps_joint_configs_and_roles() {
    let old = nine_voters();
    let new = move_voter_3_to_b(&old);
    let joint = old.joint_with(&new);

    let binary = bincode::serialize(&joint).unwrap();
    let decoded = bincode::deserialize::<Membership>(&binary).unwrap();
    assert_eq!(joint, decoded);

    let json = serde_json::to_string(&joint).unwrap();
    let decoded = serde_json::from_str::<Membership>(&json).unwrap();
    assert_eq!(joint, decoded);
}
