use std::fmt::Display;

use openraft::BasicNode;
use openraft::RaftTypeConfig;
use openraft::errors::Infallible;
use openraft::errors::InstallSnapshotError;
use openraft::errors::NetworkError;
use openraft::errors::RPCError;
use openraft::errors::RaftError;
use openraft::errors::RemoteError;
use openraft::errors::Unreachable;
use openraft::network::RPCOption;
use openraft::network::RaftNetworkFactory;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendEntriesResponse;
use openraft::raft::InstallSnapshotRequest;
use openraft::raft::InstallSnapshotResponse;
use openraft::raft::VoteRequest;
use openraft::raft::VoteResponse;
use openraft_legacy::prelude::*;
use reqwest::Client;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::io::AsyncRead;
use tokio::io::AsyncSeek;
use tokio::io::AsyncWrite;

#[cfg(feature = "actix")]
pub mod actix {
    use actix_web::web;
    use actix_web::web::Data;
    use actix_web::web::Json;
    use openraft::Raft;
    use openraft::RaftTypeConfig;
    use openraft::errors::Infallible;
    use openraft::errors::InstallSnapshotError;
    use openraft::errors::decompose::DecomposeResult;
    use openraft::raft::AppendEntriesRequest;
    use openraft::raft::AppendEntriesResponse;
    use openraft::raft::InstallSnapshotRequest;
    use openraft::raft::InstallSnapshotResponse;
    use openraft::raft::VoteRequest;
    use openraft::raft::VoteResponse;
    use openraft_legacy::prelude::ChunkedSnapshotReceiver;
    use serde::Serialize;
    use serde::de::DeserializeOwned;
    use tokio::io::AsyncRead;
    use tokio::io::AsyncSeek;
    use tokio::io::AsyncWrite;

    pub fn configure<C, SM>(cfg: &mut web::ServiceConfig)
    where
        C: RaftTypeConfig + 'static,
        SM: 'static,
        C::SnapshotData: AsyncRead + AsyncWrite + AsyncSeek + Unpin,
        VoteRequest<C>: DeserializeOwned,
        AppendEntriesRequest<C>: DeserializeOwned,
        InstallSnapshotRequest<C>: DeserializeOwned,
        Result<VoteResponse<C>, Infallible>: Serialize,
        Result<AppendEntriesResponse<C>, Infallible>: Serialize,
        Result<InstallSnapshotResponse<C>, InstallSnapshotError>: Serialize,
    {
        cfg.route("/append", web::post().to(append::<C, SM>))
            .route("/snapshot", web::post().to(snapshot::<C, SM>))
            .route("/vote", web::post().to(vote::<C, SM>));
    }

    pub async fn append<C, SM>(
        raft: Data<Raft<C, SM>>,
        req: Json<AppendEntriesRequest<C>>,
    ) -> actix_web::Result<Json<Result<AppendEntriesResponse<C>, Infallible>>>
    where
        C: RaftTypeConfig,
    {
        let res = raft.append_entries(req.0).await.decompose().unwrap();
        Ok(Json(res))
    }

    pub async fn vote<C, SM>(
        raft: Data<Raft<C, SM>>,
        req: Json<VoteRequest<C>>,
    ) -> actix_web::Result<Json<Result<VoteResponse<C>, Infallible>>>
    where
        C: RaftTypeConfig,
    {
        let res = raft.vote(req.0).await.decompose().unwrap();
        Ok(Json(res))
    }

    pub async fn snapshot<C, SM>(
        raft: Data<Raft<C, SM>>,
        req: Json<InstallSnapshotRequest<C>>,
    ) -> actix_web::Result<Json<Result<InstallSnapshotResponse<C>, InstallSnapshotError>>>
    where
        C: RaftTypeConfig,
        C::SnapshotData: AsyncRead + AsyncWrite + AsyncSeek + Unpin,
    {
        let res = raft.install_snapshot(req.0).await.decompose().unwrap();
        Ok(Json(res))
    }
}

pub struct NetworkFactory {}

impl<C> RaftNetworkFactory<C> for NetworkFactory
where
    C: RaftTypeConfig<Node = BasicNode>,
    // RaftNetwork requires the snapshot to be a file-like object that can be seeked, read from, and written to.
    <C as RaftTypeConfig>::SnapshotData: AsyncRead + AsyncWrite + AsyncSeek + Unpin,
{
    type Network = Adapter<C, Network<C>>;

    #[tracing::instrument(level = "debug", skip_all)]
    async fn new_client(&mut self, target: C::NodeId, node: &BasicNode) -> Self::Network {
        let addr = node.addr.clone();

        let client = Client::builder().no_proxy().build().unwrap();

        Network { addr, client, target }.into_v2()
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
