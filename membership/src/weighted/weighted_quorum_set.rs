use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::btree_set;

use openraft::NodeId;
use openraft_macros::since;

use crate::errors::InvalidWeights;
use crate::quorum::QuorumIntersection;
use crate::quorum::QuorumSet;

/// A quorum rule that accepts an ID set whose IDs weigh at least `threshold` together.
///
/// Each ID of the rule has a positive weight. A set counts the weight of each of its IDs once,
/// and an ID that the rule does not weigh adds nothing. A sum that would exceed `u64::MAX` exceeds
/// every threshold, so [`is_quorum()`](QuorumSet::is_quorum) saturates instead of overflowing. A
/// `threshold` above the total weight gives a rule that accepts no set.
///
/// Two disjoint sets together weigh at most the total weight. So two rules with the same weights
/// have quorum intersection when their thresholds sum to more than the total weight. This
/// condition is sufficient but not necessary, because a set adds all of a voter's weight or none
/// of it. With weights `{1:2, 2:2}`, thresholds 3 and 1 sum to only the total weight 4, yet the
/// only quorum of threshold 3, `{1,2}`, meets every quorum of threshold 1.
///
/// # Examples
///
/// ```
/// use std::collections::BTreeMap;
///
/// use openraft::membership::{QuorumIntersection, QuorumSet};
/// use openraft_membership::quorum::WeightedQuorumSet;
///
/// // Voter 1 weighs as much as voters 2 and 3 together; the total weight is 4.
/// let weights = BTreeMap::from([(1, 2), (2, 1), (3, 1)]);
/// let read = WeightedQuorumSet::new(3, weights.clone()).unwrap();
/// let write = WeightedQuorumSet::new(2, weights).unwrap();
///
/// let write_accepts_one = write.is_quorum([1].iter());
/// assert!(write_accepts_one);
/// let read_accepts_two_three = read.is_quorum([2, 3].iter());
/// assert!(!read_accepts_two_three);
///
/// // 3 + 2 > 4, so every read quorum intersects every write quorum.
/// let intersects = read.intersects_with(&write);
/// assert_eq!(Some(true), intersects);
/// ```
#[since(version = "0.10.0")]
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Deserialize, serde::Serialize),
    serde(bound(deserialize = "ID: NodeId"))
)]
pub struct WeightedQuorumSet<ID> {
    weights: BTreeMap<ID, u64>,
    threshold: u64,
}

impl<ID> WeightedQuorumSet<ID>
where ID: NodeId
{
    /// Builds a rule that accepts an ID set whose IDs weigh at least `threshold` together, with
    /// the weight of each ID in `weights`.
    ///
    /// Returns [`InvalidWeights::ZeroWeight`] for the first ID with weight 0, which would never
    /// count, and [`InvalidWeights::ZeroThreshold`] for a `threshold` of 0, which would make the
    /// empty set a quorum.
    pub fn new(threshold: u64, weights: BTreeMap<ID, u64>) -> Result<Self, InvalidWeights<ID>> {
        for (node_id, weight) in weights.iter() {
            if *weight == 0 {
                let node_id = node_id.clone();
                return Err(InvalidWeights::ZeroWeight { node_id });
            }
        }
        if threshold == 0 {
            return Err(InvalidWeights::ZeroThreshold);
        }

        let rule = Self { weights, threshold };
        Ok(rule)
    }

    /// Returns the weight of each ID. [`QuorumSet::ids()`] yields its IDs.
    pub fn weights(&self) -> &BTreeMap<ID, u64> {
        &self.weights
    }

    /// Returns the weight that an accepted set must reach.
    pub fn threshold(&self) -> u64 {
        self.threshold
    }
}

impl<ID> QuorumSet for WeightedQuorumSet<ID>
where ID: NodeId
{
    type Id = ID;
    type Iter = btree_set::IntoIter<ID>;

    fn is_quorum<'a, I>(&self, ids: I) -> bool
    where
        ID: 'a,
        I: Iterator<Item = &'a ID> + Clone,
    {
        let threshold = self.threshold();
        let mut sum = 0u64;
        for (id, weight) in self.weights.iter() {
            if ids.clone().any(|candidate| candidate == id) {
                sum = sum.saturating_add(*weight);
                if sum >= threshold {
                    return true;
                }
            }
        }
        false
    }

    fn ids(&self) -> Self::Iter {
        let keys = self.weights.keys();
        let ids: BTreeSet<ID> = keys.cloned().collect();
        ids.into_iter()
    }
}

/// Sufficient quorum intersection for two weighted rules.
///
/// Two disjoint sets together weigh at most the total weight. When two rules hold the same weights
/// and their thresholds sum to more than the total weight, a quorum of one and a quorum of the
/// other are therefore never disjoint. The check runs in O(n) for `n` IDs.
impl<ID> QuorumIntersection<WeightedQuorumSet<ID>> for WeightedQuorumSet<ID>
where ID: NodeId
{
    /// Return `Some(true)` when both rules hold the same weights and their thresholds sum to more
    /// than the total weight, and `None` otherwise, including for every pair of rules with
    /// different weights. This check never returns `Some(false)`.
    ///
    /// The sums are computed in `u128`, so they never overflow.
    fn intersects_with(&self, other: &WeightedQuorumSet<ID>) -> Option<bool> {
        if self.weights != other.weights {
            return None;
        }

        let mut total_weight = 0u128;
        for weight in self.weights.values() {
            total_weight += u128::from(*weight);
        }
        let thresholds = u128::from(self.threshold()) + u128::from(other.threshold());

        if thresholds > total_weight { Some(true) } else { None }
    }
}
