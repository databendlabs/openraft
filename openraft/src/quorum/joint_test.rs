use maplit::btreeset;
use quorum_set::verify_intersection;

use crate::quorum::Joint;
use crate::quorum::QuorumBridge;
use crate::quorum::QuorumIntersection;
use crate::quorum::QuorumSet;
use crate::quorum::ThresholdQuorumSet;
use crate::quorum::threshold_quorum_set_test::all_subsets;
use crate::quorum::threshold_quorum_set_test::all_thresholds;
use crate::quorum::threshold_quorum_set_test::threshold;

/// Returns the empty joint, and every joint of one or two rules over subsets of {1,2,3}.
///
/// Three IDs keep the checks over pairs of joints short: four IDs give about 5.5 million pairs.
fn all_joints() -> Vec<Joint<ThresholdQuorumSet<u64>>> {
    let thresholds = all_thresholds(&btreeset! {1, 2, 3});

    let mut joints = vec![Joint::default()];
    for a in &thresholds {
        joints.push(Joint::new([a.clone()]));
        for b in &thresholds {
            joints.push(Joint::new([a.clone(), b.clone()]));
        }
    }
    joints
}

/// Any two quorums of a rule intersect when its `quorum_size` is more than half of its IDs.
fn intersects_itself(rule: &ThresholdQuorumSet<u64>) -> bool {
    let id_count = rule.id_set().len() as u64;
    2 * rule.quorum_size() > id_count
}

#[test]
fn test_joint_of_thresholds() {
    let old = threshold(2, btreeset! {1, 2, 3});
    let new = threshold(2, btreeset! {2, 3, 4});
    let joint = Joint::new([old, new]);

    let mut accepted = vec![];
    for ids in all_subsets(&btreeset! {1, 2, 3, 4}) {
        let is_quorum = joint.is_quorum(ids.iter());
        if is_quorum {
            accepted.push(ids);
        }
    }

    // The smallest quorums {2,3}, {1,2,4} and {1,3,4} differ in size, so no single threshold rule
    // accepts the same sets.
    let want = vec![
        btreeset! {2, 3},
        btreeset! {1, 2, 3},
        btreeset! {1, 2, 4},
        btreeset! {1, 3, 4},
        btreeset! {2, 3, 4},
        btreeset! {1, 2, 3, 4},
    ];
    assert_eq!(want, accepted);

    let ids = joint.ids().collect::<Vec<_>>();
    assert_eq!(vec![1, 2, 3, 4], ids);
}

#[test]
fn test_empty_joint_accepts_no_set() {
    let empty = Joint::<ThresholdQuorumSet<u64>>::default();

    let empty_input = empty.is_quorum(std::iter::empty());
    assert!(!empty_input);
    let some_ids = empty.is_quorum([1, 2].iter());
    assert!(!some_ids);

    let ids = empty.ids().collect::<Vec<_>>();
    assert_eq!(Vec::<u64>::new(), ids);
}

#[test]
fn test_intersects_with() {
    let r123 = || threshold(2, btreeset! {1, 2, 3});
    let r345 = || threshold(2, btreeset! {3, 4, 5});
    let r789 = || threshold(2, btreeset! {7, 8, 9});

    let j123 = Joint::new([r123()]);
    let j345 = Joint::new([r345()]);
    let j123_345 = Joint::new([r123(), r345()]);
    let j345_789 = Joint::new([r345(), r789()]);

    // Two joints of one rule each get the answer of the rule check: {1,2} and {4,5} are disjoint
    // quorums.
    let uniform = j123.intersects_with(&j345);
    assert_eq!(Some(false), uniform);

    // Other joints answer `Some(true)` when some rule pair intersects, and `None` when no pair
    // does.
    let shared_rule = j123_345.intersects_with(&j345_789);
    assert_eq!(Some(true), shared_rule);
    let no_shared_rule = j123.intersects_with(&j345_789);
    assert_eq!(None, no_shared_rule);
}

#[test]
fn test_intersects_with_agrees_with_verify_intersection() {
    let joints = all_joints();

    // Two joints of at most one rule each always get the exact answer. For other joints,
    // `intersects_with` may answer `None`, but every `Some` answer must match the exact relation.
    for a in &joints {
        for b in &joints {
            let intersects = a.intersects_with(b);
            let at_most_one_rule = a.rules().len() <= 1 && b.rules().len() <= 1;
            if at_most_one_rule {
                let exact = verify_intersection(a, b);
                assert_eq!(Some(exact), intersects, "a: {a:?}, b: {b:?}");
                continue;
            }

            let Some(intersects) = intersects else {
                continue;
            };
            let exact = verify_intersection(a, b);
            assert_eq!(exact, intersects, "a: {a:?}, b: {b:?}");
        }
    }
}

#[test]
fn test_joint_with() {
    let r1 = || threshold(2, btreeset! {1, 2, 3});
    let r2 = || threshold(1, btreeset! {3, 4});
    let r3 = || threshold(3, btreeset! {4, 5, 6});

    let j12 = Joint::new([r1(), r2()]);
    let j3 = Joint::new([r3()]);

    let joint = j12.joint_with(j3.clone());
    let want = Joint::new([r1(), r2(), r3()]);
    assert_eq!(want, joint);

    // An empty joint has no quorum, so a joint with it has no quorum either.
    let empty = Joint::default();
    let with_empty = j12.joint_with(empty.clone());
    assert_eq!(empty, with_empty);
    let from_empty = empty.joint_with(j3);
    assert_eq!(empty, from_empty);
}

#[test]
fn test_joint_with_accepts_quorums_of_both_sides() {
    let joints = all_joints();
    let id_sets = all_subsets(&btreeset! {1, 2, 3});

    for current in &joints {
        for goal in &joints {
            let joint = current.joint_with(goal.clone());
            for ids in &id_sets {
                let in_current = current.is_quorum(ids.iter());
                let in_goal = goal.is_quorum(ids.iter());
                let in_both = in_current && in_goal;
                let in_joint = joint.is_quorum(ids.iter());
                assert_eq!(in_both, in_joint, "{current:?} joint with {goal:?} on {ids:?}");
            }
        }
    }
}

#[test]
fn test_bridge_to() {
    let r1 = || threshold(2, btreeset! {1, 2, 3});
    let r2 = || threshold(2, btreeset! {3, 4, 5});
    let r3 = || threshold(2, btreeset! {7, 8, 9});

    let j1 = Joint::new([r1()]);
    let j2 = Joint::new([r2()]);
    let j3 = Joint::new([r3()]);
    let j12 = Joint::new([r1(), r2()]);
    let j23 = Joint::new([r2(), r3()]);

    let bridge = j1.bridge_to(j1.clone());
    assert_eq!(j1, bridge);
    let bridge = j1.bridge_to(j2.clone());
    assert_eq!(j12, bridge);
    let bridge = j12.bridge_to(j1.clone());
    assert_eq!(j1, bridge);
    let bridge = j12.bridge_to(j2.clone());
    assert_eq!(j2, bridge);
    let bridge = j12.bridge_to(j3);
    assert_eq!(j23, bridge);

    // Every quorum of 2 of {1,2,3} intersects every quorum of 3 of {1,2,3,4}, so the bridge is the
    // target alone.
    let grown_rule = threshold(3, btreeset! {1, 2, 3, 4});
    let grown = Joint::new([grown_rule]);
    let bridge = j1.bridge_to(grown.clone());
    assert_eq!(grown, bridge);
}

#[test]
fn test_bridge_to_preserves_intersection_of_self_intersecting_rules() {
    let mut joints = all_joints();
    joints.retain(|joint| joint.rules().iter().all(intersects_itself));

    for current in &joints {
        for goal in &joints {
            let bridge = current.bridge_to(goal.clone());
            let current_to_bridge = verify_intersection(current, &bridge);
            assert!(current_to_bridge, "{current:?} must intersect its bridge {bridge:?}");
            let bridge_to_goal = verify_intersection(&bridge, goal);
            assert!(bridge_to_goal, "bridge {bridge:?} must intersect its goal {goal:?}");
        }
    }
}

#[cfg(feature = "serde")]
#[test]
fn test_serde() {
    let old = threshold(2, btreeset! {1, 2, 3});
    let new = threshold(1, btreeset! {3, 4});
    let joint = Joint::new([old, new]);

    let json = serde_json::to_string(&joint).unwrap();
    let want = r#"{"rules":[{"ids":[1,2,3],"quorum_size":2},{"ids":[3,4],"quorum_size":1}]}"#;
    assert_eq!(want, json);
    let decoded = serde_json::from_str::<Joint<ThresholdQuorumSet<u64>>>(&json).unwrap();
    assert_eq!(joint, decoded);

    let empty = serde_json::from_str::<Joint<ThresholdQuorumSet<u64>>>(r#"{"rules":[]}"#).unwrap();
    let want = Joint::default();
    assert_eq!(want, empty);
}
