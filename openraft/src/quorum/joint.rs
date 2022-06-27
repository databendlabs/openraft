use std::collections::BTreeSet;
use std::marker::PhantomData;

use maplit::btreeset;

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
impl<'d, ID, QS, D> QuorumSet<ID> for Joint<ID, QS, D>
where
    ID: PartialOrd + Ord + 'static,
    QS: QuorumSet<ID>,
    D: AsRef<[QS]>,
{
    fn is_quorum<'a, I: Iterator<Item = &'a ID> + Clone>(&self, ids: I) -> bool {
        todo!()
    }

    fn ids(&self) -> BTreeSet<ID> {
        todo!()
    }
}

/// Implement QuorumSet for Joint<..&[QS]>
impl<'d, ID, QS> QuorumSet<ID> for Joint<ID, QS, BTreeSet<ID>>
where
    ID: PartialOrd + Ord + 'static,
    QS: QuorumSet<ID>,
{
    fn is_quorum<'a, I: Iterator<Item = &'a ID> + Clone>(&self, ids: I) -> bool {
        todo!()
    }

    fn ids(&self) -> BTreeSet<ID> {
        todo!()
    }
}
