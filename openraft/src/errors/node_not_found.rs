use openraft_macros::since;

use crate::errors::Operation;
use crate::node::NodeId;

/// Error indicating a node was not found in the cluster.
#[since(version = "0.10.0", change = "removed `C: RaftTypeConfig` generic parameter")]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[error("Node {node_id} not found when: ({operation})")]
pub struct NodeNotFound<NID>
where NID: NodeId
{
    /// The node ID that was not found.
    pub node_id: NID,
    /// The operation that was being attempted when the node was not found.
    pub operation: Operation,
}

impl<NID> NodeNotFound<NID>
where NID: NodeId
{
    /// Create a new NodeNotFound error.
    pub fn new(node_id: NID, operation: Operation) -> Self {
        Self { node_id, operation }
    }
}
