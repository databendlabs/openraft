use std::ops::Deref;
use std::sync::Arc;

use openraft::async_trait::async_trait;
use openraft::error::NodeNotFound;
use openraft::NodeId;

/// Raft itself does not store node addresses.
/// A supporting component is required to provide node-id-to-address mapping.
#[async_trait]
pub trait NodeManager {
    async fn get_node_address(&self, node_id: NodeId) -> Result<String, NodeNotFound>;
}

#[async_trait]
impl<T> NodeManager for T
where T: Fn(NodeId) -> Result<String, NodeNotFound> + Sync
{
    async fn get_node_address(&self, node_id: NodeId) -> Result<String, NodeNotFound> {
        self(node_id)
    }
}

#[async_trait]
impl<NM: NodeManager> NodeManager for Arc<NM> {
    async fn get_node_address(&self, node_id: NodeId) -> Result<String, NodeNotFound> {
        self.as_ref().get_node_address(node_id).await
    }
}
