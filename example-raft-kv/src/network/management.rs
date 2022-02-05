use std::collections::BTreeSet;

use actix_web::get;
use actix_web::post;
use actix_web::web;
use actix_web::web::Data;
use actix_web::Responder;
use openraft::NodeId;
use web::Json;

use crate::app::ExampleApp;

// --- Cluster management

/// Add a node as **Learner**.
///
/// A Learner receives log replication from the leader but does not vote.
/// This should be done before adding a node as a member into the cluster
/// (by calling `change-membership`)
#[post("/add-learner")]
pub async fn add_learner(app: Data<ExampleApp>, req: Json<NodeId>) -> actix_web::Result<impl Responder> {
    let response = app.raft.add_learner(req.0, true).await;
    Ok(Json(response))
}

/// Changes specified learners to members, or remove members.
#[post("/change-membership")]
pub async fn change_membership(
    app: Data<ExampleApp>,
    req: Json<BTreeSet<NodeId>>,
) -> actix_web::Result<impl Responder> {
    let response = app.raft.change_membership(req.0, true).await;
    Ok(Json(response))
}

/// Initialize a single-node cluster.
#[post("/init")]
pub async fn init(app: Data<ExampleApp>) -> actix_web::Result<impl Responder> {
    let mut nodes = BTreeSet::new();
    nodes.insert(app.id);
    let response = app.raft.initialize(nodes).await;
    Ok(Json(response))
}

/// Get the latest metrics of the cluster
#[get("/metrics")]
pub async fn metrics(app: Data<ExampleApp>) -> actix_web::Result<impl Responder> {
    let response = app.raft.metrics().borrow().clone();
    Ok(Json(response))
}

/// List known nodes of the cluster.
#[get("/list-nodes")]
pub async fn list_nodes(app: Data<ExampleApp>) -> actix_web::Result<impl Responder> {
    let state_machine = app.store.state_machine.read().await;
    let response = state_machine.nodes.clone();
    Ok(Json(response))
}
