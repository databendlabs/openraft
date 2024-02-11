use std::future::Future;
use std::io::SeekFrom;
use std::pin::Pin;
use std::time::Duration;

use futures::FutureExt;
use macros::add_async_trait;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncSeekExt;

use crate::error::Fatal;
use crate::error::InstallSnapshotError;
use crate::error::RPCError;
use crate::error::RaftError;
use crate::error::ReplicationClosed;
use crate::error::StreamingError;
use crate::network::rpc_option::RPCOption;
use crate::network::Backoff;
use crate::raft::AppendEntriesRequest;
use crate::raft::AppendEntriesResponse;
use crate::raft::InstallSnapshotRequest;
use crate::raft::InstallSnapshotResponse;
use crate::raft::SnapshotResponse;
use crate::raft::VoteRequest;
use crate::raft::VoteResponse;
use crate::type_config::alias::AsyncRuntimeOf;
use crate::AsyncRuntime;
use crate::ErrorSubject;
use crate::ErrorVerb;
use crate::OptionalSend;
use crate::OptionalSync;
use crate::RaftTypeConfig;
use crate::Snapshot;
use crate::ToStorageResult;
use crate::Vote;

/// A trait defining the interface for a Raft network between cluster members.
///
/// See the [network chapter of the guide](https://datafuselabs.github.io/openraft/getting-started.html#3-impl-raftnetwork)
/// for details and discussion on this trait and how to implement it.
///
/// A single network instance is used to connect to a single target node. The network instance is
/// constructed by the [`RaftNetworkFactory`](`crate::network::RaftNetworkFactory`).
///
/// ### 2023-05-03: New API with options
///
/// - This trait introduced 3 new API `append_entries`, `install_snapshot` and `vote` which accept
///   an additional argument [`RPCOption`], and deprecated the old API `send_append_entries`,
///   `send_install_snapshot` and `send_vote`.
///
/// - The old API will be **removed** in `0.9`. An application can still implement the old API
///   without any changes. Openraft calls only the new API and the default implementation will
///   delegate to the old API.
///
/// - Implementing the new APIs will disable the old APIs.
#[add_async_trait]
pub trait RaftNetwork<C>: OptionalSend + OptionalSync + 'static
where C: RaftTypeConfig
{
    /// Send an AppendEntries RPC to the target.
    async fn append_entries(
        &mut self,
        rpc: AppendEntriesRequest<C>,
        option: RPCOption,
    ) -> Result<AppendEntriesResponse<C::NodeId>, RPCError<C::NodeId, C::Node, RaftError<C::NodeId>>> {
        let _ = option;
        #[allow(deprecated)]
        self.send_append_entries(rpc).await
    }

    /// Send an InstallSnapshot RPC to the target.
    // TODO: will be deprecated in 0.10
    // #[deprecated(note = "use `snapshot` instead. This method will be removed in 0.10")]
    async fn install_snapshot(
        &mut self,
        rpc: InstallSnapshotRequest<C>,
        option: RPCOption,
    ) -> Result<
        InstallSnapshotResponse<C::NodeId>,
        RPCError<C::NodeId, C::Node, RaftError<C::NodeId, InstallSnapshotError>>,
    > {
        let _ = option;
        #[allow(deprecated)]
        self.send_install_snapshot(rpc).await
    }

    /// Send a RequestVote RPC to the target.
    async fn vote(
        &mut self,
        rpc: VoteRequest<C::NodeId>,
        option: RPCOption,
    ) -> Result<VoteResponse<C::NodeId>, RPCError<C::NodeId, C::Node, RaftError<C::NodeId>>> {
        let _ = option;
        #[allow(deprecated)]
        self.send_vote(rpc).await
    }

    /// Send a complete Snapshot to the target.
    ///
    /// This method is responsible to fragment the snapshot and send it to the target node.
    ///
    /// The default implementation will call `install_snapshot` RPC for each fragment.
    async fn snapshot(
        &mut self,
        vote: Vote<C::NodeId>,
        snapshot: Snapshot<C>,
        cancel: impl Future<Output = ReplicationClosed> + Send,
        option: RPCOption,
    ) -> Result<SnapshotResponse<C::NodeId>, StreamingError<C, Fatal<C::NodeId>>> {
        let resp = stream_snapshot(self, vote, snapshot, cancel, option).await?;
        Ok(resp)
    }

    /// Send an AppendEntries RPC to the target Raft node (§5).
    #[deprecated(note = "use `append_entries` instead. This method will be removed in 0.9")]
    async fn send_append_entries(
        &mut self,
        rpc: AppendEntriesRequest<C>,
    ) -> Result<AppendEntriesResponse<C::NodeId>, RPCError<C::NodeId, C::Node, RaftError<C::NodeId>>> {
        let _ = rpc;
        unimplemented!("send_append_entries is deprecated")
    }

    /// Send an InstallSnapshot RPC to the target Raft node (§7).
    #[deprecated(note = "use `install_snapshot` instead. This method will be removed in 0.9")]
    async fn send_install_snapshot(
        &mut self,
        rpc: InstallSnapshotRequest<C>,
    ) -> Result<
        InstallSnapshotResponse<C::NodeId>,
        RPCError<C::NodeId, C::Node, RaftError<C::NodeId, InstallSnapshotError>>,
    > {
        let _ = rpc;
        unimplemented!("send_install_snapshot is deprecated")
    }

    /// Send a RequestVote RPC to the target Raft node (§5).
    #[deprecated(note = "use `vote` instead. This method will be removed in 0.9")]
    async fn send_vote(
        &mut self,
        rpc: VoteRequest<C::NodeId>,
    ) -> Result<VoteResponse<C::NodeId>, RPCError<C::NodeId, C::Node, RaftError<C::NodeId>>> {
        let _ = rpc;
        unimplemented!("send_vote is deprecated")
    }

    /// Build a backoff instance if the target node is temporarily(or permanently) unreachable.
    ///
    /// When a [`Unreachable`](`crate::error::Unreachable`) error is returned from the `Network`
    /// methods, Openraft does not retry connecting to a node immediately. Instead, it sleeps
    /// for a while and retries. The duration of the sleep is determined by the backoff
    /// instance.
    ///
    /// The backoff is an infinite iterator that returns the ith sleep interval before the ith
    /// retry. The returned instance will be dropped if a successful RPC is made.
    ///
    /// By default it returns a constant backoff of 500 ms.
    fn backoff(&self) -> Backoff {
        Backoff::new(std::iter::repeat(Duration::from_millis(500)))
    }
}

async fn stream_snapshot<C, Net>(
    net: &mut Net,
    vote: Vote<C::NodeId>,
    mut snapshot: Snapshot<C>,
    mut cancel: impl Future<Output = ReplicationClosed>,
    option: RPCOption,
) -> Result<SnapshotResponse<C::NodeId>, StreamingError<C, Fatal<C::NodeId>>>
where
    C: RaftTypeConfig,
    Net: RaftNetwork<C> + ?Sized,
{
    let subject_verb = || (ErrorSubject::Snapshot(Some(snapshot.meta.signature())), ErrorVerb::Read);

    let mut offset = 0;
    let end = snapshot.snapshot.seek(SeekFrom::End(0)).await.sto_res(subject_verb)?;

    loop {
        // Safety: `cancel` is a future that is polled only by this function.
        let c = unsafe { Pin::new_unchecked(&mut cancel) };

        // If canceled, return at once
        if let Some(err) = c.now_or_never() {
            return Err(err.into());
        }

        // Sleep a short time otherwise in test environment it is a dead-loop that never
        // yields.
        // Because network implementation does not yield.
        AsyncRuntimeOf::<C>::sleep(Duration::from_millis(10)).await;

        snapshot.snapshot.seek(SeekFrom::Start(offset)).await.sto_res(subject_verb)?;

        // Safe unwrap(): this function is called only by default implementation of
        // `RaftNetwork::snapshot()` and it is always set.
        let chunk_size = option.snapshot_chunk_size().unwrap();
        let mut buf = Vec::with_capacity(chunk_size);
        while buf.capacity() > buf.len() {
            let n = snapshot.snapshot.read_buf(&mut buf).await.sto_res(subject_verb)?;
            if n == 0 {
                break;
            }
        }

        let n_read = buf.len();

        let done = (offset + n_read as u64) == end;
        let req = InstallSnapshotRequest {
            vote,
            meta: snapshot.meta.clone(),
            offset,
            data: buf,
            done,
        };

        // Send the RPC over to the target.
        tracing::debug!(
            snapshot_size = req.data.len(),
            req.offset,
            end,
            req.done,
            "sending snapshot chunk"
        );

        let res = AsyncRuntimeOf::<C>::timeout(option.hard_ttl(), net.install_snapshot(req, option.clone())).await;

        let resp = match res {
            Ok(outer_res) => match outer_res {
                Ok(res) => res,
                Err(err) => {
                    tracing::warn!(error=%err, "error sending InstallSnapshot RPC to target");
                    continue;
                }
            },
            Err(err) => {
                tracing::warn!(error=%err, "timeout while sending InstallSnapshot RPC to target");
                continue;
            }
        };

        if resp.vote > vote {
            // Unfinished, return a response with a higher vote.
            // The caller checks the vote and return a HigherVote error.
            return Ok(SnapshotResponse::new(resp.vote));
        }

        if done {
            return Ok(SnapshotResponse::new(resp.vote));
        }

        offset += n_read as u64;
    }
}
