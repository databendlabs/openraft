use std::future::Future;

use crate::OptionalSend;
use crate::RaftNetwork;
use crate::RaftTypeConfig;
use crate::error::RPCError;
use crate::error::ReplicationClosed;
use crate::error::StreamingError;
use crate::error::decompose::DecomposeResult;
use crate::network::Backoff;
use crate::network::RPCOption;
use crate::network::v2::RaftNetworkV2;
use crate::raft::AppendEntriesRequest;
use crate::raft::AppendEntriesResponse;
use crate::raft::SnapshotResponse;
use crate::raft::VoteRequest;
use crate::raft::VoteResponse;
use crate::storage::Snapshot;
use crate::type_config::alias::VoteOf;

impl<C, V1> RaftNetworkV2<C> for V1
where
    C: RaftTypeConfig,
    V1: RaftNetwork<C>,
    C::SnapshotData: tokio::io::AsyncRead + tokio::io::AsyncWrite + tokio::io::AsyncSeek + Unpin,
{
    async fn append_entries(
        &mut self,
        rpc: AppendEntriesRequest<C>,
        option: RPCOption,
    ) -> Result<AppendEntriesResponse<C>, RPCError<C>> {
        RaftNetwork::<C>::append_entries(self, rpc, option).await.decompose_infallible()
    }

    async fn vote(&mut self, rpc: VoteRequest<C>, option: RPCOption) -> Result<VoteResponse<C>, RPCError<C>> {
        RaftNetwork::<C>::vote(self, rpc, option).await.decompose_infallible()
    }

    async fn full_snapshot(
        &mut self,
        vote: VoteOf<C>,
        snapshot: Snapshot<C>,
        cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
        option: RPCOption,
    ) -> Result<SnapshotResponse<C>, StreamingError<C>> {
        use crate::network::snapshot_transport::Chunked;
        use crate::network::snapshot_transport::SnapshotTransport;

        let resp = Chunked::send_snapshot(self, vote, snapshot, cancel, option).await?;
        Ok(resp)
    }

    fn backoff(&self) -> Backoff {
        RaftNetwork::<C>::backoff(self)
    }
}
