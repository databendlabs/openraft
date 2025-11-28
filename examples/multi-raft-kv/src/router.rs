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
use crate::GroupId;
use crate::NodeId;

#[derive(Debug)]
pub struct RouterError(pub String);

impl fmt::Display for RouterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for RouterError {}

/// Key for identifying a specific Raft instance (node + group)
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RouterKey {
    pub node_id: NodeId,
    pub group_id: GroupId,
}

impl RouterKey {
    pub fn new(node_id: NodeId, group_id: GroupId) -> Self {
        Self { node_id, group_id }
    }
}

/// Simulate a network router for Multi-Raft.
///
/// This router can route messages to specific (node_id, group_id) targets,
/// allowing multiple Raft groups to share the same "network" infrastructure.
#[derive(Debug, Clone, Default)]
pub struct Router {
    /// Map from (node_id, group_id) to the request sender
    pub targets: Arc<Mutex<BTreeMap<RouterKey, RequestTx>>>,
}

impl Router {
    pub fn new() -> Self {
        Self {
            targets: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    pub fn register(&self, node_id: NodeId, group_id: GroupId, tx: RequestTx) {
        let key = RouterKey::new(node_id, group_id);
        let mut targets = self.targets.lock().unwrap();
        targets.insert(key, tx);
    }

    pub fn unregister(&self, node_id: NodeId, group_id: &GroupId) -> Option<RequestTx> {
        let key = RouterKey::new(node_id, group_id.clone());
        let mut targets = self.targets.lock().unwrap();
        targets.remove(&key)
    }

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

        {
            let key = RouterKey::new(to_node, to_group.clone());
            let targets = self.targets.lock().unwrap();
            let tx = targets.get(&key).ok_or_else(|| {
                Unreachable::new(&RouterError(format!(
                    "target not found: node={}, group={}",
                    to_node, to_group
                )))
            })?;

            tx.send((path.to_string(), encoded_req, resp_tx))
                .map_err(|e| Unreachable::new(&RouterError(e.to_string())))?;
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

    pub fn has_target(&self, node_id: NodeId, group_id: &GroupId) -> bool {
        let key = RouterKey::new(node_id, group_id.clone());
        let targets = self.targets.lock().unwrap();
        targets.contains_key(&key)
    }

    pub fn all_targets(&self) -> Vec<RouterKey> {
        let targets = self.targets.lock().unwrap();
        targets.keys().cloned().collect()
    }
}
