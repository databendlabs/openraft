use std::any::Any;
use std::fmt::Display;

use async_trait::async_trait;
use openraft::error::InstallSnapshotError;
use openraft::error::NetworkError;
use openraft::error::RPCError;
use openraft::error::RaftError;
use openraft::error::RemoteError;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendEntriesResponse;
use openraft::raft::InstallSnapshotRequest;
use openraft::raft::InstallSnapshotResponse;
use openraft::raft::VoteRequest;
use openraft::raft::VoteResponse;
use openraft::AnyError;
use openraft::RaftNetwork;
use openraft::RaftNetworkFactory;
use serde::de::DeserializeOwned;
use toy_rpc::pubsub::AckModeNone;
use toy_rpc::Client;

use super::raft::RaftClientStub;
use crate::ExampleNode;
use crate::ExampleNodeId;
use crate::ExampleTypeConfig;

pub struct ExampleNetwork {}

// NOTE: This could be implemented also on `Arc<ExampleNetwork>`, but since it's empty, implemented
// directly.
#[async_trait]
impl RaftNetworkFactory<ExampleTypeConfig> for ExampleNetwork {
    type Network = ExampleNetworkConnection;

    async fn new_client(&mut self, target: ExampleNodeId, node: &ExampleNode) -> Self::Network {
        let addr = format!("ws://{}", node.rpc_addr);
        let client = Client::dial_websocket(&addr).await.ok();
        ExampleNetworkConnection { addr, client, target }
    }
}

pub struct ExampleNetworkConnection {
    addr: String,
    client: Option<toy_rpc::client::Client<AckModeNone>>,
    target: ExampleNodeId,
}
impl ExampleNetworkConnection {
    async fn c<E: std::error::Error + DeserializeOwned>(
        &mut self,
    ) -> Result<&toy_rpc::client::Client<AckModeNone>, RPCError<ExampleNodeId, ExampleNode, E>> {
        if self.client.is_none() {
            self.client = Client::dial_websocket(&self.addr).await.ok();
        }
        self.client.as_ref().ok_or_else(|| RPCError::Network(NetworkError::from(AnyError::default())))
    }
}

#[derive(Debug)]
struct ErrWrap(Box<dyn std::error::Error>);

impl Display for ErrWrap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for ErrWrap {}

fn to_error<E: std::error::Error + 'static + Clone>(
    e: toy_rpc::Error,
    target: ExampleNodeId,
) -> RPCError<ExampleNodeId, ExampleNode, E> {
    match e {
        toy_rpc::Error::IoError(e) => RPCError::Network(NetworkError::new(&e)),
        toy_rpc::Error::ParseError(e) => RPCError::Network(NetworkError::new(&ErrWrap(e))),
        toy_rpc::Error::Internal(e) => {
            let any: &dyn Any = &e;
            let error: &E = any.downcast_ref().unwrap();
            RPCError::RemoteError(RemoteError::new(target, error.clone()))
        }
        e @ (toy_rpc::Error::InvalidArgument
        | toy_rpc::Error::ServiceNotFound
        | toy_rpc::Error::MethodNotFound
        | toy_rpc::Error::ExecutionError(_)
        | toy_rpc::Error::Canceled(_)
        | toy_rpc::Error::Timeout(_)
        | toy_rpc::Error::MaxRetriesReached(_)) => RPCError::Network(NetworkError::new(&e)),
    }
}

#[async_trait]
impl RaftNetwork<ExampleTypeConfig> for ExampleNetworkConnection {
    async fn send_append_entries(
        &mut self,
        req: AppendEntriesRequest<ExampleTypeConfig>,
    ) -> Result<AppendEntriesResponse<ExampleNodeId>, RPCError<ExampleNodeId, ExampleNode, RaftError<ExampleNodeId>>>
    {
        self.c().await?.raft().append(req).await.map_err(|e| to_error(e, self.target))
    }

    async fn send_install_snapshot(
        &mut self,
        req: InstallSnapshotRequest<ExampleTypeConfig>,
    ) -> Result<
        InstallSnapshotResponse<ExampleNodeId>,
        RPCError<ExampleNodeId, ExampleNode, RaftError<ExampleNodeId, InstallSnapshotError>>,
    > {
        self.c().await?.raft().snapshot(req).await.map_err(|e| to_error(e, self.target))
    }

    async fn send_vote(
        &mut self,
        req: VoteRequest<ExampleNodeId>,
    ) -> Result<VoteResponse<ExampleNodeId>, RPCError<ExampleNodeId, ExampleNode, RaftError<ExampleNodeId>>> {
        self.c().await?.raft().vote(req).await.map_err(|e| to_error(e, self.target))
    }
}
