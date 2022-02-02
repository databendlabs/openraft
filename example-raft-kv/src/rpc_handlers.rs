use std::collections::BTreeSet;

use actix_web::get;
use actix_web::post;
use actix_web::web;
use actix_web::web::Data;
use actix_web::Responder;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::ClientWriteRequest;
use openraft::raft::EntryPayload;
use openraft::raft::InstallSnapshotRequest;
use openraft::raft::VoteRequest;
use openraft::NodeId;
use serde::Deserialize;
use serde::Serialize;
use web::Json;

use crate::app::ExampleApp;
use crate::store::ExampleRequest;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Empty {}

// --- Raft communication

#[post("/raft-vote")]
pub async fn vote(app: Data<ExampleApp>, req: Json<VoteRequest>) -> actix_web::Result<impl Responder> {
    let res = app.raft.vote(req.0).await;
    Ok(Json(res))
}

#[post("/raft-append")]
pub async fn append(
    app: Data<ExampleApp>,
    req: Json<AppendEntriesRequest<ExampleRequest>>,
) -> actix_web::Result<impl Responder> {
    let res = app.raft.append_entries(req.0).await;
    Ok(Json(res))
}

#[post("/raft-snapshot")]
pub async fn snapshot(app: Data<ExampleApp>, req: Json<InstallSnapshotRequest>) -> actix_web::Result<impl Responder> {
    let res = app.raft.install_snapshot(req.0).await;
    Ok(Json(res))
}

// --- Application API

#[post("/write")]
pub async fn write(app: Data<ExampleApp>, req: Json<ExampleRequest>) -> actix_web::Result<impl Responder> {
    let res = app.raft.client_write(ClientWriteRequest::new(EntryPayload::Normal(req.0))).await;
    Ok(Json(res))
}

#[post("/read")]
pub async fn read(app: Data<ExampleApp>, req: Json<String>) -> actix_web::Result<impl Responder> {
    let res = {
        let sm = app.store.sm.read().await;
        let key = req.0;
        let value = sm.kvs.get(&key).cloned();
        value.unwrap_or_default()
    };
    Ok(Json(res))
}

// --- Cluster management

/// Add a node as **Learner**.
///
/// A Learner receives log replication from the leader but does not vote.
/// This should be done before adding a node as a member into the cluster(by calling `change-membership`)
#[post("/add-learner")]
pub async fn add_learner(app: Data<ExampleApp>, req: Json<NodeId>) -> actix_web::Result<impl Responder> {
    let res = app.raft.add_learner(req.0, true).await;
    Ok(Json(res))
}

/// Changes specified learners to members, or remove members.
#[post("/change-membership")]
pub async fn change_membership(
    app: Data<ExampleApp>,
    req: Json<BTreeSet<NodeId>>,
) -> actix_web::Result<impl Responder> {
    let res = app.raft.change_membership(req.0, true).await;
    Ok(Json(res))
}

/// Initialize a single-node cluster.
#[post("/init")]
pub async fn init(app: Data<ExampleApp>, _req: Json<Empty>) -> actix_web::Result<impl Responder> {
    let mut nodes = BTreeSet::new();
    nodes.insert(app.id);

    let res = app.raft.initialize(nodes).await;
    Ok(Json(res))
}

/// Get the latest metrics of the cluster
#[get("/metrics")]
pub async fn metrics(app: Data<ExampleApp>) -> actix_web::Result<impl Responder> {
    let res = app.raft.metrics().borrow().clone();
    Ok(Json(res))
}

/// List known nodes of the cluster.
#[get("/list-nodes")]
pub async fn list_nodes(app: Data<ExampleApp>) -> actix_web::Result<impl Responder> {
    let res = {
        let sm = app.store.sm.read().await;
        sm.nodes.clone()
    };

    Ok(Json(res))
}
