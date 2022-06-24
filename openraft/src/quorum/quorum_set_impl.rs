use std::collections::BTreeSet;

use crate::quorum::quorum_set::QuorumSet;

/// Impl a simple majority quorum set
impl<ID> QuorumSet<ID> for BTreeSet<ID>
where ID: PartialOrd + Ord + 'static
{
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
}

/// Impl a simple majority quorum set
impl<ID> QuorumSet<ID> for Vec<ID>
where ID: PartialEq + 'static
{
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
}

/// Impl a simple majority quorum set
impl<ID> QuorumSet<ID> for &[ID]
where ID: PartialEq + 'static
{
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
}
