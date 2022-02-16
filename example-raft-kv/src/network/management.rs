use std::collections::BTreeSet;

use actix_web::get;
use actix_web::post;
use actix_web::web;
use actix_web::web::Data;
use actix_web::Responder;
use openraft::error::Infallible;
use openraft::NodeId;
use openraft::RaftMetrics;
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
    let res = app.raft.add_learner(req.0, None, true).await;
    Ok(Json(res))
}

/// Changes specified learners to members, or remove members.
#[post("/change-membership")]
pub async fn change_membership(
    app: Data<ExampleApp>,
    req: Json<BTreeSet<NodeId>>,
) -> actix_web::Result<impl Responder> {
    let res = app.raft.change_membership(req.0, true, false).await;
    Ok(Json(res))
}

/// Initialize a single-node cluster.
#[post("/init")]
pub async fn init(app: Data<ExampleApp>) -> actix_web::Result<impl Responder> {
    let mut nodes = BTreeSet::new();
    nodes.insert(app.id);
    let res = app.raft.initialize(nodes).await;
    Ok(Json(res))
}

/// Get the latest metrics of the cluster
#[get("/metrics")]
pub async fn metrics(app: Data<ExampleApp>) -> actix_web::Result<impl Responder> {
    let metrics = app.raft.metrics().borrow().clone();

    let res: Result<RaftMetrics, Infallible> = Ok(metrics);
    Ok(Json(res))
}

/// List known nodes of the cluster.
#[get("/list-nodes")]
pub async fn list_nodes(app: Data<ExampleApp>) -> actix_web::Result<impl Responder> {
    let nodes = {
        let state_machine = app.store.state_machine.read().await;
        state_machine.nodes.clone()
    };

    let res: Result<_, Infallible> = Ok(nodes);
    Ok(Json(res))
}
