//! Multi-Raft network traits for shared connections across Raft groups.
//!
//! This module defines network traits that allow multiple Raft groups to share
//! the same network connection, reducing connection overhead in Multi-Raft deployments.

use std::future::Future;
use std::time::Duration;

use openraft_macros::add_async_trait;

use super::MultiRaftTypeConfig;
use super::rpc::GroupedAppendEntriesRequest;
use super::rpc::GroupedAppendEntriesResponse;
use super::rpc::GroupedSnapshotResponse;
use super::rpc::GroupedVoteRequest;
use super::rpc::GroupedVoteResponse;
use crate::OptionalSend;
use crate::OptionalSync;
use crate::error::RPCError;
use crate::error::ReplicationClosed;
use crate::error::StreamingError;
use crate::network::Backoff;
use crate::network::RPCOption;
use crate::raft::AppendEntriesRequest;
use crate::raft::AppendEntriesResponse;
use crate::raft::SnapshotResponse;
use crate::raft::VoteRequest;
use crate::raft::VoteResponse;
use crate::raft::message::TransferLeaderRequest;
use crate::storage::Snapshot;
use crate::type_config::alias::VoteOf;

/// A network interface for Multi-Raft deployments that supports connection sharing.
///
/// Unlike [`RaftNetworkV2`] which handles a single Raft group, `MultiRaftNetwork`
/// is designed to handle multiple Raft groups over the same connection. Each request
/// includes a `group_id` to identify the target Raft group on the remote node.
///
/// ## Connection Sharing
///
/// In a Multi-Raft deployment, a single physical node may host replicas from many
/// Raft groups. Without connection sharing, each group would require its own connection
/// to every peer, leading to O(groups × nodes) connections per node.
///
/// With `MultiRaftNetwork`, all groups can share connections, reducing this to
/// O(nodes) connections per node. The `group_id` in each request allows the remote
/// node to route the request to the correct Raft instance.
///
/// ## Example Implementation
///
/// ```ignore
/// use openraft::multi_raft::{MultiRaftNetwork, MultiRaftTypeConfig};
///
/// struct SharedConnection<C: MultiRaftTypeConfig> {
///     target_node: C::NodeId,
///     client: HttpClient,  // Or gRPC, etc.
/// }
///
/// impl<C: MultiRaftTypeConfig> MultiRaftNetwork<C> for SharedConnection<C> {
///     async fn grouped_append_entries(
///         &mut self,
///         rpc: GroupedAppendEntriesRequest<C>,
///         option: RPCOption,
///     ) -> Result<GroupedAppendEntriesResponse<C>, RPCError<C>> {
///         // The group_id is included in the request
///         // Send to the same endpoint, remote will route by group_id
///         let response = self.client
///             .post("/raft/append_entries")
///             .json(&rpc)  // Contains group_id + request
///             .send()
///             .await?;
///         Ok(response.json().await?)
///     }
///     // ... other methods
/// }
/// ```
///
/// ## Server-Side Routing
///
/// On the receiving side, use [`RaftRouter`] to dispatch requests:
///
/// ```ignore
/// async fn handle_append_entries(
///     router: &RaftRouter<C>,
///     req: GroupedAppendEntriesRequest<C>,
/// ) -> Result<AppendEntriesResponse<C>, Error> {
///     let raft = router.get(&req.group_id, &target_node_id)
///         .ok_or(Error::GroupNotFound)?;
///     raft.append_entries(req.into_inner()).await
/// }
/// ```
///
/// [`RaftNetworkV2`]: crate::network::v2::RaftNetworkV2
/// [`RaftRouter`]: super::RaftRouter
#[add_async_trait]
pub trait MultiRaftNetwork<C>: OptionalSend + OptionalSync + 'static
where C: MultiRaftTypeConfig
{
    /// Send an AppendEntries RPC to the target node for a specific Raft group.
    ///
    /// The request includes the `group_id` to identify which Raft group on the
    /// target node should handle this request.
    async fn grouped_append_entries(
        &mut self,
        rpc: GroupedAppendEntriesRequest<C>,
        option: RPCOption,
    ) -> Result<GroupedAppendEntriesResponse<C>, RPCError<C>>;

    /// Send a Vote RPC to the target node for a specific Raft group.
    async fn grouped_vote(
        &mut self,
        rpc: GroupedVoteRequest<C>,
        option: RPCOption,
    ) -> Result<GroupedVoteResponse<C>, RPCError<C>>;

    /// Send a complete Snapshot to the target node for a specific Raft group.
    ///
    /// This method transmits a full snapshot to the target node. The `group_id`
    /// identifies which Raft group should receive and install the snapshot.
    async fn grouped_full_snapshot(
        &mut self,
        group_id: C::GroupId,
        vote: VoteOf<C>,
        snapshot: Snapshot<C>,
        cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
        option: RPCOption,
    ) -> Result<GroupedSnapshotResponse<C>, StreamingError<C>>;

    /// Send a TransferLeader message to the target node for a specific Raft group.
    ///
    /// Default implementation returns an error indicating the feature is not implemented.
    async fn grouped_transfer_leader(
        &mut self,
        _group_id: C::GroupId,
        _req: TransferLeaderRequest<C>,
        _option: RPCOption,
    ) -> Result<(), RPCError<C>> {
        use anyerror::AnyError;

        use crate::error::Unreachable;

        Err(RPCError::Unreachable(Unreachable::new(&AnyError::error(
            "grouped_transfer_leader not implemented",
        ))))
    }

    /// Build a backoff instance for retry logic.
    ///
    /// When an [`Unreachable`] error is returned, Openraft will wait according to
    /// this backoff before retrying.
    ///
    /// Default: constant 500ms backoff.
    ///
    /// [`Unreachable`]: crate::error::Unreachable
    fn backoff(&self) -> Backoff {
        Backoff::new(std::iter::repeat(Duration::from_millis(500)))
    }
}

/// A factory for creating [`MultiRaftNetwork`] connections.
///
/// Unlike [`RaftNetworkFactory`] which creates per-group network connections,
/// `MultiRaftNetworkFactory` creates connections that can be shared across
/// multiple Raft groups.
///
/// ## Design Choice: Per-Node vs Per-Group Connections
///
/// The factory creates one network connection per target node, not per (group, node) pair.
/// All groups targeting the same node share this connection. This significantly reduces
/// the number of connections in a Multi-Raft deployment.
///
/// ```text
/// Without sharing (RaftNetworkFactory):
///   Groups: G1, G2, G3
///   Nodes: N1, N2, N3
///   Connections per node: 3 groups × 2 peers = 6 connections
///
/// With sharing (MultiRaftNetworkFactory):
///   Connections per node: 2 peers = 2 connections
/// ```
///
/// ## Example
///
/// ```ignore
/// struct MyNetworkFactory {
///     config: NetworkConfig,
/// }
///
/// impl<C: MultiRaftTypeConfig> MultiRaftNetworkFactory<C> for MyNetworkFactory {
///     type Network = SharedConnection<C>;
///
///     async fn new_client(&mut self, target: C::NodeId, node: &C::Node) -> Self::Network {
///         // Create a single connection that handles all groups
///         SharedConnection {
///             target_node: target,
///             client: HttpClient::connect(&node.addr).await,
///         }
///     }
/// }
/// ```
///
/// [`RaftNetworkFactory`]: crate::network::RaftNetworkFactory
#[add_async_trait]
pub trait MultiRaftNetworkFactory<C>: OptionalSend + OptionalSync + 'static
where C: MultiRaftTypeConfig
{
    /// The network type that handles communication with a target node.
    type Network: MultiRaftNetwork<C>;

    /// Create a new network connection to the target node.
    ///
    /// This connection will be shared by all Raft groups communicating with
    /// the target node. The connection should not be established immediately;
    /// it should connect lazily when the first RPC is sent.
    ///
    /// ## Parameters
    ///
    /// - `target`: The NodeId of the target node
    /// - `node`: Node information (e.g., network address)
    async fn new_client(&mut self, target: C::NodeId, node: &C::Node) -> Self::Network;
}

/// An adapter that wraps a [`MultiRaftNetwork`] to implement single-group operations.
///
/// This adapter allows using a `MultiRaftNetwork` connection for a specific group,
/// providing the same interface as `RaftNetworkV2`. This is useful when you have
/// a shared connection but need to use it with existing code that expects a
/// single-group network.
///
/// ## Example
///
/// ```ignore
/// // Wrap shared network for a specific group
/// let group_network = GroupNetworkAdapter::new(shared_network, "my_group".to_string());
///
/// // Now can use as a single-group network
/// group_network.append_entries(request, option).await?;
/// ```
pub struct GroupNetworkAdapter<C, N>
where
    C: MultiRaftTypeConfig,
    N: MultiRaftNetwork<C>,
{
    /// The underlying shared network connection.
    network: N,
    /// The group ID this adapter is bound to.
    group_id: C::GroupId,
}

impl<C, N> GroupNetworkAdapter<C, N>
where
    C: MultiRaftTypeConfig,
    N: MultiRaftNetwork<C>,
{
    /// Creates a new adapter for the specified group.
    pub fn new(network: N, group_id: C::GroupId) -> Self {
        Self { network, group_id }
    }

    /// Returns a reference to the group ID.
    pub fn group_id(&self) -> &C::GroupId {
        &self.group_id
    }

    /// Returns a reference to the underlying network.
    pub fn inner(&self) -> &N {
        &self.network
    }

    /// Returns a mutable reference to the underlying network.
    pub fn inner_mut(&mut self) -> &mut N {
        &mut self.network
    }

    /// Unwraps the adapter, returning the underlying network.
    pub fn into_inner(self) -> N {
        self.network
    }

    /// Sends an AppendEntries request using the bound group ID.
    pub async fn append_entries(
        &mut self,
        rpc: AppendEntriesRequest<C>,
        option: RPCOption,
    ) -> Result<AppendEntriesResponse<C>, RPCError<C>> {
        let grouped = GroupedAppendEntriesRequest::new(self.group_id.clone(), rpc);
        let response = self.network.grouped_append_entries(grouped, option).await?;
        Ok(response.into_inner())
    }

    /// Sends a Vote request using the bound group ID.
    pub async fn vote(&mut self, rpc: VoteRequest<C>, option: RPCOption) -> Result<VoteResponse<C>, RPCError<C>> {
        let grouped = GroupedVoteRequest::new(self.group_id.clone(), rpc);
        let response = self.network.grouped_vote(grouped, option).await?;
        Ok(response.into_inner())
    }

    /// Sends a complete snapshot using the bound group ID.
    pub async fn full_snapshot(
        &mut self,
        vote: VoteOf<C>,
        snapshot: Snapshot<C>,
        cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
        option: RPCOption,
    ) -> Result<SnapshotResponse<C>, StreamingError<C>> {
        let response =
            self.network.grouped_full_snapshot(self.group_id.clone(), vote, snapshot, cancel, option).await?;
        Ok(response.into_inner())
    }

    /// Sends a TransferLeader request using the bound group ID.
    pub async fn transfer_leader(
        &mut self,
        req: TransferLeaderRequest<C>,
        option: RPCOption,
    ) -> Result<(), RPCError<C>> {
        self.network.grouped_transfer_leader(self.group_id.clone(), req, option).await
    }

    /// Returns the backoff configuration from the underlying network.
    pub fn backoff(&self) -> Backoff {
        self.network.backoff()
    }
}

// ============================================================================
// Implement RaftNetworkV2 for GroupNetworkAdapter
// ============================================================================

/// Implement `RaftNetworkV2` for `GroupNetworkAdapter`, allowing it to be used
/// anywhere a standard single-group network is expected.
///
/// This enables seamless integration between Multi-Raft shared connections
/// and existing code that uses `RaftNetworkV2`.
///
/// ## Feature Flag Requirement
///
/// **Important**: This implementation is only available when the `adapt-network-v1` feature is
/// **disabled**. When `adapt-network-v1` is enabled (which is the default), there's a blanket
/// implementation of `RaftNetworkV2` for all `RaftNetwork` implementors that would conflict.
///
/// To use `GroupNetworkAdapter` as `RaftNetworkV2`, configure your `Cargo.toml` as follows:
///
/// ```toml
/// [dependencies]
/// openraft = { version = "0.10", default-features = false, features = ["multi-raft", "tokio-rt"] }
/// ```
///
/// If you also need `serde` support:
///
/// ```toml
/// [dependencies]
/// openraft = { version = "0.10", default-features = false, features = ["multi-raft", "tokio-rt", "serde"] }
/// ```
///
/// ## Example
///
/// ```ignore
/// use openraft::multi_raft::{GroupNetworkAdapter, MultiRaftNetwork};
/// use openraft::network::v2::RaftNetworkV2;
///
/// // Create adapter from shared network
/// let adapter = GroupNetworkAdapter::new(shared_network, "my_group".to_string());
///
/// // Use as RaftNetworkV2
/// fn use_network<N: RaftNetworkV2<C>>(network: N) { /* ... */ }
/// use_network(adapter);
/// ```
#[cfg(not(feature = "adapt-network-v1"))]
impl<C, N> crate::network::v2::RaftNetworkV2<C> for GroupNetworkAdapter<C, N>
where
    C: MultiRaftTypeConfig,
    N: MultiRaftNetwork<C>,
{
    async fn append_entries(
        &mut self,
        rpc: AppendEntriesRequest<C>,
        option: RPCOption,
    ) -> Result<AppendEntriesResponse<C>, RPCError<C>> {
        let grouped = GroupedAppendEntriesRequest::new(self.group_id.clone(), rpc);
        let response = self.network.grouped_append_entries(grouped, option).await?;
        Ok(response.into_inner())
    }

    async fn vote(&mut self, rpc: VoteRequest<C>, option: RPCOption) -> Result<VoteResponse<C>, RPCError<C>> {
        let grouped = GroupedVoteRequest::new(self.group_id.clone(), rpc);
        let response = self.network.grouped_vote(grouped, option).await?;
        Ok(response.into_inner())
    }

    async fn full_snapshot(
        &mut self,
        vote: VoteOf<C>,
        snapshot: Snapshot<C>,
        cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
        option: RPCOption,
    ) -> Result<SnapshotResponse<C>, StreamingError<C>> {
        let response =
            self.network.grouped_full_snapshot(self.group_id.clone(), vote, snapshot, cancel, option).await?;
        Ok(response.into_inner())
    }

    async fn transfer_leader(&mut self, req: TransferLeaderRequest<C>, option: RPCOption) -> Result<(), RPCError<C>> {
        self.network.grouped_transfer_leader(self.group_id.clone(), req, option).await
    }

    fn backoff(&self) -> Backoff {
        self.network.backoff()
    }
}

// ============================================================================
// Implement From for convenient conversion
// ============================================================================

impl<C, N> From<(N, C::GroupId)> for GroupNetworkAdapter<C, N>
where
    C: MultiRaftTypeConfig,
    N: MultiRaftNetwork<C>,
{
    /// Create a `GroupNetworkAdapter` from a tuple of (network, group_id).
    ///
    /// ## Example
    ///
    /// ```ignore
    /// let adapter: GroupNetworkAdapter<_, _> = (shared_network, "my_group".to_string()).into();
    /// ```
    fn from((network, group_id): (N, C::GroupId)) -> Self {
        Self::new(network, group_id)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::Backoff;

    // Note: MultiRaftNetwork uses #[add_async_trait] which makes it not dyn-compatible
    // (async trait methods return impl Future). This is expected behavior.
    // The traits are designed to be used with generics, not trait objects.

    #[test]
    fn test_backoff_default() {
        // Test that default backoff works
        let backoff = Backoff::new(std::iter::repeat(Duration::from_millis(500)));
        let mut iter = backoff.into_iter();
        assert_eq!(iter.next(), Some(Duration::from_millis(500)));
    }
}
