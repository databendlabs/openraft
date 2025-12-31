//! Client-side adapter: converts `RaftNetwork` (v1) to `RaftNetworkV2`.
//!
//! This module provides [`Adapter`], a wrapper that allows any type implementing
//! `RaftNetwork` to be used where `RaftNetworkV2` is required. The key difference is
//! that `full_snapshot()` is implemented using chunked transmission via
//! `Chunked::send_snapshot()`.

use std::future::Future;
use std::marker::PhantomData;

use openraft::OptionalSend;
use openraft::RaftTypeConfig;
use openraft::error::RPCError;
use openraft::error::ReplicationClosed;
use openraft::error::StreamingError;
use openraft::error::decompose::DecomposeResult;
use openraft::network::Backoff;
use openraft::network::RPCOption;
use openraft::network::v2::RaftNetworkV2;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendEntriesResponse;
use openraft::raft::SnapshotResponse;
use openraft::raft::VoteRequest;
use openraft::raft::VoteResponse;
use openraft::storage::Snapshot;
use openraft::type_config::alias::VoteOf;

use super::RaftNetwork;
use super::sender::Sender;

/// Wrapper that adapts `RaftNetwork` (v1) to `RaftNetworkV2`.
///
/// This wrapper allows existing `RaftNetwork` implementations to be used where
/// `RaftNetworkV2` is required. The `full_snapshot()` method is implemented using
/// chunked transmission via `Chunked::send_snapshot()`, which calls `install_snapshot()`
/// for each chunk.
///
/// # Usage
///
/// ```ignore
/// use openraft_legacy::prelude::*;
///
/// // Your existing RaftNetwork implementation
/// let network: MyNetwork = /* ... */;
///
/// // Wrap it to use as RaftNetworkV2
/// let adapter = network.into_v2();
/// ```
pub struct Adapter<C, N> {
    network: N,
    _phantom: PhantomData<C>,
}

impl<C, N> Adapter<C, N>
where
    C: RaftTypeConfig,
    N: RaftNetwork<C>,
{
    /// Create a new adapter wrapping the given `RaftNetwork` implementation.
    pub fn new(network: N) -> Self {
        Self {
            network,
            _phantom: PhantomData,
        }
    }

    /// Get a reference to the underlying network.
    pub fn inner(&self) -> &N {
        &self.network
    }

    /// Get a mutable reference to the underlying network.
    pub fn inner_mut(&mut self) -> &mut N {
        &mut self.network
    }

    /// Consume the adapter and return the underlying network.
    pub fn into_inner(self) -> N {
        self.network
    }
}

impl<C, N> RaftNetworkV2<C> for Adapter<C, N>
where
    C: RaftTypeConfig,
    N: RaftNetwork<C>,
    C::SnapshotData: tokio::io::AsyncRead + tokio::io::AsyncWrite + tokio::io::AsyncSeek + Unpin,
{
    async fn append_entries(
        &mut self,
        rpc: AppendEntriesRequest<C>,
        option: RPCOption,
    ) -> Result<AppendEntriesResponse<C>, RPCError<C>> {
        RaftNetwork::<C>::append_entries(&mut self.network, rpc, option).await.decompose_infallible()
    }

    async fn vote(&mut self, rpc: VoteRequest<C>, option: RPCOption) -> Result<VoteResponse<C>, RPCError<C>> {
        RaftNetwork::<C>::vote(&mut self.network, rpc, option).await.decompose_infallible()
    }

    async fn full_snapshot(
        &mut self,
        vote: VoteOf<C>,
        snapshot: Snapshot<C>,
        cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
        option: RPCOption,
    ) -> Result<SnapshotResponse<C>, StreamingError<C>> {
        Sender::<C>::send_snapshot(&mut self.network, vote, snapshot, cancel, option).await
    }

    fn backoff(&self) -> Backoff {
        RaftNetwork::<C>::backoff(&self.network)
    }
}
