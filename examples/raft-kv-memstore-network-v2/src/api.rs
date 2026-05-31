//! This mod implements application and management APIs for the example.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use openraft::NodeInfo as Node;
use openraft::ReadPolicy;
use openraft::async_runtime::WatchReceiver;

use crate::NodeId;
use crate::app::App;
use crate::decode;
use crate::encode;
use crate::typ::*;

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
            let value = app.state_machine.get(&key).await;

            let res: Result<String, RaftError<LinearizableReadError>> = Ok(value.unwrap_or_default());
            res
        }
        Err(e) => Err(e),
    };
    encode(res)
}

// Management API

/// Add a node as **Learner**.
///
/// A Learner receives log replication from the leader but does not vote.
/// This should be done before adding a node as a member into the cluster
/// (by calling `change-membership`)
pub async fn add_learner(app: &mut App, req: String) -> String {
    let (node_id, node): (NodeId, Node) = decode(&req);
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
    nodes.insert(app.id, Node::new(app.raft_addr.clone(), ""));
    let res = app.raft.initialize(nodes).await;
    encode(res)
}

/// Get the latest metrics of the cluster
pub async fn metrics(app: &mut App) -> String {
    let metrics = app.raft.metrics().borrow_watched().clone();

    let res: Result<RaftMetrics, Infallible> = Ok(metrics);
    encode(res)
}
