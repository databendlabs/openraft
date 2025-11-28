//! Multi-Raft Network implementation using `MultiRaftNetwork` trait.
//!
//! This module demonstrates how to implement `MultiRaftNetwork` for connection sharing
//! across multiple Raft groups (shards). Unlike per-group network factories, this approach
//! allows all shards targeting the same node to share a single network connection.
//!
//! ## Design
//!
//! ```text
//! Without MultiRaftNetwork (per-group connections):
//!   shard_a -> Node 2: Connection A
//!   shard_b -> Node 2: Connection B
//!   Total: 2 connections to Node 2
//!
//! With MultiRaftNetwork (shared connections):
//!   shard_a -> Node 2: Shared Connection (group_id in request)
//!   shard_b -> Node 2: Shared Connection (group_id in request)
//!   Total: 1 connection to Node 2
//! ```

use std::future::Future;
use std::time::Duration;

use openraft::error::RPCError;
use openraft::error::ReplicationClosed;
use openraft::error::StreamingError;
use openraft::multi_raft::GroupNetworkAdapter;
use openraft::multi_raft::GroupedAppendEntriesRequest;
use openraft::multi_raft::GroupedAppendEntriesResponse;
use openraft::multi_raft::GroupedResponse;
use openraft::multi_raft::GroupedSnapshotResponse;
use openraft::multi_raft::GroupedVoteRequest;
use openraft::multi_raft::GroupedVoteResponse;
use openraft::multi_raft::MultiRaftNetwork;
use openraft::network::Backoff;
use openraft::network::RPCOption;
use openraft::BasicNode;
use openraft::OptionalSend;
use openraft::RaftNetworkFactory;

use crate::router::Router;
use crate::typ::*;
use crate::NodeId;
use crate::ShardId;
use crate::TypeConfig;

// ============================================================================
// SharedConnection - A connection shared across all shards to a target node
// ============================================================================

/// A shared network connection to a target node.
///
/// This connection can handle RPCs for any shard (group). The `group_id` is included
/// in each request to allow the target node to route to the correct Raft instance.
///
/// ## Connection Sharing
///
/// Instead of creating one connection per (shard, target_node) pair, we create
/// one connection per target_node. All shards use the same connection, with
/// the `group_id` in the request identifying the target shard.
pub struct SharedConnection {
    /// The shared router for message delivery.
    router: Router,
    /// The target node ID for this connection.
    target: NodeId,
}

impl SharedConnection {
    /// Create a new shared connection to a target node.
    pub fn new(router: Router, target: NodeId) -> Self {
        Self { router, target }
    }
}

/// Implement `MultiRaftNetwork` for `SharedConnection`.
///
/// Each RPC method receives a `GroupedRequest` that contains:
/// - `group_id`: Which shard this request is for
/// - `request`: The actual Raft RPC payload
///
/// The target node uses the `group_id` to route the request to the correct
/// Raft instance.
impl MultiRaftNetwork<TypeConfig> for SharedConnection {
    /// Send a grouped AppendEntries RPC.
    ///
    /// The request contains the `shard_id` so the target node knows which
    /// Raft group should handle this AppendEntries.
    async fn grouped_append_entries(
        &mut self,
        rpc: GroupedAppendEntriesRequest<TypeConfig>,
        _option: RPCOption,
    ) -> Result<GroupedAppendEntriesResponse<TypeConfig>, RPCError<TypeConfig>> {
        let group_id = rpc.group_id.clone();
        let resp: AppendEntriesResponse = self.router.send(self.target, &group_id, "/raft/append", rpc.request).await?;
        Ok(GroupedResponse::new(group_id, resp))
    }

    /// Send a grouped Vote RPC.
    async fn grouped_vote(
        &mut self,
        rpc: GroupedVoteRequest<TypeConfig>,
        _option: RPCOption,
    ) -> Result<GroupedVoteResponse<TypeConfig>, RPCError<TypeConfig>> {
        let group_id = rpc.group_id.clone();
        let resp: VoteResponse = self.router.send(self.target, &group_id, "/raft/vote", rpc.request).await?;
        Ok(GroupedResponse::new(group_id, resp))
    }

    /// Send a grouped full snapshot.
    ///
    /// The snapshot is for a specific shard, identified by `group_id`.
    async fn grouped_full_snapshot(
        &mut self,
        group_id: ShardId,
        vote: Vote,
        snapshot: Snapshot,
        _cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
        _option: RPCOption,
    ) -> Result<GroupedSnapshotResponse<TypeConfig>, StreamingError<TypeConfig>> {
        let resp: SnapshotResponse = self
            .router
            .send(
                self.target,
                &group_id,
                "/raft/snapshot",
                (vote, snapshot.meta, snapshot.snapshot),
            )
            .await
            .map_err(StreamingError::Unreachable)?;
        Ok(GroupedResponse::new(group_id, resp))
    }

    /// Backoff configuration for retries.
    fn backoff(&self) -> Backoff {
        Backoff::new(std::iter::repeat(Duration::from_millis(500)))
    }
}

// ============================================================================
// ShardNetworkFactory - Creates per-shard adapters from shared connections
// ============================================================================

/// Network factory for creating shard-specific network adapters.
///
/// This factory creates `GroupNetworkAdapter` instances that wrap a `SharedConnection`
/// and bind it to a specific shard. The adapter translates standard `RaftNetworkV2`
/// calls into `MultiRaftNetwork` calls with the appropriate `group_id`.
///
/// ## How it works
///
/// ```text
/// ShardNetworkFactory
///   └── creates GroupNetworkAdapter<SharedConnection>
///         └── wraps SharedConnection + shard_id
///               └── sends GroupedRequest via SharedConnection
/// ```
#[derive(Clone)]
pub struct ShardNetworkFactory {
    /// The shard ID for connections created by this factory.
    shard_id: ShardId,
    /// The shared router.
    router: Router,
}

impl ShardNetworkFactory {
    /// Create a new network factory for a specific shard.
    pub fn new(shard_id: ShardId, router: Router) -> Self {
        Self { shard_id, router }
    }
}

/// Implement `RaftNetworkFactory` for `ShardNetworkFactory`.
///
/// This allows each Raft instance to get its own `GroupNetworkAdapter`,
/// which internally uses the shared `MultiRaftNetwork` connection.
impl RaftNetworkFactory<TypeConfig> for ShardNetworkFactory {
    /// The network type is a `GroupNetworkAdapter` wrapping our `SharedConnection`.
    ///
    /// `GroupNetworkAdapter` implements `RaftNetworkV2` (when `adapt-network-v1` is disabled),
    /// so it can be used directly with OpenRaft.
    type Network = GroupNetworkAdapter<TypeConfig, SharedConnection>;

    /// Create a new network adapter for a target node.
    ///
    /// The adapter wraps a `SharedConnection` and binds it to this factory's `shard_id`.
    async fn new_client(&mut self, target: NodeId, _node: &BasicNode) -> Self::Network {
        let shared_connection = SharedConnection::new(self.router.clone(), target);
        GroupNetworkAdapter::new(shared_connection, self.shard_id.clone())
    }
}
