use std::marker::PhantomData;

use crate::quorum::QuorumSet;

/// Use another data as a joint quorum set.
///
/// The ids has to be a quorum in every sub-config to constitute a joint-quorum.
pub(crate) trait AsJoint<'d, ID, QS>
where
    ID: 'static,
    QS: QuorumSet<ID>,
{
    fn as_joint(&'d self) -> Joint<'d, ID, QS>;
}

/// A wrapper that uses other data to define a joint quorum set.
///
/// The input ids has to be a quorum in every sub-config to constitute a joint-quorum.
pub(crate) struct Joint<'d, ID, QS>
where
    ID: 'static,
    QS: QuorumSet<ID>,
{
    data: &'d [QS],
    _p: PhantomData<ID>,
}

impl<'d, ID, QS> Joint<'d, ID, QS>
where
    ID: 'static,
    QS: QuorumSet<ID>,
{
    pub(crate) fn new(data: &'d [QS]) -> Self {
        Self { data, _p: PhantomData }
    }
}

impl<'d, ID, QS> QuorumSet<ID> for Joint<'d, ID, QS>
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
