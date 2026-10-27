use std::collections::BTreeMap;
use std::collections::BTreeSet;

use maplit::btreemap;
use maplit::btreeset;
use quorum_set::verify_intersection;

use crate::errors::InvalidGroups;
use crate::quorum::Group;
use crate::quorum::HierarchicalQuorumSet;
use crate::quorum::QuorumIntersection;
use crate::quorum::QuorumSet;
use crate::quorum::testing::all_subsets;
use crate::quorum::testing::assert_quorum_set_contract;

/// Returns the groups `a={1}`, `b={2,3}` and `c={4,5,6}`.
fn uneven_groups() -> BTreeMap<String, BTreeSet<u64>> {
    btreemap! {
        "a".to_string() => btreeset! {1},
        "b".to_string() => btreeset! {2, 3},
        "c".to_string() => btreeset! {4, 5, 6},
    }
}

/// Returns the quorum sizes `sizes`, keyed by group ID.
fn sizes(sizes: &[(&str, u64)]) -> BTreeMap<String, u64> {
    let mut map = BTreeMap::new();
    for (group, size) in sizes {
        map.insert(group.to_string(), *size);
    }
    map
}

/// Returns each group of `layout` with its quorum size in `sizes`, which names every group.
fn sized(layout: &BTreeMap<String, BTreeSet<u64>>, sizes: &BTreeMap<String, u64>) -> BTreeMap<String, Group<u64>> {
    let mut groups = BTreeMap::new();
    for (group, voters) in layout {
        let voters = voters.clone();
        let quorum_size = sizes[group];
        groups.insert(group.clone(), Group { voters, quorum_size });
    }
    groups
}

/// Returns every rule over `groups`: each number of groups to reach, with each quorum size of each
/// group.
fn all_rules(groups: &BTreeMap<String, BTreeSet<u64>>) -> Vec<HierarchicalQuorumSet<u64>> {
    let mut all_sizes = vec![BTreeMap::new()];
    for (group, voters) in groups {
        let voter_count = voters.len() as u64;
        let mut extended = vec![];
        for sizes in &all_sizes {
            for size in 1..=voter_count {
                let mut sizes = sizes.clone();
                sizes.insert(group.clone(), size);
                extended.push(sizes);
            }
        }
        all_sizes = extended;
    }

    let group_count = groups.len() as u64;
    let mut rules = vec![];
    for quorum_groups in 1..=group_count {
        for sizes in &all_sizes {
            let rule = HierarchicalQuorumSet::new(quorum_groups, sized(groups, sizes));
            rules.push(rule.unwrap());
        }
    }
    rules
}

#[test]
fn test_new_rejects_unusable_groups_and_thresholds() {
    let shared = btreemap! {
        "a".to_string() => btreeset! {1, 2},
        "b".to_string() => btreeset! {2, 3},
    };
    let with_empty = btreemap! {
        "a".to_string() => btreeset! {1},
        "b".to_string() => btreeset! {},
    };
    let all_one = sizes(&[("a", 1), ("b", 1), ("c", 1)]);

    let cases = vec![
        (BTreeMap::new(), 1, sizes(&[]), InvalidGroups::NoGroup),
        (with_empty, 1, sizes(&[("a", 1), ("b", 1)]), InvalidGroups::EmptyGroup {
            group: "b".to_string(),
        }),
        (shared, 1, sizes(&[("a", 1), ("b", 1)]), InvalidGroups::SharedVoter {
            node_id: 2,
            first: "a".to_string(),
            second: "b".to_string(),
        }),
        (
            uneven_groups(),
            0,
            all_one.clone(),
            InvalidGroups::InvalidQuorumGroups {
                quorum_groups: 0,
                group_count: 3,
            },
        ),
        (uneven_groups(), 4, all_one, InvalidGroups::InvalidQuorumGroups {
            quorum_groups: 4,
            group_count: 3,
        }),
        (
            uneven_groups(),
            1,
            sizes(&[("a", 1), ("b", 0), ("c", 1)]),
            InvalidGroups::InvalidQuorumSize {
                group: "b".to_string(),
                quorum_size: 0,
                voter_count: 2,
            },
        ),
        (
            uneven_groups(),
            1,
            sizes(&[("a", 1), ("b", 3), ("c", 1)]),
            InvalidGroups::InvalidQuorumSize {
                group: "b".to_string(),
                quorum_size: 3,
                voter_count: 2,
            },
        ),
    ];
    for (layout, quorum_groups, quorum_sizes, invalid) in cases {
        let groups = sized(&layout, &quorum_sizes);
        let res = HierarchicalQuorumSet::new(quorum_groups, groups);
        assert_eq!(Err(invalid), res);
    }
}

#[test]
fn test_is_quorum_reaches_quorum_groups_groups() {
    let groups = uneven_groups();
    let subsets = all_subsets(&btreeset! {1, 2, 3, 4, 5, 6});

    for rule in all_rules(&groups) {
        for subset in &subsets {
            let mut reached = 0;
            for (group, voters) in &groups {
                let held = voters.intersection(subset).count() as u64;
                if held >= rule.groups()[group].quorum_size {
                    reached += 1;
                }
            }
            let want = reached >= rule.quorum_groups();

            let got = rule.is_quorum(subset.iter());
            assert_eq!(want, got, "rule={rule:?}, subset={subset:?}");
        }
    }
}

#[test]
fn test_is_quorum_ignores_unknown_and_repeated_ids() {
    let quorum_sizes = sizes(&[("a", 1), ("b", 2), ("c", 2)]);
    let rule = HierarchicalQuorumSet::new(2, sized(&uneven_groups(), &quorum_sizes)).unwrap();

    // Voter 1 reaches group a, and voters 4 and 5 reach group c.
    let accepts = rule.is_quorum([5, 1, 9, 4, 1].iter());
    assert!(accepts);
    // Voter 4 counts once, so group c is not reached.
    let accepts = rule.is_quorum([4, 4, 1, 9].iter());
    assert!(!accepts);
    let accepts = rule.is_quorum([].iter());
    assert!(!accepts);

    let ids: Vec<u64> = rule.ids().collect();
    assert_eq!(vec![1, 2, 3, 4, 5, 6], ids);
}

#[test]
fn test_rules_keep_the_quorum_set_contract() {
    for rule in all_rules(&uneven_groups()) {
        assert_quorum_set_contract(&rule);
    }
}

#[test]
fn test_intersects_with() {
    let groups = uneven_groups();
    let rule = |quorum_groups, quorum_sizes| {
        let rule = HierarchicalQuorumSet::new(quorum_groups, sized(&groups, &quorum_sizes));
        rule.unwrap()
    };

    // Two quorums share one of the 3 groups, and more than half of its voters.
    let majority = rule(2, sizes(&[("a", 1), ("b", 2), ("c", 2)]));
    let intersects = majority.intersects_with(&majority);
    assert_eq!(Some(true), intersects);

    // Quorums of one group each can reach different groups, such as {1} and {2,3}.
    let one_group = rule(1, sizes(&[("a", 1), ("b", 2), ("c", 3)]));
    let intersects = one_group.intersects_with(&one_group);
    assert_eq!(None, intersects);

    // Quorums of 2 groups share at least one group, and group b splits into {2} and {3}:
    // {1,2} and {3,4,5} are disjoint quorums.
    let two_groups = rule(2, sizes(&[("a", 1), ("b", 1), ("c", 2)]));
    let intersects = two_groups.intersects_with(&two_groups);
    assert_eq!(None, intersects);

    // `two_groups` is the read rule that a config derives from the write rule `majority`. Each sum
    // exceeds its bound by exactly 1: 2 + 2 groups of 3, and in each group two quorum sizes that
    // sum to one more than its voter count.
    let intersects = two_groups.intersects_with(&majority);
    assert_eq!(Some(true), intersects);

    // A quorum of all 3 groups shares 2 groups with a quorum of 2 groups. Only group b splits, so
    // they share a voter in their other common group, but the check proves no pair where a group
    // splits.
    let all_groups = rule(3, sizes(&[("a", 1), ("b", 1), ("c", 2)]));
    let exact = verify_intersection(&all_groups, &two_groups);
    assert!(exact);
    let intersects = all_groups.intersects_with(&two_groups);
    assert_eq!(None, intersects);

    // Moving voter 3 to group c changes the groups, so the check proves nothing.
    let moved = btreemap! {
        "a".to_string() => btreeset! {1},
        "b".to_string() => btreeset! {2},
        "c".to_string() => btreeset! {3, 4, 5, 6},
    };
    let moved = sized(&moved, &sizes(&[("a", 1), ("b", 1), ("c", 3)]));
    let moved = HierarchicalQuorumSet::new(2, moved).unwrap();
    let intersects = majority.intersects_with(&moved);
    assert_eq!(None, intersects);
}

#[test]
fn test_intersects_with_proves_only_true_intersections() {
    let rules = all_rules(&uneven_groups());

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

#[test]
fn test_intersects_with_proves_nothing_for_other_groups() {
    let rules = all_rules(&uneven_groups());

    // The same group IDs and voters, with voter 3 moved from group b to group c.
    let moved = btreemap! {
        "a".to_string() => btreeset! {1},
        "b".to_string() => btreeset! {2},
        "c".to_string() => btreeset! {3, 4, 5, 6},
    };
    let moved_rules = all_rules(&moved);

    for a in &rules {
        for b in &moved_rules {
            let intersects = a.intersects_with(b);
            assert_eq!(None, intersects, "a={a:?}, b={b:?}");
        }
    }
}

#[test]
fn test_dual_accepts_exactly_the_sets_that_meet_every_quorum() {
    let groups = uneven_groups();
    let ids = btreeset! {1, 2, 3, 4, 5, 6};
    let subsets = all_subsets(&ids);

    let majority = HierarchicalQuorumSet::new(2, sized(&groups, &sizes(&[("a", 1), ("b", 2), ("c", 2)]))).unwrap();
    let want = HierarchicalQuorumSet::new(2, sized(&groups, &sizes(&[("a", 1), ("b", 1), ("c", 2)]))).unwrap();
    let dual = majority.dual();
    assert_eq!(want, dual);

    for rule in all_rules(&groups) {
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

#[cfg(feature = "serde")]
#[test]
fn test_serde() {
    let groups = btreemap! {
        "a".to_string() => btreeset! {1, 2},
        "b".to_string() => btreeset! {3},
    };
    let rule = HierarchicalQuorumSet::new(1, sized(&groups, &sizes(&[("a", 2), ("b", 1)]))).unwrap();

    let json = serde_json::to_string(&rule).unwrap();
    let want =
        r#"{"groups":{"a":{"voters":[1,2],"quorum_size":2},"b":{"voters":[3],"quorum_size":1}},"quorum_groups":1}"#;
    assert_eq!(want, json);
    let decoded = serde_json::from_str::<HierarchicalQuorumSet<u64>>(&json).unwrap();
    assert_eq!(rule, decoded);

    let binary = bincode::serialize(&rule).unwrap();
    let decoded = bincode::deserialize::<HierarchicalQuorumSet<u64>>(&binary).unwrap();
    assert_eq!(rule, decoded);
}
