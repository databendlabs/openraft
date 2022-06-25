use std::marker::PhantomData;

use crate::quorum::QuorumSet;

/// Use another data as a joint quorum set.
///
/// The ids has to be a quorum in every sub-config to constitute a joint-quorum.
pub(crate) trait AsJoint<'d, ID, QS, D>
where
    ID: 'static,
    QS: QuorumSet<ID>,
{
    fn as_joint(&'d self) -> Joint<ID, QS, D>
    where D: 'd;
}

/// A wrapper that uses other data to define a joint quorum set.
///
/// The input ids has to be a quorum in every sub-config to constitute a joint-quorum.
pub(crate) struct Joint<ID, QS, D>
where
    ID: 'static,
    QS: QuorumSet<ID>,
{
    data: D,
    _p: PhantomData<(ID, QS)>,
}

impl<ID, QS, D> Joint<ID, QS, D>
where
    ID: 'static,
    QS: QuorumSet<ID>,
{
    pub(crate) fn new(data: D) -> Self {
        Self { data, _p: PhantomData }
    }
}

/// Implement QuorumSet for Joint<..&[QS]>
impl<'d, ID, QS> QuorumSet<ID> for Joint<ID, QS, &'d [QS]>
where
    ID: 'static,
    QS: QuorumSet<ID>,
{
    fn is_quorum<'a, I: Iterator<Item = &'a ID> + Clone>(&self, ids: I) -> bool {
        for child in self.data.iter() {
            if !child.is_quorum(ids.clone()) {
                return false;
            }
        }
        true
    }
}

/// Implement QuorumSet for Joint<..Vec<QS>>
impl<ID, QS> QuorumSet<ID> for Joint<ID, QS, Vec<QS>>
where
    ID: 'static,
    QS: QuorumSet<ID>,
{
    fn is_quorum<'a, I: Iterator<Item = &'a ID> + Clone>(&self, ids: I) -> bool {
        for child in self.data.iter() {
            if !child.is_quorum(ids.clone()) {
                return false;
            }
        }
        true
    }
}
