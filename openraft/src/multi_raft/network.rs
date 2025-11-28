//! Network adapters for Multi-Raft connection sharing.
//!
//! - [`GroupedRpc`] - Trait for sending RPCs with group_id
//! - [`GroupNetworkAdapter`] - Wraps `GroupedRpc` and implements `RaftNetworkV2`
//! - [`GroupNetworkFactory`] - Simple wrapper for factory + group_id

use std::future::Future;
use std::time::Duration;

use crate::OptionalSend;
use crate::OptionalSync;
use crate::RaftTypeConfig;
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
use crate::storage::Snapshot;
use crate::type_config::alias::VoteOf;

/// Trait for sending Raft RPCs with group routing.
///
/// Implement this on your shared connection to enable multi-group RPC routing.
/// Then wrap it with [`GroupNetworkAdapter`] to get a `RaftNetworkV2` implementation.
pub trait GroupedRpc<C, G>: OptionalSend + OptionalSync + 'static
where C: RaftTypeConfig
{
    /// Send AppendEntries with group routing.
    fn send_append_entries(
        &mut self,
        group_id: G,
        rpc: AppendEntriesRequest<C>,
        option: RPCOption,
    ) -> impl Future<Output = Result<AppendEntriesResponse<C>, RPCError<C>>> + OptionalSend;

    /// Send Vote with group routing.
    fn send_vote(
        &mut self,
        group_id: G,
        rpc: VoteRequest<C>,
        option: RPCOption,
    ) -> impl Future<Output = Result<VoteResponse<C>, RPCError<C>>> + OptionalSend;

    /// Send snapshot with group routing.
    fn send_snapshot(
        &mut self,
        group_id: G,
        vote: VoteOf<C>,
        snapshot: Snapshot<C>,
        cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
        option: RPCOption,
    ) -> impl Future<Output = Result<SnapshotResponse<C>, StreamingError<C>>> + OptionalSend;

    /// Backoff strategy for retries. Default: 500ms constant.
    fn backoff(&self) -> Backoff {
        Backoff::new(std::iter::repeat(Duration::from_millis(500)))
    }
}

/// Adapter that wraps a [`GroupedRpc`] and implements `RaftNetworkV2`.
///
/// This is the key adapter: it binds a `group_id` to a shared connection,
/// allowing it to be used as a standard `RaftNetworkV2`.
///
/// ## Example
///
/// ```ignore
/// // Create adapter for "users" group
/// let network = GroupNetworkAdapter::new(shared_conn.clone(), "users".to_string());
///
/// // Now `network` implements RaftNetworkV2 and can be used with Raft
/// ```
pub struct GroupNetworkAdapter<C, G, N>
where
    C: RaftTypeConfig,
    N: GroupedRpc<C, G>,
{
    network: N,
    group_id: G,
    _phantom: std::marker::PhantomData<C>,
}

impl<C, G, N> Clone for GroupNetworkAdapter<C, G, N>
where
    C: RaftTypeConfig,
    G: Clone,
    N: GroupedRpc<C, G> + Clone,
{
    fn clone(&self) -> Self {
        Self {
            network: self.network.clone(),
            group_id: self.group_id.clone(),
            _phantom: std::marker::PhantomData,
        }
    }
}

impl<C, G, N> GroupNetworkAdapter<C, G, N>
where
    C: RaftTypeConfig,
    N: GroupedRpc<C, G>,
{
    /// Create adapter binding a shared connection to a specific group.
    pub fn new(network: N, group_id: G) -> Self {
        Self {
            network,
            group_id,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Returns a reference to the group_id.
    pub fn group_id(&self) -> &G {
        &self.group_id
    }
}

// Implement RaftNetworkV2 for GroupNetworkAdapter.
// Requires disabling `adapt-network-v1` to avoid conflict with blanket impl.
#[cfg(not(feature = "adapt-network-v1"))]
impl<C, G, N> crate::network::v2::RaftNetworkV2<C> for GroupNetworkAdapter<C, G, N>
where
    C: RaftTypeConfig,
    G: Clone + OptionalSend + OptionalSync + 'static,
    N: GroupedRpc<C, G>,
{
    async fn append_entries(
        &mut self,
        rpc: AppendEntriesRequest<C>,
        option: RPCOption,
    ) -> Result<AppendEntriesResponse<C>, RPCError<C>> {
        self.network.send_append_entries(self.group_id.clone(), rpc, option).await
    }

    async fn vote(&mut self, rpc: VoteRequest<C>, option: RPCOption) -> Result<VoteResponse<C>, RPCError<C>> {
        self.network.send_vote(self.group_id.clone(), rpc, option).await
    }

    async fn full_snapshot(
        &mut self,
        vote: VoteOf<C>,
        snapshot: Snapshot<C>,
        cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
        option: RPCOption,
    ) -> Result<SnapshotResponse<C>, StreamingError<C>> {
        self.network.send_snapshot(self.group_id.clone(), vote, snapshot, cancel, option).await
    }

    fn backoff(&self) -> Backoff {
        self.network.backoff()
    }
}

/// Simple wrapper for network factory + group_id.
#[derive(Clone)]
pub struct GroupNetworkFactory<F, G> {
    /// The underlying factory.
    pub factory: F,
    /// The group ID.
    pub group_id: G,
}

impl<F, G> GroupNetworkFactory<F, G> {
    /// Create a new factory wrapper.
    pub fn new(factory: F, group_id: G) -> Self {
        Self { factory, group_id }
    }
}
