use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::sync::Arc;

use openraft::error::Infallible;
use openraft::Node;
use openraft::RaftMetrics;
use tide::Body;
use tide::Request;
use tide::Response;
use tide::StatusCode;

use crate::app::ExampleApp;
use crate::ExampleNodeId;
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
async fn add_learner(mut req: Request<Arc<ExampleApp>>) -> tide::Result {
    let (node_id, api_addr, addr): (ExampleNodeId, String, String) = req.body_json().await?;
    let mut data = BTreeMap::new();
    data.insert("api_addr".into(), api_addr);
    let node = Node { addr, data };
    let res = req.state().raft.add_learner(node_id, Some(node), true).await;
    Ok(Response::builder(StatusCode::Ok).body(Body::from_json(&res)?).build())
}

/// Changes specified learners to members, or remove members.
async fn change_membership(mut req: Request<Arc<ExampleApp>>) -> tide::Result {
    let body: BTreeSet<ExampleNodeId> = req.body_json().await?;
    let res = req.state().raft.change_membership(body, true, false).await;
    Ok(Response::builder(StatusCode::Ok).body(Body::from_json(&res)?).build())
}

/// Initialize a single-node cluster.
async fn init(req: Request<Arc<ExampleApp>>) -> tide::Result {
    let mut nodes = BTreeMap::new();
    let mut data = BTreeMap::new();
    data.insert("api_addr".into(), req.state().api_addr.clone());
    let addr = req.state().rcp_addr.clone();
    nodes.insert(req.state().id, Node { addr, data });
    let res = req.state().raft.initialize(nodes).await;
    Ok(Response::builder(StatusCode::Ok).body(Body::from_json(&res)?).build())
}

/// Get the latest metrics of the cluster
async fn metrics(req: Request<Arc<ExampleApp>>) -> tide::Result {
    let metrics = req.state().raft.metrics().borrow().clone();

    let res: Result<RaftMetrics<ExampleNodeId>, Infallible> = Ok(metrics);
    Ok(Response::builder(StatusCode::Ok).body(Body::from_json(&res)?).build())
}
