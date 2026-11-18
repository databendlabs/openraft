use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use openraft::OptionalSend;
use openraft::OptionalSync;
use openraft::RaftTypeConfig;
use openraft::async_runtime::MpscSender;
use openraft::error::NetworkError;
use openraft::error::RPCError;
use openraft::error::ReplicationClosed;
use openraft::error::StreamingError;
use openraft::network::Backoff;
use openraft::network::RPCOption;
use openraft::openraft_macros::since;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendResult;
use openraft::raft::SnapshotResponse;
use openraft::raft::TransferLeaderRequest;
use openraft::raft::TransferLeaderResponse;
use openraft::raft::VoteRequest;
use openraft::raft::VoteResponse;
use openraft::type_config::TypeConfigExt;
use openraft::type_config::alias::SnapshotOf;
use openraft::type_config::alias::VoteOf;

use crate::BatchConfig;
use crate::BatchItem;
use crate::BatchTransport;
use crate::GroupRouter;
use crate::peer_batcher::PeerBatcher;
use crate::peer_batcher::PeerQueue;
use crate::peer_batcher::Pending;

/// The queue of the batching task of each peer node.
type PeerQueues<C, G> = Arc<Mutex<BTreeMap<<C as RaftTypeConfig>::NodeId, PeerQueue<C, G>>>>;

/// A [`GroupRouter`] that sends the empty AppendEntries requests of many groups to one peer node
/// in shared batches.
///
/// An AppendEntries request without entries, a heartbeat or an empty replication probe, goes to a
/// queue per peer node. A task per peer sends the queued requests through `transport` in batches
/// that [`BatchConfig`] limits. Every other request goes to `inner` unchanged. Each caller gets the
/// result of its own group, as if it had sent its request alone.
///
/// Use it where `inner` would be used, for example in a
/// [`GroupNetworkAdapter`](crate::GroupNetworkAdapter). Every peer must accept batches through the
/// receiving side of `transport`.
#[since(version = "0.10.1")]
pub struct BatchingRouter<C, G, R, T>
where
    C: RaftTypeConfig,
    G: OptionalSend + 'static,
{
    inner: R,
    transport: T,
    config: BatchConfig,

    /// The queue of the batching task of each peer node, created on the first request to it.
    peers: PeerQueues<C, G>,

    /// The id of the next batched request.
    next_id: Arc<AtomicU64>,
}

impl<C, G, R, T> Clone for BatchingRouter<C, G, R, T>
where
    C: RaftTypeConfig,
    G: OptionalSend + 'static,
    R: Clone,
    T: Clone,
{
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            transport: self.transport.clone(),
            config: self.config.clone(),
            peers: self.peers.clone(),
            next_id: self.next_id.clone(),
        }
    }
}

impl<C, G, R, T> BatchingRouter<C, G, R, T>
where
    C: RaftTypeConfig,
    G: OptionalSend + 'static,
    T: BatchTransport<C, G>,
{
    /// Wrap `inner`, and send its empty AppendEntries requests through `transport` in batches.
    pub fn new(inner: R, transport: T, config: BatchConfig) -> Self {
        Self {
            inner,
            transport,
            config,
            peers: Arc::new(Mutex::new(BTreeMap::new())),
            next_id: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Return the router that sends every request that is not batched.
    pub fn inner(&self) -> &R {
        &self.inner
    }

    /// Queue `rpc` for the batching task of `target`, and wait for its reply.
    async fn append_in_batch(
        &self,
        target: C::NodeId,
        group_id: G,
        rpc: AppendEntriesRequest<C>,
        option: RPCOption,
    ) -> Result<AppendResult<C>, RPCError<C>> {
        let no_reply = || {
            let message = format!("no reply for a batched AppendEntries to node {}", target);
            RPCError::Network(NetworkError::from_string(message))
        };

        let queue = self.queue(&target);
        let (reply_tx, reply_rx) = C::oneshot();
        let pending = Pending {
            item: BatchItem {
                id: self.next_id.fetch_add(1, Ordering::Relaxed),
                group_id,
                rpc,
            },
            deadline: C::now() + option.hard_ttl(),
            reply_tx,
        };

        if queue.send(pending).await.is_err() {
            return Err(no_reply());
        }
        reply_rx.await.unwrap_or_else(|_| Err(no_reply()))
    }

    /// Return the queue of the batching task of `target`, and start the task on first use.
    fn queue(&self, target: &C::NodeId) -> PeerQueue<C, G> {
        let mut peers = self.peers.lock().unwrap();
        let queue = peers
            .entry(target.clone())
            .or_insert_with(|| PeerBatcher::spawn(target.clone(), self.transport.clone(), self.config.clone()));
        queue.clone()
    }
}

impl<C, G, R, T> GroupRouter<C, G> for BatchingRouter<C, G, R, T>
where
    C: RaftTypeConfig,
    G: Clone + OptionalSend + OptionalSync + 'static,
    R: GroupRouter<C, G>,
    T: BatchTransport<C, G>,
{
    type SnapshotData = R::SnapshotData;

    async fn append_entries(
        &self,
        target: C::NodeId,
        group_id: G,
        rpc: AppendEntriesRequest<C>,
        option: RPCOption,
    ) -> Result<AppendResult<C>, RPCError<C>> {
        if !rpc.entries.is_empty() {
            return self.inner.append_entries(target, group_id, rpc, option).await;
        }
        self.append_in_batch(target, group_id, rpc, option).await
    }

    async fn vote(
        &self,
        target: C::NodeId,
        group_id: G,
        rpc: VoteRequest<C>,
        option: RPCOption,
    ) -> Result<VoteResponse<C>, RPCError<C>> {
        self.inner.vote(target, group_id, rpc, option).await
    }

    async fn pre_vote(
        &self,
        target: C::NodeId,
        group_id: G,
        rpc: VoteRequest<C>,
        option: RPCOption,
    ) -> Result<VoteResponse<C>, RPCError<C>> {
        self.inner.pre_vote(target, group_id, rpc, option).await
    }

    async fn full_snapshot(
        &self,
        target: C::NodeId,
        group_id: G,
        vote: VoteOf<C>,
        snapshot: SnapshotOf<C, Self::SnapshotData>,
        cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
        option: RPCOption,
    ) -> Result<SnapshotResponse<C>, StreamingError<C>> {
        self.inner.full_snapshot(target, group_id, vote, snapshot, cancel, option).await
    }

    async fn transfer_leader(
        &self,
        target: C::NodeId,
        group_id: G,
        req: TransferLeaderRequest<C>,
        option: RPCOption,
    ) -> Result<TransferLeaderResponse<C>, RPCError<C>> {
        self.inner.transfer_leader(target, group_id, req, option).await
    }

    fn backoff(&self) -> Option<Backoff> {
        self.inner.backoff()
    }
}

#[cfg(test)]
mod batching_router_test;
