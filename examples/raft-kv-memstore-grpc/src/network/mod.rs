use bincode::deserialize;
use bincode::serialize;
use openraft::error::InstallSnapshotError;
use openraft::error::NetworkError;
use openraft::error::RPCError;
use openraft::error::RaftError;
use openraft::network::RPCOption;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendEntriesResponse;
use openraft::raft::InstallSnapshotRequest;
use openraft::raft::InstallSnapshotResponse;
use openraft::raft::VoteRequest;
use openraft::raft::VoteResponse;
use openraft::RaftNetwork;
use openraft::RaftNetworkFactory;
use tonic::transport::Channel;

use crate::protobuf::internal_service_client::InternalServiceClient;
use crate::protobuf::RaftRequestBytes;
use crate::Node;
use crate::NodeId;
use crate::TypeConfig;

/// Network implementation for gRPC-based Raft communication.
/// Provides the networking layer for Raft nodes to communicate with each other.
pub struct Network {}

type RaftServiceClient = InternalServiceClient<Channel>;

impl Network {}

/// Implementation of the RaftNetworkFactory trait for creating new network connections.
/// This factory creates gRPC client connections to other Raft nodes.
impl RaftNetworkFactory<TypeConfig> for Network {
    type Network = NetworkConnection;

    #[tracing::instrument(level = "debug", skip_all)]
    async fn new_client(&mut self, _: NodeId, node: &Node) -> Self::Network {
        let channel = Channel::builder(format!("http://{}", node.rpc_addr).parse().unwrap())
            .connect()
            .await
            .unwrap();
        NetworkConnection::new(InternalServiceClient::new(channel))
    }
}

/// Represents an active network connection to a remote Raft node.
/// Handles serialization and deserialization of Raft messages over gRPC.
pub struct NetworkConnection {
    client: RaftServiceClient,
}

impl NetworkConnection {
    /// Creates a new NetworkConnection with the provided gRPC client.
    pub fn new(client: RaftServiceClient) -> Self {
        NetworkConnection { client }
    }
}

/// Implementation of RaftNetwork trait for handling Raft protocol communications.
#[allow(clippy::blocks_in_conditions)]
impl RaftNetwork<TypeConfig> for NetworkConnection {
    async fn append_entries(
        &mut self,
        req: AppendEntriesRequest<TypeConfig>,
        _option: RPCOption,
    ) -> Result<AppendEntriesResponse<TypeConfig>, RPCError<TypeConfig, RaftError<TypeConfig>>> {
        let value = serialize(&req).map_err(|e| RPCError::Network(NetworkError::new(&e)))?;
        let request = RaftRequestBytes { value };
        let response = self.client.append(request).await.map_err(|e| RPCError::Network(NetworkError::new(&e)))?;
        let message = response.into_inner();
        let result = deserialize(&message.value).map_err(|e| RPCError::Network(NetworkError::new(&e)))?;
        Ok(result)
    }

    async fn install_snapshot(
        &mut self,
        req: InstallSnapshotRequest<TypeConfig>,
        _option: RPCOption,
    ) -> Result<InstallSnapshotResponse<TypeConfig>, RPCError<TypeConfig, RaftError<TypeConfig, InstallSnapshotError>>>
    {
        let value = serialize(&req).map_err(|e| RPCError::Network(NetworkError::new(&e)))?;
        let request = RaftRequestBytes { value };
        let response = self.client.snapshot(request).await.map_err(|e| RPCError::Network(NetworkError::new(&e)))?;
        let message = response.into_inner();
        let result = deserialize(&message.value).map_err(|e| RPCError::Network(NetworkError::new(&e)))?;
        Ok(result)
    }

    async fn vote(
        &mut self,
        req: VoteRequest<TypeConfig>,
        _option: RPCOption,
    ) -> Result<VoteResponse<TypeConfig>, RPCError<TypeConfig, RaftError<TypeConfig>>> {
        let value = serialize(&req).map_err(|e| RPCError::Network(NetworkError::new(&e)))?;
        let request = RaftRequestBytes { value };
        let response = self.client.vote(request).await.map_err(|e| RPCError::Network(NetworkError::new(&e)))?;
        let message = response.into_inner();
        let result = deserialize(&message.value).map_err(|e| RPCError::Network(NetworkError::new(&e)))?;
        Ok(result)
    }
}
