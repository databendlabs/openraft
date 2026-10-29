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
use crate::quorum::Group;
use crate::quorum::HierarchicalQuorumSet;
use crate::quorum::QuorumSet;
use crate::quorum::testing::accepted;
use crate::quorum::testing::all_group_sizes;
use crate::quorum::testing::all_subsets;
use crate::quorum::testing::holding_one_of;
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

/// Returns the path of a group from the IDs `ids`.
fn path(ids: &[&str]) -> Vec<String> {
    let mut path = vec![];
    for id in ids {
        path.push(id.to_string());
    }
    path
}

/// Returns the root group over the zones `a={1,2,3}`, `b={4,5,6}` and `c={7,8,9}`, with the write
/// sizes `root`, `a`, `b` and `c`.
fn zones(root: u64, a: u64, b: u64, c: u64) -> Group<u64> {
    Group::of_groups(root, [
        ("a", Group::of_voters(a, [1, 2, 3])),
        ("b", Group::of_voters(b, [4, 5, 6])),
        ("c", Group::of_voters(c, [7, 8, 9])),
    ])
}

/// Returns a tree of three levels whose branches have different depths. Datacenter `dc1` holds the
/// racks `r1={1,2}` and `r2={3,4}`, datacenter `dc2` holds voters 5 and 6, and the root holds the
/// witness voter 7. A write reaches 2 of `dc1`, `dc2` and voter 7: `dc1` with a voter of each rack,
/// and `dc2` with both of its voters.
fn datacenters() -> Group<u64> {
    let dc1 = Group::of_groups(2, [
        ("r1", Group::of_voters(1, [1, 2])),
        ("r2", Group::of_voters(1, [3, 4])),
    ]);
    Group {
        quorum_size: 2,
        voters: btreeset! {7},
        groups: btreemap! {"dc1".to_string() => dc1, "dc2".to_string() => Group::of_voters(2, [5, 6])},
    }
}

/// Returns a membership whose only config arranges its voters in `groups`, with a `()` node for
/// each voter.
fn hierarchical(groups: Group<u64>) -> Membership {
    let rule = HierarchicalQuorumSet::new(groups.clone()).unwrap();
    let nodes: BTreeSet<u64> = rule.ids().collect();
    HierarchicalMembership::new(groups, nodes).unwrap()
}

/// Returns a membership whose only config places `voters` in group a, where a write needs 2 of
/// them, with each node of `nodes`.
fn one_group<N, T>(voters: BTreeSet<u64>, nodes: T) -> HierarchicalMembership<u64, N>
where
    N: Node,
    T: IntoNodes<u64, N>,
{
    let groups = Group::of_groups(1, [("a", Group::of_voters(2, voters))]);
    HierarchicalMembership::new(groups, nodes).unwrap()
}

/// Returns the voters of [`zones()`], where reads and writes each need 2 voters in each of 2
/// zones.
fn nine_voters() -> Membership {
    hierarchical(zones(2, 2, 2, 2))
}

/// Returns `old` with voter 3 moved from zone a to zone b. In the result, voter 3 receives logs
/// but never starts an election.
fn move_voter_3_to_b(old: &Membership) -> Membership {
    let moved = Group::of_groups(2, [
        ("a", Group::of_voters(2, [1, 2])),
        ("b", Group::of_voters(2, [3, 4, 5, 6])),
        ("c", Group::of_voters(2, [7, 8, 9])),
    ]);
    let change = Change::groups(moved);
    let moved = change.apply(old, true).unwrap();

    let roles = btreemap! {3 => NodeRole::ReceiveLogs};
    moved.with_roles(roles).unwrap()
}

/// Returns the sets in `subsets` that reach `root`, a group of groups of voters: the sets that
/// hold the quorum size of voters in each of at least `root.quorum_size` groups, in their order.
fn reaching(root: &Group<u64>, subsets: &[BTreeSet<u64>]) -> Vec<BTreeSet<u64>> {
    let mut reaching = vec![];
    for subset in subsets {
        let mut reached = 0;
        for group in root.groups.values() {
            let held = group.voters.intersection(subset).count() as u64;
            if held >= group.quorum_size {
                reached += 1;
            }
        }
        if reached >= root.quorum_size {
            reaching.push(subset.clone());
        }
    }
    reaching
}

#[test]
fn test_new() {
    let two_groups = |root, a, b| {
        Group::of_groups(root, [
            ("a", Group::of_voters(a, [1, 2])),
            ("b", Group::of_voters(b, [3, 4])),
        ])
    };
    let res = Membership::new(two_groups(2, 1, 2), btreeset! {1, 2, 3, 4, 5});
    let membership = res.unwrap();
    let want = "{configs: [Config { read: HierarchicalQuorumSet { root: Group { quorum_size: 1, voters: {}, \
                groups: {\"a\": Group { quorum_size: 2, voters: {1, 2}, groups: {} }, \"b\": Group { quorum_size: 1, \
                voters: {3, 4}, groups: {} }} } }, write: HierarchicalQuorumSet { root: Group { quorum_size: 2, \
                voters: {}, groups: {\"a\": Group { quorum_size: 1, voters: {1, 2}, groups: {} }, \"b\": Group { \
                quorum_size: 2, voters: {3, 4}, groups: {} }} } }, \
                nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), 2: ((), NodeRole(Campaigner | ReceiveLogs)), \
                3: ((), NodeRole(Campaigner | ReceiveLogs)), 4: ((), NodeRole(Campaigner | ReceiveLogs)), \
                5: ((), NodeRole(ReceiveLogs))} }]}";
    let display = membership.to_string();
    assert_eq!(want, display);

    // The groups and their write sizes get the checks of the write rule, at every level.
    let shared_voter = Group::of_groups(1, [
        ("a", Group::of_voters(1, [1, 2])),
        ("b", Group::of_groups(1, [("c", Group::of_voters(1, [2]))])),
    ]);
    let res = Membership::new(shared_voter, btreeset! {1, 2});
    let shared = InvalidGroups::SharedVoter {
        node_id: 2,
        first: path(&["a"]),
        second: path(&["b", "c"]),
    };
    assert_eq!(Err(MembershipError::UserDefined(shared)), res);

    let empty_rack = Group::of_groups(1, [("a", Group::of_groups(1, [("r1", Group::of_voters(1, []))]))]);
    let res = Membership::new(empty_rack, btreeset! {1});
    let empty = InvalidGroups::EmptyGroup {
        group: path(&["a", "r1"]),
    };
    assert_eq!(Err(MembershipError::UserDefined(empty)), res);

    let res = Membership::new(two_groups(3, 1, 2), btreeset! {1, 2, 3, 4});
    let invalid = InvalidGroups::InvalidQuorumSize {
        group: path(&[]),
        quorum_size: 3,
        member_count: 2,
    };
    assert_eq!(Err(MembershipError::UserDefined(invalid)), res);

    let res = Membership::new(two_groups(2, 1, 3), btreeset! {1, 2, 3, 4});
    let invalid = InvalidGroups::InvalidQuorumSize {
        group: path(&["b"]),
        quorum_size: 3,
        member_count: 2,
    };
    assert_eq!(Err(MembershipError::UserDefined(invalid)), res);

    // Voter 4 has no node.
    let res = Membership::new(two_groups(2, 1, 2), btreeset! {1, 2, 3});
    let want = Err(MembershipError::NodeNotFound(NodeNotFound::new(4, Operation::None)));
    assert_eq!(want, res);
}

#[test]
fn test_nine_voters_need_two_voters_in_each_of_two_groups() {
    let membership = nine_voters();
    let subsets = all_subsets(&btreeset! {1, 2, 3, 4, 5, 6, 7, 8, 9});

    // 4 of the 8 subsets of a zone hold 2 of its voters, so 3 * 4^3 sets reach exactly 2 zones
    // and 4^3 sets reach all 3.
    let want = reaching(&zones(2, 2, 2, 2), &subsets);
    let want_count = want.len();
    assert_eq!(256, want_count);

    let read_accepted = accepted(&membership.read_quorum_set(), &subsets);
    assert_eq!(want, read_accepted);
    let write_accepted = accepted(&membership.write_quorum_set(), &subsets);
    assert_eq!(want, write_accepted);

    let want = "{configs: [Config { read: HierarchicalQuorumSet { root: Group { quorum_size: 2, voters: {}, \
                groups: {\"a\": Group { quorum_size: 2, voters: {1, 2, 3}, groups: {} }, \"b\": Group { \
                quorum_size: 2, voters: {4, 5, 6}, groups: {} }, \"c\": Group { quorum_size: 2, voters: {7, 8, 9}, \
                groups: {} }} } }, write: HierarchicalQuorumSet { root: Group { quorum_size: 2, voters: {}, \
                groups: {\"a\": Group { quorum_size: 2, voters: {1, 2, 3}, groups: {} }, \"b\": Group { \
                quorum_size: 2, voters: {4, 5, 6}, groups: {} }, \"c\": Group { quorum_size: 2, voters: {7, 8, 9}, \
                groups: {} }} } }, \
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
fn test_three_levels_need_enough_members_at_every_level() {
    let membership = hierarchical(datacenters());
    let subsets = all_subsets(&btreeset! {1, 2, 3, 4, 5, 6, 7});

    // A write reaches 2 of dc1, dc2 and the witness 7.
    let writes = vec![
        btreeset! {1, 3, 5, 6},
        btreeset! {1, 4, 5, 6},
        btreeset! {2, 3, 5, 6},
        btreeset! {2, 4, 5, 6},
        btreeset! {1, 3, 7},
        btreeset! {1, 4, 7},
        btreeset! {2, 3, 7},
        btreeset! {2, 4, 7},
        btreeset! {5, 6, 7},
    ];
    let want = holding_one_of(&writes, &subsets);
    let write_accepted = accepted(&membership.write_quorum_set(), &subsets);
    assert_eq!(want, write_accepted);

    // A read reaches dc1 with both voters of a rack, and dc2 with one voter.
    let reads = vec![
        btreeset! {1, 2, 5},
        btreeset! {1, 2, 6},
        btreeset! {3, 4, 5},
        btreeset! {3, 4, 6},
        btreeset! {1, 2, 7},
        btreeset! {3, 4, 7},
        btreeset! {5, 7},
        btreeset! {6, 7},
    ];
    let want = holding_one_of(&reads, &subsets);
    let read_accepted = accepted(&membership.read_quorum_set(), &subsets);
    assert_eq!(want, read_accepted);

    let read = membership.read_quorum_set();
    let write = membership.write_quorum_set();
    let cases = [
        // dc2 is lost: dc1 and the witness commit and elect.
        (btreeset! {1, 2, 3, 4, 7}, true, true),
        // The witness is lost: both datacenters commit and elect.
        (btreeset! {1, 2, 3, 4, 5, 6}, true, true),
        // Rack r2 and voter 6 are lost: no write reaches dc1 or dc2, but a read reaches both.
        (btreeset! {1, 2, 5, 7}, false, true),
        // The witness and one voter of each rack are lost: no read reaches dc1.
        (btreeset! {2, 4, 5, 6}, true, false),
    ];
    for (live, want_writes, want_reads) in cases {
        let writes = write.is_quorum(live.iter());
        assert_eq!(want_writes, writes, "live voters {live:?}");
        let reads = read.is_quorum(live.iter());
        assert_eq!(want_reads, reads, "live voters {live:?}");
    }
}

#[test]
fn test_moving_a_voter_between_groups_goes_through_a_joint() {
    let old = nine_voters();
    let subsets = all_subsets(&btreeset! {1, 2, 3, 4, 5, 6, 7, 8, 9});

    // Voter 3 moves from zone a to zone b, and every zone keeps its write size 2.
    let moved = |a, b, c| {
        Group::of_groups(2, [
            ("a", Group::of_voters(a, [1, 2])),
            ("b", Group::of_voters(b, [3, 4, 5, 6])),
            ("c", Group::of_voters(c, [7, 8, 9])),
        ])
    };
    let change = Change::groups(moved(2, 2, 2));
    let new = change.apply(&old, true).unwrap();

    // A read quorum reaches 2 zones, with 1 voter of a, 3 of b or 2 of c.
    let new_reads = reaching(&moved(1, 3, 2), &subsets);
    let read_accepted = accepted(&new.read_quorum_set(), &subsets);
    assert_eq!(new_reads, read_accepted);
    let new_writes = reaching(&moved(2, 2, 2), &subsets);
    let write_accepted = accepted(&new.write_quorum_set(), &subsets);
    assert_eq!(new_writes, write_accepted);

    // The groups changed, so the configs share no rules and OpenRaft appends a joint first.
    let direct = old.supports_direct_transition_to(&new);
    assert!(!direct);

    // A joint quorum is a quorum of both configs.
    let joint = old.joint_with(&new);
    let old_quorums = reaching(&zones(2, 2, 2, 2), &subsets);

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
    let target = change.apply(&joint, true).unwrap();
    assert_eq!(new, target);
    let direct = joint.supports_direct_transition_to(&target);
    assert!(direct);
}

#[test]
fn test_nesting_groups_goes_through_a_joint() {
    let old = nine_voters();
    let subsets = all_subsets(&btreeset! {1, 2, 3, 4, 5, 6, 7, 8, 9});

    // Zones a and b move into region ab. A write reaches region ab with 2 voters of zone a or of
    // zone b, and zone c with 2 voters.
    let ab = Group::of_groups(1, [
        ("a", Group::of_voters(2, [1, 2, 3])),
        ("b", Group::of_voters(2, [4, 5, 6])),
    ]);
    let nested = Group::of_groups(2, [("ab", ab), ("c", Group::of_voters(2, [7, 8, 9]))]);
    let change = Change::groups(nested.clone());
    let new = change.apply(&old, true).unwrap();
    assert_eq!(hierarchical(nested), new);

    // The new read quorum {7,8} misses the old write quorum {1,2,4,5}, so OpenRaft appends a joint
    // first.
    let new_reads = new.read_quorum_set().is_quorum([7, 8].iter());
    assert!(new_reads);
    let old_writes = old.write_quorum_set().is_quorum([1, 2, 4, 5].iter());
    assert!(old_writes);
    let direct = old.supports_direct_transition_to(&new);
    assert!(!direct);

    // A joint quorum is a quorum of both configs, at every level of the new tree.
    let joint = old.joint_with(&new);

    let old_reads = accepted(&old.read_quorum_set(), &subsets);
    let new_reads = accepted(&new.read_quorum_set(), &subsets);
    let mut both_reads = old_reads;
    both_reads.retain(|subset| new_reads.contains(subset));
    let joint_reads = accepted(&joint.read_quorum_set(), &subsets);
    assert_eq!(both_reads, joint_reads);

    let old_writes = accepted(&old.write_quorum_set(), &subsets);
    let new_writes = accepted(&new.write_quorum_set(), &subsets);
    let mut both_writes = old_writes;
    both_writes.retain(|subset| new_writes.contains(subset));
    let joint_writes = accepted(&joint.write_quorum_set(), &subsets);
    assert_eq!(both_writes, joint_writes);

    assert_claims_hold(&old, &new);
    let change = <Change as RaftMembershipEdit<Membership>>::make_uniform();
    let target = change.apply(&joint, true).unwrap();
    assert_eq!(new, target);
}

#[test]
fn test_get_node() {
    // Voter 3 sits at the root, beside group a.
    let groups = Group {
        quorum_size: 1,
        voters: btreeset! {3},
        groups: btreemap! {"a".to_string() => Group::of_voters(2, [1, 2])},
    };
    let membership = Membership::new(groups, btreeset! {1, 2, 3, 4}).unwrap();

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
fn test_from_node_map_puts_every_node_in_the_root_group() {
    let nodes: BTreeMap<u64, ()> = btreemap! {1 => (), 2 => (), 3 => (), 4 => ()};
    let membership = Membership::from(nodes);

    // 4 voters: writes need 3 of them, and reads need 2.
    let want = hierarchical(Group::of_voters(3, [1, 2, 3, 4]));
    assert_eq!(want, membership);
    let want = "{configs: [Config { read: HierarchicalQuorumSet { root: Group { quorum_size: 2, \
                voters: {1, 2, 3, 4}, groups: {} } }, write: HierarchicalQuorumSet { root: Group { quorum_size: 3, \
                voters: {1, 2, 3, 4}, groups: {} } }, \
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

    // Writes to 3 zones meet every old read, but new reads of 1 zone miss old writes to the
    // other 2 zones.
    let change = Change::groups(zones(3, 2, 2, 2));
    let three_zones = change.apply(&membership, true).unwrap();
    let to_three = membership.supports_direct_transition_to(&three_zones);
    assert!(!to_three);
    let from_three = three_zones.supports_direct_transition_to(&membership);
    assert!(!from_three);

    // Writes of all 3 voters of zone a meet every old read, but new reads of 1 voter of zone a
    // miss old writes that hold the other 2.
    let change = Change::groups(zones(2, 3, 2, 2));
    let all_of_a = change.apply(&membership, true).unwrap();
    let to_all_of_a = membership.supports_direct_transition_to(&all_of_a);
    assert!(!to_all_of_a);

    // A joint goes directly to its target, and adding a learner changes no quorum.
    let joint = membership.joint_with(&three_zones);
    let to_target = joint.supports_direct_transition_to(&three_zones);
    assert!(to_target);

    let add_learner = <Change as RaftMembershipEdit<Membership>>::add_learner(10, ());
    let with_learner = add_learner.apply(&membership, true).unwrap();
    let to_learner = membership.supports_direct_transition_to(&with_learner);
    assert!(to_learner);
}

#[test]
fn test_direct_transitions_are_exact_within_one_layout() {
    // Every tree of voters {1,2,3,4}: one group; two groups where group a holds voter 1; voter 1
    // beside a group of the others; and three levels.
    let voters = btreeset! {1, 2, 3, 4};
    let mut layouts = vec![Group::of_voters(1, [1, 2, 3, 4])];
    for others in all_subsets(&btreeset! {2, 3, 4}) {
        let mut a = others;
        a.insert(1);
        let b: BTreeSet<u64> = voters.difference(&a).copied().collect();
        if b.is_empty() {
            continue;
        }
        let layout = Group::of_groups(1, [("a", Group::of_voters(1, a)), ("b", Group::of_voters(1, b))]);
        layouts.push(layout);
    }
    layouts.push(Group {
        quorum_size: 1,
        voters: btreeset! {1},
        groups: btreemap! {"a".to_string() => Group::of_voters(1, [2, 3, 4])},
    });
    let x = Group::of_groups(1, [("b", Group::of_voters(1, [2])), ("c", Group::of_voters(1, [3, 4]))]);
    layouts.push(Group::of_groups(1, [("a", Group::of_voters(1, [1])), ("x", x)]));

    // Every membership of these layouts: each write size of each group.
    let mut memberships = vec![];
    for (layout_index, layout) in layouts.iter().enumerate() {
        for groups in all_group_sizes(layout) {
            let membership = hierarchical(groups);
            assert_quorum_sets_are_valid(&membership);
            memberships.push((layout_index, membership));
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
fn test_apply_sets_the_groups_and_their_write_sizes() {
    let membership = nine_voters();

    // The empty change keeps the groups and their write sizes.
    let change = Change::default();
    let res = change.apply(&membership, true);
    assert_eq!(Ok(membership.clone()), res);

    // A new tree replaces the groups and every write size.
    let change = Change::groups(datacenters());
    let res = change.apply(&membership, true);
    let nodes = btreeset! {1, 2, 3, 4, 5, 6, 7, 8, 9};
    let want = Membership::new(datacenters(), nodes);
    assert_eq!(want, res);

    // A new tree gets the checks of the write rule at every level.
    let shared = Group::of_groups(2, [
        ("a", Group::of_voters(2, [1, 2, 3])),
        ("b", Group::of_groups(1, [("r1", Group::of_voters(1, [3, 4]))])),
    ]);
    let res = Change::groups(shared).apply(&membership, true);
    let invalid = InvalidGroups::SharedVoter {
        node_id: 3,
        first: path(&["a"]),
        second: path(&["b", "r1"]),
    };
    assert_eq!(Err(MembershipError::UserDefined(invalid)), res);

    let too_large = Group::of_groups(1, [(
        "ab",
        Group::of_groups(3, [
            ("a", Group::of_voters(2, [1, 2, 3])),
            ("b", Group::of_voters(2, [4, 5, 6])),
        ]),
    )]);
    let res = Change::groups(too_large).apply(&membership, true);
    let invalid = InvalidGroups::InvalidQuorumSize {
        group: path(&["ab"]),
        quorum_size: 3,
        member_count: 2,
    };
    assert_eq!(Err(MembershipError::UserDefined(invalid)), res);

    // Voter 10 of the new tree has no node.
    let with_10 = Group::of_voters(2, [1, 2, 10]);
    let res = Change::groups(with_10).apply(&membership, true);
    let not_found = NodeNotFound::new(10, Operation::None);
    assert_eq!(Err(MembershipError::NodeNotFound(not_found)), res);
}

#[test]
fn test_apply_edits_nodes() {
    let nodes = btreemap! {1 => "n1".to_string(), 2 => "n2".to_string(), 3 => "n3".to_string()};
    let one_group = Group::of_voters(2, [1, 2, 3]);
    let membership = HierarchicalMembership::new(one_group.clone(), nodes.clone()).unwrap();

    // Adding nodes keeps the metadata of a known node; removing a learner drops it.
    let change = HierarchicalChange {
        add_nodes: btreemap! {1 => "z".to_string(), 4 => "n4".to_string()},
        ..HierarchicalChange::default()
    };
    let with_four = change.apply(&membership, true).unwrap();
    let mut want_nodes = nodes;
    want_nodes.insert(4, "n4".to_string());
    let want = HierarchicalMembership::new(one_group, want_nodes).unwrap();
    assert_eq!(want, with_four);

    let change = HierarchicalChange {
        remove_nodes: btreeset! {4},
        ..HierarchicalChange::default()
    };
    let res = change.apply(&with_four, true);
    let want = membership.clone();
    assert_eq!(Ok(want), res);

    // Removing a voter fails, because every voter needs a node.
    let change = HierarchicalChange {
        remove_nodes: btreeset! {3},
        ..HierarchicalChange::default()
    };
    let res = change.apply(&membership, true);
    let not_found = NodeNotFound::new(3, Operation::None);
    let want = Err(MembershipError::from(not_found));
    assert_eq!(want, res);

    // `add_learner()` never replaces the node of a known ID.
    type StringChange = HierarchicalChange<u64, String>;
    type Target = HierarchicalMembership<u64, String>;
    let change = <StringChange as RaftMembershipEdit<Target>>::add_learner(2, "z".to_string());
    let res = change.apply(&membership, true);
    assert_eq!(Ok(membership), res);

    // A membership with no config has no target to change.
    let empty = Target::default();
    let change = StringChange::groups(Group::of_voters(1, [1]));
    let res = change.apply(&empty, true);
    let want = Err(MembershipError::EmptyMembership(EmptyMembership {}));
    assert_eq!(want, res);
}

#[test]
fn test_make_uniform_selects_target_and_keeps_old_voters_as_learners() {
    let old = nine_voters();

    // Zone c leaves, and a write quorum still needs both remaining zones.
    let two_zones = Group::of_groups(2, [
        ("a", Group::of_voters(2, [1, 2, 3])),
        ("b", Group::of_voters(2, [4, 5, 6])),
    ]);
    let change = Change::groups(two_zones.clone());
    let new = change.apply(&old, true).unwrap();
    let joint = old.joint_with(&new);

    let change = <Change as RaftMembershipEdit<Membership>>::make_uniform();
    let target = change.apply(&joint, true).unwrap();

    // Voters 7, 8 and 9 of the old config stay as learners of the target.
    let nodes = btreeset! {1, 2, 3, 4, 5, 6, 7, 8, 9};
    let want = Membership::new(two_zones, nodes).unwrap();
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
    let want = "{configs: [Config { read: HierarchicalQuorumSet { root: Group { quorum_size: 1, voters: {}, \
                groups: {\"a\": Group { quorum_size: 2, voters: {1, 2, 3}, groups: {} }} } }, \
                write: HierarchicalQuorumSet { root: Group { quorum_size: 1, voters: {}, groups: {\"a\": Group { \
                quorum_size: 2, voters: {1, 2, 3}, groups: {} }} } }, \
                nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), 2: ((), NodeRole(Campaigner | ReceiveLogs)), \
                3: ((), NodeRole(ReceiveLogs)), 4: ((), NodeRole(Campaigner | ReceiveLogs)), \
                5: ((), NodeRole(Empty))} }]}";
    assert_with_roles(one_group, Membership::with_roles, want);
}

#[test]
fn test_role_and_quorum_change_through_joint() {
    let want = "{configs: [Config { read: HierarchicalQuorumSet { root: Group { quorum_size: 1, voters: {}, \
                groups: {\"a\": Group { quorum_size: 2, voters: {2, 3, 4}, groups: {} }} } }, \
                write: HierarchicalQuorumSet { root: Group { quorum_size: 1, voters: {}, groups: {\"a\": Group { \
                quorum_size: 2, voters: {2, 3, 4}, groups: {} }} } }, nodes: {1: ((), NodeRole(ReceiveLogs)), \
                2: ((), NodeRole(Campaigner | ReceiveLogs)), 3: ((), NodeRole(ReceiveLogs)), \
                4: ((), NodeRole(Campaigner | ReceiveLogs)), 5: ((), NodeRole(Empty))} }]}";
    assert_role_and_quorum_change_through_joint(one_group, Membership::with_roles, want, |voters, roles| {
        let groups = Group::of_groups(1, [("a", Group::of_voters(2, voters))]);
        Change::groups(groups).with_roles(roles)
    });
}

#[test]
fn test_change_through_joint_keeps_node_metadata() {
    assert_change_through_joint_keeps_node_metadata(one_group, |voters, remove_nodes| {
        let groups = Group::of_groups(1, [("a", Group::of_voters(2, voters))]);
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
    // Voter 3 sits at the root, beside group a.
    let groups = Group {
        quorum_size: 1,
        voters: btreeset! {3},
        groups: btreemap! {"a".to_string() => Group::of_voters(2, [1, 2])},
    };
    let membership = Membership::new(groups, btreeset! {1, 2, 3, 4}).unwrap();

    let json = serde_json::to_string(&membership).unwrap();
    let want = r#"{"configs":[{"read":{"root":{"quorum_size":2,"voters":[3],"groups":{"a":{"quorum_size":1,"voters":[1,2],"groups":{}}}}},"write":{"root":{"quorum_size":1,"voters":[3],"groups":{"a":{"quorum_size":2,"voters":[1,2],"groups":{}}}}},"nodes":{"1":[null,3],"2":[null,3],"3":[null,3],"4":[null,2]}}]}"#;
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
    // The joint of two zone layouts, and of zones and datacenters with a non-campaigning voter.
    let old = nine_voters();
    let moved = move_voter_3_to_b(&old);
    let roles = btreemap! {5 => NodeRole::ReceiveLogs};
    let change = Change::groups(datacenters()).with_roles(roles);
    let nested = change.apply(&old, true).unwrap();

    for new in [moved, nested] {
        let joint = old.joint_with(&new);

        let binary = bincode::serialize(&joint).unwrap();
        let decoded = bincode::deserialize::<Membership>(&binary).unwrap();
        assert_eq!(joint, decoded);

        let json = serde_json::to_string(&joint).unwrap();
        let decoded = serde_json::from_str::<Membership>(&json).unwrap();
        assert_eq!(joint, decoded);
    }
}
