use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;
use std::sync::Mutex;

use openraft::error::Unreachable;
use tokio::sync::oneshot;

use crate::decode;
use crate::encode;
use crate::typ::RaftError;
use crate::GroupId;
use crate::NodeId;

pub type NodeTx = tokio::sync::mpsc::UnboundedSender<NodeMessage>;
pub type NodeRx = tokio::sync::mpsc::UnboundedReceiver<NodeMessage>;

#[derive(Debug)]
pub struct RouterError(pub String);

impl fmt::Display for RouterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for RouterError {}

/// Message sent through a node connection, containing group_id for routing.
pub struct NodeMessage {
    pub group_id: GroupId,
    pub path: String,
    pub payload: String,
    pub response_tx: oneshot::Sender<String>,
}

/// Multi-Raft Router with per-node connection sharing.
#[derive(Debug, Clone, Default)]
pub struct Router {
    /// Map from node_id to node connection.
    /// All groups on the same node share this connection.
    pub nodes: Arc<Mutex<BTreeMap<NodeId, NodeTx>>>,
}

impl Router {
    pub fn new() -> Self {
        Self {
            nodes: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    /// Register a node connection. All groups on this node will use this connection.
    pub fn register_node(&self, node_id: NodeId, tx: NodeTx) {
        let mut nodes = self.nodes.lock().unwrap();
        nodes.insert(node_id, tx);
    }

    /// Unregister a node connection.
    pub fn unregister_node(&self, node_id: NodeId) -> Option<NodeTx> {
        let mut nodes = self.nodes.lock().unwrap();
        nodes.remove(&node_id)
    }

    /// Send a request to a specific (node, group).
    pub async fn send<Req, Resp>(
        &self,
        to_node: NodeId,
        to_group: &GroupId,
        path: &str,
        req: Req,
    ) -> Result<Resp, Unreachable>
    where
        Req: serde::Serialize,
        Result<Resp, RaftError>: serde::de::DeserializeOwned,
    {
        let (resp_tx, resp_rx) = oneshot::channel();

        let encoded_req = encode(&req);
        tracing::debug!(
            "send to: node={}, group={}, path={}, req={}",
            to_node,
            to_group,
            path,
            encoded_req
        );

        // Send through the shared node connection
        {
            let nodes = self.nodes.lock().unwrap();
            let tx = nodes
                .get(&to_node)
                .ok_or_else(|| Unreachable::new(&RouterError(format!("node {} not connected", to_node))))?;

            let msg = NodeMessage {
                group_id: to_group.clone(),
                path: path.to_string(),
                payload: encoded_req,
                response_tx: resp_tx,
            };

            tx.send(msg).map_err(|e| Unreachable::new(&RouterError(e.to_string())))?;
        }

        let resp_str = resp_rx.await.map_err(|e| Unreachable::new(&RouterError(e.to_string())))?;
        tracing::debug!(
            "resp from: node={}, group={}, path={}, resp={}",
            to_node,
            to_group,
            path,
            resp_str
        );

        let res = decode::<Result<Resp, RaftError>>(&resp_str);
        res.map_err(|e| Unreachable::new(&RouterError(e.to_string())))
    }

    pub fn has_node(&self, node_id: NodeId) -> bool {
        let nodes = self.nodes.lock().unwrap();
        nodes.contains_key(&node_id)
    }
}
