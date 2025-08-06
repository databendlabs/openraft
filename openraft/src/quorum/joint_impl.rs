use crate::quorum::AsJoint;
use crate::quorum::Joint;
use crate::quorum::QuorumSet;

/// Use a vec of some implementation of `QuorumSet` as a joint quorum set.
impl<'d, ID, QS> AsJoint<'d, ID, QS, &'d [QS]> for Vec<QS>
where
    ID: 'static,
    QS: QuorumSet<ID>,
{
    fn as_joint(&'d self) -> Joint<ID, QS, &'d [QS]>
    where &'d [QS]: 'd {
        Joint::new(self)
    }
}

impl<ID, QS> From<Vec<QS>> for Joint<ID, QS, Vec<QS>>
where
    ID: 'static,
    QS: QuorumSet<ID>,
{
    fn from(v: Vec<QS>) -> Self {
        Joint::new(v)
    }
}
