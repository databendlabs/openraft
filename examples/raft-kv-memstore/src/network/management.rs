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
use openraft::LogId;
use openraft::RaftMetrics;
use openraft::ReadPolicy;
use serde::Deserialize;
use serde::Serialize;

use crate::app::App;
use crate::NodeId;
use crate::TypeConfig;

/// Serializable representation of linearizer data for follower reads
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinearizerData {
    pub node_id: NodeId,
    pub read_log_id: LogId<TypeConfig>,
    pub applied: Option<LogId<TypeConfig>>,
}

// --- Cluster management

/// Add a node as **Learner**.
///
/// A Learner receives log replication from the leader but does not vote.
/// This should be done before adding a node as a member into the cluster
/// (by calling `change-membership`)
#[post("/add-learner")]
pub async fn add_learner(app: Data<App>, req: Json<(NodeId, String)>) -> actix_web::Result<impl Responder> {
    let node_id = req.0 .0;
    let node = BasicNode { addr: req.0 .1.clone() };
    let res = app.raft.add_learner(node_id, node, true).await.decompose().unwrap();
    Ok(Json(res))
}

/// Changes specified learners to members, or remove members.
#[post("/change-membership")]
pub async fn change_membership(app: Data<App>, req: Json<BTreeSet<NodeId>>) -> actix_web::Result<impl Responder> {
    let res = app.raft.change_membership(req.0, false).await.decompose().unwrap();
    Ok(Json(res))
}

/// Initialize a single-node cluster if the `req` is empty vec.
/// Otherwise initialize a cluster with the `req` specified vec of node-id and node-address
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

    let res: Result<RaftMetrics<TypeConfig>, Infallible> = Ok(metrics);
    Ok(Json(res))
}

/// Get linearizer data for performing linearizable reads on followers
///
/// This endpoint is used by followers to obtain linearizer data from the leader.
/// The follower can then reconstruct a Linearizer and wait for its local state
/// machine to catch up before performing a linearizable read.
#[post("/get_linearizer")]
pub async fn get_linearizer(app: Data<App>) -> actix_web::Result<impl Responder> {
    let linearizer = app.raft.get_read_linearizer(ReadPolicy::ReadIndex).await.decompose().unwrap();

    let data = match linearizer {
        Ok(lin) => {
            let data = LinearizerData {
                node_id: *lin.node_id(),
                read_log_id: *lin.read_log_id(),
                applied: lin.applied().cloned(),
            };
            Ok(data)
        }
        Err(e) => Err(e),
    };

    Ok(Json(data))
}
