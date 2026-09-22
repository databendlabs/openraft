use std::future::Future;

use dir_transfer::DirFrameProducer;
use openraft::NodeInfo;
use openraft::OptionalSend;
use openraft::errors::NetworkError;
use openraft::errors::ReplicationClosed;
use openraft::errors::Unreachable;
use openraft::network::RPCOption;
use openraft::network::RaftNetworkFactory;
use openraft::network::v2::RaftNetworkV2;
use openraft::raft::TransferLeaderRequest;
use openraft::raft::TransferLeaderResponse;
use reqwest::Client as HttpClient;

use super::snapshot_stream;
use crate::NodeId;
use crate::TypeConfig;
use crate::typ::*;

/// Size of one `Chunk` frame when the RPC option names no snapshot chunk size.
const DEFAULT_CHUNK_SIZE: usize = 1024 * 1024;

/// Creates one [`Network`] per target node.
pub struct NetworkFactory {
    inner: network_v2_http::NetworkFactory,
    client: HttpClient,
}

impl Default for NetworkFactory {
    fn default() -> Self {
        Self::new()
    }
}

impl NetworkFactory {
    pub fn new() -> Self {
        Self {
            inner: network_v2_http::NetworkFactory::new(),
            client: HttpClient::builder().no_proxy().build().unwrap(),
        }
    }
}

impl RaftNetworkFactory<TypeConfig> for NetworkFactory {
    type Network = Network;

    async fn new_client(&mut self, target: NodeId, node: &NodeInfo) -> Self::Network {
        Network {
            inner: RaftNetworkFactory::<TypeConfig>::new_client(&mut self.inner, target, node).await,
            snapshot_url: format!("http://{}/snapshot", node.raft_addr),
            client: self.client.clone(),
        }
    }
}

/// The connection to one node.
///
/// JSON RPCs are delegated to the `network-v2-http` client. `full_snapshot` streams the
/// checkpoint directory of a [`SnapshotData`] as `dir-transfer` frames.
pub struct Network {
    inner: network_v2_http::Client,
    snapshot_url: String,
    client: HttpClient,
}

impl RaftNetworkV2<TypeConfig> for Network {
    type SnapshotData = SnapshotData;

    async fn append_entries(
        &mut self,
        req: AppendEntriesRequest,
        option: RPCOption,
    ) -> Result<AppendEntriesResponse, RPCError> {
        self.inner.append_entries(req, option).await
    }

    async fn full_snapshot(
        &mut self,
        vote: Vote,
        snapshot: Snapshot,
        cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
        option: RPCOption,
    ) -> Result<SnapshotResponse, StreamingError> {
        let configured_chunk_size = option.snapshot_chunk_size().unwrap_or(DEFAULT_CHUNK_SIZE);
        let chunk_size = configured_chunk_size.min(dir_transfer::MAX_CHUNK_SIZE);
        let producer =
            DirFrameProducer::new(snapshot.snapshot.db_path(), chunk_size).map_err(|e| NetworkError::new(&e))?;
        let body =
            snapshot_stream::request_body(&(vote, snapshot.meta), producer).map_err(|e| NetworkError::new(&e))?;
        let request = self.client.post(&self.snapshot_url).body(body).timeout(option.soft_ttl()).send();

        tokio::pin!(cancel);
        let response = tokio::select! {
            closed = &mut cancel => return Err(StreamingError::Closed(closed)),
            response = request => response,
        };

        let response = response.map_err(|e| {
            if e.is_connect() {
                StreamingError::Unreachable(Unreachable::new(&e))
            } else {
                StreamingError::Network(NetworkError::new(&e))
            }
        })?;

        let status = response.status();
        if !status.is_success() {
            let message = format!("HTTP {status} from {}", self.snapshot_url);
            return Err(StreamingError::Network(NetworkError::from_string(message)));
        }

        let result: Result<SnapshotResponse, RaftError> = response.json().await.map_err(|e| NetworkError::new(&e))?;
        result.map_err(|e| StreamingError::Unreachable(Unreachable::new(&e)))
    }

    async fn transfer_leader(
        &mut self,
        req: TransferLeaderRequest<TypeConfig>,
        option: RPCOption,
    ) -> Result<TransferLeaderResponse<TypeConfig>, RPCError> {
        self.inner.transfer_leader(req, option).await
    }

    async fn vote(&mut self, req: VoteRequest, option: RPCOption) -> Result<VoteResponse, RPCError> {
        self.inner.vote(req, option).await
    }

    async fn pre_vote(&mut self, req: VoteRequest, option: RPCOption) -> Result<VoteResponse, RPCError> {
        self.inner.pre_vote(req, option).await
    }
}
