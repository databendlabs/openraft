use crate::RaftTypeConfig;
use crate::error::Operation;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[error("Node {node_id} not found when: ({operation})")]
pub struct NodeNotFound<C: RaftTypeConfig> {
    pub node_id: C::NodeId,
    pub operation: Operation,
}

impl<C: RaftTypeConfig> NodeNotFound<C> {
    pub fn new(node_id: C::NodeId, operation: Operation) -> Self {
        Self { node_id, operation }
    }
}
