use std::fmt::Formatter;

use serde::Deserialize;
use serde::Serialize;

use crate::RaftTypeConfig;

/// LeaderId is identifier of a `leader`.
///
/// TODO(xp): this might be changed in future:
/// In raft spec that in a term there is at most one leader, thus a `term` is enough to differentiate leaders.
/// That is why raft uses `(term, index)` to uniquely identify a log entry.
///
/// But under this(dirty and stupid) simplification, a `Leader` is actually identified by `(term, node_id)`.
/// By introducing `LeaderId {term, node_id}`, things become easier to understand.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct LeaderId<C: RaftTypeConfig> {
    pub term: u64,
    pub node_id: C::NodeId,
}

impl<C: RaftTypeConfig> std::fmt::Display for LeaderId<C> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}-{}", self.term, self.node_id)
    }
}

impl<C: RaftTypeConfig> LeaderId<C> {
    pub fn new(term: u64, node_id: C::NodeId) -> Self {
        Self { term, node_id }
    }
}
