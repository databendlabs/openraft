//! Turmoil-based network implementation for OpenRaft.

use std::future::Future;
use std::io;
use std::io::Cursor;
use std::sync::Arc;

use openraft::async_runtime::WatchReceiver;
use openraft::error::RPCError;
use openraft::error::ReplicationClosed;
use openraft::error::StreamingError;
use openraft::error::Unreachable;
use openraft::network::RPCOption;
use openraft::network::RaftNetworkFactory;
use openraft::network::v2::RaftNetworkV2;
use serde::Deserialize;
use serde::Serialize;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use turmoil::net::TcpStream;

use crate::client::ClientWriteRequest;
use crate::client::ClientWriteResponse;
use crate::typ::*;

/// RPC message types for the wire protocol.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[repr(u8)]
pub enum RpcType {
    AppendEntries = 1,
    Vote = 2,
    InstallSnapshot = 3,
    ClientWrite = 100,
    /// Query node state for invariant checking.
    StateQuery = 101,
}

/// Snapshot request for wire transmission.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotRequest {
    pub vote: Vote,
    pub meta: SnapshotMeta,
    pub data: Vec<u8>,
}

/// State query request for invariant checking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateQueryRequest;

/// State query response containing node state for invariant checking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateQueryResponse {
    pub node_id: NodeId,
    pub metrics: RaftMetrics,
}

/// Network factory that creates turmoil-based connections.
#[derive(Clone)]
pub struct TurmoilNetwork;

impl RaftNetworkFactory<TypeConfig> for TurmoilNetwork {
    type Network = TurmoilConnection;

    async fn new_client(&mut self, target: NodeId, node: &Node) -> Self::Network {
        TurmoilConnection {
            target,
            addr: node.addr.clone(),
        }
    }
}

/// A single connection to a target node using turmoil's simulated TCP.
pub struct TurmoilConnection {
    target: NodeId,
    addr: String,
}

impl TurmoilConnection {
    /// Send an RPC and receive a response.
    async fn send_rpc<Req, Resp>(&self, rpc_type: RpcType, req: &Req) -> Result<Resp, RPCError<TypeConfig>>
    where
        Req: Serialize,
        Resp: for<'de> Deserialize<'de>,
    {
        let mut stream = TcpStream::connect(&self.addr).await.map_err(|e| {
            tracing::warn!(target = self.target, error = %e, "Failed to connect");
            RPCError::Unreachable(Unreachable::new(&e))
        })?;

        // Serialize request
        let payload = bincode::serialize(req).map_err(|e| RPCError::Unreachable(Unreachable::new(&e)))?;

        // Write: [rpc_type: u8][len: u32][payload]
        stream.write_u8(rpc_type as u8).await.map_err(|e| RPCError::Unreachable(Unreachable::new(&e)))?;
        stream
            .write_u32(payload.len() as u32)
            .await
            .map_err(|e| RPCError::Unreachable(Unreachable::new(&e)))?;
        stream.write_all(&payload).await.map_err(|e| RPCError::Unreachable(Unreachable::new(&e)))?;

        // Read response: [len: u32][payload]
        let resp_len = stream.read_u32().await.map_err(|e| RPCError::Unreachable(Unreachable::new(&e)))?;
        let mut resp_buf = vec![0u8; resp_len as usize];
        stream.read_exact(&mut resp_buf).await.map_err(|e| RPCError::Unreachable(Unreachable::new(&e)))?;

        bincode::deserialize(&resp_buf).map_err(|e| RPCError::Unreachable(Unreachable::new(&e)))
    }
}

impl RaftNetworkV2<TypeConfig> for TurmoilConnection {
    async fn append_entries(
        &mut self,
        rpc: AppendEntriesRequest,
        _option: RPCOption,
    ) -> Result<AppendEntriesResponse, RPCError<TypeConfig>> {
        self.send_rpc(RpcType::AppendEntries, &rpc).await
    }

    async fn vote(&mut self, rpc: VoteRequest, _option: RPCOption) -> Result<VoteResponse, RPCError<TypeConfig>> {
        self.send_rpc(RpcType::Vote, &rpc).await
    }

    async fn full_snapshot(
        &mut self,
        vote: Vote,
        snapshot: Snapshot,
        _cancel: impl Future<Output = ReplicationClosed> + Send + 'static,
        _option: RPCOption,
    ) -> Result<SnapshotResponse, StreamingError<TypeConfig>> {
        // For simplicity, we send the entire snapshot in one message.
        let req = SnapshotRequest {
            vote,
            meta: snapshot.meta,
            data: snapshot.snapshot.into_inner(),
        };

        let resp: SnapshotResponse = self.send_rpc(RpcType::InstallSnapshot, &req).await.map_err(|e| match e {
            RPCError::Unreachable(u) => StreamingError::Unreachable(u),
            RPCError::Network(n) => StreamingError::Network(n),
            RPCError::Timeout(t) => StreamingError::Timeout(t),
            // No RemoteError variant in StreamingError, map to Unreachable
            RPCError::RemoteError(r) => StreamingError::Unreachable(Unreachable::new(&r)),
        })?;

        Ok(resp)
    }
}

fn deser<T>(payload: &[u8]) -> io::Result<T>
where T: for<'de> Deserialize<'de> {
    bincode::deserialize(payload).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

fn ser<T>(resp: &T) -> io::Result<Vec<u8>>
where T: Serialize {
    bincode::serialize(resp).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Handle incoming RPCs on a node.
pub async fn handle_rpc(raft: Arc<Raft>, mut stream: TcpStream) -> io::Result<()> {
    let rpc_type = stream.read_u8().await?;

    let len = stream.read_u32().await?;
    let mut payload = vec![0u8; len as usize];
    stream.read_exact(&mut payload).await?;

    let response: Vec<u8> = match rpc_type {
        x if x == RpcType::AppendEntries as u8 => {
            let req: AppendEntriesRequest = deser(&payload)?;
            ser(&raft.append_entries(req).await.map_err(io::Error::other)?)?
        }
        x if x == RpcType::Vote as u8 => {
            let req: VoteRequest = deser(&payload)?;
            ser(&raft.vote(req).await.map_err(io::Error::other)?)?
        }
        x if x == RpcType::InstallSnapshot as u8 => {
            let req: SnapshotRequest = deser(&payload)?;
            let snapshot = Snapshot {
                meta: req.meta,
                snapshot: Cursor::new(req.data),
            };
            ser(&raft.install_full_snapshot(req.vote, snapshot).await.map_err(io::Error::other)?)?
        }
        x if x == RpcType::ClientWrite as u8 => {
            let req: ClientWriteRequest = deser(&payload)?;
            let resp = match raft.client_write(req.request).await {
                Ok(client_resp) => ClientWriteResponse::Success(client_resp.response().clone()),
                Err(e) => match e.forward_to_leader() {
                    Some(forward) => ClientWriteResponse::NotLeader {
                        leader_id: forward.leader_id,
                    },
                    None => ClientWriteResponse::Error(e.to_string()),
                },
            };
            ser(&resp)?
        }
        x if x == RpcType::StateQuery as u8 => {
            let _req: StateQueryRequest = deser(&payload)?;
            let metrics = raft.metrics().borrow_watched().clone();
            let resp = StateQueryResponse {
                node_id: metrics.id,
                metrics,
            };
            ser(&resp)?
        }
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Unknown RPC type: {}", rpc_type),
            ));
        }
    };

    stream.write_u32(response.len() as u32).await?;
    stream.write_all(&response).await?;

    Ok(())
}
