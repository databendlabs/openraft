use std::collections::BTreeSet;
use std::collections::btree_set;

use openraft_macros::since;

use crate::quorum::QuorumIntersection;
use crate::quorum::QuorumSet;

/// A quorum rule that accepts an ID set holding every ID of one of its listed quorums.
///
/// The rule keeps only the minimal quorums. [`new()`](Self::new) sorts the list, drops duplicate
/// quorums, and drops each quorum that holds another listed quorum, because every set that holds
/// the larger quorum also holds the smaller one. Two rules that accept the same sets therefore
/// hold the same list.
///
/// Every accepted set holds a listed quorum, so two rules have quorum intersection exactly when
/// every listed quorum of one meets every listed quorum of the other. The check compares each
/// pair of listed quorums. A rule with no listed quorum accepts no set.
///
/// Listing quorums costs memory that grows with the policy. Some policies need exponentially many
/// minimal quorums: a majority of `n` voters has `C(n, n/2 + 1)` of them, 126 for 9 voters and
/// 6435 for 15.
///
/// # Examples
///
/// ```
/// use std::collections::BTreeSet;
///
/// use openraft::membership::{QuorumIntersection, QuorumSet};
/// use openraft_membership::quorum::ExplicitQuorumSet;
///
/// // {1,2,3} holds {1,2}, so the rule keeps only {1,2} and {3,4}.
/// let quorums = [BTreeSet::from([1, 2, 3]), BTreeSet::from([3, 4]), BTreeSet::from([1, 2])];
/// let rule = ExplicitQuorumSet::new(quorums).unwrap();
/// assert_eq!(&[BTreeSet::from([1, 2]), BTreeSet::from([3, 4])], rule.quorums());
///
/// let accepts = rule.is_quorum([4, 3, 9].iter());
/// assert!(accepts);
/// let accepts = rule.is_quorum([1, 3].iter());
/// assert!(!accepts);
///
/// // {3,4} misses {1,2}.
/// let intersects = rule.intersects_with(&rule);
/// assert_eq!(Some(false), intersects);
/// ```
#[since(version = "0.10.0")]
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Deserialize, serde::Serialize),
    serde(bound(deserialize = "ID: Ord + Clone + serde::Deserialize<'de>"))
)]
pub struct ExplicitQuorumSet<ID> {
    /// The minimal quorums, sorted.
    quorums: Vec<BTreeSet<ID>>,
}

impl<ID> ExplicitQuorumSet<ID>
where ID: Ord + Clone
{
    /// Builds a rule that accepts an ID set holding every ID of one of `quorums`.
    ///
    /// Returns `None` when a listed quorum is empty: every set holds the empty set, so every set,
    /// the empty one included, would be a quorum.
    pub fn new<I>(quorums: I) -> Option<Self>
    where I: IntoIterator<Item = BTreeSet<ID>> {
        let mut quorums: Vec<BTreeSet<ID>> = quorums.into_iter().collect();
        let has_empty_quorum = quorums.iter().any(|quorum| quorum.is_empty());
        if has_empty_quorum {
            return None;
        }

        quorums.sort();
        quorums.dedup();

        let mut minimal = Vec::new();
        for quorum in &quorums {
            let holds_other = quorums.iter().any(|other| other != quorum && other.is_subset(quorum));
            if holds_other {
                continue;
            }
            minimal.push(quorum.clone());
        }

        let rule = Self { quorums: minimal };
        Some(rule)
    }

    /// Returns the minimal quorums, sorted.
    pub fn quorums(&self) -> &[BTreeSet<ID>] {
        &self.quorums
    }

    /// Returns the first listed quorum of `self` and of `other` that share no ID, if any.
    pub(crate) fn disjoint_quorums<'a>(&'a self, other: &'a Self) -> Option<(&'a BTreeSet<ID>, &'a BTreeSet<ID>)> {
        for quorum in &self.quorums {
            for other_quorum in &other.quorums {
                if quorum.is_disjoint(other_quorum) {
                    return Some((quorum, other_quorum));
                }
            }
        }
        None
    }
}

impl<ID> QuorumSet for ExplicitQuorumSet<ID>
where ID: Ord + Clone
{
    type Id = ID;
    type Iter = btree_set::IntoIter<ID>;

    fn is_quorum<'a, I>(&self, ids: I) -> bool
    where
        ID: 'a,
        I: Iterator<Item = &'a ID> + Clone,
    {
        let candidates: BTreeSet<&ID> = ids.collect();
        for quorum in &self.quorums {
            let holds_quorum = quorum.iter().all(|id| candidates.contains(id));
            if holds_quorum {
                return true;
            }
        }
        false
    }

    fn ids(&self) -> Self::Iter {
        let mut ids = BTreeSet::new();
        for quorum in &self.quorums {
            ids.extend(quorum.iter().cloned());
        }
        ids.into_iter()
    }
}

/// Exact quorum intersection for two lists of minimal quorums.
///
/// Every quorum of a rule holds one of its listed quorums, and every listed quorum is a quorum.
/// Two quorums are therefore disjoint only when two listed quorums are, so the check is exact.
/// It runs in `O(a * b * n)` for `a` and `b` listed quorums of at most `n` IDs.
impl<ID> QuorumIntersection<ExplicitQuorumSet<ID>> for ExplicitQuorumSet<ID>
where ID: Ord + Clone
{
    /// Return `Some(false)` when a listed quorum of `self` and one of `other` are disjoint, and
    /// `Some(true)` otherwise. This check never returns `None`.
    ///
    /// A rule with no listed quorum has no quorum, so it has quorum intersection with every rule.
    fn intersects_with(&self, other: &ExplicitQuorumSet<ID>) -> Option<bool> {
        let disjoint = self.disjoint_quorums(other);
        Some(disjoint.is_none())
    }
}
