use std::collections::BTreeMap;
use std::collections::BTreeSet;

use openraft::BasicNode;

use crate::app::App;
use crate::decode;
use crate::encode;
use crate::store::Request;
use crate::typ::*;
use crate::NodeId;

// =============================================================================
// Application API
// =============================================================================

/// Write a request to the shard.
///
/// This handles all write operations including:
/// - Set: Store a key-value pair
/// - Delete: Remove a key
/// - Split: Split the shard (TiKV-style)
pub async fn write(app: &mut App, req: String) -> String {
    let request: Request = decode(&req);

    tracing::debug!(
        shard_id = %app.shard_id,
        request = %request,
        "processing write request"
    );

    let res = app.raft.client_write(request).await;
    encode(res)
}

/// Read a key from the shard using linearizable read.
///
/// This ensures the read sees all committed writes by:
/// 1. Getting a read linearizer
/// 2. Waiting for it to be ready (confirms leadership)
/// 3. Reading from the state machine
pub async fn read(app: &mut App, req: String) -> String {
    use openraft::ReadPolicy;

    let key: String = decode(&req);

    let ret = app.raft.ensure_linearizable(ReadPolicy::ReadIndex).await;

    let res: Result<Option<String>, RaftError<Infallible>> = match ret {
        Ok(_) => {
            let state_machine = app.state_machine.state_machine.lock().await;
            let value = state_machine.data.get(&key).cloned();
            Ok(value)
        }
        Err(e) => Err(RaftError::Fatal(e.into_fatal().unwrap())),
    };
    encode(res)
}

// =============================================================================
// Raft Protocol API
// =============================================================================

/// Handle Vote RPC from other nodes.
pub async fn vote(app: &mut App, req: String) -> String {
    let res = app.raft.vote(decode(&req)).await;
    encode(res)
}

/// Handle AppendEntries RPC from the leader.
pub async fn append(app: &mut App, req: String) -> String {
    let res = app.raft.append_entries(decode(&req)).await;
    encode(res)
}

/// Handle snapshot installation from the leader.
pub async fn snapshot(app: &mut App, req: String) -> String {
    let (vote, snapshot_meta, snapshot_data): (Vote, SnapshotMeta, SnapshotData) = decode(&req);
    let snapshot = Snapshot {
        meta: snapshot_meta,
        snapshot: snapshot_data,
    };
    let res = app.raft.install_full_snapshot(vote, snapshot).await.map_err(RaftError::<Infallible>::Fatal);
    encode(res)
}

// =============================================================================
// Management API
// =============================================================================

pub async fn add_learner(app: &mut App, req: String) -> String {
    let node_id: NodeId = decode(&req);
    let node = BasicNode { addr: "".to_string() };

    tracing::info!(
        shard_id = %app.shard_id,
        learner_id = %node_id,
        "adding learner to shard"
    );

    let res = app.raft.add_learner(node_id, node, true).await;
    encode(res)
}

/// Change the membership of this shard.
///
/// This is used to:
/// - Promote learners to voters
/// - Remove nodes from the shard
///
/// # Arguments
/// * `req` - JSON-encoded BTreeSet<NodeId> of new member IDs
pub async fn change_membership(app: &mut App, req: String) -> String {
    let node_ids: BTreeSet<NodeId> = decode(&req);

    tracing::info!(
        shard_id = %app.shard_id,
        new_members = ?node_ids,
        "changing shard membership"
    );

    let res = app.raft.change_membership(node_ids, false).await;
    encode(res)
}

/// Initialize a single-node cluster for this shard.
///
/// This is called on the first node to bootstrap the shard.
pub async fn init(app: &mut App) -> String {
    let mut nodes = BTreeMap::new();
    nodes.insert(app.node_id, BasicNode { addr: "".to_string() });

    tracing::info!(
        shard_id = %app.shard_id,
        node_id = %app.node_id,
        "initializing shard"
    );

    let res = app.raft.initialize(nodes).await;
    encode(res)
}

/// Get the current metrics for this shard.
pub async fn metrics(app: &mut App) -> String {
    let metrics = app.raft.metrics().borrow().clone();
    let res: Result<RaftMetrics, Infallible> = Ok(metrics);
    encode(res)
}
