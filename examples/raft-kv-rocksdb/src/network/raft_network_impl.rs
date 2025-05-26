use std::fmt::Display;

use openraft::error::InstallSnapshotError;
use openraft::error::NetworkError;
use openraft::error::RemoteError;
use openraft::error::Unreachable;
use openraft::network::RPCOption;
use openraft::network::RaftNetwork;
use openraft::network::RaftNetworkFactory;
use reqwest::Client;
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::typ::*;
use crate::Node;
use crate::NodeId;
use crate::TypeConfig;

pub struct Network {}

// NOTE: This could be implemented also on `Arc<ExampleNetwork>`, but since it's empty, implemented
// directly.
impl RaftNetworkFactory<TypeConfig> for Network {
    type Network = NetworkConnection;

    #[tracing::instrument(level = "debug", skip_all)]
    async fn new_client(&mut self, target: NodeId, node: &Node) -> Self::Network {
        let addr = node.addr.clone();

        let client = Client::builder().no_proxy().build().unwrap();

        NetworkConnection { addr, client, target }
    }
}

pub struct NetworkConnection {
    addr: String,
    client: Client,
    target: NodeId,
}
impl NetworkConnection {
    async fn request<Req, Resp, Err>(&mut self, uri: impl Display, req: Req) -> Result<Result<Resp, Err>, RPCError>
    where
        Req: Serialize + 'static,
        Resp: Serialize + DeserializeOwned,
        Err: std::error::Error + Serialize + DeserializeOwned,
    {
        let url = format!("http://{}/{}", self.addr, uri);
        // println!(
        //     ">>> network send request to {}: {}",
        //     url,
        //     serde_json::to_string_pretty(&req).unwrap()
        // );

        let resp = self.client.post(url.clone()).json(&req).send().await.map_err(|e| {
            if e.is_connect() {
                // `Unreachable` informs the caller to backoff for a short while to avoid error log flush.
                return RPCError::Unreachable(Unreachable::new(&e));
            }
            RPCError::Network(NetworkError::new(&e))
        })?;

        let res: Result<Resp, Err> = resp.json().await.map_err(|e| RPCError::Network(NetworkError::new(&e)))?;
        // println!(
        //     "<<< network recv reply from {}: {}",
        //     url,
        //     serde_json::to_string_pretty(&res).unwrap()
        // );

        Ok(res)
    }
}

#[allow(clippy::blocks_in_conditions)]
impl RaftNetwork<TypeConfig> for NetworkConnection {
    #[tracing::instrument(level = "debug", skip_all, err(Debug))]
    async fn append_entries(
        &mut self,
        req: AppendEntriesRequest,
        _option: RPCOption,
    ) -> Result<AppendEntriesResponse, RPCError<RaftError>> {
        let res = self.request::<_, _, Infallible>("append", req).await.map_err(RPCError::with_raft_error)?;
        Ok(res.unwrap())
    }

    #[tracing::instrument(level = "debug", skip_all, err(Debug))]
    async fn install_snapshot(
        &mut self,
        req: InstallSnapshotRequest,
        _option: RPCOption,
    ) -> Result<InstallSnapshotResponse, RPCError<RaftError<InstallSnapshotError>>> {
        let res = self.request("snapshot", req).await.map_err(RPCError::with_raft_error)?;
        match res {
            Ok(resp) => Ok(resp),
            Err(e) => Err(RPCError::RemoteError(RemoteError::new(
                self.target,
                RaftError::APIError(e),
            ))),
        }
    }

    #[tracing::instrument(level = "debug", skip_all, err(Debug))]
    async fn vote(&mut self, req: VoteRequest, _option: RPCOption) -> Result<VoteResponse, RPCError<RaftError>> {
        let res = self.request::<_, _, Infallible>("vote", req).await.map_err(RPCError::with_raft_error)?;
        Ok(res.unwrap())
    }
}
