//! Quorum rule generators shared by unit tests.

use std::collections::BTreeSet;
use std::num::NonZeroU64;

use crate::quorum::ThresholdQuorumSet;

/// Returns a rule that accepts a set holding at least `quorum_size` of `ids`.
pub(super) fn threshold(quorum_size: u64, ids: BTreeSet<u64>) -> ThresholdQuorumSet<u64> {
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
