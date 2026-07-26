//! Handlers for the Raft protocol RPCs this node receives from its peers.

use openraft_macros::since;

use crate::OptionalSend;
use crate::Raft;
use crate::RaftTypeConfig;
use crate::errors::Fatal;
use crate::errors::RaftError;
use crate::errors::into_raft_result::IntoRaftResult;
use crate::raft::message::AppendEntriesRequest;
use crate::raft::message::AppendEntriesResponse;
use crate::raft::message::SnapshotResponse;
use crate::raft::message::TransferLeaderRequest;
use crate::raft::message::TransferLeaderResponse;
use crate::raft::message::VoteRequest;
use crate::raft::message::VoteResponse;
use crate::raft::stream_append::StreamAppendResult;
use crate::storage::RaftStateMachine;
use crate::type_config::alias::SnapshotDataOf;
use crate::type_config::alias::SnapshotOf;
use crate::type_config::alias::VoteOf;

/// Implement the receiving side of the Raft protocol: every method here is driven by an
/// incoming RPC from another node, not by the application.
impl<C, SM> Raft<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    /// Submit an AppendEntries RPC to this Raft node.
    ///
    /// These RPCs are sent by the cluster leader to replicate log entries (§5.3), and are also
    /// used as heartbeats (§5.2).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn append_entries(&self, rpc: AppendEntriesRequest<C>) -> Result<AppendEntriesResponse<C>, RaftError<C>> {
        self.protocol_api().append_entries(rpc).await.into_raft_result()
    }

    /// Submit a stream of AppendEntries RPCs to this Raft node.
    ///
    /// This is a stream-oriented version of [`Self::append_entries`] with pipelining support.
    /// It spawns a background task that reads from the input stream, sends requests to RaftCore,
    /// and forwards response receivers to the output stream. Responses are yielded in order.
    ///
    /// ## Pipelining Behavior
    ///
    /// - A background task reads from the input stream and sends to RaftCore
    /// - Uses a bounded channel (64 slots) for backpressure
    /// - Responses are yielded in order (FIFO) as they complete
    ///
    /// ## Output
    ///
    /// The output stream emits:
    /// - `Ok(log_id)` when logs are successfully flushed
    /// - `Err(e)` when an error occurs, which terminates the stream
    ///
    /// ## Pinning
    ///
    /// The returned stream is `!Unpin` because it uses async closures internally.
    /// You must pin the stream before calling `.next()`:
    ///
    /// ```ignore
    /// use std::pin::pin;
    ///
    /// let mut output = pin!(raft.stream_append(input));
    /// while let Some(result) = output.next().await { /* ... */ }
    /// ```
    ///
    /// Alternatively, use `Box::pin` for heap pinning if the stream needs to be stored or returned:
    ///
    /// ```ignore
    /// let mut output = Box::pin(raft.stream_append(input));
    /// ```
    ///
    /// # Example
    ///
    /// ```ignore
    /// use std::pin::pin;
    /// use futures_util::StreamExt;
    ///
    /// let input_stream = futures_util::stream::iter(vec![request1, request2, request3]);
    /// let mut output_stream = pin!(raft.stream_append(input_stream));
    ///
    /// while let Some(result) = output_stream.next().await {
    ///     match result {
    ///         Ok(Ok(log_id)) => println!("Flushed: {:?}", log_id),
    ///         Ok(Err(err)) => {
    ///             println!("Append error: {}", err);
    ///             break;
    ///         }
    ///         Err(fatal) => {
    ///             println!("Fatal: {}", fatal);
    ///             break;
    ///         }
    ///     }
    /// }
    /// ```
    #[since(version = "0.10.0", change = "stream item contains Fatal")]
    #[since(version = "0.10.0")]
    pub fn stream_append<S>(
        &self,
        stream: S,
    ) -> impl futures_util::Stream<Item = Result<StreamAppendResult<C>, Fatal<C>>> + OptionalSend + 'static
    where
        S: futures_util::Stream<Item = AppendEntriesRequest<C>> + OptionalSend + 'static,
    {
        self.protocol_api().stream_append(stream)
    }

    /// Submit a VoteRequest (RequestVote in the spec) RPC to this Raft node.
    ///
    /// These RPCs are sent by cluster peers which are in candidate state attempting to gather votes
    /// (§5.2).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn vote(&self, rpc: VoteRequest<C>) -> Result<VoteResponse<C>, RaftError<C>> {
        self.protocol_api().vote(rpc).await.into_raft_result()
    }

    /// Submit a Pre-Vote RPC to this Raft node.
    ///
    /// A pre-candidate sends this before incrementing its term to ask whether peers *would* grant
    /// it a vote at `rpc.vote` (a hypothetical next term). Handling this RPC never persists a
    /// vote or changes this node's term; it only reports whether the vote would be granted,
    /// judged by the same leader-lease and last-log-id rules as [`Raft::vote()`]. Pre-Vote is
    /// only used when [`Config::enable_pre_vote`](crate::Config::enable_pre_vote) is enabled on
    /// the sender.
    #[since(version = "0.10.0", change = "added for the Pre-Vote feature")]
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn pre_vote(&self, rpc: VoteRequest<C>) -> Result<VoteResponse<C>, RaftError<C>> {
        self.protocol_api().pre_vote(rpc).await.into_raft_result()
    }

    /// Get the latest snapshot from the state machine.
    ///
    /// The request is served directly by the state-machine worker, not `RaftCore`. It returns an
    /// error only when that worker fails to serve it, e.g., encountering a storage error or having
    /// stopped (`Fatal::Stopped`), which can occur while `RaftCore` is still running.
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn get_snapshot(&self) -> Result<Option<SnapshotOf<C, SM::SnapshotData>>, RaftError<C>> {
        self.protocol_api().get_snapshot().await.into_raft_result()
    }

    /// Get a snapshot data for receiving snapshot from the leader.
    ///
    /// It does not check `Vote` because it is a read operation and does not break raft
    /// protocol.
    #[since(version = "0.10.0", change = "SnapshotData without Box")]
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn begin_receiving_snapshot(&self) -> Result<SnapshotDataOf<C, SM>, RaftError<C>> {
        self.protocol_api().begin_receiving_snapshot().await.into_raft_result()
    }

    /// Install a completely received snapshot to the state machine.
    ///
    /// This method is used to implement an application defined snapshot transmission.
    /// The application receives a snapshot from the leader, in chunks or a stream, and
    /// then rebuild a snapshot, then pass the snapshot to Raft to install.
    #[since(version = "0.9.0")]
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn install_full_snapshot(
        &self,
        vote: VoteOf<C>,
        snapshot: SnapshotOf<C, SM::SnapshotData>,
    ) -> Result<SnapshotResponse<C>, Fatal<C>> {
        self.protocol_api().install_full_snapshot(vote, snapshot).await
    }

    /// Handle the LeaderTransfer request from a Leader node.
    ///
    /// If this node is the `to` node, it resets the Leader lease and triggers an election when the
    /// expected log entries are flushed.
    /// Otherwise, it just resets the Leader lease to allow the `to` node to become the Leader.
    ///
    /// The application calls
    /// [`Raft::trigger().transfer_leader()`](crate::raft::trigger::Trigger::transfer_leader) to
    /// submit Transfer Leader command. Then, the current Leader will broadcast it to every node in
    /// the cluster via [`RaftNetworkV2::transfer_leader`] and the implementation on the remote node
    /// responds to transfer leader request by calling this method.
    ///
    /// [`RaftNetworkV2::transfer_leader`]: crate::network::RaftNetworkV2::transfer_leader
    #[since(version = "0.10.0", change = "returns TransferLeaderResponse")]
    #[since(version = "0.10.0")]
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn handle_transfer_leader(
        &self,
        req: TransferLeaderRequest<C>,
    ) -> Result<TransferLeaderResponse<C>, Fatal<C>> {
        self.protocol_api().handle_transfer_leader(req).await
    }
}
