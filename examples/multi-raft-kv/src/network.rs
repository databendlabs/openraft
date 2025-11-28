//! Multi-Raft Network implementation using `MultiRaftNetwork` trait.
//!
//! This module demonstrates how to implement `MultiRaftNetwork` for connection sharing
//! across multiple Raft groups. Unlike per-group network factories, this approach
//! allows all groups targeting the same node to share a single network connection.
//!
//! ## Design
//!
//! ```text
//! Without MultiRaftNetwork (per-group connections):
//!   users_group  -> Node 2: Connection A
//!   orders_group -> Node 2: Connection B
//!   Total: 2 connections to Node 2
//!
//! With MultiRaftNetwork (shared connections):
//!   users_group  -> Node 2: Shared Connection (group_id in request)
//!   orders_group -> Node 2: Shared Connection (group_id in request)
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
use crate::GroupId;
use crate::NodeId;
use crate::TypeConfig;

// ============================================================================
// SharedConnection - A connection shared across all groups to a target node
// ============================================================================

/// A shared network connection to a target node.
///
/// This connection can handle RPCs for any Raft group. The `group_id` is included
/// in each request to allow the target node to route to the correct Raft instance.
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
/// Each RPC method receives a grouped request that contains:
/// - `group_id`: Which Raft group this request is for
/// - `request`: The actual Raft RPC payload
impl MultiRaftNetwork<TypeConfig> for SharedConnection {
    /// Send a grouped AppendEntries RPC.
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
    async fn grouped_full_snapshot(
        &mut self,
        group_id: GroupId,
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
// GroupNetworkFactory - Creates per-group adapters from shared connections
// ============================================================================

/// Network factory for creating group-specific network adapters.
///
/// This factory creates `GroupNetworkAdapter` instances that wrap a `SharedConnection`
/// and bind it to a specific group.
#[derive(Clone)]
pub struct GroupNetworkFactory {
    /// The group ID for connections created by this factory.
    group_id: GroupId,
    /// The shared router.
    router: Router,
}

impl GroupNetworkFactory {
    /// Create a new network factory for a specific group.
    pub fn new(group_id: GroupId, router: Router) -> Self {
        Self { group_id, router }
    }
}

/// Implement `RaftNetworkFactory` for `GroupNetworkFactory`.
impl RaftNetworkFactory<TypeConfig> for GroupNetworkFactory {
    /// The network type is a `GroupNetworkAdapter` wrapping our `SharedConnection`.
    type Network = GroupNetworkAdapter<TypeConfig, SharedConnection>;

    /// Create a new network adapter for a target node.
    async fn new_client(&mut self, target: NodeId, _node: &BasicNode) -> Self::Network {
        let shared_connection = SharedConnection::new(self.router.clone(), target);
        GroupNetworkAdapter::new(shared_connection, self.group_id.clone())
    }
}
