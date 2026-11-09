use std::future::Future;
use std::io;

use openraft::OptionalSend;
use openraft::alias::VoteOf;
use openraft::errors::RPCError;
use openraft::errors::ReplicationClosed;
use openraft::errors::StreamingError;
use openraft::errors::Unreachable;
use openraft::network::RPCOption;
use openraft::network::v2::RaftNetworkV2;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendEntriesResponse;
use openraft::raft::AppendResult;
use openraft::raft::SnapshotResponse;
use openraft::raft::VoteRequest;
use openraft::raft::VoteResponse;

use crate::Raft;
use crate::Snapshot;
use crate::TypeConfig;
use crate::codec::Codec;
use crate::network::Network;

pub(crate) struct Connection {
    pub(crate) network: Network,
    pub(crate) target: u64,
}

impl Connection {
    async fn target(&self) -> io::Result<Raft> {
        self.network.0.read().await.get(&self.target).cloned().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotConnected,
                format!("node {} is not connected", self.target),
            )
        })
    }

    fn unreachable<E>(error: E) -> RPCError<TypeConfig>
    where E: std::error::Error + 'static {
        RPCError::Unreachable(Unreachable::new(&error))
    }

    async fn send<T>(
        &self,
        option: RPCOption,
        future: impl Future<Output = io::Result<T>>,
    ) -> Result<T, RPCError<TypeConfig>> {
        tokio::time::timeout(option.soft_ttl(), future)
            .await
            .map_err(Self::unreachable)?
            .map_err(Self::unreachable)
    }
}

impl RaftNetworkV2<TypeConfig> for Connection {
    type SnapshotData = std::io::Cursor<Vec<u8>>;

    async fn append_entries(
        &mut self,
        request: AppendEntriesRequest<TypeConfig>,
        option: RPCOption,
    ) -> Result<AppendResult<TypeConfig>, RPCError<TypeConfig>> {
        let prev = request.prev_log_id;
        let last = request.last_log_id();
        self.send(option, async {
            let target = self.target().await?;
            // ArchiveBuffer's serde implementation copies its archived bytes, not its fields.
            // Decode constructs fresh, validated aligned buffers before calling the follower.
            let request = Codec::round_trip(&request)?;
            let result = target.append_entries(request).await.map_err(io::Error::other)?;
            let response = AppendEntriesResponse::from(result);
            let response: AppendEntriesResponse<TypeConfig> = Codec::round_trip(&response)?;
            Ok(response.into_append_result(prev, last))
        })
        .await
    }

    async fn vote(
        &mut self,
        request: VoteRequest<TypeConfig>,
        option: RPCOption,
    ) -> Result<VoteResponse<TypeConfig>, RPCError<TypeConfig>> {
        self.send(option, async {
            let target = self.target().await?;
            let request = Codec::round_trip(&request)?;
            let response = target.vote(request).await.map_err(io::Error::other)?;
            Codec::round_trip(&response)
        })
        .await
    }

    async fn pre_vote(
        &mut self,
        request: VoteRequest<TypeConfig>,
        option: RPCOption,
    ) -> Result<VoteResponse<TypeConfig>, RPCError<TypeConfig>> {
        self.send(option, async {
            let target = self.target().await?;
            let request = Codec::round_trip(&request)?;
            let response = target.pre_vote(request).await.map_err(io::Error::other)?;
            Codec::round_trip(&response)
        })
        .await
    }

    async fn full_snapshot(
        &mut self,
        vote: VoteOf<TypeConfig>,
        snapshot: Snapshot,
        cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
        option: RPCOption,
    ) -> Result<SnapshotResponse<TypeConfig>, StreamingError<TypeConfig>> {
        let send = self.send(option, async {
            let target = self.target().await?;
            let (vote, meta, bytes) = Codec::round_trip(&(vote, snapshot.meta, snapshot.snapshot.into_inner()))?;
            let snapshot = Snapshot {
                meta,
                snapshot: std::io::Cursor::new(bytes),
            };
            let response = target.install_full_snapshot(vote, snapshot).await.map_err(io::Error::other)?;
            Codec::round_trip(&response)
        });
        tokio::select! {
            result = send => result.map_err(StreamingError::from),
            closed = cancel => Err(StreamingError::Closed(closed)),
        }
    }
}
