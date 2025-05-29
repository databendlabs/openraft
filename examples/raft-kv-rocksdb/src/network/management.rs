use std::collections::BTreeMap;
use std::collections::BTreeSet;

use actix_web::get;
use actix_web::post;
use actix_web::web::Data;
use actix_web::web::Json;
use actix_web::Responder;
use openraft::error::decompose::DecomposeResult;
use openraft::error::Infallible;
use openraft::BasicNode;

use crate::app::App;
use crate::typ::*;
use crate::NodeId;

// --- Cluster management

/// Add a node as **Learner**.
///
/// A Learner receives log replication from the leader but does not vote.
/// This should be done before adding a node as a member into the cluster
/// (by calling `change-membership`)
#[post("/add-learner")]
pub async fn add_learner(app: Data<App>, req: Json<(NodeId, String)>) -> actix_web::Result<impl Responder> {
    let (node_id, api_addr) = req.0;
    let node = Node { addr: api_addr };
    let res = app.raft.add_learner(node_id, node, true).await.decompose().unwrap();
    Ok(Json(res))
}

/// Changes specified learners to members, or remove members.
#[post("/change-membership")]
pub async fn change_membership(app: Data<App>, req: Json<BTreeSet<NodeId>>) -> actix_web::Result<impl Responder> {
    let body = req.0;
    let res = app.raft.change_membership(body, false).await.decompose().unwrap();
    Ok(Json(res))
}

/// Initialize a cluster.
#[post("/init")]
pub async fn init(app: Data<App>, req: Json<Vec<(NodeId, String)>>) -> actix_web::Result<impl Responder> {
    let mut nodes = BTreeMap::new();
    if req.0.is_empty() {
        nodes.insert(app.id, BasicNode { addr: app.addr.clone() });
    } else {
        for (id, addr) in req.0.into_iter() {
            nodes.insert(id, BasicNode { addr });
        }
    };
    let res = app.raft.initialize(nodes).await.decompose().unwrap();
    Ok(Json(res))
}

/// Get the latest metrics of the cluster
#[get("/metrics")]
pub async fn metrics(app: Data<App>) -> actix_web::Result<impl Responder> {
    let metrics = app.raft.metrics().borrow().clone();

    let res: Result<RaftMetrics, Infallible> = Ok(metrics);
    Ok(Json(res))
}
