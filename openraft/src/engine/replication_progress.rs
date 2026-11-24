use std::fmt;

use crate::RaftTypeConfig;
use crate::progress::entry::ProgressEntry;

/// Replication progress state for a single target node.
#[derive(Debug)]
#[derive(PartialEq, Eq)]
pub(crate) struct TargetProgress<C: RaftTypeConfig> {
    /// The node info of the replication target.
    pub(crate) target_node: C::Node,

    /// The current replication progress to this target. Its `id` is the target's node ID.
    pub(crate) progress: ProgressEntry<C>,
}

impl<C> fmt::Display for TargetProgress<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TargetProgress({}={})", self.progress.id, self.progress)
    }
}
