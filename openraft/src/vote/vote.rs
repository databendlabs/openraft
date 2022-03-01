use std::fmt::Formatter;

use serde::Deserialize;
use serde::Serialize;

use crate::LeaderId;
use crate::NodeId;

/// `Vote` represent the privilege of a node.
#[derive(Debug, Clone, Copy, Default, PartialOrd, Ord, PartialEq, Eq, Serialize, Deserialize)]
#[serde(bound = "")]
pub struct Vote<NID: NodeId> {
    pub term: u64,
    pub node_id: NID,
    pub committed: bool,
}

impl<NID: NodeId> std::fmt::Display for Vote<NID> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "vote:{}-{}", self.term, self.node_id)
    }
}

impl<NID: NodeId> Vote<NID> {
    pub fn new(term: u64, node_id: NID) -> Self {
        Self {
            term,
            node_id,
            committed: false,
        }
    }
    pub fn new_committed(term: u64, node_id: NID) -> Self {
        Self {
            term,
            node_id,
            committed: true,
        }
    }

    pub fn commit(&mut self) {
        self.committed = true
    }

    pub fn leader_id(&self) -> LeaderId<NID> {
        LeaderId::new(self.term, self.node_id)
    }

    /// Get the leader node id.
    ///
    /// Only when a committed vote is seen(granted by a quorum) the voted node id is a valid leader.
    pub fn leader(&self) -> Option<NID> {
        if self.committed {
            Some(self.node_id)
        } else {
            None
        }
    }
}
