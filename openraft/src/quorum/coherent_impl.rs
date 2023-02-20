use crate::quorum::coherent::FindCoherent;
use crate::quorum::Coherent;
use crate::quorum::Joint;
use crate::quorum::QuorumSet;

impl<ID, QS> Coherent<ID, Joint<ID, QS, Vec<QS>>> for Joint<ID, QS, Vec<QS>>
where
    ID: PartialOrd + Ord + 'static,
    QS: QuorumSet<ID> + PartialEq,
{
    /// Check if two `joint` are coherent.
    ///
    /// Read more about:
    /// [safe-membership-change](https://datafuselabs.github.io/openraft/dynamic-membership.html#the-safe-to-relation)
    fn is_coherent_with(&self, other: &Joint<ID, QS, Vec<QS>>) -> bool {
        for a in self.children() {
            for b in other.children() {
                if a == b {
                    return true;
                }
            }
        }

        false
    }
}

impl<ID, QS> Coherent<ID, Joint<ID, QS, &[QS]>> for Joint<ID, QS, &[QS]>
where
    ID: PartialOrd + Ord + 'static,
    QS: QuorumSet<ID> + PartialEq,
{
    fn is_coherent_with(&self, other: &Joint<ID, QS, &[QS]>) -> bool {
        for a in self.children().iter() {
            for b in other.children().iter() {
                if a == b {
                    return true;
                }
            }
        }

        false
    }
}

impl<ID, QS> Coherent<ID, QS> for Joint<ID, QS, Vec<QS>>
where
    ID: PartialOrd + Ord + 'static,
    QS: QuorumSet<ID> + PartialEq,
{
    fn is_coherent_with(&self, other: &QS) -> bool {
        for a in self.children().iter() {
            if a == other {
                return true;
            }
        }

        false
    }
}

/// Impl to build a intermediate quorum set that is coherent with a joint and a uniform quorum set.
impl<ID, QS> FindCoherent<ID, QS> for Joint<ID, QS, Vec<QS>>
where
    ID: PartialOrd + Ord + 'static,
    QS: QuorumSet<ID> + PartialEq + Clone,
{
    fn find_coherent(&self, other: QS) -> Self {
        if self.is_coherent_with(&other) {
            Joint::from(vec![other])
        } else {
            Joint::from(vec![self.children().last().unwrap().clone(), other])
        }
    }
}
