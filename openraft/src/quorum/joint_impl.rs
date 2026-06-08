use crate::quorum::Joint;
use crate::quorum::QuorumSet;

impl<ID, QS> From<Vec<QS>> for Joint<ID, QS, Vec<QS>>
where
    ID: 'static,
    QS: QuorumSet<Id = ID>,
{
    fn from(v: Vec<QS>) -> Self {
        Joint::new(v)
    }
}
