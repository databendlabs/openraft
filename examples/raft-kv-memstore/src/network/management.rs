use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::sync::Arc;

use openraft::NodeInfo as Node;
use openraft::ReadPolicy;
use openraft::alias::LogIdOf;
use openraft::async_runtime::WatchReceiver;
use openraft::errors::decompose::DecomposeResult;
use serde::Deserialize;
use serde::Serialize;

use crate::NodeId;
use crate::TypeConfig;
use crate::app::App;
use crate::typ::ClientWriteError;
use crate::typ::ClientWriteResponse;
use crate::typ::Infallible;
use crate::typ::InitializeError;
use crate::typ::LinearizableReadError;
use crate::typ::RaftMetrics;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddLearnerRequest {
    pub node_id: NodeId,
    pub api_addr: String,
    pub raft_addr: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinearizerData {
    pub node_id: NodeId,
    pub read_log_id: LogIdOf<TypeConfig>,
    pub applied: Option<LogIdOf<TypeConfig>>,
}

// --- Cluster management

/// Add a node as **Learner**.
///
/// A Learner receives log replication from the leader but does not vote.
/// This should be done before adding a node as a member into the cluster
/// (by calling `change-membership`)
pub async fn add_learner(app: Arc<App>, req: AddLearnerRequest) -> Result<ClientWriteResponse, ClientWriteError> {
    let node = Node::new(req.raft_addr, req.api_addr);
    app.raft.add_learner(req.node_id, node, true).await.decompose().unwrap()
}

/// Changes specified learners to members, or remove members.
pub async fn change_membership(
    app: Arc<App>,
    node_ids: BTreeSet<NodeId>,
) -> Result<ClientWriteResponse, ClientWriteError> {
    app.raft.change_membership(node_ids, false).await.decompose().unwrap()
}

/// Initialize a single-node cluster if the `req` is empty vec.
/// Otherwise initialize a cluster with the `req` specified vec of node-id and node-address
pub async fn init(app: Arc<App>, req: Vec<(NodeId, Node)>) -> Result<(), InitializeError> {
    let mut nodes = BTreeMap::new();
    if req.is_empty() {
        nodes.insert(app.id, Node::new(app.raft_addr.clone(), app.api_addr.clone()));
    } else {
        for (id, node) in req.into_iter() {
            nodes.insert(id, node);
        }
    };
    app.raft.initialize(nodes).await.decompose().unwrap()
}

/// Get the latest metrics of the cluster
pub async fn metrics(app: Arc<App>) -> Result<RaftMetrics, Infallible> {
    let metrics = app.raft.metrics().borrow_watched().clone();

    Ok(metrics)
}

pub async fn get_linearizer(app: Arc<App>) -> Result<LinearizerData, LinearizableReadError> {
    let linearizer = app.raft.get_read_linearizer(ReadPolicy::ReadIndex).await.decompose().unwrap()?;

    Ok(LinearizerData {
        node_id: *linearizer.node_id(),
        read_log_id: *linearizer.read_log_id(),
        applied: linearizer.applied().cloned(),
    })
}
