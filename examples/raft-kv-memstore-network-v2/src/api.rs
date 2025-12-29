//! This mod implements a network API for raft node.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::io::Cursor;

use openraft::BasicNode;
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

            let inner = app.state_machine.inner().lock().await;
            let value = inner.state_machine.data.get(&key).cloned();

            let res: Result<String, RaftError<LinearizableReadError>> = Ok(value.unwrap_or_default());
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

/// Receive a snapshot and install it.
pub async fn snapshot(app: &mut App, req: String) -> String {
    // Receive Vec<u8> and wrap with Cursor for SnapshotData
    let (vote, snapshot_meta, snapshot_data): (Vote, SnapshotMeta, Vec<u8>) = decode(&req);
    let snapshot = Snapshot {
        meta: snapshot_meta,
        snapshot: Cursor::new(snapshot_data),
    };
    let res = app.raft.install_full_snapshot(vote, snapshot).await.map_err(RaftError::<Infallible>::Fatal);
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
    let metrics = app.raft.metrics().borrow_watched().clone();

    let res: Result<RaftMetrics, Infallible> = Ok(metrics);
    encode(res)
}
