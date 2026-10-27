use std::collections::BTreeMap;
use std::collections::BTreeSet;

use maplit::btreemap;
use maplit::btreeset;
use quorum_set::verify_intersection;

use crate::errors::InvalidWeights;
use crate::quorum::QuorumIntersection;
use crate::quorum::QuorumSet;
use crate::quorum::WeightedQuorumSet;
use crate::quorum::testing::all_subsets;
use crate::quorum::testing::assert_quorum_set_contract;

/// Returns a rule that accepts a set weighing at least `threshold`, with these weights, each at
/// least 1.
fn weighted(threshold: u64, weights: BTreeMap<u64, u64>) -> WeightedQuorumSet<u64> {
    WeightedQuorumSet::new(threshold, weights).expect("the threshold and every weight are at least 1")
}

/// Returns the sets in `subsets` that `rule` accepts, in their order.
fn accepted(rule: &WeightedQuorumSet<u64>, subsets: &[BTreeSet<u64>]) -> Vec<BTreeSet<u64>> {
    let mut accepted = vec![];
    for subset in subsets {
        let is_quorum = rule.is_quorum(subset.iter());
        if is_quorum {
            accepted.push(subset.clone());
        }
    }
    accepted
}

/// Returns the sum of the weights of the IDs in `subset`; an ID without a weight adds nothing.
fn weight_of(weights: &BTreeMap<u64, u64>, subset: &BTreeSet<u64>) -> u64 {
    let mut sum = 0;
    for id in subset {
        let Some(weight) = weights.get(id) else {
            continue;
        };
        sum += weight;
    }
    sum
}

#[test]
fn test_is_quorum_accepts_exactly_the_sets_reaching_the_threshold() {
    // Voter 1 weighs as much as voters 2 and 3 together; the total weight is 4.
    let weights = btreemap! {1 => 2, 2 => 1, 3 => 1};
    let subsets = all_subsets(&btreeset! {1, 2, 3});

    let cases = [
        (1, vec![
            btreeset! {1},
            btreeset! {2},
            btreeset! {1, 2},
            btreeset! {3},
            btreeset! {1, 3},
            btreeset! {2, 3},
            btreeset! {1, 2, 3},
        ]),
        (2, vec![
            btreeset! {1},
            btreeset! {1, 2},
            btreeset! {1, 3},
            btreeset! {2, 3},
            btreeset! {1, 2, 3},
        ]),
        (3, vec![btreeset! {1, 2}, btreeset! {1, 3}, btreeset! {1, 2, 3}]),
        (4, vec![btreeset! {1, 2, 3}]),
        // A threshold above the total weight accepts no set.
        (5, vec![]),
    ];
    for (threshold, want) in cases {
        let rule = weighted(threshold, weights.clone());
        let got = accepted(&rule, &subsets);
        assert_eq!(want, got, "threshold={threshold}");
    }
}

#[test]
fn test_is_quorum_agrees_with_summed_weights() {
    // ID 6 has no weight, so it never changes a result.
    let weights = btreemap! {1 => 5, 2 => 3, 3 => 2, 4 => 1, 5 => 1};
    let subsets = all_subsets(&btreeset! {1, 2, 3, 4, 5, 6});

    for threshold in 1..=13 {
        let rule = weighted(threshold, weights.clone());
        for subset in &subsets {
            let want = weight_of(&weights, subset) >= threshold;
            let got = rule.is_quorum(subset.iter());
            assert_eq!(want, got, "threshold={threshold}, subset={subset:?}");
        }
    }
}

#[test]
fn test_is_quorum_counts_each_voter_once() {
    let rule = weighted(3, btreemap! {1 => 2, 2 => 1, 3 => 1});

    // A repeated ID counts once, and an ID outside the rule adds nothing.
    let repeated = rule.is_quorum([1, 1, 1].iter());
    assert!(!repeated);
    let outside = rule.is_quorum([2, 9, 9, 3].iter());
    assert!(!outside);
    let two_voters = rule.is_quorum([1, 1, 2].iter());
    assert!(two_voters);
    let empty = rule.is_quorum([].iter());
    assert!(!empty);
}

#[test]
fn test_small_rules_keep_the_quorum_set_contract() {
    // Every rule that weighs voters {1,2,3} from 1 to 3, with each threshold from 1 to one above
    // the total weight.
    for w1 in 1..=3 {
        for w2 in 1..=3 {
            for w3 in 1..=3 {
                let weights = btreemap! {1 => w1, 2 => w2, 3 => w3};
                let total_weight = w1 + w2 + w3;
                for threshold in 1..=total_weight + 1 {
                    let rule = weighted(threshold, weights.clone());
                    assert_quorum_set_contract(&rule);
                }
            }
        }
    }
}

#[test]
fn test_is_quorum_never_overflows() {
    // Voters 1 and 2 together weigh more than u64::MAX.
    let rule = weighted(u64::MAX, btreemap! {1 => u64::MAX - 1, 2 => u64::MAX});

    let first = rule.is_quorum([1].iter());
    assert!(!first);
    let both = rule.is_quorum([1, 2].iter());
    assert!(both);
}

#[test]
fn test_new() {
    let weights = btreemap! {1 => 2, 2 => 1};
    let rule = WeightedQuorumSet::new(2, weights.clone()).unwrap();

    let got_weights = rule.weights();
    assert_eq!(&weights, got_weights);
    let threshold = rule.threshold();
    assert_eq!(2, threshold);
    let ids: Vec<u64> = rule.ids().collect();
    assert_eq!(vec![1, 2], ids);

    // A weight of 0 would add a voter that never counts, and a threshold of 0 would make the
    // empty set a quorum.
    let zero_weight = WeightedQuorumSet::new(1, btreemap! {1 => 1, 2 => 0});
    let invalid = InvalidWeights::ZeroWeight { node_id: 2 };
    assert_eq!(Err(invalid), zero_weight);
    let zero_threshold = WeightedQuorumSet::new(0, weights);
    assert_eq!(Err(InvalidWeights::ZeroThreshold), zero_threshold);
}

#[test]
fn test_intersects_with() {
    let weights = btreemap! {1 => 2, 2 => 1, 3 => 1};
    let three = weighted(3, weights.clone());
    let two = weighted(2, weights);

    // 3 + 2 > 4: a quorum weighing 3 and a quorum weighing 2 always share a voter.
    let sum_exceeds_total = three.intersects_with(&two);
    assert_eq!(Some(true), sum_exceeds_total);

    // 2 + 2 = 4: {1} and {2,3} are disjoint quorums. The check proves only intersection.
    let sum_equals_total = two.intersects_with(&two);
    assert_eq!(None, sum_equals_total);

    // With weights {1:2, 2:2}, every quorum weighing 3 holds both voters, so it meets every
    // quorum weighing 1, although 3 + 1 does not exceed the total weight 4.
    let even = btreemap! {1 => 2, 2 => 2};
    let all = weighted(3, even.clone());
    let any = weighted(1, even);
    let exact = verify_intersection(&all, &any);
    assert!(exact);
    let unproven = all.intersects_with(&any);
    assert_eq!(None, unproven);

    // Different weights prove nothing, even where every quorum of each rule holds voter 1.
    let heavier = weighted(4, btreemap! {1 => 3, 2 => 1, 3 => 1});
    let exact = verify_intersection(&three, &heavier);
    assert!(exact);
    let different_weights = three.intersects_with(&heavier);
    assert_eq!(None, different_weights);
}

#[test]
fn test_intersects_with_never_overflows() {
    // The thresholds sum to more than u64::MAX, and they still exceed the total weight.
    let max = weighted(u64::MAX, btreemap! {1 => u64::MAX});
    let intersects = max.intersects_with(&max);
    assert_eq!(Some(true), intersects);

    // The total weight exceeds u64::MAX, and {1} and {2} are disjoint quorums.
    let two_max = weighted(u64::MAX, btreemap! {1 => u64::MAX, 2 => u64::MAX});
    let unproven = two_max.intersects_with(&two_max);
    assert_eq!(None, unproven);
}

#[test]
fn test_intersects_with_proves_only_true_intersections() {
    // Every rule that weighs voters {1,2,3} from 1 to 3, with each threshold from 1 to one above
    // the total weight.
    let mut rules = vec![];
    for w1 in 1..=3 {
        for w2 in 1..=3 {
            for w3 in 1..=3 {
                let weights = btreemap! {1 => w1, 2 => w2, 3 => w3};
                let total_weight = w1 + w2 + w3;
                for threshold in 1..=total_weight + 1 {
                    rules.push((weighted(threshold, weights.clone()), total_weight));
                }
            }
        }
    }

    for (a, total_weight) in &rules {
        for (b, _) in &rules {
            let same_weights = a.weights() == b.weights();
            let thresholds_exceed_total = a.threshold() + b.threshold() > *total_weight;
            let want = if same_weights && thresholds_exceed_total {
                Some(true)
            } else {
                None
            };
            let got = a.intersects_with(b);
            assert_eq!(want, got, "a={a:?}, b={b:?}");

            if got == Some(true) {
                let exact = verify_intersection(a, b);
                assert!(exact, "a={a:?}, b={b:?}");
            }
        }
    }
}

#[cfg(feature = "serde")]
#[test]
fn test_serde() {
    let rule = weighted(3, btreemap! {1 => 2, 2 => 1, 3 => 1});

    let json = serde_json::to_string(&rule).unwrap();
    assert_eq!(r#"{"weights":{"1":2,"2":1,"3":1},"threshold":3}"#, json);
    let decoded = serde_json::from_str::<WeightedQuorumSet<u64>>(&json).unwrap();
    assert_eq!(rule, decoded);

    let binary = bincode::serialize(&rule).unwrap();
    let decoded = bincode::deserialize::<WeightedQuorumSet<u64>>(&binary).unwrap();
    assert_eq!(rule, decoded);
}
