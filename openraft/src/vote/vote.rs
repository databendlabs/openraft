use std::fmt::Formatter;

use serde::Deserialize;
use serde::Serialize;

use crate::LeaderId;
use crate::RaftTypeConfig;

/// `Vote` represent the privilege of a node.
#[derive(Debug, Clone, Copy, Default, PartialOrd, Ord, PartialEq, Eq, Serialize, Deserialize)]
pub struct Vote<C: RaftTypeConfig> {
    pub term: u64,
    pub node_id: C::NodeId,
    pub committed: bool,
}

impl<C: RaftTypeConfig> std::fmt::Display for Vote<C> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "vote:{}-{}", self.term, self.node_id)
    }
}

impl<C: RaftTypeConfig> Vote<C> {
    pub fn new(term: u64, node_id: C::NodeId) -> Self {
        Self {
            term,
            node_id,
            committed: false,
        }
    }
    pub fn new_committed(term: u64, node_id: C::NodeId) -> Self {
        Self {
            term,
            node_id,
            committed: true,
        }
    }

    pub fn commit(&mut self) {
        self.committed = true
    }

    pub fn leader_id(&self) -> LeaderId<C> {
        LeaderId::new(self.term, self.node_id)
    }

    /// Get the leader node id.
    ///
    /// Only when a committed vote is seen(granted by a quorum) the voted node id is a valid leader.
    pub fn leader(&self) -> Option<C::NodeId> {
        if self.committed {
            Some(self.node_id)
        } else {
            None
        }
    }
}
