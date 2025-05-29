use std::fmt::Display;

use openraft::error::Infallible;
use openraft::error::InstallSnapshotError;
use openraft::error::NetworkError;
use openraft::error::RPCError;
use openraft::error::RaftError;
use openraft::error::RemoteError;
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
use openraft::RaftTypeConfig;
use reqwest::Client;
use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::io::AsyncRead;
use tokio::io::AsyncSeek;
use tokio::io::AsyncWrite;

pub struct NetworkFactory {}

impl<C> RaftNetworkFactory<C> for NetworkFactory
where
    C: RaftTypeConfig<Node = BasicNode>,
    // RaftNetworkV2 is implemented automatically for RaftNetwork, but requires the following trait bounds.
    // In V2 network, the snapshot has no constraints, but RaftNetwork assumes a Snapshot is a file-like
    // object that can be seeked, read from, and written to.
    <C as RaftTypeConfig>::SnapshotData: AsyncRead + AsyncWrite + AsyncSeek + Unpin,
{
    type Network = Network<C>;

    #[tracing::instrument(level = "debug", skip_all)]
    async fn new_client(&mut self, target: C::NodeId, node: &BasicNode) -> Self::Network {
        let addr = node.addr.clone();

        let client = Client::builder().no_proxy().build().unwrap();

        Network { addr, client, target }
    }
}

pub struct Network<C>
where C: RaftTypeConfig
{
    addr: String,
    client: Client,
    target: C::NodeId,
}

impl<C> Network<C>
where C: RaftTypeConfig
{
    async fn request<Req, Resp, Err>(&mut self, uri: impl Display, req: Req) -> Result<Result<Resp, Err>, RPCError<C>>
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
                RPCError::Unreachable(Unreachable::new(&e))
            } else {
                RPCError::Network(NetworkError::new(&e))
            }
        })?;

        let res: Result<Resp, Err> = resp.json().await.map_err(|e| NetworkError::new(&e))?;
        // println!(
        //     "<<< network recv reply from {}: {}",
        //     url,
        //     serde_json::to_string_pretty(&res).unwrap()
        // );

        Ok(res)
    }
}

#[allow(clippy::blocks_in_conditions)]
impl<C> RaftNetwork<C> for Network<C>
where C: RaftTypeConfig
{
    #[tracing::instrument(level = "debug", skip_all, err(Debug))]
    async fn append_entries(
        &mut self,
        req: AppendEntriesRequest<C>,
        _option: RPCOption,
    ) -> Result<AppendEntriesResponse<C>, RPCError<C, RaftError<C>>> {
        let res = self.request::<_, _, Infallible>("append", req).await.map_err(RPCError::with_raft_error)?;
        Ok(res.unwrap())
    }

    #[tracing::instrument(level = "debug", skip_all, err(Debug))]
    async fn install_snapshot(
        &mut self,
        req: InstallSnapshotRequest<C>,
        _option: RPCOption,
    ) -> Result<InstallSnapshotResponse<C>, RPCError<C, RaftError<C, InstallSnapshotError>>> {
        let res = self.request("snapshot", req).await.map_err(RPCError::with_raft_error)?;
        match res {
            Ok(resp) => Ok(resp),
            Err(e) => Err(RPCError::RemoteError(RemoteError::new(
                self.target.clone(),
                RaftError::APIError(e),
            ))),
        }
    }

    #[tracing::instrument(level = "debug", skip_all, err(Debug))]
    async fn vote(
        &mut self,
        req: VoteRequest<C>,
        _option: RPCOption,
    ) -> Result<VoteResponse<C>, RPCError<C, RaftError<C>>> {
        let res = self.request::<_, _, Infallible>("vote", req).await.map_err(RPCError::with_raft_error)?;
        Ok(res.unwrap())
    }
}
