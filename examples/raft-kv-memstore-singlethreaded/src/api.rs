//! This mod implements a network API for raft node.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use openraft::error::Infallible;
use openraft::BasicNode;
use openraft::ReadPolicy;

use crate::app::App;
use crate::decode;
use crate::encode;
use crate::typ::*;
use crate::NodeId;

pub async fn write(app: &mut App, req: String) -> String {
    let res = app.raft.client_write(decode(&req)).await;
    encode(res)
}

pub async fn read(app: &mut App, req: String) -> String {
    let key: String = decode(&req);

    let ret = app.raft.get_read_linearizer(ReadPolicy::ReadIndex).await;

    let res = match ret {
        Ok(linearizer) => {
            linearizer.await_ready(&app.raft).await.unwrap();

            let state_machine = app.state_machine.state_machine.borrow();
            let value = state_machine.data.get(&key).cloned();

            let res: Result<String, RaftError<CheckIsLeaderError>> = Ok(value.unwrap_or_default());
            res
        }
        Err(e) => Err(e),
    };
    encode(res)
}

// Raft API

pub async fn vote(app: &mut App, req: String) -> String {
    let res = app.raft.vote(decode(&req)).await;
    encode(res)
}

pub async fn append(app: &mut App, req: String) -> String {
    let res = app.raft.append_entries(decode(&req)).await;
    encode(res)
}

pub async fn snapshot(app: &mut App, req: String) -> String {
    let req = decode(&req);
    let res = app.raft.install_snapshot(req).await;
    encode(res)
}

// Management API

/// Add a node as **Learner**.
///
/// A Learner receives log replication from the leader but does not vote.
/// This should be done before adding a node as a member into the cluster
/// (by calling `change-membership`)
pub async fn add_learner(app: &mut App, req: String) -> String {
    let node_id: NodeId = decode(&req);
    let node = BasicNode { addr: "".to_string() };
    let res = app.raft.add_learner(node_id, node, true).await;
    encode(res)
}

/// Changes specified learners to members, or remove members.
pub async fn change_membership(app: &mut App, req: String) -> String {
    let node_ids: BTreeSet<NodeId> = decode(&req);
    let res = app.raft.change_membership(node_ids, false).await;
    encode(res)
}

/// Initialize a single-node cluster.
pub async fn init(app: &mut App) -> String {
    let mut nodes = BTreeMap::new();
    nodes.insert(app.id, BasicNode { addr: "".to_string() });
    let res = app.raft.initialize(nodes).await;
    encode(res)
}

/// Get the latest metrics of the cluster
pub async fn metrics(app: &mut App) -> String {
    let metrics = app.raft.metrics().borrow().clone();

    let res: Result<RaftMetrics, Infallible> = Ok(metrics);
    encode(res)
}
