use std::future::Future;
use std::io;
use std::time::Duration;

use openraft::AnyError;
use openraft::OptionalSend;
use openraft::RaftNetworkFactory;
use openraft::errors::NetworkError;
use openraft::errors::ReplicationClosed;
use openraft::errors::Unreachable;
use openraft::network::Backoff;
use openraft::network::RPCOption;
use openraft::network::v2::RaftNetworkV2;
use openraft::raft::TransferLeaderRequest;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

use crate::NodeId;
use crate::TypeConfig;
use crate::typ::*;

/// This is the network implementation for the raft network over TCP.
pub struct Network {}
impl Network {}

/// Implementation of the RaftNetworkFactory trait for creating new network connections.
impl RaftNetworkFactory<TypeConfig> for Network {
    type Network = NetworkConnection;

    #[tracing::instrument(level = "debug", skip_all)]
    async fn new_client(&mut self, _: NodeId, node: &Node) -> Self::Network {
        NetworkConnection::new(node.clone())
    }
}

#[derive(Debug, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
enum WireRequest {
    Vote(VoteRequest),
    AppendEntries(AppendEntriesRequest),
    FullSnapshot { vote: Vote, snapshot: WireSnapshot },
    TransferLeader(TransferLeaderRequest<TypeConfig>),
}

#[derive(Debug, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
struct WireSnapshot {
    meta: SnapshotMeta,
    data: Vec<u8>,
}

#[derive(Debug, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
enum WireResponse {
    Vote(Result<VoteResponse, String>),
    AppendEntries(Result<AppendEntriesResponse, String>),
    FullSnapshot(Result<SnapshotResponse, String>),
    TransferLeader(Result<(), String>),
}

/// Represents an active network connection to a remote Raft node.
pub struct NetworkConnection {
    target_node: Node,
}

impl NetworkConnection {
    /// Creates a new NetworkConnection to a target node.
    pub fn new(target_node: Node) -> Self {
        NetworkConnection { target_node }
    }

    /// Creates a TCP client connection to the target node.
    async fn make_client(&self) -> Result<TcpStream, RPCError> {
        let server_addr = &self.target_node.rpc_addr;
        let stream = TcpStream::connect(server_addr)
            .await
            .map_err(|e| RPCError::Unreachable(Unreachable::<TypeConfig>::new(&e)))?;

        Ok(stream)
    }

    fn rpc_not_implemented_error(&self, rpc_name: &str) -> RPCError {
        RPCError::Unreachable(Unreachable::<TypeConfig>::new(&AnyError::error(format!(
            "{rpc_name} RPC is not implemented for target {}",
            self.target_node.rpc_addr
        ))))
    }

    async fn send_rpc<'a>(&self, req: &WireRequest, buf: &'a mut [u8]) -> Result<WireResponse, RPCError> {
        let mut stream = self.make_client().await?;

        write(&mut stream, req).await.map_err(|e| RPCError::Network(NetworkError::<TypeConfig>::new(&e)))?;

        read(&mut stream, buf).await.map_err(|e| RPCError::Network(NetworkError::<TypeConfig>::new(&e)))
    }

    fn unexpected_response_error(&self, expected: &str) -> RPCError {
        RPCError::Network(NetworkError::<TypeConfig>::new(&AnyError::error(format!(
            "unexpected response type while waiting for {expected}",
        ))))
    }
}

impl RaftNetworkV2<TypeConfig> for NetworkConnection {
    async fn append_entries(
        &mut self,
        rpc: AppendEntriesRequest,
        _option: RPCOption,
    ) -> Result<AppendEntriesResponse, RPCError> {
        let req = WireRequest::AppendEntries(rpc);
        let mut buf = vec![0u8; u16::MAX as usize];
        let resp = self.send_rpc(&req, &mut buf).await?;

        match resp {
            WireResponse::AppendEntries(result) => {
                result.map_err(|e| RPCError::Network(NetworkError::<TypeConfig>::new(&AnyError::error(e))))
            }
            _ => Err(self.unexpected_response_error("append_entries")),
        }
    }

    async fn vote(&mut self, rpc: VoteRequest, _option: RPCOption) -> Result<VoteResponse, RPCError> {
        let req = WireRequest::Vote(rpc);
        let mut buf = vec![0u8; u16::MAX as usize];
        let resp = self.send_rpc(&req, &mut buf).await?;

        match resp {
            WireResponse::Vote(result) => {
                result.map_err(|e| RPCError::Network(NetworkError::<TypeConfig>::new(&AnyError::error(e))))
            }
            _ => Err(self.unexpected_response_error("vote")),
        }
    }

    async fn full_snapshot(
        &mut self,
        vote: Vote,
        snapshot: Snapshot,
        _cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
        _option: RPCOption,
    ) -> Result<SnapshotResponse, StreamingError> {
        let req = WireRequest::FullSnapshot {
            vote,
            snapshot: WireSnapshot {
                meta: snapshot.meta,
                data: snapshot.snapshot,
            },
        };

        let mut buf = vec![0u8; u16::MAX as usize];
        let resp = self.send_rpc(&req, &mut buf).await.map_err(StreamingError::from)?;

        match resp {
            WireResponse::FullSnapshot(result) => {
                result.map_err(|e| StreamingError::Network(NetworkError::<TypeConfig>::new(&AnyError::error(e))))
            }
            _ => Err(StreamingError::Network(NetworkError::<TypeConfig>::new(
                &AnyError::error("unexpected response type while waiting for full_snapshot"),
            ))),
        }
    }

    async fn transfer_leader(
        &mut self,
        req: TransferLeaderRequest<TypeConfig>,
        _option: RPCOption,
    ) -> Result<(), RPCError> {
        let mut buf = vec![0u8; u16::MAX as usize];
        let resp = self.send_rpc(&WireRequest::TransferLeader(req), &mut buf).await?;

        match resp {
            WireResponse::TransferLeader(result) => {
                result.map_err(|e| RPCError::Network(NetworkError::<TypeConfig>::new(&AnyError::error(e))))
            }
            _ => Err(self.rpc_not_implemented_error("transfer_leader")),
        }
    }

    fn backoff(&self) -> Backoff {
        Backoff::new(std::iter::repeat(Duration::from_millis(200)))
    }
}

pub async fn process_socket(mut socket: TcpStream, raft: Raft) -> io::Result<()> {
    // Largest TCP can be is u16 MAX
    let mut buf = vec![0; u16::MAX as usize];
    loop {
        let req: WireRequest = match read(&mut socket, &mut buf).await {
            Ok(req) => req,
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(e),
        };

        let resp = handle_request(&raft, &req).await;
        write(&mut socket, &resp).await?;
    }
}

async fn handle_request(raft: &Raft, req: &WireRequest) -> WireResponse {
    match req {
        WireRequest::Vote(req) => WireResponse::Vote(raft.vote(req.clone()).await.map_err(|e| e.to_string())),
        WireRequest::AppendEntries(req) => {
            WireResponse::AppendEntries(raft.append_entries(req.clone()).await.map_err(|e| e.to_string()))
        }
        WireRequest::FullSnapshot { vote, snapshot } => WireResponse::FullSnapshot(
            raft.install_full_snapshot(vote.clone(), Snapshot {
                meta: snapshot.meta.clone(),
                snapshot: snapshot.data.clone(),
            })
            .await
            .map_err(|e| e.to_string()),
        ),
        WireRequest::TransferLeader(req) => {
            WireResponse::TransferLeader(raft.handle_transfer_leader(req.clone()).await.map_err(|e| e.to_string()))
        }
    }
}

async fn write<T>(stream: &mut TcpStream, value: &T) -> io::Result<()>
where
    T: rkyv::Archive,
    T: for<'any> rkyv::Serialize<
            rkyv::api::high::HighSerializer<
                rkyv::util::AlignedVec,
                rkyv::ser::allocator::ArenaHandle<'any>,
                rkyv::rancor::Error,
            >,
        >,
{
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(value)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("failed to serialize frame: {e}")))?;
    let len = u32::try_from(bytes.len()).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "frame too large"))?;

    stream.write_u32(len).await?;
    stream.write_all(&bytes).await?;
    stream.flush().await
}

async fn read<T>(stream: &mut TcpStream, buf: &mut [u8]) -> io::Result<T>
where
    T: rkyv::Archive,
    // This comes by reccomendation of rust cargo check
    T::Archived: for<'arch> rkyv::bytecheck::CheckBytes<rkyv::api::high::HighValidator<'arch, rkyv::rancor::Error>>
        + rkyv::Deserialize<T, rkyv::api::high::HighDeserializer<rkyv::rancor::Error>>,
{
    let len = stream.read_u32().await? as usize;
    if len > buf.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame too large for read buffer",
        ));
    }
    stream.read_exact(&mut buf[..len]).await?;

    rkyv::from_bytes::<T, rkyv::rancor::Error>(&buf[..len]).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}
