#![allow(clippy::uninlined_format_args)]
#![deny(unused_qualifications)]

pub mod app;
pub mod network;
pub mod store;

pub type NodeId = u64;

use openraft_proto::TypeConfig;

pub type LogStore = store::LogStore;
pub type StateMachineStore = store::StateMachineStore;
pub type StateMachineStoreWrapper = store::StateMachineStoreWrapper;
pub type Raft = openraft::Raft<TypeConfig>;

pub mod typ {

    use crate::TypeConfig;

    pub type RaftError<E = openraft::error::Infallible> = openraft::error::RaftError<TypeConfig, E>;
    pub type RPCError<E = openraft::error::Infallible> = openraft::error::RPCError<TypeConfig, RaftError<E>>;

    pub type ClientWriteError = openraft::error::ClientWriteError<TypeConfig>;
    pub type CheckIsLeaderError = openraft::error::CheckIsLeaderError<TypeConfig>;
    pub type ForwardToLeader = openraft::error::ForwardToLeader<TypeConfig>;
    pub type InitializeError = openraft::error::InitializeError<TypeConfig>;

    pub type ClientWriteResponse = openraft::raft::ClientWriteResponse<TypeConfig>;
}

#[cfg(test)]
mod tests {
    use openraft_proto::protobuf::InitRequest;
    use tonic::transport::Channel;
    use tracing::info;

    #[tokio::test]
    async fn test_init_rpc() -> Result<(), Box<dyn std::error::Error>> {
        tracing_subscriber::fmt().with_max_level(tracing::Level::INFO).init();

        let server_addr = "[::1]:50051";
        // Create client
        let channel = Channel::builder(format!("http://{}", server_addr).parse()?).connect().await?;
        info!("Client connected");

        let mut client = openraft_proto::protobuf::management_service_client::ManagementServiceClient::new(channel);

        // Make RPC call
        let request = tonic::Request::new(InitRequest {
            nodes: vec![openraft_proto::protobuf::BasicNode {
                id: 1,
                addr: "hello".to_string(),
            }],
        });

        info!("Sending request: {:?}", request);
        let response = client.init(request).await?;
        info!("Response received: {:?}", response);

        Ok(())
    }
}
