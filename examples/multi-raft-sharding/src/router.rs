use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;
use std::sync::Mutex;

use openraft::error::Unreachable;
use tokio::sync::oneshot;

use crate::app::RequestTx;
use crate::decode;
use crate::encode;
use crate::typ::RaftError;
use crate::NodeId;
use crate::ShardId;

#[derive(Debug)]
pub struct RouterError(pub String);

impl fmt::Display for RouterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for RouterError {}

/// Unique identifier for a Raft instance (node + shard).
///
/// In Multi-Raft, a single physical node can host multiple Raft instances,
/// one for each shard. This key uniquely identifies each instance.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RouterKey {
    /// The physical node ID.
    pub node_id: NodeId,
    /// The shard (Raft group) ID.
    pub shard_id: ShardId,
}

impl RouterKey {
    pub fn new(node_id: NodeId, shard_id: ShardId) -> Self {
        Self { node_id, shard_id }
    }
}

/// Message router for Multi-Raft communication.
///
/// This router manages the mapping from (node_id, shard_id) to message channels,
/// allowing Raft RPCs to be delivered to the correct shard on each node.
#[derive(Debug, Clone, Default)]
pub struct Router {
    pub targets: Arc<Mutex<BTreeMap<RouterKey, RequestTx>>>,
}

impl Router {
    /// Create a new router.
    pub fn new() -> Self {
        Self {
            targets: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    /// Register a message handler for a specific (node, shard) pair.
    pub fn register(&self, node_id: NodeId, shard_id: ShardId, tx: RequestTx) {
        let key = RouterKey::new(node_id, shard_id);
        let mut targets = self.targets.lock().unwrap();
        targets.insert(key, tx);
    }

    /// Unregister a message handler.
    pub fn unregister(&self, node_id: NodeId, shard_id: &ShardId) -> Option<RequestTx> {
        let key = RouterKey::new(node_id, shard_id.clone());
        let mut targets = self.targets.lock().unwrap();
        targets.remove(&key)
    }

    /// Send a request to a specific (node, shard) pair and wait for response.
    pub async fn send<Req, Resp>(
        &self,
        to_node: NodeId,
        to_shard: &ShardId,
        path: &str,
        req: Req,
    ) -> Result<Resp, Unreachable>
    where
        Req: serde::Serialize,
        Result<Resp, RaftError>: serde::de::DeserializeOwned,
    {
        let (resp_tx, resp_rx) = oneshot::channel();

        let encoded_req = encode(&req);
        tracing::trace!(
            to_node = %to_node,
            to_shard = %to_shard,
            path = %path,
            "sending request"
        );

        {
            let key = RouterKey::new(to_node, to_shard.clone());
            let targets = self.targets.lock().unwrap();
            let tx = targets.get(&key).ok_or_else(|| {
                Unreachable::new(&RouterError(format!(
                    "target not found: node={}, shard={}",
                    to_node, to_shard
                )))
            })?;

            tx.send((path.to_string(), encoded_req, resp_tx))
                .map_err(|e| Unreachable::new(&RouterError(e.to_string())))?;
        }

        let resp_str = resp_rx.await.map_err(|e| Unreachable::new(&RouterError(e.to_string())))?;

        tracing::trace!(
            from_node = %to_node,
            from_shard = %to_shard,
            path = %path,
            "received response"
        );

        let res = decode::<Result<Resp, RaftError>>(&resp_str);
        res.map_err(|e| Unreachable::new(&RouterError(e.to_string())))
    }

    /// Check if a target is registered.
    pub fn has_target(&self, node_id: NodeId, shard_id: &ShardId) -> bool {
        let key = RouterKey::new(node_id, shard_id.clone());
        let targets = self.targets.lock().unwrap();
        targets.contains_key(&key)
    }

    /// Get all registered targets.
    pub fn all_targets(&self) -> Vec<RouterKey> {
        let targets = self.targets.lock().unwrap();
        targets.keys().cloned().collect()
    }

    /// Get the number of registered targets.
    pub fn target_count(&self) -> usize {
        let targets = self.targets.lock().unwrap();
        targets.len()
    }
}
