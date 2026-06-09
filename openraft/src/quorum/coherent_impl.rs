use std::collections::BTreeSet;

use crate::quorum::Coherent;
use crate::quorum::coherent::FindCoherent;

/// Two joint configs are coherent iff they share at least one config: then every quorum of one
/// intersects every quorum of the other.
impl<NID> Coherent<NID, Vec<BTreeSet<NID>>> for Vec<BTreeSet<NID>>
where NID: PartialOrd + Ord + Clone + 'static
{
    /// Check if two `joint` are coherent.
    ///
    /// Read more about:
    /// [Extended membership change](crate::docs::data::extended_membership)
    fn is_coherent_with(&self, other: &Vec<BTreeSet<NID>>) -> bool {
        for a in self {
            for b in other {
                if a == b {
                    return true;
                }
            }
        }
        false
    }
}

/// Impl to build an intermediate quorum set that is coherent with a joint and a uniform quorum set.
impl<NID> FindCoherent<NID, BTreeSet<NID>> for Vec<BTreeSet<NID>>
where NID: PartialOrd + Ord + Clone + 'static
{
    fn find_coherent(&self, other: BTreeSet<NID>) -> Self {
        if self.is_coherent_with(&vec![other.clone()]) {
            vec![other]
        } else if let Some(last) = self.last() {
            vec![last.clone(), other]
        } else {
            vec![other]
        }
    }
}
