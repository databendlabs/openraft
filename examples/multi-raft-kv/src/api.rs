use std::collections::BTreeMap;
use std::collections::BTreeSet;

use openraft::raft::TransferLeaderRequest;
use openraft::BasicNode;
use openraft::ReadPolicy;

use crate::app::GroupApp;
use crate::decode;
use crate::encode;
use crate::typ::*;
use crate::NodeId;

/// Write a key-value pair to the group's state machine
pub async fn write(app: &mut GroupApp, req: String) -> String {
    let res = app.raft.client_write(decode(&req)).await;
    encode(res)
}

/// Read a value from the group's state machine using linearizable read
pub async fn read(app: &mut GroupApp, req: String) -> String {
    let key: String = decode(&req);

    let ret = app.raft.get_read_linearizer(ReadPolicy::ReadIndex).await;

    let res = match ret {
        Ok(linearizer) => {
            linearizer.await_ready(&app.raft).await.unwrap();

            let state_machine = app.state_machine.state_machine.lock().await;
            let value = state_machine.data.get(&key).cloned();

            let res: Result<String, RaftError<LinearizableReadError>> = Ok(value.unwrap_or_default());
            res
        }
        Err(e) => Err(e),
    };
    encode(res)
}

// ============================================================================
// Raft Protocol API
// ============================================================================

/// Handle vote request
pub async fn vote(app: &mut GroupApp, req: String) -> String {
    let res = app.raft.vote(decode(&req)).await;
    encode(res)
}

/// Handle append entries request
pub async fn append(app: &mut GroupApp, req: String) -> String {
    let res = app.raft.append_entries(decode(&req)).await;
    encode(res)
}

/// Receive a snapshot and install it
pub async fn snapshot(app: &mut GroupApp, req: String) -> String {
    let (vote, snapshot_meta, snapshot_data): (Vote, SnapshotMeta, SnapshotData) = decode(&req);
    let snapshot = Snapshot {
        meta: snapshot_meta,
        snapshot: snapshot_data,
    };
    let res = app.raft.install_full_snapshot(vote, snapshot).await.map_err(RaftError::<Infallible>::Fatal);
    encode(res)
}

/// Handle transfer leader request
pub async fn transfer_leader(app: &mut GroupApp, req: String) -> String {
    let transfer_req: TransferLeaderRequest<crate::TypeConfig> = decode(&req);
    let res = app.raft.handle_transfer_leader(transfer_req).await;
    encode(res)
}

// ============================================================================
// Management API
// ============================================================================

/// Add a node as **Learner** to this group.
///
/// This should be done before adding a node as a member into the cluster
/// (by calling `change-membership`)
pub async fn add_learner(app: &mut GroupApp, req: String) -> String {
    let node_id: NodeId = decode(&req);
    let node = BasicNode { addr: "".to_string() };
    let res = app.raft.add_learner(node_id, node, true).await;
    encode(res)
}

/// Changes specified learners to members, or remove members from this group.
pub async fn change_membership(app: &mut GroupApp, req: String) -> String {
    let node_ids: BTreeSet<NodeId> = decode(&req);
    let res = app.raft.change_membership(node_ids, false).await;
    encode(res)
}

/// Initialize a single-node cluster for this group.
pub async fn init(app: &mut GroupApp) -> String {
    let mut nodes = BTreeMap::new();
    nodes.insert(app.node_id, BasicNode { addr: "".to_string() });
    let res = app.raft.initialize(nodes).await;
    encode(res)
}

/// Get the latest metrics of this Raft group
pub async fn metrics(app: &mut GroupApp) -> String {
    let metrics = app.raft.metrics().borrow().clone();

    let res: Result<RaftMetrics, Infallible> = Ok(metrics);
    encode(res)
}
