use std::fmt::Formatter;

use serde::Deserialize;
use serde::Serialize;

use crate::NodeId;

/// LeaderId is identifier of a `leader`.
///
/// TODO(xp): this might be changed in future:
/// In raft spec that in a term there is at most one leader, thus a `term` is enough to differentiate leaders.
/// That is why raft uses `(term, index)` to uniquely identify a log entry.
///
/// But under this(dirty and stupid) simplification, a `Leader` is actually identified by `(term, node_id)`.
/// By introducing `LeaderId {term, node_id}`, things become easier to understand.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
// Clear the bound so that serde will generate required bounds.
#[serde(bound = "")]
pub struct LeaderId<NID>
where NID: NodeId
{
    pub term: u64,
    pub node_id: NID,
}

impl<NID: NodeId> std::fmt::Display for LeaderId<NID> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}-{}", self.term, self.node_id)
    }
}

impl<NID: NodeId> LeaderId<NID> {
    pub fn new(term: u64, node_id: NID) -> Self {
        Self { term, node_id }
    }
}
