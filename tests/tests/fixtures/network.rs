use std::future::Future;
use std::sync::Arc;
use std::sync::Mutex;

use futures::Stream;
use futures::StreamExt;
use openraft::OptionalSend;
use openraft::RaftTypeConfig;
use openraft::Vote;
use openraft::alias::SnapshotOf;
use openraft::async_runtime::Mutex as AsyncMutex;
use openraft::base::BoxFuture;
use openraft::base::BoxStream;
use openraft::entry::RaftEntry;
use openraft::errors::Infallible;
use openraft::errors::RPCError;
use openraft::errors::ReplicationClosed;
use openraft::errors::StreamingError;
use openraft::errors::Unreachable;
use openraft::network::RPCOption;
use openraft::network::RaftNetworkFactory;
use openraft::network::stream_append_sequential;
use openraft::network::v2::RaftNetworkV2;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendEntriesResponse;
use openraft::raft::AppendEntriesResponseWithHint;
use openraft::raft::SnapshotResponse;
use openraft::raft::StreamAppendResult;
use openraft::raft::TransferLeaderRequest;
use openraft::raft::TransferLeaderResponse;
use openraft::raft::VoteRequest;
use openraft::raft::VoteResponse;
use openraft::type_config::TypeConfigExt;
use openraft::type_config::alias::MutexOf;
use openraft::vote::RaftLeaderIdExt;
use openraft_legacy::network_v1::InstallSnapshotRequest;
use openraft_legacy::network_v1::RaftNetwork as LegacyRaftNetwork;
use openraft_memstore::MemNodeId;
use openraft_memstore::TypeConfig as MemConfig;

use crate::fixtures::MemRaft;
use crate::fixtures::MemSnapshotData;
use crate::fixtures::RaftRouter;
use crate::fixtures::TestingVoteExt;
use crate::fixtures::chunked_network::ChunkedNetwork;
use crate::fixtures::rpc_request::RpcRequest;
use crate::fixtures::rpc_response::RpcResponse;

impl RaftNetworkFactory<MemConfig> for RaftRouter {
    type Network = RaftRouterNetwork;

    async fn new_client(&mut self, target: MemNodeId, _node: &()) -> Self::Network {
        RaftRouterNetwork {
            target,
            owner: self.clone(),
            append_entries_lock: Arc::new(MemConfig::mutex(())),
        }
    }
}

#[derive(Clone)]
pub struct RaftRouterNetwork {
    target: MemNodeId,
    owner: RaftRouter,
    append_entries_lock: Arc<MutexOf<MemConfig, ()>>,
}

impl RaftNetworkV2<MemConfig> for RaftRouterNetwork {
    type SnapshotData = MemSnapshotData;

    fn stream_append<'s, S>(
        &'s mut self,
        input: S,
        option: RPCOption,
    ) -> BoxFuture<
        's,
        Result<BoxStream<'s, Result<StreamAppendResult<MemConfig>, RPCError<MemConfig>>>, RPCError<MemConfig>>,
    >
    where
        S: Stream<Item = AppendEntriesRequest<MemConfig>> + OptionalSend + Unpin + 'static,
    {
        if !self.owner.pipelined_append {
            return stream_append_sequential(self, input, option);
        }
        const PIPELINE_CAPACITY: usize = 64;
        let network = self.clone();
        let requests = input.map(move |request| {
            let network = network.clone();
            let option = option.clone();
            network.append_stream_request(request, option)
        });
        let responses = requests.buffered(PIPELINE_CAPACITY);
        let responses: BoxStream<'s, _> = Box::pin(responses);
        Box::pin(futures::future::ready(Ok(responses)))
    }

    async fn append_entries_with_hint(
        &mut self,
        rpc: &AppendEntriesRequest<MemConfig>,
        _option: RPCOption,
    ) -> Result<Option<AppendEntriesResponseWithHint<MemConfig>>, RPCError<MemConfig>> {
        if !self.owner.conflict_hints {
            return Ok(None);
        }
        let has_entries = !rpc.entries.is_empty();
        assert!(has_entries, "commit-only requests use the streaming path");
        let mut rpc = rpc.clone();
        let delivery = self.append_entries_lock.lock().await;
        let from = rpc.vote.to_leader_node_id();
        self.owner.before_rpc::<Infallible>(rpc.clone(), from, self.target).await?;
        let truncated = self.truncate_by_quota(&mut rpc);
        let node = self.owner.get_raft_handle(&self.target)?;
        let result = node.append_entries_with_hint(rpc.clone()).await;
        let mut response = result.map_err(|error| RPCError::Unreachable(Unreachable::new(&error)))?;
        if let Some(truncated) = truncated
            && response.response.is_success()
        {
            response.response = AppendEntriesResponse::PartialSuccess(truncated);
        }
        drop(delivery);
        self.owner.call_rpc_post_hook(rpc, response.response.clone(), from, self.target).await?;
        Ok(Some(response))
    }

    /// Send an AppendEntries RPC to the target Raft node (§5).
    async fn append_entries(
        &mut self,
        mut rpc: AppendEntriesRequest<MemConfig>,
        _option: RPCOption,
    ) -> Result<AppendEntriesResponse<MemConfig>, RPCError<MemConfig>> {
        let delivery = self.append_entries_lock.lock().await;
        let from_id = rpc.vote.to_leader_node_id();

        tracing::debug!("append_entries to id={} {}", self.target, rpc);
        self.owner.before_rpc::<Infallible>(rpc.clone(), from_id, self.target).await?;

        // decrease quota if quota is set
        let truncated = self.truncate_by_quota(&mut rpc);

        {
            let x = self.owner.append_entries_quota.lock().unwrap();
            tracing::debug!("quota after consumption: {:?}", *x);
        }
        tracing::debug!("append_entries truncated: {:?}", truncated);

        let node = self.owner.get_raft_handle(&self.target)?;

        let resp = node.append_entries(rpc.clone()).await;

        tracing::debug!("append_entries: recv resp from id={} {:?}", self.target, resp);
        let resp = resp.map_err(|e| {
            RPCError::Unreachable(Unreachable::<MemConfig>::from_string(format!(
                "error: {} target={}",
                e, self.target
            )))
        })?;

        // If entries are truncated by quota, return an partial success response.
        let resp = if let Some(truncated) = truncated {
            match resp {
                AppendEntriesResponse::Success => AppendEntriesResponse::PartialSuccess(truncated),
                response => response,
            }
        } else {
            resp
        };
        // Response hooks may block while later requests are delivered in order.
        drop(delivery);
        self.owner.call_rpc_post_hook(rpc, resp.clone(), from_id, self.target).await?;
        Ok(resp)
    }

    async fn full_snapshot(
        &mut self,
        vote: Vote<<MemConfig as RaftTypeConfig>::LeaderId>,
        snapshot: SnapshotOf<MemConfig, Self::SnapshotData>,
        cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
        option: RPCOption,
    ) -> Result<SnapshotResponse<MemConfig>, StreamingError<MemConfig>> {
        let from_id = vote.leader_id().to_node_id();
        let operation = self.transmit_full_snapshot(vote, snapshot, from_id, option);
        let cancel = std::pin::pin!(cancel);
        let operation = std::pin::pin!(operation);
        let raced = futures::future::select(cancel, operation).await;
        match raced {
            futures::future::Either::Left((closed, _operation)) => Err(StreamingError::Closed(closed)),
            futures::future::Either::Right((result, _cancel)) => result,
        }
    }

    /// Send a RequestVote RPC to the target Raft node (§5).
    async fn vote(
        &mut self,
        rpc: VoteRequest<MemConfig>,
        _option: RPCOption,
    ) -> Result<VoteResponse<MemConfig>, RPCError<MemConfig>> {
        let from_id = rpc.vote.leader_id().to_node_id();

        self.owner.before_rpc::<Infallible>(rpc.clone(), from_id, self.target).await?;

        let node = self.owner.get_raft_handle(&self.target)?;

        let resp = node.vote(rpc.clone()).await;
        let resp = resp.map_err(|e| {
            RPCError::Unreachable(Unreachable::<MemConfig>::from_string(format!(
                "error: {} target={}",
                e, self.target
            )))
        })?;

        self.owner.call_rpc_post_hook(rpc, resp.clone(), from_id, self.target).await?;

        Ok(resp)
    }

    async fn pre_vote(
        &mut self,
        rpc: VoteRequest<MemConfig>,
        _option: RPCOption,
    ) -> Result<VoteResponse<MemConfig>, RPCError<MemConfig>> {
        let from_id = rpc.vote.leader_id().to_node_id();

        let request = RpcRequest::PreVote(rpc.clone());
        self.owner.before_rpc::<Infallible>(request, from_id, self.target).await?;

        let node = self.owner.get_raft_handle(&self.target)?;

        let resp = node.pre_vote(rpc.clone()).await;
        let resp = resp.map_err(|e| {
            RPCError::Unreachable(Unreachable::<MemConfig>::from_string(format!(
                "error: {} target={}",
                e, self.target
            )))
        })?;

        let request = RpcRequest::PreVote(rpc);
        let response = RpcResponse::PreVote(resp.clone());
        self.owner.call_rpc_post_hook(request, response, from_id, self.target).await?;

        Ok(resp)
    }

    async fn transfer_leader(
        &mut self,
        rpc: TransferLeaderRequest<MemConfig>,
        _option: RPCOption,
    ) -> Result<TransferLeaderResponse<MemConfig>, RPCError<MemConfig>> {
        let from_id = rpc.from_leader().leader_id().to_node_id();

        self.owner.before_rpc::<Infallible>(rpc.clone(), from_id, self.target).await?;

        let node = self.owner.get_raft_handle(&self.target)?;

        let resp = node.handle_transfer_leader(rpc.clone()).await;
        let resp = resp.map_err(|err| {
            RPCError::Unreachable(Unreachable::<MemConfig>::from_string(format!(
                "error: {} target={}",
                err, self.target
            )))
        })?;

        self.owner.call_rpc_post_hook(rpc, resp.clone(), from_id, self.target).await?;

        Ok(resp)
    }
}

impl RaftRouterNetwork {
    fn truncate_by_quota(
        &self,
        rpc: &mut AppendEntriesRequest<MemConfig>,
    ) -> Option<Option<openraft::alias::LogIdOf<MemConfig>>> {
        let mut quota = self.owner.append_entries_quota.lock().unwrap();
        let remaining = (*quota)?;
        let len = rpc.entries.len() as u64;
        if remaining >= len {
            *quota = Some(remaining - len);
            return None;
        }
        rpc.entries.truncate(remaining as usize);
        *quota = Some(0);
        let last = match rpc.entries.last() {
            Some(entry) => Some(entry.log_id()),
            None => rpc.prev_log_id,
        };
        Some(last)
    }

    async fn append_stream_request(
        mut self,
        request: AppendEntriesRequest<MemConfig>,
        option: RPCOption,
    ) -> Result<StreamAppendResult<MemConfig>, RPCError<MemConfig>> {
        let prev = request.prev_log_id;
        let last = if let Some(entry) = request.entries.last() {
            let log_id = entry.log_id();
            Some(log_id)
        } else {
            prev
        };
        let response = self.append_entries(request, option).await?;
        let result = response.into_stream_result(prev, last);
        Ok(result)
    }

    async fn transmit_full_snapshot(
        &self,
        vote: Vote<<MemConfig as RaftTypeConfig>::LeaderId>,
        snapshot: SnapshotOf<MemConfig, MemSnapshotData>,
        from_id: MemNodeId,
        option: RPCOption,
    ) -> Result<SnapshotResponse<MemConfig>, StreamingError<MemConfig>> {
        self.owner.before_rpc::<Infallible>(snapshot.clone(), from_id, self.target).await?;
        let node = self.owner.get_raft_handle(&self.target)?;
        let chunks = self.owner.snapshot_chunks.clone();
        let resp = if let Some(requests) = chunks {
            self.deliver_chunked_snapshot(node, requests, vote, snapshot.clone(), option).await?
        } else {
            self.deliver_direct_snapshot(node, vote, snapshot.clone()).await?
        };
        self.owner.call_rpc_post_hook(snapshot, resp.clone(), from_id, self.target).await?;
        Ok(resp)
    }

    async fn deliver_chunked_snapshot(
        &self,
        node: MemRaft,
        requests: Arc<Mutex<Vec<InstallSnapshotRequest<MemConfig>>>>,
        vote: Vote<<MemConfig as RaftTypeConfig>::LeaderId>,
        snapshot: SnapshotOf<MemConfig, MemSnapshotData>,
        option: RPCOption,
    ) -> Result<SnapshotResponse<MemConfig>, StreamingError<MemConfig>> {
        let network = ChunkedNetwork {
            node,
            owner: self.owner.clone(),
            requests,
        };
        let mut network = network.into_v2::<MemSnapshotData>();
        // The outer select owns cancellation. The adapter still requires a future.
        let cancel = std::future::pending::<ReplicationClosed>();
        network.full_snapshot(vote, snapshot, cancel, option).await
    }

    async fn deliver_direct_snapshot(
        &self,
        node: MemRaft,
        vote: Vote<<MemConfig as RaftTypeConfig>::LeaderId>,
        snapshot: SnapshotOf<MemConfig, MemSnapshotData>,
    ) -> Result<SnapshotResponse<MemConfig>, StreamingError<MemConfig>> {
        let response = node.install_full_snapshot(vote, snapshot).await;
        let resp = response.map_err(|error| {
            let message = format!("error: {} target={}", error, self.target);
            let unreachable = Unreachable::<MemConfig>::from_string(message);
            RPCError::Unreachable(unreachable)
        })?;
        Ok(resp)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use anyhow::Result;
    use openraft::Config;
    use openraft::Instant;
    use openraft::alias::VoteOf;
    use openraft::network::RPCOption;
    use openraft::network::RaftNetworkFactory;
    use openraft::network::v2::RaftNetworkV2;
    use openraft::raft::VoteRequest;
    use openraft::type_config::TypeConfigExt;
    use openraft_memstore::TypeConfig;

    use crate::fixtures::RaftRouter;
    use crate::fixtures::ut_harness;

    const SEND_DELAY_MS: u64 = 50;

    /// One router RPC observes the send delay set after build.
    #[tracing::instrument]
    #[test_harness::test(harness = ut_harness)]
    async fn router_send_delay_bounds_one_rpc() -> Result<()> {
        let config = Config {
            enable_tick: false,
            ..Default::default()
        };
        let mut router = RaftRouter::new(config)?;
        router.new_raft_node(2).await?;
        router.network_send_delay(SEND_DELAY_MS);

        let mut client = RaftNetworkFactory::new_client(&mut router, 2, &()).await;
        let vote = VoteOf::<TypeConfig>::new(1, 1);
        let request = VoteRequest::new(vote, None);
        let option = RPCOption::new(Duration::from_secs(1));

        let started = TypeConfig::now();
        let _response = client.vote(request, option).await?;
        let finished = TypeConfig::now();
        let elapsed = finished.saturating_duration_since(started);
        let elapsed_ms = elapsed.as_millis();
        assert!(elapsed_ms >= 1, "elapsed {elapsed_ms} ms, expected at least 1");

        if cfg!(feature = "rt-sim") {
            let upper_ms = u128::from(SEND_DELAY_MS);
            assert!(
                elapsed_ms <= upper_ms,
                "elapsed {elapsed_ms} ms, expected at most {upper_ms}"
            );
        }
        Ok(())
    }
}
