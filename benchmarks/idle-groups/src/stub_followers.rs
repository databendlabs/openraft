use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use openraft::OptionalSend;
use openraft::error::RPCError;
use openraft::error::ReplicationClosed;
use openraft::error::StreamingError;
use openraft::network::RPCOption;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendResult;
use openraft::raft::SnapshotResponse;
use openraft::raft::VoteRequest;
use openraft::raft::VoteResponse;
use openraft::type_config::alias::SnapshotOf;
use openraft::type_config::alias::VoteOf;
use openraft_multi::GroupRouter;

use crate::C;

/// Stands in for the followers of every group: it grants every vote, acknowledges every
/// AppendEntries request, and counts the AppendEntries requests.
#[derive(Clone)]
pub(crate) struct StubFollowers {
    appends: Arc<AtomicU64>,
}

impl StubFollowers {
    /// Count the AppendEntries requests in `appends`.
    pub(crate) fn new(appends: Arc<AtomicU64>) -> Self {
        Self { appends }
    }
}

impl GroupRouter<C, u64> for StubFollowers {
    type SnapshotData = std::io::Cursor<Vec<u8>>;

    async fn append_entries(
        &self,
        _target: u64,
        _group_id: u64,
        rpc: AppendEntriesRequest<C>,
        _option: RPCOption,
    ) -> Result<AppendResult<C>, RPCError<C>> {
        self.appends.fetch_add(1, Ordering::Relaxed);
        Ok(Ok(rpc.last_log_id()))
    }

    async fn vote(
        &self,
        _target: u64,
        _group_id: u64,
        rpc: VoteRequest<C>,
        _option: RPCOption,
    ) -> Result<VoteResponse<C>, RPCError<C>> {
        Ok(VoteResponse::new(rpc.vote, rpc.last_log_id, true))
    }

    async fn pre_vote(
        &self,
        _target: u64,
        _group_id: u64,
        rpc: VoteRequest<C>,
        _option: RPCOption,
    ) -> Result<VoteResponse<C>, RPCError<C>> {
        Ok(VoteResponse::new(rpc.vote, rpc.last_log_id, true))
    }

    async fn full_snapshot(
        &self,
        _target: u64,
        _group_id: u64,
        vote: VoteOf<C>,
        _snapshot: SnapshotOf<C, Self::SnapshotData>,
        _cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
        _option: RPCOption,
    ) -> Result<SnapshotResponse<C>, StreamingError<C>> {
        Ok(SnapshotResponse::new(vote))
    }
}
