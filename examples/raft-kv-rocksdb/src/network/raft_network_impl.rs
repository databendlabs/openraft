use std::any::Any;
use std::fmt::Display;

use async_trait::async_trait;
use openraft::error::AppendEntriesError;
use openraft::error::InstallSnapshotError;
use openraft::error::NetworkError;
use openraft::error::NodeNotFound;
use openraft::error::RPCError;
use openraft::error::RemoteError;
use openraft::error::VoteError;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendEntriesResponse;
use openraft::raft::InstallSnapshotRequest;
use openraft::raft::InstallSnapshotResponse;
use openraft::raft::VoteRequest;
use openraft::raft::VoteResponse;
use openraft::AnyError;
use openraft::Node;
use openraft::RaftNetwork;
use openraft::RaftNetworkFactory;
use serde::de::DeserializeOwned;
use serde::Serialize;
use toy_rpc::pubsub::AckModeNone;
use toy_rpc::Client;

use super::raft::RaftClientStub;
use crate::ExampleNodeId;
use crate::ExampleTypeConfig;

pub struct ExampleNetwork {}

impl ExampleNetwork {
    pub async fn send_rpc<Req, Resp, Err>(
        &self,
        target: ExampleNodeId,
        target_node: Option<&Node>,
        uri: &str,
        req: Req,
    ) -> Result<Resp, RPCError<ExampleNodeId, Err>>
    where
        Req: Serialize,
        Err: std::error::Error + DeserializeOwned,
        Resp: DeserializeOwned,
    {
        let addr = target_node.map(|x| &x.addr).ok_or(RPCError::NodeNotFound(NodeNotFound {
            node_id: target,
            source: AnyError::default(),
        }))?;

        let url = format!("http://{}/{}", addr, uri);
        let client = reqwest::Client::new();

        let resp = client.post(url).json(&req).send().await.map_err(|e| RPCError::Network(NetworkError::new(&e)))?;

        let res: Result<Resp, Err> = resp.json().await.map_err(|e| RPCError::Network(NetworkError::new(&e)))?;

        res.map_err(|e| RPCError::RemoteError(RemoteError::new(target, e)))
    }
}

// NOTE: This could be implemented also on `Arc<ExampleNetwork>`, but since it's empty, implemented directly.
#[async_trait]
impl RaftNetworkFactory<ExampleTypeConfig> for ExampleNetwork {
    type Network = ExampleNetworkConnection;

    async fn connect(&mut self, target: ExampleNodeId, node: Option<&Node>) -> Self::Network {
        dbg!(&node);
        let addr = node.map(|x| format!("ws://{}", x.addr)).unwrap();
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
    ) -> Result<&toy_rpc::client::Client<AckModeNone>, RPCError<ExampleNodeId, E>> {
        if self.client.is_none() {
            self.client = Client::dial_websocket(&self.addr).await.ok();
        }
        self.client.as_ref().ok_or(RPCError::Network(NetworkError::from(AnyError::default())))
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
) -> RPCError<ExampleNodeId, E> {
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
    ) -> Result<AppendEntriesResponse<ExampleNodeId>, RPCError<ExampleNodeId, AppendEntriesError<ExampleNodeId>>> {
        self.c().await?.raft().append(req).await.map_err(|e| to_error(e, self.target))
    }

    async fn send_install_snapshot(
        &mut self,
        req: InstallSnapshotRequest<ExampleTypeConfig>,
    ) -> Result<InstallSnapshotResponse<ExampleNodeId>, RPCError<ExampleNodeId, InstallSnapshotError<ExampleNodeId>>>
    {
        self.c().await?.raft().snapshot(req).await.map_err(|e| to_error(e, self.target))
    }

    async fn send_vote(
        &mut self,
        req: VoteRequest<ExampleNodeId>,
    ) -> Result<VoteResponse<ExampleNodeId>, RPCError<ExampleNodeId, VoteError<ExampleNodeId>>> {
        self.c().await?.raft().vote(req).await.map_err(|e| to_error(e, self.target))
    }
}
