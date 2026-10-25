use std::collections::BTreeSet;
use std::collections::btree_set;

use openraft_macros::since;

use crate::quorum::QuorumBridge;
use crate::quorum::QuorumIntersection;
use crate::quorum::QuorumSet;

/// A quorum rule that accepts an ID set only when every one of its rules accepts it.
///
/// A membership change can move through a joint of the old and the new rule, whose every quorum
/// is a quorum of both. An empty `Joint` accepts no set, like an empty `Vec<BTreeSet<ID>>`.
///
/// [`ids()`](QuorumSet::ids) returns the union of the IDs of every rule, each ID once.
///
/// # Examples
///
/// ```
/// use std::collections::BTreeSet;
/// use std::num::NonZeroU64;
///
/// use openraft::membership::QuorumSet;
/// use openraft_membership::quorum::{Joint, ThresholdQuorumSet};
///
/// let two = NonZeroU64::new(2).unwrap();
/// let old = ThresholdQuorumSet::new(two, BTreeSet::from([1, 2, 3]));
/// let new = ThresholdQuorumSet::new(two, BTreeSet::from([2, 3, 4]));
/// let joint = Joint::new([old, new]);
///
/// // {1,2} holds 2 of the old IDs but only 1 of the new IDs.
/// let accepts_one_two = joint.is_quorum([1, 2].iter());
/// assert!(!accepts_one_two);
/// let accepts_one_two_four = joint.is_quorum([1, 2, 4].iter());
/// assert!(accepts_one_two_four);
/// ```
#[since(version = "0.10.0")]
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct Joint<Q> {
    rules: Vec<Q>,
}

impl<Q> Joint<Q> {
    /// Builds a rule that accepts an ID set only when every one of `rules` accepts it.
    pub fn new(rules: impl IntoIterator<Item = Q>) -> Self {
        let rules = rules.into_iter().collect();
        Self { rules }
    }

    /// Returns the rules, in the order they were given.
    pub fn rules(&self) -> &[Q] {
        &self.rules
    }
}

/// Written by hand: `derive(Default)` would require `Q: Default`.
impl<Q> Default for Joint<Q> {
    fn default() -> Self {
        Self { rules: vec![] }
    }
}

impl<Q> QuorumSet for Joint<Q>
where
    Q: QuorumSet,
    Q::Id: Ord,
{
    type Id = Q::Id;
    type Iter = btree_set::IntoIter<Q::Id>;

    fn is_quorum<'a, I>(&self, ids: I) -> bool
    where
        Self::Id: 'a,
        I: Iterator<Item = &'a Self::Id> + Clone,
    {
        if self.rules.is_empty() {
            return false;
        }

        for rule in &self.rules {
            if !rule.is_quorum(ids.clone()) {
                return false;
            }
        }
        true
    }

    fn ids(&self) -> Self::Iter {
        let mut ids = BTreeSet::new();
        for rule in &self.rules {
            ids.extend(rule.ids());
        }
        ids.into_iter()
    }
}

/// Two joints are treated as having quorum intersection when some rule of one intersects some
/// rule of the other: every quorum of a joint is also a quorum of each of its rules, so every
/// quorum of one joint then intersects every quorum of the other. This is sufficient but not
/// necessary for that property, so this check answers `None` for some pairs of joints whose
/// quorums do in fact all intersect.
impl<Q> QuorumIntersection<Joint<Q>> for Joint<Q>
where
    Q: QuorumIntersection<Q>,
    Q::Id: Ord,
{
    /// Return the answer of the rule check when each joint holds one rule. For other joints,
    /// return `Some(true)` when some rule of `self` intersects some rule of `other`.
    ///
    /// When either joint is empty, return `Some(true)`: an empty joint has no quorum, so no pair
    /// of quorums can be disjoint.
    ///
    /// Return `None` for every other pair: two rules that do not intersect prove nothing about
    /// two joints that contain them.
    fn intersects_with(&self, other: &Joint<Q>) -> Option<bool> {
        let either_is_empty = self.rules.is_empty() || other.rules.is_empty();
        if either_is_empty {
            return Some(true);
        }

        let both_uniform = self.rules.len() == 1 && other.rules.len() == 1;
        if both_uniform {
            return self.rules[0].intersects_with(&other.rules[0]);
        }

        for a in &self.rules {
            for b in &other.rules {
                let intersects = a.intersects_with(b);
                if intersects == Some(true) {
                    return Some(true);
                }
            }
        }
        None
    }
}

/// Builds a joint that requires a quorum of both `self` and the target.
impl<Q> QuorumBridge<Joint<Q>> for Joint<Q>
where
    Q: QuorumSet + Clone,
    Q::Id: Ord,
{
    /// Return the rules of `self` followed by the rules of `other`. Return an empty joint when
    /// `self` or `other` is empty: an empty joint has no quorum, so a joint with it has no quorum
    /// either.
    fn joint_with(&self, other: Joint<Q>) -> Self {
        if self.rules.is_empty() || other.rules.is_empty() {
            return Self::default();
        }
        let mut rules = self.rules.clone();
        rules.extend(other.rules);
        Self { rules }
    }
}
