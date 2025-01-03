//! Implement [`RaftLeaderId`] for protobuf defined LeaderId, so that it can be used in OpenRaft

use std::cmp::Ordering;
use std::fmt;

use openraft::vote::LeaderIdCompare;
use openraft::vote::RaftLeaderId;

use crate::protobuf as pb;
use crate::TypeConfig;

/// Implements PartialOrd for LeaderId to enforce the standard Raft behavior of at most one leader
/// per term.
///
/// In standard Raft, each term can have at most one leader. This is enforced by making leader IDs
/// with the same term incomparable (returning None), unless they refer to the same node.
///
/// This differs from the [`PartialOrd`] default implementation which would allow multiple leaders
/// in the same term by comparing node IDs.
impl PartialOrd for pb::LeaderId {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        LeaderIdCompare::std(self, other)
    }
}

impl fmt::Display for pb::LeaderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "T{}-N{}", self.term, self.node_id)
    }
}

impl RaftLeaderId<TypeConfig> for pb::LeaderId {
    type Committed = u64;

    fn new(term: u64, node_id: u64) -> Self {
        Self { term, node_id }
    }

    fn term(&self) -> u64 {
        self.term
    }

    fn node_id(&self) -> Option<&u64> {
        Some(&self.node_id)
    }

    fn to_committed(&self) -> Self::Committed {
        self.term
    }
}
