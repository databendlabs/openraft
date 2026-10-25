use std::collections::BTreeSet;
use std::collections::btree_set;
use std::num::NonZeroU64;
use std::sync::Arc;

use openraft_macros::since;

use crate::quorum::QuorumIntersection;
use crate::quorum::QuorumSet;

/// A quorum rule that accepts an ID set holding at least `quorum_size` of its IDs.
///
/// A read rule and a write rule over the same voters can share one ID set: build both from one
/// [`Arc`]. Two rules over the same IDs have quorum intersection exactly when the sum of their
/// quorum sizes exceeds the number of IDs.
///
/// With the `serde` feature, decoding gives each rule its own ID set, even when the encoded rules
/// shared one.
///
/// # Examples
///
/// ```
/// use std::collections::BTreeSet;
/// use std::num::NonZeroU64;
/// use std::sync::Arc;
///
/// use openraft::membership::{QuorumIntersection, QuorumSet};
/// use openraft_membership::ThresholdQuorumSet;
///
/// let voters = Arc::new(BTreeSet::from([1, 2, 3, 4, 5]));
/// let read = ThresholdQuorumSet::new(NonZeroU64::new(2).unwrap(), voters.clone());
/// let write = ThresholdQuorumSet::new(NonZeroU64::new(4).unwrap(), voters);
///
/// let read_accepts_two = read.is_quorum([1, 2].iter());
/// assert!(read_accepts_two);
/// let write_accepts_three = write.is_quorum([1, 2, 3].iter());
/// assert!(!write_accepts_three);
///
/// // 2 + 4 > 5, so every read quorum intersects every write quorum.
/// let intersects = read.intersects_with(&write);
/// assert_eq!(Some(true), intersects);
///
/// let shared = Arc::ptr_eq(read.id_set(), write.id_set());
/// assert!(shared);
/// ```
#[since(version = "0.10.0")]
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Deserialize, serde::Serialize),
    serde(bound(deserialize = "ID: Ord + serde::Deserialize<'de>"))
)]
pub struct ThresholdQuorumSet<ID> {
    ids: Arc<BTreeSet<ID>>,
    quorum_size: NonZeroU64,
}

impl<ID> ThresholdQuorumSet<ID> {
    /// Builds a rule that accepts an ID set holding at least `quorum_size` of `ids`.
    ///
    /// `ids` is a [`BTreeSet`], or an [`Arc`] of one to share it with another rule. A
    /// `quorum_size` above the number of `ids` gives a rule that accepts no set.
    pub fn new(quorum_size: NonZeroU64, ids: impl Into<Arc<BTreeSet<ID>>>) -> Self {
        let ids = ids.into();
        Self { ids, quorum_size }
    }

    /// Returns the shared ID set. [`QuorumSet::ids()`] yields the same IDs as an iterator.
    pub fn id_set(&self) -> &Arc<BTreeSet<ID>> {
        &self.ids
    }

    /// Returns the number of IDs that an accepted set must hold.
    pub fn quorum_size(&self) -> u64 {
        self.quorum_size.get()
    }
}

impl<ID> QuorumSet for ThresholdQuorumSet<ID>
where ID: Ord + Clone
{
    type Id = ID;
    type Iter = btree_set::IntoIter<ID>;

    fn is_quorum<'a, I>(&self, ids: I) -> bool
    where
        ID: 'a,
        I: Iterator<Item = &'a ID> + Clone,
    {
        let quorum_size = self.quorum_size();
        let mut count = 0;
        for id in self.ids.iter() {
            if ids.clone().any(|candidate| candidate == id) {
                count += 1;
                if count >= quorum_size {
                    return true;
                }
            }
        }
        false
    }

    fn ids(&self) -> Self::Iter {
        let ids = self.ids.as_ref().clone();
        ids.into_iter()
    }
}

/// Exact quorum intersection for two threshold rules.
///
/// A smallest quorum of `self` first takes the IDs that `other` lacks, and then takes shared IDs
/// only as needed. A smallest quorum of `other` does the same. Two disjoint quorums exist exactly
/// when the shared IDs are enough for both, so the check is exact and runs in O(n).
impl<ID> QuorumIntersection<ThresholdQuorumSet<ID>> for ThresholdQuorumSet<ID>
where ID: Ord + Clone
{
    /// Return `Some(true)` when every quorum of `self` intersects every quorum of `other`, and
    /// `Some(false)` otherwise. This check never returns `None`.
    ///
    /// A rule whose `quorum_size` exceeds its number of IDs has no quorum, so it has quorum
    /// intersection with every rule.
    fn intersects_with(&self, other: &ThresholdQuorumSet<ID>) -> Option<bool> {
        let only_self = self.ids.difference(&other.ids).count() as u64;
        let only_other = other.ids.difference(&self.ids).count() as u64;
        let shared = self.ids.intersection(&other.ids).count() as u64;

        let need_self = self.quorum_size().saturating_sub(only_self);
        let need_other = other.quorum_size().saturating_sub(only_other);
        let need_both = need_self.saturating_add(need_other);

        Some(need_both > shared)
    }
}
