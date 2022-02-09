use std::fmt::Formatter;

use serde::Deserialize;
use serde::Serialize;

use crate::LeaderId;
use crate::NodeId;

/// `Vote` represent the privilege of a node.
#[derive(Debug, Clone, Copy, Default, PartialOrd, Ord, PartialEq, Eq, Serialize, Deserialize)]
pub struct Vote {
    pub term: u64,
    pub node_id: NodeId,
    pub committed: bool,
}

impl std::fmt::Display for Vote {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "vote:{}-{}", self.term, self.node_id)
    }
}

impl Vote {
    pub fn new(term: u64, node_id: u64) -> Self {
        Self {
            term,
            node_id,
            committed: false,
        }
    }
    pub fn new_committed(term: u64, node_id: u64) -> Self {
        Self {
            term,
            node_id,
            committed: true,
        }
    }

    pub fn commit(&mut self) {
        self.committed = true
    }

    pub fn leader_id(&self) -> LeaderId {
        LeaderId::new(self.term, self.node_id)
    }

    /// Get the leader node id.
    ///
    /// Only when a committed vote is seen(granted by a quorum) the voted node id is a valid leader.
    pub fn leader(&self) -> Option<NodeId> {
        if self.committed {
            Some(self.node_id)
        } else {
            None
        }
    }
}
