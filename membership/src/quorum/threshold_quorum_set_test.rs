use std::num::NonZeroU64;
use std::sync::Arc;

use maplit::btreeset;
use quorum_set::verify_intersection;

use crate::quorum::QuorumIntersection;
use crate::quorum::QuorumSet;
use crate::quorum::ThresholdQuorumSet;
use crate::quorum::testing::all_subsets;
use crate::quorum::testing::all_thresholds;
use crate::quorum::testing::threshold;

#[test]
fn test_is_quorum_accepts_every_set_holding_quorum_size_ids() {
    let ids = btreeset! {1, 2, 3, 4, 5};
    let three_of_five = threshold(3, ids.clone());

    let subsets = all_subsets(&ids);
    let mut accepted = vec![];
    for subset in &subsets {
        let is_quorum = three_of_five.is_quorum(subset.iter());
        if is_quorum {
            accepted.push(subset.clone());
        }
    }

    // 10 subsets hold 3 IDs, 5 hold 4, and 1 holds all 5.
    let want = subsets.into_iter().filter(|subset| subset.len() >= 3).collect::<Vec<_>>();
    assert_eq!(16, want.len());
    assert_eq!(want, accepted);
}

#[test]
fn test_is_quorum_counts_each_rule_id_once() {
    let two_of_three = threshold(2, btreeset! {1, 2, 3});

    // A repeated ID counts once, and an ID outside the rule never counts.
    let repeated = two_of_three.is_quorum([1, 1, 1].iter());
    assert!(!repeated);
    let outside = two_of_three.is_quorum([1, 4, 5].iter());
    assert!(!outside);
    let two_ids = two_of_three.is_quorum([1, 1, 2].iter());
    assert!(two_ids);
}

#[test]
fn test_quorum_size_above_id_count_accepts_no_set() {
    let four_of_three = threshold(4, btreeset! {1, 2, 3});
    let all_ids = four_of_three.is_quorum([1, 2, 3, 4].iter());
    assert!(!all_ids);

    let one_of_none = threshold(1, btreeset! {});
    let empty_input = one_of_none.is_quorum(std::iter::empty());
    assert!(!empty_input);
    let some_ids = one_of_none.is_quorum([1].iter());
    assert!(!some_ids);
}

#[test]
fn test_rules_share_one_id_set() {
    let voters = Arc::new(btreeset! {1, 2, 3});
    let three = NonZeroU64::new(3).unwrap();
    let read = ThresholdQuorumSet::new(NonZeroU64::MIN, voters.clone());
    let write = ThresholdQuorumSet::new(three, voters.clone());

    let read_shares = Arc::ptr_eq(&voters, read.id_set());
    assert!(read_shares);
    let write_shares = Arc::ptr_eq(&voters, write.id_set());
    assert!(write_shares);

    assert_eq!(1, read.quorum_size());
    assert_eq!(3, write.quorum_size());

    let ids = read.ids().collect::<Vec<_>>();
    assert_eq!(vec![1, 2, 3], ids);
}

#[test]
fn test_intersects_with() {
    let voters = btreeset! {1, 2, 3, 4, 5};
    let two_of_five = threshold(2, voters.clone());
    let three_of_five = threshold(3, voters.clone());
    let four_of_five = threshold(4, voters);

    // 2 + 4 > 5: a quorum of 2 and a quorum of 4 always share an ID.
    let sizes_exceed_ids = two_of_five.intersects_with(&four_of_five);
    assert_eq!(Some(true), sizes_exceed_ids);

    // 2 + 3 = 5: {1,2} and {3,4,5} are disjoint quorums.
    let sizes_equal_ids = two_of_five.intersects_with(&three_of_five);
    assert_eq!(Some(false), sizes_equal_ids);

    // A quorum of 3 of {1,2,3,4} holds at least 2 of {1,2,3}, so it meets every quorum of 2 of
    // {1,2,3}.
    let old = threshold(2, btreeset! {1, 2, 3});
    let new = threshold(3, btreeset! {1, 2, 3, 4});
    let different_ids = old.intersects_with(&new);
    assert_eq!(Some(true), different_ids);
}

#[test]
fn test_intersects_with_agrees_with_verify_intersection() {
    let thresholds = all_thresholds(&btreeset! {1, 2, 3, 4});

    // The check always answers, and every answer matches the exact relation.
    for a in &thresholds {
        for b in &thresholds {
            let intersects = a.intersects_with(b);
            let exact = verify_intersection(a, b);
            assert_eq!(Some(exact), intersects, "a: {a:?}, b: {b:?}");
        }
    }
}

#[cfg(feature = "serde")]
#[test]
fn test_serde() {
    let two_of_three = threshold(2, btreeset! {1, 2, 3});

    let json = serde_json::to_string(&two_of_three).unwrap();
    assert_eq!(r#"{"ids":[1,2,3],"quorum_size":2}"#, json);
    let decoded = serde_json::from_str::<ThresholdQuorumSet<u64>>(&json).unwrap();
    assert_eq!(two_of_three, decoded);

    // A quorum_size of 0 would make the empty set a quorum, so decoding rejects it.
    let zero = r#"{"ids":[1,2,3],"quorum_size":0}"#;
    let err = serde_json::from_str::<ThresholdQuorumSet<u64>>(zero).unwrap_err();
    assert!(err.is_data(), "{err}");
}
