use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::sync::Arc;

use openraft::error::Infallible;
use openraft::RaftMetrics;
use tide::Body;
use tide::Request;
use tide::Response;
use tide::StatusCode;

use crate::app::App;
use crate::Node;
use crate::NodeId;
use crate::Server;

// --- Cluster management

pub fn rest(app: &mut Server) {
    let mut cluster = app.at("/cluster");
    cluster.at("/add-learner").post(add_learner);
    cluster.at("/change-membership").post(change_membership);
    cluster.at("/init").post(init);
    cluster.at("/metrics").get(metrics);
}

/// Add a node as **Learner**.
///
/// A Learner receives log replication from the leader but does not vote.
/// This should be done before adding a node as a member into the cluster
/// (by calling `change-membership`)
async fn add_learner(mut req: Request<Arc<App>>) -> tide::Result {
    let (node_id, api_addr, rpc_addr): (NodeId, String, String) = req.body_json().await?;
    let node = Node { rpc_addr, api_addr };
    let tokio_handle = req.state().tokio_handle.clone();
    let raft = req.state().raft.clone();
    let res = tokio_handle
        .spawn(async move { raft.add_learner(node_id, node, true).await })
        .await
        .map_err(|e| tide::Error::from_str(StatusCode::InternalServerError, e.to_string()))?;
    Ok(Response::builder(StatusCode::Ok).body(Body::from_json(&res)?).build())
}

/// Changes specified learners to members, or remove members.
async fn change_membership(mut req: Request<Arc<App>>) -> tide::Result {
    let body: BTreeSet<NodeId> = req.body_json().await?;
    let tokio_handle = req.state().tokio_handle.clone();
    let raft = req.state().raft.clone();
    let res = tokio_handle
        .spawn(async move { raft.change_membership(body, false).await })
        .await
        .map_err(|e| tide::Error::from_str(StatusCode::InternalServerError, e.to_string()))?;
    Ok(Response::builder(StatusCode::Ok).body(Body::from_json(&res)?).build())
}

/// Initialize a single-node cluster.
async fn init(req: Request<Arc<App>>) -> tide::Result {
    let mut nodes = BTreeMap::new();
    let node = Node {
        api_addr: req.state().api_addr.clone(),
        rpc_addr: req.state().rpc_addr.clone(),
    };

    nodes.insert(req.state().id, node);
    let tokio_handle = req.state().tokio_handle.clone();
    let raft = req.state().raft.clone();
    let res = tokio_handle
        .spawn(async move { raft.initialize(nodes).await })
        .await
        .map_err(|e| tide::Error::from_str(StatusCode::InternalServerError, e.to_string()))?;
    Ok(Response::builder(StatusCode::Ok).body(Body::from_json(&res)?).build())
}

/// Get the latest metrics of the cluster
async fn metrics(req: Request<Arc<App>>) -> tide::Result {
    let metrics = req.state().raft.metrics().borrow().clone();

    let res: Result<RaftMetrics<NodeId, Node>, Infallible> = Ok(metrics);
    Ok(Response::builder(StatusCode::Ok).body(Body::from_json(&res)?).build())
}
