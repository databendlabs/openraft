use std::collections::BTreeSet;

use crate::quorum::quorum_set::QuorumSet;

/// Impl a simple majority quorum set
impl<ID> QuorumSet for BTreeSet<ID>
where ID: PartialOrd + Ord + Clone + 'static
{
    type Id = ID;
    type Iter = std::collections::btree_set::IntoIter<ID>;

    fn is_quorum<'a, I: Iterator<Item = &'a ID> + Clone>(&self, ids: I) -> bool {
        let mut count = 0;
        let limit = self.len();
        for id in ids {
            if self.contains(id) {
                count += 2;
                if count > limit {
                    return true;
                }
            }
        }
        false
    }

    fn ids(&self) -> Self::Iter {
        self.clone().into_iter()
    }
}

/// Impl a joint quorum set: a node set is a quorum iff it is a majority in every config.
impl<NID> QuorumSet for Vec<BTreeSet<NID>>
where NID: PartialOrd + Ord + Clone + 'static
{
    type Id = NID;
    type Iter = std::collections::btree_set::IntoIter<NID>;

    fn is_quorum<'a, I: Iterator<Item = &'a NID> + Clone>(&self, ids: I) -> bool {
        for config in self {
            if !config.is_quorum(ids.clone()) {
                return false;
            }
        }
        true
    }

    fn ids(&self) -> Self::Iter {
        let mut ids = BTreeSet::new();
        for config in self {
            ids.extend(config.iter().cloned());
        }
        ids.into_iter()
    }
}

/// Impl a simple majority quorum set
impl<ID> QuorumSet for &[ID]
where ID: PartialOrd + Ord + Copy + 'static
{
    type Id = ID;
    type Iter = std::collections::btree_set::IntoIter<ID>;

    fn is_quorum<'a, I: Iterator<Item = &'a ID> + Clone>(&self, ids: I) -> bool {
        let mut count = 0;
        let limit = self.len();
        for id in ids {
            if self.contains(id) {
                count += 2;
                if count > limit {
                    return true;
                }
            }
        }
        false
    }

    fn ids(&self) -> Self::Iter {
        BTreeSet::from_iter(self.iter().copied()).into_iter()
    }
}
