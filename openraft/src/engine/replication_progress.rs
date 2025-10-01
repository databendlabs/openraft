use std::fmt;

use crate::RaftTypeConfig;
use crate::progress::entry::ProgressEntry;

#[derive(Debug)]
#[derive(PartialEq, Eq)]
pub(crate) struct ReplicationProgress<C: RaftTypeConfig>(pub C::NodeId, pub ProgressEntry<C>);

impl<C> fmt::Display for ReplicationProgress<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ReplicationProgress({}={})", self.0, self.1)
    }
}
