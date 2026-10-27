//! Quorum rule generators and checks shared by unit tests.

use std::collections::BTreeSet;
use std::fmt;
use std::num::NonZeroU64;

use crate::quorum::QuorumSet;
use crate::quorum::ThresholdQuorumSet;

/// Returns a rule that accepts a set holding at least `quorum_size` of `ids`.
pub(crate) fn threshold(quorum_size: u64, ids: BTreeSet<u64>) -> ThresholdQuorumSet<u64> {
    let quorum_size = NonZeroU64::new(quorum_size).expect("quorum_size is at least 1");
    ThresholdQuorumSet::new(quorum_size, ids)
}

/// Returns every subset of `ids`, ordered by the bit mask that selects it.
pub(crate) fn all_subsets(ids: &BTreeSet<u64>) -> Vec<BTreeSet<u64>> {
    let ids = ids.iter().copied().collect::<Vec<_>>();
    let mut subsets = vec![];
    for mask in 0..1u64 << ids.len() {
        let selected = ids.iter().enumerate().filter(|(i, _)| mask & (1u64 << i) != 0);
        let subset = selected.map(|(_, id)| *id).collect();
        subsets.push(subset);
    }
    subsets
}

/// Returns every rule over a subset of `ids`, with each `quorum_size` from 1 to one above the
/// subset size. The largest size gives a rule that accepts no set.
pub(super) fn all_thresholds(ids: &BTreeSet<u64>) -> Vec<ThresholdQuorumSet<u64>> {
    let mut thresholds = vec![];
    for subset in all_subsets(ids) {
        let max_quorum_size = subset.len() as u64 + 1;
        for quorum_size in 1..=max_quorum_size {
            thresholds.push(threshold(quorum_size, subset.clone()));
        }
    }
    thresholds
}

/// Returns the sets in `subsets` that `quorum_set` accepts, in their order.
pub(crate) fn accepted<Q>(quorum_set: &Q, subsets: &[BTreeSet<u64>]) -> Vec<BTreeSet<u64>>
where Q: QuorumSet<Id = u64> {
    let mut accepted = vec![];
    for subset in subsets {
        let is_quorum = quorum_set.is_quorum(subset.iter());
        if is_quorum {
            accepted.push(subset.clone());
        }
    }
    accepted
}

/// Asserts the [`QuorumSet`] contract on every subset of the IDs of `quorum_set`: the empty set
/// is no quorum, repeated IDs and an unknown ID change no result, and adding an ID to a quorum
/// keeps it a quorum.
pub(crate) fn assert_quorum_set_contract<Q>(quorum_set: &Q)
where Q: QuorumSet<Id = u64> + fmt::Debug {
    let ids: BTreeSet<u64> = quorum_set.ids().collect();
    let max_id = ids.last().copied();
    let unknown = max_id.unwrap_or(0) + 1;

    let accepts_empty = quorum_set.is_quorum([].iter());
    assert!(!accepts_empty, "{quorum_set:?} accepts the empty set");

    for subset in all_subsets(&ids) {
        let accepts = quorum_set.is_quorum(subset.iter());

        let mut repeated: Vec<u64> = subset.iter().copied().collect();
        repeated.extend(&subset);
        let accepts_repeated = quorum_set.is_quorum(repeated.iter());
        assert_eq!(accepts, accepts_repeated, "{quorum_set:?} on {repeated:?}");

        let mut with_unknown = subset.clone();
        with_unknown.insert(unknown);
        let accepts_with_unknown = quorum_set.is_quorum(with_unknown.iter());
        assert_eq!(accepts, accepts_with_unknown, "{quorum_set:?} on {with_unknown:?}");

        if !accepts {
            continue;
        }
        for id in &ids {
            let mut larger = subset.clone();
            larger.insert(*id);
            let accepts_larger = quorum_set.is_quorum(larger.iter());
            assert!(accepts_larger, "{quorum_set:?} accepts {subset:?} but not {larger:?}");
        }
    }
}
