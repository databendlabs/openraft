use openraft::error::InstallSnapshotError;
use openraft::error::Unreachable;
use openraft::network::RPCOption;
use openraft::network::RaftNetwork;
use openraft::network::RaftNetworkFactory;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendEntriesResponse;
use openraft::raft::InstallSnapshotRequest;
use openraft::raft::InstallSnapshotResponse;
use openraft::raft::VoteRequest;
use openraft::raft::VoteResponse;
use openraft::BasicNode;
use openraft_proto::protobuf::internal_service_client::InternalServiceClient;
use openraft_proto::protobuf::RaftRequest;
use openraft_proto::protobuf::SnapshotChunkRequest;
use tonic::transport::Channel;

use crate::typ;
use crate::NodeId;
use crate::TypeConfig;

pub struct Network {}

// NOTE: This could be implemented also on `Arc<ExampleNetwork>`, but since it's empty, implemented
// directly.
impl RaftNetworkFactory<TypeConfig> for Network {
    type Network = NetworkConnection;

    async fn new_client(&mut self, target: NodeId, node: &BasicNode) -> Self::Network {
        NetworkConnection {
            owner: Network {},
            target,
            target_node: node.clone(),
        }
    }
}

#[allow(dead_code)]
pub struct NetworkConnection {
    owner: Network,
    target: NodeId,
    target_node: BasicNode,
}

#[allow(unused_variables)]
impl RaftNetwork<TypeConfig> for NetworkConnection {
    async fn append_entries(
        &mut self,
        req: AppendEntriesRequest<TypeConfig>,
        _option: RPCOption,
    ) -> Result<AppendEntriesResponse<TypeConfig>, typ::RPCError> {
        let server_addr = self.target_node.addr.clone();
        let channel = match Channel::builder(format!("http://{}", server_addr).parse().unwrap()).connect().await {
            Ok(channel) => channel,
            Err(e) => {
                return Err(openraft::error::RPCError::Unreachable(Unreachable::new(&e)));
            }
        };

        let mut client = InternalServiceClient::new(channel);
        let request = tonic::Request::new(RaftRequest {
            data: serde_json::to_string(&req).expect("Failed to convert to string"),
        });
        let response = client.append_entries(request).await.unwrap();
        Ok(serde_json::from_str(&response.into_inner().data).expect("Failed to deserialize from RaftReply"))
    }

    async fn install_snapshot(
        &mut self,
        req: InstallSnapshotRequest<TypeConfig>,
        _option: RPCOption,
    ) -> Result<InstallSnapshotResponse<TypeConfig>, typ::RPCError<InstallSnapshotError>> {
        let server_addr = self.target_node.addr.clone();
        let channel = match Channel::builder(format!("http://{}", server_addr).parse().unwrap()).connect().await {
            Ok(channel) => channel,
            Err(e) => {
                return Err(openraft::error::RPCError::Unreachable(Unreachable::new(&e)));
            }
        };

        let mut client = InternalServiceClient::new(channel);
        let request = tonic::Request::new(SnapshotChunkRequest {
            rpc_meta: String::from_utf8(req.data).unwrap(),
            ver: Default::default(),
            chunk: Default::default(),
        });
        let response = client.install_snapshot(request).await.unwrap();
        Ok(serde_json::from_str(&response.into_inner().data).expect("Failed to deserialize from RaftReply"))
    }

    async fn vote(
        &mut self,
        req: VoteRequest<TypeConfig>,
        _option: RPCOption,
    ) -> Result<VoteResponse<TypeConfig>, typ::RPCError> {
        let server_addr = self.target_node.addr.clone();
        let channel = match Channel::builder(format!("http://{}", server_addr).parse().unwrap()).connect().await {
            Ok(channel) => channel,
            Err(e) => {
                return Err(openraft::error::RPCError::Unreachable(Unreachable::new(&e)));
            }
        };

        let mut client = InternalServiceClient::new(channel);
        let request = tonic::Request::new(RaftRequest {
            data: serde_json::to_string(&req).expect("Failed to deserialize to string"),
        });
        let response = client.vote(request).await.unwrap();
        Ok(serde_json::from_str(&response.into_inner().data).expect("Failed to deserialize from RaftReply"))
    }
}
