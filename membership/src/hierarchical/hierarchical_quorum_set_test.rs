use std::collections::BTreeSet;

use maplit::btreemap;
use maplit::btreeset;
use quorum_set::verify_intersection;

use crate::errors::InvalidGroups;
use crate::quorum::Group;
use crate::quorum::HierarchicalQuorumSet;
use crate::quorum::QuorumIntersection;
use crate::quorum::QuorumSet;
use crate::quorum::testing::accepted;
use crate::quorum::testing::all_group_sizes;
use crate::quorum::testing::all_subsets;
use crate::quorum::testing::assert_quorum_set_contract;
use crate::quorum::testing::holding_one_of;

/// Returns the root group over the groups `a={1}`, `b={2,3}` and `c={4,5,6}`, with the quorum
/// sizes `root`, `a`, `b` and `c`.
fn two_levels(root: u64, a: u64, b: u64, c: u64) -> Group<u64> {
    Group::of_groups(root, [
        ("a", Group::of_voters(a, [1])),
        ("b", Group::of_voters(b, [2, 3])),
        ("c", Group::of_voters(c, [4, 5, 6])),
    ])
}

/// Returns a tree of three levels whose branches have different depths, with the quorum sizes
/// `root`, `x`, `a`, `b` and `c`. The root holds voter 7 and the groups `x` and `c={5,6}`, and
/// group `x` holds the groups `a={1,2}` and `b={3,4}`.
fn three_levels(root: u64, x: u64, a: u64, b: u64, c: u64) -> Group<u64> {
    let x = Group::of_groups(x, [
        ("a", Group::of_voters(a, [1, 2])),
        ("b", Group::of_voters(b, [3, 4])),
    ]);
    Group {
        quorum_size: root,
        voters: btreeset! {7},
        groups: btreemap! {"x".to_string() => x, "c".to_string() => Group::of_voters(c, [5, 6])},
    }
}

/// Returns the path of a group from the IDs `ids`.
fn path(ids: &[&str]) -> Vec<String> {
    let mut path = vec![];
    for id in ids {
        path.push(id.to_string());
    }
    path
}

/// Returns every rule over the tree of `layout`.
fn all_rules(layout: &Group<u64>) -> Vec<HierarchicalQuorumSet<u64>> {
    let mut rules = vec![];
    for root in all_group_sizes(layout) {
        let rule = HierarchicalQuorumSet::new(root).unwrap();
        rules.push(rule);
    }
    rules
}

#[test]
fn test_new_rejects_unusable_groups_and_thresholds() {
    let empty_root = Group::of_voters(1, []);
    let empty_nested = Group::of_groups(1, [(
        "x",
        Group::of_groups(1, [("a", Group::of_voters(1, [1])), ("b", Group::of_voters(1, []))]),
    )]);
    let shared_by_siblings = Group::of_groups(1, [
        ("a", Group::of_voters(1, [1, 2])),
        ("b", Group::of_voters(1, [2, 3])),
    ]);
    // Voter 1 sits at the root and in group x.a, two levels below.
    let shared_across_levels = Group {
        quorum_size: 1,
        voters: btreeset! {1},
        groups: btreemap! {"x".to_string() => Group::of_groups(1, [("a", Group::of_voters(1, [1]))])},
    };

    let cases = vec![
        (empty_root, InvalidGroups::EmptyGroup { group: path(&[]) }),
        (empty_nested, InvalidGroups::EmptyGroup {
            group: path(&["x", "b"]),
        }),
        (shared_by_siblings, InvalidGroups::SharedVoter {
            node_id: 2,
            first: path(&["a"]),
            second: path(&["b"]),
        }),
        (shared_across_levels, InvalidGroups::SharedVoter {
            node_id: 1,
            first: path(&[]),
            second: path(&["x", "a"]),
        }),
        (three_levels(0, 1, 1, 1, 1), InvalidGroups::InvalidQuorumSize {
            group: path(&[]),
            quorum_size: 0,
            member_count: 3,
        }),
        (three_levels(4, 1, 1, 1, 1), InvalidGroups::InvalidQuorumSize {
            group: path(&[]),
            quorum_size: 4,
            member_count: 3,
        }),
        (three_levels(1, 3, 1, 1, 1), InvalidGroups::InvalidQuorumSize {
            group: path(&["x"]),
            quorum_size: 3,
            member_count: 2,
        }),
        (three_levels(1, 1, 1, 0, 1), InvalidGroups::InvalidQuorumSize {
            group: path(&["x", "b"]),
            quorum_size: 0,
            member_count: 2,
        }),
    ];
    for (root, invalid) in cases {
        let res = HierarchicalQuorumSet::new(root);
        assert_eq!(Err(invalid), res);
    }
}

#[test]
fn test_is_quorum_reaches_groups_at_every_level() {
    let subsets = all_subsets(&btreeset! {1, 2, 3, 4, 5, 6, 7});

    // A quorum reaches 2 of x, c and voter 7: x with a voter of a and a voter of b, c with both
    // of its voters.
    let root = three_levels(2, 2, 1, 1, 2);
    let rule = HierarchicalQuorumSet::new(root).unwrap();
    let minimal = vec![
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
    let want = holding_one_of(&minimal, &subsets);
    let got = accepted(&rule, &subsets);
    assert_eq!(want, got);

    // The dual reaches x with both voters of a or of b, and c with one voter.
    let dual = rule.dual();
    let want = three_levels(2, 1, 2, 2, 1);
    assert_eq!(&want, dual.root());
    let minimal = vec![
        btreeset! {1, 2, 5},
        btreeset! {1, 2, 6},
        btreeset! {3, 4, 5},
        btreeset! {3, 4, 6},
        btreeset! {1, 2, 7},
        btreeset! {3, 4, 7},
        btreeset! {5, 7},
        btreeset! {6, 7},
    ];
    let want = holding_one_of(&minimal, &subsets);
    let got = accepted(&dual, &subsets);
    assert_eq!(want, got);
}

#[test]
fn test_is_quorum_ignores_unknown_and_repeated_ids() {
    let root = three_levels(2, 2, 1, 1, 2);
    let rule = HierarchicalQuorumSet::new(root).unwrap();

    // Voters 1 and 3 reach group x, and voter 7 sits at the root.
    let accepts = rule.is_quorum([7, 1, 9, 3, 1].iter());
    assert!(accepts);
    // Voter 5 counts once, so group c is not reached.
    let accepts = rule.is_quorum([5, 5, 7, 9].iter());
    assert!(!accepts);
    let accepts = rule.is_quorum([].iter());
    assert!(!accepts);

    let ids: Vec<u64> = rule.ids().collect();
    assert_eq!(vec![1, 2, 3, 4, 5, 6, 7], ids);
}

#[test]
fn test_rules_keep_the_quorum_set_contract() {
    for layout in [two_levels(1, 1, 1, 1), three_levels(1, 1, 1, 1, 1)] {
        for rule in all_rules(&layout) {
            assert_quorum_set_contract(&rule);
        }
    }
}

#[test]
fn test_intersects_with() {
    let rule = |root, x, a, b, c| {
        let root = three_levels(root, x, a, b, c);
        HierarchicalQuorumSet::new(root).unwrap()
    };

    // `read` is the dual of `write`: in every group, the two quorum sizes sum to one more than the
    // member count.
    let write = rule(2, 2, 1, 1, 2);
    let read = rule(2, 1, 2, 2, 1);
    let intersects = read.intersects_with(&write);
    assert_eq!(Some(true), intersects);

    // With a majority of the members of every group, two quorums reach a common member in every
    // group that both reach.
    let majority = rule(2, 2, 2, 2, 2);
    let intersects = majority.intersects_with(&majority);
    assert_eq!(Some(true), intersects);

    // Two write quorums reach group x through different voters of group a and of group b:
    // {1,3,7} and {2,4,5,6} are disjoint.
    let exact = verify_intersection(&write, &write);
    assert!(!exact);
    let intersects = write.intersects_with(&write);
    assert_eq!(None, intersects);

    // A quorum of all 3 root members holds voter 7 and both voters of c, which every write quorum
    // meets. Groups a and b split, so the check proves nothing.
    let all_members = rule(3, 2, 1, 1, 2);
    let exact = verify_intersection(&all_members, &write);
    assert!(exact);
    let intersects = all_members.intersects_with(&write);
    assert_eq!(None, intersects);

    // Moving voter 4 from group x.b to group c changes the tree, so the check proves nothing.
    let x = Group::of_groups(2, [("a", Group::of_voters(2, [1, 2])), ("b", Group::of_voters(1, [3]))]);
    let moved = Group {
        quorum_size: 2,
        voters: btreeset! {7},
        groups: btreemap! {"x".to_string() => x, "c".to_string() => Group::of_voters(2, [4, 5, 6])},
    };
    let moved = HierarchicalQuorumSet::new(moved).unwrap();
    let intersects = majority.intersects_with(&moved);
    assert_eq!(None, intersects);
}

#[test]
fn test_intersects_with_proves_only_true_intersections() {
    for layout in [two_levels(1, 1, 1, 1), three_levels(1, 1, 1, 1, 1)] {
        let rules = all_rules(&layout);

        for a in &rules {
            // A membership config uses the dual of its write rule as its read rule.
            let read = a.dual();
            let intersects = read.intersects_with(a);
            assert_eq!(Some(true), intersects, "a={a:?}");

            for b in &rules {
                let intersects = a.intersects_with(b);
                assert_ne!(Some(false), intersects, "a={a:?}, b={b:?}");
                if intersects == Some(true) {
                    let exact = verify_intersection(a, b);
                    assert!(exact, "a={a:?}, b={b:?}");
                }
            }
        }
    }
}

#[test]
fn test_intersects_with_proves_nothing_for_other_trees() {
    let rules = all_rules(&three_levels(1, 1, 1, 1, 1));

    // The same voters, with voter 4 moved from group x.b to group c.
    let x = Group::of_groups(1, [("a", Group::of_voters(1, [1, 2])), ("b", Group::of_voters(1, [3]))]);
    let moved = Group {
        quorum_size: 1,
        voters: btreeset! {7},
        groups: btreemap! {"x".to_string() => x, "c".to_string() => Group::of_voters(1, [4, 5, 6])},
    };
    // The same groups a, b and c, with a and b directly under the root.
    let flat = Group {
        quorum_size: 1,
        voters: btreeset! {7},
        groups: btreemap! {
            "a".to_string() => Group::of_voters(1, [1, 2]),
            "b".to_string() => Group::of_voters(1, [3, 4]),
            "c".to_string() => Group::of_voters(1, [5, 6]),
        },
    };

    for other_layout in [moved, flat] {
        for a in &rules {
            for b in &all_rules(&other_layout) {
                let intersects = a.intersects_with(b);
                assert_eq!(None, intersects, "a={a:?}, b={b:?}");
            }
        }
    }
}

#[test]
fn test_dual_accepts_exactly_the_sets_that_meet_every_quorum() {
    let majority = HierarchicalQuorumSet::new(two_levels(2, 1, 2, 2)).unwrap();
    let dual = majority.dual();
    let want = two_levels(2, 1, 1, 2);
    assert_eq!(&want, dual.root());

    for layout in [two_levels(1, 1, 1, 1), three_levels(1, 1, 1, 1, 1)] {
        let rules = all_rules(&layout);
        let ids: BTreeSet<u64> = rules[0].ids().collect();
        let subsets = all_subsets(&ids);

        for rule in rules {
            let dual = rule.dual();
            for subset in &subsets {
                // Some quorum misses `subset` exactly when the voters outside it form a quorum.
                let outside: BTreeSet<u64> = ids.difference(subset).copied().collect();
                let meets_every_quorum = !rule.is_quorum(outside.iter());
                let accepts = dual.is_quorum(subset.iter());
                assert_eq!(meets_every_quorum, accepts, "rule={rule:?}, subset={subset:?}");
            }

            let dual_of_dual = dual.dual();
            assert_eq!(rule, dual_of_dual);
        }
    }
}

#[cfg(feature = "serde")]
#[test]
fn test_serde() {
    // The root holds voter 3 and group a, which holds group b={1,2}.
    let a = Group::of_groups(1, [("b", Group::of_voters(2, [1, 2]))]);
    let root = Group {
        quorum_size: 1,
        voters: btreeset! {3},
        groups: btreemap! {"a".to_string() => a},
    };
    let rule = HierarchicalQuorumSet::new(root).unwrap();

    let json = serde_json::to_string(&rule).unwrap();
    let want = r#"{"root":{"quorum_size":1,"voters":[3],"groups":{"a":{"quorum_size":1,"voters":[],"groups":{"b":{"quorum_size":2,"voters":[1,2],"groups":{}}}}}}}"#;
    assert_eq!(want, json);
    let decoded = serde_json::from_str::<HierarchicalQuorumSet<u64>>(&json).unwrap();
    assert_eq!(rule, decoded);

    let binary = bincode::serialize(&rule).unwrap();
    let decoded = bincode::deserialize::<HierarchicalQuorumSet<u64>>(&binary).unwrap();
    assert_eq!(rule, decoded);
}
