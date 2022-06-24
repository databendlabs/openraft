use crate::quorum::AsJoint;
use crate::quorum::Joint;
use crate::quorum::QuorumSet;

/// Use a vec of some impl of `QuorumSet` as a joint quorum set.
impl<'d, ID, QS> AsJoint<'d, ID, QS> for Vec<QS>
where
    ID: 'static,
    QS: QuorumSet<ID>,
{
    fn as_joint(&'d self) -> Joint<'d, ID, QS> {
        Joint::new(self)
    }
}
