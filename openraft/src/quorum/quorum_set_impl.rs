use std::collections::BTreeSet;

use crate::quorum::quorum_set::QuorumSet;
use crate::quorum::util::majority_of;

/// Impl a simple majority quorum set
impl<ID> QuorumSet<ID> for BTreeSet<ID>
where ID: PartialOrd + Ord + 'static
{
    fn is_quorum<'a, I: Iterator<Item = &'a ID> + Clone>(&self, ids: I) -> bool {
        let mut count = 0;
        let majority = majority_of(self.len());
        for id in ids {
            if self.contains(id) {
                count += 1;
                if count >= majority {
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
        let majority = majority_of(self.len());

        // TODO(xp): benchmark these two implementations
        // let mut count = 0;
        // for id in ids {
        //     if self.contains(id) {
        //         count += 1;
        //         if count >= majority {
        //             return true;
        //         }
        //     }
        // }
        // false

        let count = ids.fold(0, |v, id| if self.contains(id) { v + 1 } else { v });
        count >= majority
    }
}

/// Impl joint quorum set.
/// The input ids has to be a quorum in every sub-config to constitute a joint-quorum.
impl<ID, QS> QuorumSet<ID> for Vec<QS>
where
    ID: 'static,
    QS: QuorumSet<ID>,
{
    fn is_quorum<'a, I: Iterator<Item = &'a ID> + Clone>(&self, ids: I) -> bool {
        for child in self.iter() {
            if !child.is_quorum(ids.clone()) {
                return false;
            }
        }
        true
    }
}
