//! Network adapters for Multi-Raft connection sharing.
//!
//! - [`GroupRouter`] - Trait for sending RPCs with target + group routing
//! - [`GroupNetworkAdapter`] - Wraps `GroupRouter` with (target, group_id), implements
//!   `RaftNetworkV2`
//! - [`GroupNetworkFactory`] - Simple wrapper for factory + group_id
//!
//! See `examples/multi-raft-kv` for usage.

use std::future::Future;
use std::time::Duration;

use openraft::error::RPCError;
use openraft::error::ReplicationClosed;
use openraft::error::StreamingError;
use openraft::error::Unreachable;
use openraft::network::v2::RaftNetworkV2;
use openraft::network::Backoff;
use openraft::network::RPCOption;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendEntriesResponse;
use openraft::raft::SnapshotResponse;
use openraft::raft::TransferLeaderRequest;
use openraft::raft::VoteRequest;
use openraft::raft::VoteResponse;
use openraft::storage::Snapshot;
use openraft::type_config::alias::VoteOf;
use openraft::OptionalSend;
use openraft::OptionalSync;
use openraft::RaftTypeConfig;

/// Trait for sending Raft RPCs with target and group routing.
///
/// Implement this on your shared router/connection pool to enable connection
/// sharing across all Raft groups. The adapter will bind (target, group_id).
pub trait GroupRouter<C, G>: Clone + OptionalSend + OptionalSync + 'static
where C: RaftTypeConfig
{
    /// Send AppendEntries to target node for a specific group.
    fn append_entries(
        &self,
        target: C::NodeId,
        group_id: G,
        rpc: AppendEntriesRequest<C>,
        option: RPCOption,
    ) -> impl Future<Output = Result<AppendEntriesResponse<C>, RPCError<C>>> + OptionalSend;

    /// Send Vote to target node for a specific group.
    fn vote(
        &self,
        target: C::NodeId,
        group_id: G,
        rpc: VoteRequest<C>,
        option: RPCOption,
    ) -> impl Future<Output = Result<VoteResponse<C>, RPCError<C>>> + OptionalSend;

    /// Send snapshot to target node for a specific group.
    fn full_snapshot(
        &self,
        target: C::NodeId,
        group_id: G,
        vote: VoteOf<C>,
        snapshot: Snapshot<C>,
        cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
        option: RPCOption,
    ) -> impl Future<Output = Result<SnapshotResponse<C>, StreamingError<C>>> + OptionalSend;

    /// Send TransferLeader to target node for a specific group.
    /// Default: returns "not implemented" error.
    fn transfer_leader(
        &self,
        _target: C::NodeId,
        _group_id: G,
        _req: TransferLeaderRequest<C>,
        _option: RPCOption,
    ) -> impl Future<Output = Result<(), RPCError<C>>> + OptionalSend {
        async {
            Err(RPCError::Unreachable(Unreachable::new(&anyerror::AnyError::error(
                "transfer_leader not implemented",
            ))))
        }
    }

    /// Backoff strategy for retries. Default: 500ms constant.
    fn backoff(&self) -> Backoff {
        Backoff::new(std::iter::repeat(Duration::from_millis(500)))
    }
}

/// Adapter that binds (target, group_id) to a shared router.
///
/// This wraps a [`GroupRouter`] implementation (e.g., your Router) and
/// automatically implements `RaftNetworkV2` for a specific (target, group).
pub struct GroupNetworkAdapter<C, G, N>
where
    C: RaftTypeConfig,
    N: GroupRouter<C, G>,
{
    router: N,
    target: C::NodeId,
    group_id: G,
}

impl<C, G, N> Clone for GroupNetworkAdapter<C, G, N>
where
    C: RaftTypeConfig,
    G: Clone,
    N: GroupRouter<C, G>,
{
    fn clone(&self) -> Self {
        Self {
            router: self.router.clone(),
            target: self.target.clone(),
            group_id: self.group_id.clone(),
        }
    }
}

impl<C, G, N> GroupNetworkAdapter<C, G, N>
where
    C: RaftTypeConfig,
    N: GroupRouter<C, G>,
{
    /// Create adapter binding router to specific (target, group).
    pub fn new(router: N, target: C::NodeId, group_id: G) -> Self {
        Self {
            router,
            target,
            group_id,
        }
    }

    /// Returns the target node ID.
    pub fn target(&self) -> &C::NodeId {
        &self.target
    }

    /// Returns the group ID.
    pub fn group_id(&self) -> &G {
        &self.group_id
    }
}

// Implement RaftNetworkV2 for GroupNetworkAdapter.
impl<C, G, N> RaftNetworkV2<C> for GroupNetworkAdapter<C, G, N>
where
    C: RaftTypeConfig,
    G: Clone + OptionalSend + OptionalSync + 'static,
    N: GroupRouter<C, G>,
{
    async fn append_entries(
        &mut self,
        rpc: AppendEntriesRequest<C>,
        option: RPCOption,
    ) -> Result<AppendEntriesResponse<C>, RPCError<C>> {
        self.router.append_entries(self.target.clone(), self.group_id.clone(), rpc, option).await
    }

    async fn vote(&mut self, rpc: VoteRequest<C>, option: RPCOption) -> Result<VoteResponse<C>, RPCError<C>> {
        self.router.vote(self.target.clone(), self.group_id.clone(), rpc, option).await
    }

    async fn full_snapshot(
        &mut self,
        vote: VoteOf<C>,
        snapshot: Snapshot<C>,
        cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
        option: RPCOption,
    ) -> Result<SnapshotResponse<C>, StreamingError<C>> {
        self.router
            .full_snapshot(
                self.target.clone(),
                self.group_id.clone(),
                vote,
                snapshot,
                cancel,
                option,
            )
            .await
    }

    async fn transfer_leader(&mut self, req: TransferLeaderRequest<C>, option: RPCOption) -> Result<(), RPCError<C>> {
        self.router.transfer_leader(self.target.clone(), self.group_id.clone(), req, option).await
    }

    fn backoff(&self) -> Backoff {
        self.router.backoff()
    }
}

/// Simple wrapper for network factory + group_id.
#[derive(Clone)]
pub struct GroupNetworkFactory<F, G> {
    /// The underlying factory (e.g., Router).
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
