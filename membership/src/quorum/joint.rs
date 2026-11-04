use std::collections::BTreeSet;
use std::collections::btree_set;

use openraft_macros::since;

use crate::quorum::QuorumIntersection;
use crate::quorum::QuorumSet;

/// Parts that apply together, kept in the order they were given.
///
/// A joint of quorum rules is a quorum rule: it accepts an ID set only when every one of its rules
/// accepts it. A membership change can move through a joint of the old and the new rule, whose
/// every quorum is a quorum of both. An empty `Joint` accepts no set, like an empty
/// `Vec<BTreeSet<ID>>`. [`ids()`](QuorumSet::ids) returns the union of the IDs of every rule, each
/// ID once.
///
/// A joint of [`Config`]s is a membership: [`ExplicitMembership`], [`GridMembership`],
/// [`HierarchicalMembership`] and [`WeightedMembership`] are aliases of `Joint<Config<..>>`. It
/// holds one config, or the old configs and the new one during a joint change.
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
/// let joint = Joint::from_rules([old, new]);
///
/// // {1,2} holds 2 of the old IDs but only 1 of the new IDs.
/// let accepts_one_two = joint.is_quorum([1, 2].iter());
/// assert!(!accepts_one_two);
/// let accepts_one_two_four = joint.is_quorum([1, 2, 4].iter());
/// assert!(accepts_one_two_four);
/// ```
///
/// [`Config`]: crate::Config
/// [`ExplicitMembership`]: crate::ExplicitMembership
/// [`GridMembership`]: crate::GridMembership
/// [`HierarchicalMembership`]: crate::HierarchicalMembership
/// [`WeightedMembership`]: crate::WeightedMembership
#[since(version = "0.10.0")]
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct Joint<T> {
    /// The joined parts: quorum rules, or the configs of a membership.
    pub(crate) parts: Vec<T>,
}

impl<Q> Joint<Q>
where Q: QuorumSet
{
    /// Builds a rule that accepts an ID set only when every one of `rules` accepts it.
    pub fn from_rules(rules: impl IntoIterator<Item = Q>) -> Self {
        let parts = rules.into_iter().collect();
        Self { parts }
    }

    /// Returns the rules, in the order they were given.
    pub fn rules(&self) -> &[Q] {
        &self.parts
    }
}

/// Written by hand: `derive(Default)` would require `T: Default`.
impl<T> Default for Joint<T> {
    fn default() -> Self {
        Self { parts: vec![] }
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
        if self.parts.is_empty() {
            return false;
        }

        for rule in &self.parts {
            if !rule.is_quorum(ids.clone()) {
                return false;
            }
        }
        true
    }

    fn ids(&self) -> Self::Iter {
        let mut ids = BTreeSet::new();
        for rule in &self.parts {
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
        let either_is_empty = self.parts.is_empty() || other.parts.is_empty();
        if either_is_empty {
            return Some(true);
        }

        let both_uniform = self.parts.len() == 1 && other.parts.len() == 1;
        if both_uniform {
            return self.parts[0].intersects_with(&other.parts[0]);
        }

        for a in &self.parts {
            for b in &other.parts {
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
impl<T> Joint<T>
where T: Clone
{
    /// Return the parts of `self` followed by the parts of `other`. Return an empty joint when
    /// `self` or `other` is empty: an empty joint has no quorum, so a joint with it has no quorum
    /// either.
    ///
    /// For a joint of configs,
    /// [`RaftMembership::joint_with()`](openraft::membership::RaftMembership::joint_with) builds on
    /// it and also gives each active node one metadata in every config.
    pub fn joint_with(&self, other: &Joint<T>) -> Self {
        if self.parts.is_empty() || other.parts.is_empty() {
            return Self::default();
        }
        let mut parts = self.parts.clone();
        parts.extend_from_slice(&other.parts);
        Self { parts }
    }
}
